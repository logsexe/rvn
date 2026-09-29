mod adapters;
mod core;
mod surfaces;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use chrono::Local;
use slint::{ModelRc, Timer, TimerMode, VecModel};
use tracing::info;
use tracing_subscriber::EnvFilter;

use adapters::{boot, Platform};
use core::hardware::{PowerSource, Readiness};
use core::{AppState, Surface};
use surfaces::run_line;

slint::include_modules!();

#[derive(Clone)]
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
    let platform = boot();
    let controls = Arc::new(Mutex::new(LiveControls::default()));
    platform.set_radio_freq(433.0);
    platform.set_radio_streaming(false);
    let state = Arc::new(Mutex::new(AppState {
        clock: Local::now().format("%H:%M:%S").to_string(),
        platform: platform.poll(),
        ..AppState::default()
    }));

    {
        let snap = state.lock().unwrap().clone();
        let ctrl = controls.lock().unwrap().clone();
        push_ui(&ui, &snap, &ctrl);
    }

    let ui_timer = Timer::default();
    {
        let ui_weak = ui.as_weak();
        let platform = platform.clone();
        let state = state.clone();
        let controls = controls.clone();
        ui_timer.start(TimerMode::Repeated, Duration::from_millis(500), move || {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
            let inbox = platform.take_mesh_inbox();
            let snap = {
                let mut st = state.lock().unwrap();
                st.clock = Local::now().format("%H:%M:%S").to_string();
                st.platform = platform.poll();
                st.sample_track();
                if !inbox.is_empty() {
                    let time = Local::now().format("%H:%M:%S").to_string();
                    for message in &inbox {
                        st.push_mesh_message(&time, &message.who, &message.body, false);
                    }
                }
                st.clone()
            };
            let ctrl = controls.lock().unwrap().clone();
            push_ui(&ui, &snap, &ctrl);
        });
    }

    {
        let state = state.clone();
        ui.on_surface_selected(move |id| {
            let surface = Surface::from_id(id);
            state.lock().unwrap().active_surface = surface;
            info!("Surface selected: {}", surface.name());
        });
    }
    {
        let state = state.clone();
        ui.on_go_home(move || {
            state.lock().unwrap().active_surface = Surface::Home;
            info!("Returned to home");
        });
    }

    {
        let platform = platform.clone();
        let controls = controls.clone();
        let state = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_radio_tune_up(move || {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
            let ctrl = {
                let mut c = controls.lock().unwrap();
                c.radio_freq_mhz += 0.1;
                info!("RADIO tune → {:.3} MHz", c.radio_freq_mhz);
                c.clone()
            };
            platform.set_radio_freq(ctrl.radio_freq_mhz);
            let snap = state.lock().unwrap().clone();
            push_ui(&ui, &snap, &ctrl);
        });
    }
    {
        let platform = platform.clone();
        let controls = controls.clone();
        let state = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_radio_tune_down(move || {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
            let ctrl = {
                let mut c = controls.lock().unwrap();
                c.radio_freq_mhz = (c.radio_freq_mhz - 0.1).max(0.1);
                info!("RADIO tune → {:.3} MHz", c.radio_freq_mhz);
                c.clone()
            };
            platform.set_radio_freq(ctrl.radio_freq_mhz);
            let snap = state.lock().unwrap().clone();
            push_ui(&ui, &snap, &ctrl);
        });
    }
    {
        let platform = platform.clone();
        let controls = controls.clone();
        let state = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_radio_toggle_stream(move || {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
            let ctrl = {
                let mut c = controls.lock().unwrap();
                c.radio_streaming = !c.radio_streaming;
                info!(
                    "RADIO stream {}",
                    if c.radio_streaming { "ON" } else { "OFF" }
                );
                c.clone()
            };
            platform.set_radio_streaming(ctrl.radio_streaming);
            let snap = state.lock().unwrap().clone();
            push_ui(&ui, &snap, &ctrl);
        });
    }

    {
        let controls = controls.clone();
        let state = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_mesh_toggle_tx(move || {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
            let ctrl = {
                let mut c = controls.lock().unwrap();
                c.mesh_tx_enabled = !c.mesh_tx_enabled;
                info!("MESH TX {}", if c.mesh_tx_enabled { "ARMED" } else { "SAFE" });
                c.clone()
            };
            let snap = state.lock().unwrap().clone();
            push_ui(&ui, &snap, &ctrl);
        });
    }
    {
        let platform = platform.clone();
        let controls = controls.clone();
        let state = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_mesh_send(move || {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
            let draft = ui.get_mesh_draft().trim().to_string();
            if draft.is_empty() {
                return;
            }
            let armed = controls.lock().unwrap().mesh_tx_enabled;
            let mesh_up = {
                let st = state.lock().unwrap();
                matches!(
                    st.platform.mesh.readiness,
                    Readiness::Ready | Readiness::Active
                )
            };
            let sent = armed && mesh_up && platform.send_mesh_text(&draft);
            let snap = {
                let mut st = state.lock().unwrap();
                if sent {
                    let time = Local::now().format("%H:%M:%S").to_string();
                    st.push_mesh_message(&time, "YOU", &draft, true);
                    info!("MESH send: {draft}");
                } else {
                    info!("MESH send refused");
                }
                st.clone()
            };
            if armed {
                ui.set_mesh_draft("".into());
            }
            let ctrl = controls.lock().unwrap().clone();
            push_ui(&ui, &snap, &ctrl);
        });
    }

    {
        let controls = controls.clone();
        let state = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_nav_mark(move || {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
            let time = Local::now().format("%H:%M:%S").to_string();
            let snap = {
                let mut st = state.lock().unwrap();
                st.mark_waypoint(&time);
                info!("NAV {}", st.nav_notice);
                st.clone()
            };
            let ctrl = controls.lock().unwrap().clone();
            push_ui(&ui, &snap, &ctrl);
        });
    }
    {
        let controls = controls.clone();
        let state = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_nav_toggle_track(move || {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
            let snap = {
                let mut st = state.lock().unwrap();
                st.toggle_track();
                info!("NAV {}", st.nav_notice);
                st.clone()
            };
            let ctrl = controls.lock().unwrap().clone();
            push_ui(&ui, &snap, &ctrl);
        });
    }
    {
        let controls = controls.clone();
        let state = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_nav_copy(move || {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
            let snap = {
                let mut st = state.lock().unwrap();
                match st.coords_text() {
                    Some(coords) => {
                        if copy_text(&coords) {
                            st.nav_notice = format!("COPIED  {coords}");
                        } else {
                            st.nav_notice = format!("COORDS  {coords}");
                        }
                    }
                    None => st.nav_notice = "NO FIX".into(),
                }
                info!("NAV {}", st.nav_notice);
                st.clone()
            };
            let ctrl = controls.lock().unwrap().clone();
            push_ui(&ui, &snap, &ctrl);
        });
    }

    {
        let ui_weak = ui.as_weak();
        ui.on_term_submit(move |cmd| {
            let cmd = cmd.to_string();
            if let Some(ui) = ui_weak.upgrade() {
                let prompt = format!("~ › {cmd}\n");
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
                info!("terminal: {cmd}");
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

fn push_ui(ui: &AppWindow, state: &AppState, ctrl: &LiveControls) {
    sync_ui(ui, state, ctrl, &state.platform.radio.spectrum);
}

fn copy_text(text: &str) -> bool {
    arboard::Clipboard::new()
        .and_then(|mut clip| clip.set_text(text))
        .is_ok()
}

fn sync_ui(ui: &AppWindow, state: &AppState, ctrl: &LiveControls, spectrum: &[f32]) {
    let p = &state.platform;
    let radio_status = if ctrl.radio_streaming
        && matches!(p.radio.readiness, Readiness::Ready | Readiness::Active)
    {
        "active"
    } else {
        p.radio.readiness.as_status_str()
    };

    ui.set_current_time(state.clock.clone().into());
    ui.set_op_name(state.operation_name.clone().into());
    ui.set_gps_status(state.gps_display().into());
    ui.set_mesh_status(state.mesh_display().into());
    ui.set_power_str(state.power_display().into());

    ui.set_nav_status(state.nav_card_status().into());
    ui.set_radio_status(radio_status.into());
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
    ui.set_sys_radio_readiness(radio_status.into());
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
    ui.set_nav_heading(state.heading_display().into());
    ui.set_nav_grid(state.grid_display().into());
    ui.set_nav_accuracy(state.nav_accuracy().into());
    ui.set_nav_age(state.nav_age().into());
    ui.set_nav_tracking(state.tracking);
    ui.set_nav_track_count(state.track_points as i32);
    ui.set_nav_notice(state.nav_notice.clone().into());
    ui.set_nav_waypoints(ModelRc::new(VecModel::from(waypoint_rows(state))));

    ui.set_radio_readiness(radio_status.into());
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
    ui.set_radio_simulated(p.radio.simulated);
    ui.set_radio_spectrum(ModelRc::new(VecModel::from(spectrum.to_vec())));

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
    ui.set_mesh_region(p.mesh.region.clone().into());
    ui.set_mesh_tx_enabled(ctrl.mesh_tx_enabled);
    ui.set_mesh_nodes(ModelRc::new(VecModel::from(mesh_node_rows(&p.mesh.peers))));
    ui.set_mesh_log(ModelRc::new(VecModel::from(mesh_rows(state))));
}

fn mesh_node_rows(peers: &[core::hardware::MeshPeer]) -> Vec<MeshNode> {
    peers
        .iter()
        .map(|peer| MeshNode {
            id: peer.id.clone().into(),
            role: peer.role.clone().into(),
            own: peer.own,
        })
        .collect()
}

fn waypoint_rows(state: &AppState) -> Vec<WaypointRow> {
    state
        .waypoints
        .iter()
        .rev()
        .take(12)
        .map(|wp| WaypointRow {
            name: wp.id.clone().into(),
            coords: format!("{:.5}   {:.5}", wp.lat, wp.lon).into(),
            marked: wp.marked_at.clone().into(),
        })
        .collect()
}

fn mesh_rows(state: &AppState) -> Vec<MeshLine> {
    state
        .mesh_messages
        .iter()
        .rev()
        .map(|msg| MeshLine {
            time: msg.time.clone().into(),
            who: msg.who.clone().into(),
            body: msg.body.clone().into(),
            outbound: msg.outbound,
        })
        .collect()
}
