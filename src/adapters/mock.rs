//! Mock hardware adapter — simulates devices coming online.
//! Used when RVN_MOCK=1, and for machines without the real peripherals.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::core::bands::{mock_power, power_peaks, scan_plan};
use crate::core::hardware::*;
use crate::surfaces::spectrum_bins;

use super::ports::attachments;
use super::{MeshInbound, Platform};

const PEERS: [&str; 3] = ["!b2c3d4e5", "!c3d4e5f6", "!d4e5f6a7"];
const LINES: [&str; 4] = [
    "ridge clear, holding",
    "wx good, continuing",
    "standing by",
    "grid check ok",
];

enum ScanPhase {
    Idle,
    Running {
        started: Instant,
        label: String,
        points: Vec<f32>,
    },
    Done {
        label: String,
        hits: Vec<ScanHit>,
    },
}

pub struct MockAdapter {
    start: Instant,
    radio_freq_mhz: Mutex<f32>,
    radio_streaming: Mutex<bool>,
    inbox_step: Mutex<u32>,
    last_rx: Mutex<Option<Instant>>,
    scan: Mutex<ScanPhase>,
}

impl MockAdapter {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
            radio_freq_mhz: Mutex::new(433.0),
            radio_streaming: Mutex::new(false),
            inbox_step: Mutex::new(0),
            last_rx: Mutex::new(None),
            scan: Mutex::new(ScanPhase::Idle),
        }
    }

    pub fn elapsed_secs(&self) -> f32 {
        self.start.elapsed().as_secs_f32()
    }
}

struct ScanOverlay {
    scanning: bool,
    progress: f32,
    label: String,
    hits: Vec<ScanHit>,
}

fn sweep_shown(elapsed: Duration, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    ((elapsed.as_secs_f32()) * 45.0).floor() as usize
}

fn hits_for(points: &[f32]) -> Vec<ScanHit> {
    let samples: Vec<(f32, f32)> = points.iter().map(|mhz| (*mhz, mock_power(*mhz))).collect();
    power_peaks(&samples)
}

impl MockAdapter {
    fn scan_overlay(&self) -> ScanOverlay {
        let mut phase = self.scan.lock().unwrap();
        let running = match &*phase {
            ScanPhase::Running { started, label, points } => {
                Some((started.elapsed(), label.clone(), points.clone()))
            }
            _ => None,
        };
        if let Some((elapsed, label, points)) = running {
            let shown = sweep_shown(elapsed, points.len()).min(points.len());
            let hits = hits_for(&points[..shown]);
            let progress = if points.is_empty() {
                1.0
            } else {
                shown as f32 / points.len() as f32
            };
            if !points.is_empty() && shown >= points.len() {
                *phase = ScanPhase::Done {
                    label: label.clone(),
                    hits: hits.clone(),
                };
                return ScanOverlay {
                    scanning: false,
                    progress: 1.0,
                    label,
                    hits,
                };
            }
            return ScanOverlay {
                scanning: true,
                progress,
                label: format!("{label} · {:.0}%", progress * 100.0),
                hits,
            };
        }
        match &*phase {
            ScanPhase::Done { label, hits } => ScanOverlay {
                scanning: false,
                progress: 1.0,
                label: label.clone(),
                hits: hits.clone(),
            },
            _ => ScanOverlay {
                scanning: false,
                progress: 0.0,
                label: String::new(),
                hits: Vec::new(),
            },
        }
    }
}

