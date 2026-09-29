//! Hardware adapters.
//! Each adapter talks to real hardware, or to the mock when `RVN_MOCK=1`.

mod gps;
mod host;
mod live;
mod mesh;
pub mod mock;
mod nmea;
mod ports;
mod proto;
mod sdr;

use std::sync::Arc;

use crate::core::hardware::PlatformStatus;

pub use live::LivePlatform;
pub use mock::MockAdapter;

/// One inbound mesh text, drained by the UI timer.
#[derive(Debug, Clone)]
pub struct MeshInbound {
    pub who: String,
    pub body: String,
}

pub trait Platform: Send + Sync {
    fn poll(&self) -> PlatformStatus;
    fn set_radio_freq(&self, mhz: f32);
    fn set_radio_streaming(&self, on: bool);
    fn start_radio_scan(&self, band_id: &str, center_mhz: f32);
    fn cancel_radio_scan(&self);
    fn take_mesh_inbox(&self) -> Vec<MeshInbound>;
    fn send_mesh_text(&self, text: &str) -> bool;
}

/// Live USB devices, unless `RVN_MOCK=1`.
pub fn boot() -> Arc<dyn Platform> {
    match std::env::var("RVN_MOCK") {
        Ok(value) if value == "1" => {
            tracing::info!("RVN_MOCK=1 — simulated devices");
            Arc::new(MockAdapter::new())
        }
        _ => {
            tracing::info!("listening for USB GPS, RTL-SDR, and Meshtastic");
            Arc::new(LivePlatform::start())
        }
    }
}
