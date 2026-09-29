//! Hardware readiness and status model.
//! Every piece of hardware reports through this layer.
//! Missing hardware is never an error — it is simply `NotPresent`.

use serde::{Deserialize, Serialize};

/// Lifecycle state of any hardware module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Readiness {
    /// Device not detected / not connected
    #[default]
    NotPresent,
    /// Detected but not yet initialised or has a problem
    Degraded,
    /// Present and healthy
    Ready,
    /// Actively in use / streaming / transmitting
    Active,
}

impl Readiness {
    pub fn as_status_str(self) -> &'static str {
        match self {
            Self::NotPresent => "offline",
            Self::Degraded => "warn",
            Self::Ready => "ok",
            Self::Active => "active",
        }
    }
}

/// High-level view of the entire platform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformStatus {
    pub compute: ComputeStatus,
    pub power: PowerStatus,
    pub gps: GpsStatus,
    pub radio: RadioStatus,
    pub mesh: MeshStatus,
    pub network: NetworkStatus,
    pub storage: StorageStatus,
    pub attachments: Vec<Attachment>,
}

impl Default for PlatformStatus {
    fn default() -> Self {
        Self {
            compute: ComputeStatus::default(),
            power: PowerStatus::default(),
            gps: GpsStatus::default(),
            radio: RadioStatus::default(),
            mesh: MeshStatus::default(),
            network: NetworkStatus::default(),
            storage: StorageStatus::default(),
            attachments: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Sub-system status structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComputeStatus {
    pub readiness: Readiness,
    pub model: String,
    pub cpu_percent: f32,
    pub temp_c: f32,
    pub mem_used_mb: u32,
    pub mem_total_mb: u32,
}

impl Default for ComputeStatus {
    fn default() -> Self {
        Self {
            readiness: Readiness::Ready,
            model: "Raspberry Pi 5".into(),
            cpu_percent: 0.0,
            temp_c: 0.0,
            mem_used_mb: 0,
            mem_total_mb: 8192,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PowerStatus {
    pub readiness: Readiness,
    pub source: PowerSource,
    pub percent: Option<u8>,
    pub voltage: Option<f32>,
    pub throttled: bool,
}

impl Default for PowerStatus {
    fn default() -> Self {
        Self {
            readiness: Readiness::NotPresent,
            source: PowerSource::Unknown,
            percent: None,
            voltage: None,
            throttled: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PowerSource {
    #[default]
    Unknown,
    Mains,
    Battery,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpsStatus {
    pub readiness: Readiness,
    pub fix: GpsFix,
    pub satellites: u8,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub altitude_m: Option<f32>,
    pub speed_kmh: Option<f32>,
    pub course_deg: Option<f32>,
    /// Horizontal dilution of precision from the latest GGA, when the receiver sent one.
    pub hdop: Option<f32>,
    /// Milliseconds since the last accepted NMEA sentence.
    pub age_ms: Option<u64>,
    /// Serial path once a receiver is claimed, such as `/dev/ttyACM0`.
    pub device: String,
    /// USB product string for that port, when the host provides one.
    pub label: String,
}

impl Default for GpsStatus {
    fn default() -> Self {
        Self {
            readiness: Readiness::NotPresent,
            fix: GpsFix::None,
            satellites: 0,
            latitude: None,
            longitude: None,
            altitude_m: None,
            speed_kmh: None,
            course_deg: None,
            hdop: None,
            age_ms: None,
            device: "—".into(),
            label: "—".into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum GpsFix {
    #[default]
    None,
    DeadReckoning,
    Fix2D,
    Fix3D,
}

impl GpsFix {
    pub fn as_display(self) -> &'static str {
        match self {
            Self::None => "NO FIX",
            Self::DeadReckoning => "DR",
            Self::Fix2D => "2D FIX",
            Self::Fix3D => "3D FIX",
        }
    }
}

/// Bins drawn across the RADIO spectrum plot.
pub const SPECTRUM_BINS: usize = 48;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RadioStatus {
    pub readiness: Readiness,
    pub device: String,
    pub center_freq_mhz: f32,
    pub sample_rate: u32,
    /// True when the bins are a stand-in rather than samples from a dongle.
    pub simulated: bool,
    pub spectrum: Vec<f32>,
}

impl Default for RadioStatus {
    fn default() -> Self {
        Self {
            readiness: Readiness::NotPresent,
            device: "—".into(),
            center_freq_mhz: 0.0,
            sample_rate: 0,
            simulated: false,
            spectrum: vec![0.0; SPECTRUM_BINS],
        }
    }
}

/// One row in the MESH node list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshPeer {
    pub id: String,
    pub role: String,
    pub own: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshStatus {
    pub readiness: Readiness,
    pub node_id: String,
    pub nodes_heard: u32,
    pub last_rx: Option<String>,
    /// LoRa region reported by the radio. Live adapters leave this blank until a region is read.
    pub region: String,
    pub peers: Vec<MeshPeer>,
    /// Serial path once a Meshtastic radio is claimed.
    pub port: String,
    pub label: String,
}

impl Default for MeshStatus {
    fn default() -> Self {
        Self {
            readiness: Readiness::NotPresent,
            node_id: "—".into(),
            nodes_heard: 0,
            last_rx: None,
            region: "—".into(),
            peers: Vec::new(),
            port: "—".into(),
            label: "—".into(),
        }
    }
}

/// One USB assignment the shell is willing to say out loud.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Attachment {
    pub panel: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkStatus {
    pub readiness: Readiness,
    pub interfaces: Vec<String>,
}

impl Default for NetworkStatus {
    fn default() -> Self {
        Self {
            readiness: Readiness::Ready,
            interfaces: vec!["lo".into()],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageStatus {
    pub readiness: Readiness,
    pub root_used_percent: f32,
    pub data_free_gb: f32,
}

impl Default for StorageStatus {
    fn default() -> Self {
        Self {
            readiness: Readiness::Ready,
            root_used_percent: 0.0,
            data_free_gb: 0.0,
        }
    }
}
