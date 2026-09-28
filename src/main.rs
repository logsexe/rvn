mod adapters;
mod core;
mod surfaces;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use chrono::Local;
use tracing::info;
use tracing_subscriber::EnvFilter;

use adapters::MockAdapter;
use core::hardware::{PowerSource, Readiness};
use core::{AppState, Surface};
use surfaces::run_line;

slint::include_modules!();

struct LiveControls {
    radio_freq_mhz: f32,
    radio_streaming: bool,
    mesh_tx_enabled: bool,
}

impl Default for LiveControls {
    fn default() -> Self {
        Self {
            radio_freq_mhz: 433.0,
            radio_streaming: false,
            mesh_tx_enabled: false,
        }
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("rvn=info".parse()?))
        .init();

    info!("RVN // FIELD//OS starting");

    let ui = AppWindow::new()?;
    let mock = MockAdapter::new();
    let controls = Arc::new(Mutex::new(LiveControls::default()));

    let state = AppState {
        clock: Local::now().format("%H:%M:%S").to_string(),
        platform: mock.poll(),
        ..AppState::default()
    };
    sync_ui(&ui, &state, &controls.lock().unwrap());

    let ui_weak = ui.as_weak();
    let controls_tick = controls.clone();
    std::thread::spawn(move || {
        let mock = MockAdapter::new();
        loop {
            std::thread::sleep(Duration::from_millis(500));
            let ui = match ui_weak.upgrade() {
                Some(u) => u,
                None => break,
            };
            let state = AppState {
                clock: Local::now().format("%H:%M:%S").to_string(),
                platform: mock.poll(),
                ..AppState::default()
            };
            let ctrl = controls_tick.lock().unwrap();
            sync_ui(&ui, &state, &ctrl);
        }
    });

    ui.on_surface_selected(|id| {
        info!("Surface selected: {}", Surface::from_id(id).name());
    });
    ui.on_go_home(|| info!("Returned to home"));

    {
        let controls = controls.clone();
        let ui_weak = ui.as_weak();
        ui.on_radio_tune_up(move || {
            let mut c = controls.lock().unwrap();
            c.radio_freq_mhz += 0.1;
            info!("RADIO tune → {:.3} MHz", c.radio_freq_mhz);
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_radio_freq(format!("{:.3} MHz", c.radio_freq_mhz).into());
            }
        });
    }
    {
        let controls = controls.clone();
        let ui_weak = ui.as_weak();
        ui.on_radio_tune_down(move || {
            let mut c = controls.lock().unwrap();
            c.radio_freq_mhz = (c.radio_freq_mhz - 0.1).max(0.1);
            info!("RADIO tune → {:.3} MHz", c.radio_freq_mhz);
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_radio_freq(format!("{:.3} MHz", c.radio_freq_mhz).into());
            }
        });
    }
    {
        let controls = controls.clone();
        let ui_weak = ui.as_weak();
        ui.on_radio_toggle_stream(move || {
            let mut c = controls.lock().unwrap();
            c.radio_streaming = !c.radio_streaming;
            info!("RADIO stream {}", if c.radio_streaming { "ON" } else { "OFF" });
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_radio_streaming(c.radio_streaming);
            }
        });
    }

    {
        let controls = controls.clone();
        let ui_weak = ui.as_weak();
        ui.on_mesh_toggle_tx(move || {
            let mut c = controls.lock().unwrap();
            c.mesh_tx_enabled = !c.mesh_tx_enabled;
            info!("MESH TX {}", if c.mesh_tx_enabled { "ARMED" } else { "SAFE" });
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_mesh_tx_enabled(c.mesh_tx_enabled);
            }
        });
    }
    {
        let ui_weak = ui.as_weak();
        ui.on_mesh_send(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let draft = ui.get_mesh_draft();
                info!("MESH send requested: {:?}", draft);
                ui.set_mesh_draft("".into());
            }
        });
    }


    // TERMINAL
    {
        let ui_weak = ui.as_weak();
        ui.on_term_submit(move |cmd| {
            let cmd = cmd.to_string();
            if let Some(ui) = ui_weak.upgrade() {
                let prompt = format!("~ › {}
", cmd);
                let mut out = ui.get_term_output().to_string();
                if out.is_empty() {
                    out = "RVN terminal ready.\nType help for commands.\n".into();
                }
                out.push_str(&prompt);

                ui.set_term_busy(true);
                let result = run_line(&cmd, "~");
                if result.clear {
                    out = "RVN terminal ready.\nType help for commands.\n".into();
                } else {
                    out.push_str(&result.stdout);
                }
                ui.set_term_output(out.into());
                ui.set_term_input("".into());
                ui.set_term_busy(false);
                tracing::info!("terminal: {}", cmd);
            }
        });
    }
    {
        let ui_weak = ui.as_weak();
        ui.on_term_clear(move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_term_output("RVN terminal ready.\nType help for commands.\n".into());
                ui.set_term_input("".into());
            }
        });
    }

    ui.run()?;
    Ok(())
}

