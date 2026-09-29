//! Host counters for the SYSTEM panel: CPU, memory, disk, network, and the
//! Pi thermal / throttle files when they exist. Battery percent is left unknown.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use sysinfo::{Components, Disks, Networks, System};

use crate::core::hardware::{
    ComputeStatus, NetworkStatus, PowerSource, PowerStatus, Readiness, StorageStatus,
};

pub struct HostSnap {
    pub compute: ComputeStatus,
    pub power: PowerStatus,
    pub network: NetworkStatus,
    pub storage: StorageStatus,
}

pub struct HostAdapter {
    snap: Arc<Mutex<HostSnap>>,
}

impl HostAdapter {
    pub fn start() -> Self {
        let snap = Arc::new(Mutex::new(HostSnap {
            compute: ComputeStatus {
                readiness: Readiness::Ready,
                model: "—".into(),
                cpu_percent: 0.0,
                temp_c: 0.0,
                mem_used_mb: 0,
                mem_total_mb: 0,
            },
            power: PowerStatus::default(),
            network: NetworkStatus {
                readiness: Readiness::NotPresent,
                interfaces: Vec::new(),
            },
            storage: StorageStatus {
                readiness: Readiness::NotPresent,
                root_used_percent: 0.0,
                data_free_gb: 0.0,
            },
        }));
        let worker = snap.clone();
        if let Err(err) = std::thread::Builder::new()
            .name("rvn-host".into())
            .spawn(move || host_loop(worker))
        {
            tracing::warn!("host thread: {err}");
        }
        Self { snap }
    }

    pub fn snapshot(&self) -> HostSnap {
        let snap = self.snap.lock().unwrap();
        HostSnap {
            compute: snap.compute.clone(),
            power: snap.power.clone(),
            network: snap.network.clone(),
            storage: snap.storage.clone(),
        }
    }
}

fn host_loop(slot: Arc<Mutex<HostSnap>>) {
    let mut sys = System::new();
    sys.refresh_cpu_usage();
    std::thread::sleep(Duration::from_millis(250));
    loop {
        sys.refresh_cpu_usage();
        sys.refresh_memory();
        let next = sample(&sys);
        *slot.lock().unwrap() = next;
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn sample(sys: &System) -> HostSnap {
    let model = machine_model(sys);
    let temp_c = cpu_temp_c();
    let throttled = throttled_now(&model);
    let mem_total = (sys.total_memory() / 1024 / 1024) as u32;
    let mem_used = (sys.used_memory() / 1024 / 1024) as u32;
    HostSnap {
        compute: ComputeStatus {
            readiness: Readiness::Ready,
            model,
            cpu_percent: sys.global_cpu_usage(),
            temp_c,
            mem_used_mb: mem_used,
            mem_total_mb: mem_total.max(1),
        },
        power: PowerStatus {
            readiness: if throttled {
                Readiness::Degraded
            } else {
                Readiness::NotPresent
            },
            source: PowerSource::Unknown,
            percent: None,
            voltage: None,
            throttled,
        },
        network: network_snapshot(),
        storage: storage_snapshot(),
    }
}

fn machine_model(sys: &System) -> String {
    if let Some(model) = device_tree_model() {
        return model;
    }
    if let Some(cpu) = sys.cpus().first() {
        let brand = cpu.brand().trim();
        if !brand.is_empty() {
            return brand.to_string();
        }
    }
    System::host_name().unwrap_or_else(|| "host".into())
}

fn device_tree_model() -> Option<String> {
    let bytes = std::fs::read("/proc/device-tree/model").ok()?;
    let model = String::from_utf8_lossy(&bytes)
        .trim_matches(char::from(0))
        .trim()
        .to_string();
    if model.is_empty() {
        None
    } else {
        Some(model)
    }
}

fn cpu_temp_c() -> f32 {
    if let Some(temp) = linux_thermal() {
        return temp;
    }
    let components = Components::new_with_refreshed_list();
    let mut best = 0.0_f32;
    for component in &components {
        if let Some(temp) = component.temperature() {
            if temp > best {
                best = temp;
            }
        }
    }
    best
}

fn linux_thermal() -> Option<f32> {
    let entries = std::fs::read_dir("/sys/class/thermal").ok()?;
    let mut fallback = None;
    for entry in entries.flatten() {
        let path = entry.path();
        let kind = std::fs::read_to_string(path.join("type")).unwrap_or_default();
        let Ok(raw) = std::fs::read_to_string(path.join("temp")) else {
            continue;
        };
        let Ok(milli) = raw.trim().parse::<f32>() else {
            continue;
        };
        let temp = milli / 1000.0;
        if kind.contains("cpu") || kind.contains("pkg") {
            return Some(temp);
        }
        fallback.get_or_insert(temp);
    }
    fallback
}

fn throttled_now(model: &str) -> bool {
    if !model.to_ascii_lowercase().contains("raspberry") {
        return false;
    }
    let Ok(output) = std::process::Command::new("vcgencmd")
        .arg("get_throttled")
        .output()
    else {
        return false;
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let Some(hex) = text.split('=').nth(1) else {
        return false;
    };
    let hex = hex.trim().trim_start_matches("0x");
    let Ok(bits) = u32::from_str_radix(hex, 16) else {
        return false;
    };
    bits & 0b0111 != 0
}

fn storage_snapshot() -> StorageStatus {
    let disks = Disks::new_with_refreshed_list();
    let mut chosen: Option<(u64, u64, bool)> = None;
    for disk in disks.list() {
        let total = disk.total_space();
        let avail = disk.available_space();
        if total == 0 {
            continue;
        }
        let mount = disk.mount_point();
        let is_root = mount == std::path::Path::new("/")
            || mount == std::path::Path::new("C:\\")
            || mount == std::path::Path::new("C:/");
        let replace = match chosen {
            None => true,
            Some((_, _, was_root)) if is_root && !was_root => true,
            Some((prev, _, was_root)) if !was_root && !is_root && total > prev => true,
            _ => false,
        };
        if replace {
            chosen = Some((total, avail, is_root));
        }
    }
    match chosen {
        Some((total, avail, _)) => {
            let used = total.saturating_sub(avail);
            StorageStatus {
                readiness: Readiness::Ready,
                root_used_percent: (used as f32 / total as f32) * 100.0,
                data_free_gb: avail as f32 / 1_000_000_000.0,
            }
        }
        None => StorageStatus {
            readiness: Readiness::NotPresent,
            root_used_percent: 0.0,
            data_free_gb: 0.0,
        },
    }
}

fn network_snapshot() -> NetworkStatus {
    let networks = Networks::new_with_refreshed_list();
    let mut interfaces = Vec::new();
    for (name, _) in &networks {
        let label = name.to_string();
        if !label.is_empty() {
            interfaces.push(label);
        }
    }
    interfaces.sort();
    interfaces.dedup();
    let readiness = if interfaces.is_empty() {
        Readiness::NotPresent
    } else {
        Readiness::Ready
    };
    NetworkStatus {
        readiness,
        interfaces,
    }
}
