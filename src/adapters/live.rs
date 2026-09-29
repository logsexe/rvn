//! Live platform: host counters plus independent GPS, RTL-SDR, and Meshtastic workers.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::core::hardware::PlatformStatus;

use super::gps::GpsAdapter;
use super::host::HostAdapter;
use super::mesh::MeshAdapter;
use super::ports::{attachments, PortClaims};
use super::sdr::RadioAdapter;
use super::{MeshInbound, Platform};

pub struct LivePlatform {
    host: HostAdapter,
    gps: GpsAdapter,
    radio: RadioAdapter,
    mesh: MeshAdapter,
}

impl LivePlatform {
    pub fn start() -> Self {
        let claims = Arc::new(PortClaims::default());
        let gate = Arc::new(AtomicBool::new(false));
        Self {
            host: HostAdapter::start(),
            gps: GpsAdapter::start(claims.clone(), gate.clone()),
            radio: RadioAdapter::start(),
            mesh: MeshAdapter::start(claims, gate),
        }
    }
}

impl Platform for LivePlatform {
    fn poll(&self) -> PlatformStatus {
        let host = self.host.snapshot();
        let gps = self.gps.snapshot();
        let radio = self.radio.snapshot();
        let mesh = self.mesh.snapshot();
        let attachments = attachments(&gps, &radio, &mesh);
        PlatformStatus {
            compute: host.compute,
            power: host.power,
            gps,
            radio,
            mesh,
            network: host.network,
            storage: host.storage,
            attachments,
        }
    }

    fn set_radio_freq(&self, mhz: f32) {
        self.radio.set_freq(mhz);
    }

    fn set_radio_streaming(&self, on: bool) {
        self.radio.set_streaming(on);
    }

    fn take_mesh_inbox(&self) -> Vec<MeshInbound> {
        self.mesh.take_inbox()
    }

    fn send_mesh_text(&self, text: &str) -> bool {
        self.mesh.send_text(text)
    }
}
