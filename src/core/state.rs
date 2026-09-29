//! Application state — single source of truth.

use super::geo;
use super::hardware::{PlatformStatus, Readiness};
use serde::{Deserialize, Serialize};

/// Operator-dropped position.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Waypoint {
    pub id: String,
    pub lat: f64,
    pub lon: f64,
    pub alt_m: Option<f32>,
    pub marked_at: String,
}

/// One line in the MESH traffic log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshMessage {
    pub time: String,
    pub who: String,
    pub body: String,
    pub outbound: bool,
}

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
    pub waypoints: Vec<Waypoint>,
    pub tracking: bool,
    /// Walked path, oldest first. Capped so a long day does not grow without limit.
    pub track: Vec<(f64, f64)>,
    pub track_points: u32,
    pub last_track_lat: Option<f64>,
    pub last_track_lon: Option<f64>,
    pub nav_notice: String,
    pub mesh_messages: Vec<MeshMessage>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            active_surface: Surface::Home,
            operation_name: "STANDBY".into(),
            platform: PlatformStatus::default(),
            clock: "00:00:00".into(),
            waypoints: Vec::new(),
            tracking: false,
            track: Vec::new(),
            track_points: 0,
            last_track_lat: None,
            last_track_lon: None,
            nav_notice: String::new(),
            mesh_messages: Vec::new(),
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
        let gps = &self.platform.gps;
        if gps.readiness == Readiness::NotPresent {
            return "OFFLINE".into();
        }
        if gps.latitude.is_none() {
            return "LINKED".into();
        }
        gps.fix.as_display().to_string()
    }

    pub fn mesh_display(&self) -> String {
        match self.platform.mesh.readiness {
            Readiness::NotPresent => "OFFLINE".into(),
            _ => format!("{} NODES", self.platform.mesh.nodes_heard),
        }
    }

    pub fn power_display(&self) -> String {
        match self.platform.power.percent {
            Some(p) => format!("{p}%"),
            None => "—".into(),
        }
    }

    pub fn grid_display(&self) -> String {
        match (self.platform.gps.latitude, self.platform.gps.longitude) {
            (Some(lat), Some(lon)) if self.platform.gps.readiness != Readiness::NotPresent => {
                geo::maidenhead(lat, lon)
            }
            _ => "—".into(),
        }
    }

    pub fn heading_display(&self) -> String {
        let gps = &self.platform.gps;
        match (gps.course_deg, gps.speed_kmh) {
            (Some(heading), Some(speed))
                if speed >= 0.5 && gps.readiness != Readiness::NotPresent =>
            {
                format!("{heading:03.0}°")
            }
            _ => "—".into(),
        }
    }

    pub fn nav_accuracy(&self) -> String {
        let gps = &self.platform.gps;
        if gps.readiness == Readiness::NotPresent {
            return "—".into();
        }
        match gps.hdop {
            Some(hdop) => format!("HDOP {hdop:.1}"),
            None => "—".into(),
        }
    }

    pub fn nav_age(&self) -> String {
        let gps = &self.platform.gps;
        if gps.readiness == Readiness::NotPresent {
            return "—".into();
        }
        match gps.age_ms {
            Some(ms) if ms < 1_500 => "live".into(),
            Some(ms) => format!("{}s", ms / 1000),
            None => "—".into(),
        }
    }

    pub fn coords_text(&self) -> Option<String> {
        match (self.platform.gps.latitude, self.platform.gps.longitude) {
            (Some(lat), Some(lon)) if self.platform.gps.readiness != Readiness::NotPresent => {
                Some(format!("{lat:.6}, {lon:.6}"))
            }
            _ => None,
        }
    }

    /// Drop a waypoint at the current fix. Returns false when there is no position.
    pub fn mark_waypoint(&mut self, marked_at: &str) -> bool {
        let (Some(lat), Some(lon)) = (self.platform.gps.latitude, self.platform.gps.longitude)
        else {
            self.nav_notice = "NO FIX".into();
            return false;
        };
        if self.platform.gps.readiness == Readiness::NotPresent {
            self.nav_notice = "NO FIX".into();
            return false;
        }
        let id = format!("WP-{:02}", self.waypoints.len() + 1);
        self.nav_notice = format!("MARKED {id}");
        self.waypoints.push(Waypoint {
            id,
            lat,
            lon,
            alt_m: self.platform.gps.altitude_m,
            marked_at: marked_at.to_string(),
        });
        true
    }

    pub fn toggle_track(&mut self) {
        self.tracking = !self.tracking;
        if self.tracking {
            self.track.clear();
            self.track_points = 0;
            self.last_track_lat = None;
            self.last_track_lon = None;
            self.operation_name = "TRACKING".into();
            self.nav_notice = "TRACK STARTED".into();
            self.sample_track();
        } else {
            self.operation_name = "STANDBY".into();
            self.nav_notice = format!("TRACK STOPPED · {} PTS", self.track_points);
        }
    }

    /// Record a track point when the fix has moved at least 8 metres.
    pub fn sample_track(&mut self) {
        if !self.tracking {
            return;
        }
        let (Some(lat), Some(lon)) = (self.platform.gps.latitude, self.platform.gps.longitude)
        else {
            return;
        };
        if self.platform.gps.readiness == Readiness::NotPresent {
            return;
        }
        let moved = match (self.last_track_lat, self.last_track_lon) {
            (Some(prev_lat), Some(prev_lon)) => geo::haversine_m(prev_lat, prev_lon, lat, lon) >= 8.0,
            _ => true,
        };
        if moved {
            self.track.push((lat, lon));
            if self.track.len() > 1_500 {
                let extra = self.track.len() - 1_500;
                self.track.drain(0..extra);
            }
            self.track_points = self.track.len() as u32;
            self.last_track_lat = Some(lat);
            self.last_track_lon = Some(lon);
        }
    }

    pub fn push_mesh_message(&mut self, time: &str, who: &str, body: &str, outbound: bool) {
        self.mesh_messages.push(MeshMessage {
            time: time.to_string(),
            who: who.to_string(),
            body: body.to_string(),
            outbound,
        });
        if self.mesh_messages.len() > 40 {
            let extra = self.mesh_messages.len() - 40;
            self.mesh_messages.drain(0..extra);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::hardware::Readiness;
    use super::*;

    fn with_fix() -> AppState {
        let mut state = AppState::default();
        state.platform.gps.readiness = Readiness::Ready;
        state.platform.gps.latitude = Some(-27.47);
        state.platform.gps.longitude = Some(153.02);
        state.platform.gps.altitude_m = Some(12.0);
        state
    }

    #[test]
    fn accuracy_uses_hdop() {
        let mut state = with_fix();
        state.platform.gps.hdop = Some(1.34);
        state.platform.gps.age_ms = Some(400);
        assert_eq!(state.nav_accuracy(), "HDOP 1.3");
        assert_eq!(state.nav_age(), "live");
        state.platform.gps.age_ms = Some(4_200);
        assert_eq!(state.nav_age(), "4s");
    }

    #[test]
    fn mark_requires_a_fix() {
        let mut state = AppState::default();
        assert!(!state.mark_waypoint("12:00:00"));
        assert!(state.waypoints.is_empty());
    }

    #[test]
    fn mark_numbers_waypoints() {
        let mut state = with_fix();
        assert!(state.mark_waypoint("12:00:01"));
        assert!(state.mark_waypoint("12:00:02"));
        assert_eq!(state.waypoints[0].id, "WP-01");
        assert_eq!(state.waypoints[1].id, "WP-02");
        assert_eq!(state.nav_notice, "MARKED WP-02");
    }

    #[test]
    fn track_waits_for_movement() {
        let mut state = with_fix();
        state.toggle_track();
        assert_eq!(state.track_points, 1);
        state.sample_track();
        assert_eq!(state.track_points, 1);
        state.platform.gps.latitude = Some(-27.48);
        state.sample_track();
        assert_eq!(state.track_points, 2);
        assert_eq!(state.track.len(), 2);
        state.toggle_track();
        assert!(!state.tracking);
        assert_eq!(state.track.len(), 2);
        assert_eq!(state.operation_name, "STANDBY");
    }

    #[test]
    fn mesh_log_caps_at_forty() {
        let mut state = AppState::default();
        for i in 0..45 {
            state.push_mesh_message("12:00:00", "PEER", &format!("m{i}"), false);
        }
        assert_eq!(state.mesh_messages.len(), 40);
        assert_eq!(state.mesh_messages[0].body, "m5");
    }
}