fn sync_ui(ui: &AppWindow, state: &AppState, ctrl: &LiveControls) {
    let p = &state.platform;

    ui.set_current_time(state.clock.clone().into());
    ui.set_op_name(state.operation_name.clone().into());
    ui.set_gps_status(state.gps_display().into());
    ui.set_mesh_status(state.mesh_display().into());
    ui.set_power_str(state.power_display().into());

    ui.set_nav_status(state.nav_card_status().into());
    ui.set_radio_status(state.radio_card_status().into());
    ui.set_mesh_card_status(state.mesh_card_status().into());
    ui.set_terminal_status(state.terminal_card_status().into());
    ui.set_system_status(state.system_card_status().into());

    ui.set_sys_cpu_model(p.compute.model.clone().into());
    ui.set_sys_cpu_percent(p.compute.cpu_percent);
    ui.set_sys_temp_c(p.compute.temp_c);
    ui.set_sys_mem_used(p.compute.mem_used_mb as i32);
    ui.set_sys_mem_total(p.compute.mem_total_mb as i32);
    ui.set_sys_power_source(
        match p.power.source {
            PowerSource::Mains => "Mains",
            PowerSource::Battery => "Battery",
            PowerSource::Unknown => "Unknown",
        }
        .into(),
    );
    ui.set_sys_power_percent(state.power_display().into());
    ui.set_sys_power_voltage(
        p.power
            .voltage
            .map(|v| format!("{v:.1} V"))
            .unwrap_or_else(|| "—".into())
            .into(),
    );
    ui.set_sys_throttled(p.power.throttled);
    ui.set_sys_gps_readiness(p.gps.readiness.as_status_str().into());
    ui.set_sys_gps_fix(p.gps.fix.as_display().into());
    ui.set_sys_gps_sats(p.gps.satellites as i32);
    ui.set_sys_gps_lat(
        p.gps
            .latitude
            .map(|v| format!("{v:.5}"))
            .unwrap_or_else(|| "—".into())
            .into(),
    );
    ui.set_sys_gps_lon(
        p.gps
            .longitude
            .map(|v| format!("{v:.5}"))
            .unwrap_or_else(|| "—".into())
            .into(),
    );
    ui.set_sys_radio_readiness(p.radio.readiness.as_status_str().into());
    ui.set_sys_radio_device(p.radio.device.clone().into());
    ui.set_sys_radio_freq(
        if p.radio.readiness == Readiness::NotPresent {
            "—".into()
        } else {
            format!("{:.3} MHz", ctrl.radio_freq_mhz).into()
        },
    );
    ui.set_sys_mesh_readiness(p.mesh.readiness.as_status_str().into());
    ui.set_sys_mesh_node(p.mesh.node_id.clone().into());
    ui.set_sys_mesh_nodes(p.mesh.nodes_heard as i32);
    ui.set_sys_storage_used(p.storage.root_used_percent);
    ui.set_sys_storage_free(format!("{:.0} GB free", p.storage.data_free_gb).into());
    ui.set_sys_net_ifaces(p.network.interfaces.join(", ").into());

    ui.set_nav_readiness(p.gps.readiness.as_status_str().into());
    ui.set_nav_fix(p.gps.fix.as_display().into());
    ui.set_nav_sats(p.gps.satellites as i32);
    ui.set_nav_lat(
        p.gps
            .latitude
            .map(|v| format!("{v:.6}"))
            .unwrap_or_else(|| "—".into())
            .into(),
    );
    ui.set_nav_lon(
        p.gps
            .longitude
            .map(|v| format!("{v:.6}"))
            .unwrap_or_else(|| "—".into())
            .into(),
    );
    ui.set_nav_alt(
        p.gps
            .altitude_m
            .map(|v| format!("{v:.1} m"))
            .unwrap_or_else(|| "—".into())
            .into(),
    );
    ui.set_nav_speed(
        p.gps
            .speed_kmh
            .map(|v| format!("{v:.1} km/h"))
            .unwrap_or_else(|| "—".into())
            .into(),
    );
    ui.set_nav_heading("—".into());
    ui.set_nav_grid("—".into());
    ui.set_nav_accuracy(
        if p.gps.readiness == Readiness::NotPresent {
            "—".into()
        } else {
            "± 3 m".into()
        },
    );
    ui.set_nav_age(
        if p.gps.readiness == Readiness::NotPresent {
            "—".into()
        } else {
            "live".into()
        },
    );

    ui.set_radio_readiness(p.radio.readiness.as_status_str().into());
    ui.set_radio_device(p.radio.device.clone().into());
    ui.set_radio_freq(
        if p.radio.readiness == Readiness::NotPresent {
            "—".into()
        } else {
            format!("{:.3} MHz", ctrl.radio_freq_mhz).into()
        },
    );
    ui.set_radio_rate(
        if p.radio.readiness == Readiness::NotPresent {
            "—".into()
        } else {
            format!("{} Hz", p.radio.sample_rate).into()
        },
    );
    ui.set_radio_streaming(ctrl.radio_streaming);

    ui.set_mesh_readiness(p.mesh.readiness.as_status_str().into());
    ui.set_mesh_node_id(p.mesh.node_id.clone().into());
    ui.set_mesh_nodes_heard(p.mesh.nodes_heard as i32);
    ui.set_mesh_last_rx(
        p.mesh
            .last_rx
            .clone()
            .unwrap_or_else(|| "—".into())
            .into(),
    );
    ui.set_mesh_region("AU915".into());
    ui.set_mesh_tx_enabled(ctrl.mesh_tx_enabled);
}
