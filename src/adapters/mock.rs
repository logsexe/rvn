//! Mock hardware adapter — simulates devices coming online.
//! Used for development and for running on machines without the real peripherals.

use std::time::{Duration, Instant};

use crate::core::hardware::*;

pub struct MockAdapter {
    start: Instant,
}

impl MockAdapter {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }

    /// Produce a realistic PlatformStatus based on how long the app has been running.
    pub fn poll(&self) -> PlatformStatus {
        let elapsed = self.start.elapsed();

        let mut status = PlatformStatus::default();

        // Compute is always present
        status.compute = ComputeStatus {
            readiness: Readiness::Ready,
            model: "Raspberry Pi 5".into(),
            cpu_percent: 12.0 + (elapsed.as_secs() % 20) as f32,
            temp_c: 48.0 + (elapsed.as_secs() % 10) as f32 * 0.3,
            mem_used_mb: 1100,
            mem_total_mb: 8192,
        };

        // Storage
        status.storage = StorageStatus {
            readiness: Readiness::Ready,
            root_used_percent: 34.0,
            data_free_gb: 180.0,
        };

        // Network
        status.network = NetworkStatus {
            readiness: Readiness::Ready,
            interfaces: vec!["lo".into(), "eth0".into(), "wlan0".into()],
        };

        // Staggered bring-up so the UI feels alive
        if elapsed > Duration::from_secs(3) {
            status.gps = GpsStatus {
                readiness: Readiness::Ready,
                fix: GpsFix::Fix3D,
                satellites: 11,
                latitude: Some(-27.4701),
                longitude: Some(153.0211),
                altitude_m: Some(12.4),
                speed_kmh: Some(0.0),
            };
        }

        if elapsed > Duration::from_secs(5) {
            status.mesh = MeshStatus {
                readiness: Readiness::Active,
                node_id: "!a1b2c3d4".into(),
                nodes_heard: 4,
                last_rx: Some("12s ago".into()),
            };
        }

        if elapsed > Duration::from_secs(7) {
            status.radio = RadioStatus {
                readiness: Readiness::Ready,
                device: "NESDR SMArt v5".into(),
                center_freq_mhz: 433.0,
                sample_rate: 2_048_000,
            };
        }

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
}

impl Default for MockAdapter {
    fn default() -> Self {
        Self::new()
    }
}