impl Default for MockAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl Platform for MockAdapter {
    fn poll(&self) -> PlatformStatus {
        let elapsed = self.start.elapsed();
        let freq = *self.radio_freq_mhz.lock().unwrap();
        let streaming = *self.radio_streaming.lock().unwrap();
        let last_rx = *self.last_rx.lock().unwrap();
        let mut status = PlatformStatus::default();

        status.compute = ComputeStatus {
            readiness: Readiness::Ready,
            model: "Raspberry Pi 5".into(),
            cpu_percent: 12.0 + (elapsed.as_secs() % 20) as f32,
            temp_c: 48.0 + (elapsed.as_secs() % 10) as f32 * 0.3,
            mem_used_mb: 1100,
            mem_total_mb: 8192,
        };

        status.storage = StorageStatus {
            readiness: Readiness::Ready,
            root_used_percent: 34.0,
            data_free_gb: 180.0,
        };

        status.network = NetworkStatus {
            readiness: Readiness::Ready,
            interfaces: vec!["lo".into(), "eth0".into(), "wlan0".into()],
        };

        if elapsed > Duration::from_secs(3) {
            let t = elapsed.as_secs_f32();
            status.gps = GpsStatus {
                readiness: Readiness::Active,
                fix: GpsFix::Fix3D,
                satellites: 11,
                latitude: Some(-27.4701 + f64::from(t) * 0.000008),
                longitude: Some(153.0211 + f64::from(t) * 0.000006),
                altitude_m: Some(12.4),
                speed_kmh: Some(3.2),
                course_deg: Some(48.0 + (t * 1.5) % 24.0),
                hdop: Some(0.9),
                age_ms: Some(400),
                device: "/dev/ttyACM0".into(),
                label: "simulated GNSS".into(),
            };
        }

        if elapsed > Duration::from_secs(5) {
            let peers = vec![
                MeshPeer {
                    id: "!a1b2c3d4".into(),
                    role: "THIS NODE".into(),
                    own: true,
                },
                MeshPeer {
                    id: PEERS[0].into(),
                    role: "PEER".into(),
                    own: false,
                },
                MeshPeer {
                    id: PEERS[1].into(),
                    role: "PEER".into(),
                    own: false,
                },
                MeshPeer {
                    id: PEERS[2].into(),
                    role: "PEER".into(),
                    own: false,
                },
            ];
            status.mesh = MeshStatus {
                readiness: if last_rx.is_some() {
                    Readiness::Active
                } else {
                    Readiness::Ready
                },
                node_id: "!a1b2c3d4".into(),
                nodes_heard: peers.len() as u32,
                last_rx: last_rx.map(|at| {
                    let secs = at.elapsed().as_secs();
                    if secs < 2 {
                        "live".into()
                    } else {
                        format!("{secs}s ago")
                    }
                }),
                region: "AU915".into(),
                peers,
                fixes: vec![crate::core::hardware::MeshFix {
                    name: "CAMP".into(),
                    lat: -27.475,
                    lon: 153.03,
                }],
                port: "/dev/ttyUSB0".into(),
                label: "simulated mesh".into(),
            };
        }

        let scan = self.scan_overlay();
        if elapsed > Duration::from_secs(7) {
            status.radio = RadioStatus {
                readiness: if streaming {
                    Readiness::Active
                } else {
                    Readiness::Ready
                },
                device: "NESDR SMArt v5".into(),
                center_freq_mhz: freq,
                sample_rate: 2_048_000,
                simulated: true,
                spectrum: spectrum_bins(elapsed.as_secs_f32(), streaming && !scan.scanning, true, freq),
                scanning: scan.scanning,
                scan_progress: scan.progress,
                scan_label: scan.label,
                scan_hits: scan.hits,
            };
        }

        status.attachments = attachments(&status.gps, &status.radio, &status.mesh);

        if elapsed > Duration::from_secs(9) {
            status.power = PowerStatus {
                readiness: Readiness::Ready,
                source: PowerSource::Battery,
                percent: Some(87),
                voltage: Some(12.4),
                throttled: false,
            };
        }

        status
    }

    fn set_radio_freq(&self, mhz: f32) {
        *self.radio_freq_mhz.lock().unwrap() = mhz;
    }

    fn set_radio_streaming(&self, on: bool) {
        *self.radio_streaming.lock().unwrap() = on;
    }

    fn start_radio_scan(&self, band_id: &str, center_mhz: f32) {
        let plan = scan_plan(band_id, center_mhz);
        *self.scan.lock().unwrap() = ScanPhase::Running {
            started: Instant::now(),
            label: plan.label,
            points: plan.points,
        };
    }

    fn cancel_radio_scan(&self) {
        let mut phase = self.scan.lock().unwrap();
        if let ScanPhase::Running { label, points, started } = &*phase {
            let shown = sweep_shown(started.elapsed(), points.len());
            let hits = hits_for(&points[..shown]);
            *phase = ScanPhase::Done {
                label: label.clone(),
                hits,
            };
        }
    }

    fn take_mesh_inbox(&self) -> Vec<MeshInbound> {
        if self.start.elapsed() <= Duration::from_secs(5) {
            return Vec::new();
        }
        let due = (self.start.elapsed().as_secs() / 10) as u32;
        let mut step = self.inbox_step.lock().unwrap();
        if due == 0 || due <= *step {
            return Vec::new();
        }
        *step = due;
        drop(step);
        *self.last_rx.lock().unwrap() = Some(Instant::now());
        let index = (due as usize).saturating_sub(1);
        vec![MeshInbound {
            who: PEERS[index % PEERS.len()].into(),
            body: LINES[index % LINES.len()].into(),
        }]
    }

    fn send_mesh_text(&self, text: &str) -> bool {
        !text.trim().is_empty() && self.start.elapsed() > Duration::from_secs(5)
    }
}
