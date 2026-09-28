//! Application state — single source of truth.

use super::hardware::PlatformStatus;
use serde::{Deserialize, Serialize};

/// Which surface is currently visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Surface {
    #[default]
    Home,
    Nav,
    Radio,
    Mesh,
    Terminal,
    System,
}

impl Surface {
    pub fn from_id(id: i32) -> Self {
        match id {
            0 => Self::Nav,
            1 => Self::Radio,
            2 => Self::Mesh,
            3 => Self::Terminal,
            4 => Self::System,
            _ => Self::Home,
        }
    }

    pub fn id(self) -> i32 {
        match self {
            Self::Home => -1,
            Self::Nav => 0,
            Self::Radio => 1,
            Self::Mesh => 2,
            Self::Terminal => 3,
            Self::System => 4,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Home => "HOME",
            Self::Nav => "NAV",
            Self::Radio => "RADIO",
            Self::Mesh => "MESH",
            Self::Terminal => "TERMINAL",
            Self::System => "SYSTEM",
        }
    }
}

/// Top-level application state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppState {
    pub active_surface: Surface,
    pub operation_name: String,
    pub platform: PlatformStatus,
    pub clock: String,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            active_surface: Surface::Home,
            operation_name: "STANDBY".into(),
            platform: PlatformStatus::default(),
            clock: "00:00:00".into(),
        }
    }
}

impl AppState {
    /// Derived status string for the NAV card.
    pub fn nav_card_status(&self) -> &'static str {
        self.platform.gps.readiness.as_status_str()
    }

    pub fn radio_card_status(&self) -> &'static str {
        self.platform.radio.readiness.as_status_str()
    }

    pub fn mesh_card_status(&self) -> &'static str {
        self.platform.mesh.readiness.as_status_str()
    }

    pub fn terminal_card_status(&self) -> &'static str {
        "ok" // terminal is always available
    }

    pub fn system_card_status(&self) -> &'static str {
        if self.platform.compute.temp_c > 80.0 || self.platform.power.throttled {
            "warn"
        } else {
            "ok"
        }
    }

    pub fn gps_display(&self) -> String {
        self.platform.gps.fix.as_display().to_string()
    }

    pub fn mesh_display(&self) -> String {
        match self.platform.mesh.readiness {
            super::hardware::Readiness::NotPresent => "OFFLINE".into(),
            _ => format!("{} NODES", self.platform.mesh.nodes_heard),
        }
    }

    pub fn power_display(&self) -> String {
        match self.platform.power.percent {
            Some(p) => format!("{p}%"),
            None => "—".into(),
        }
    }
}
