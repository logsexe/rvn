//! The operation file. Waypoints, the track, the dial, and the mesh log
//! survive a restart. A missing file just starts a fresh day.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::state::{Bookmark, MeshMessage, Waypoint};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Operation {
    pub waypoints: Vec<Waypoint>,
    pub track: Vec<(f64, f64)>,
    pub selected_mark: String,
    pub map_lat: f64,
    pub map_lon: f64,
    pub map_zoom: f64,
    pub map_follow: bool,
    pub radio_mhz: f32,
    pub bookmarks: Vec<Bookmark>,
    pub mesh_messages: Vec<MeshMessage>,
    pub night: bool,
}

#[derive(Debug, Deserialize)]
struct Measured {
    pack_wh: f64,
    draw_w: f64,
}

pub fn data_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        return PathBuf::from(xdg).join("rvn");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".local").join("share").join("rvn");
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        return PathBuf::from(local).join("rvn");
    }
    PathBuf::from("rvn-data")
}

pub fn operation_path() -> PathBuf {
    data_dir().join("operation.json")
}

pub fn load(path: &Path) -> Option<Operation> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn save(path: &Path, operation: &Operation) -> bool {
    let Ok(text) = serde_json::to_string_pretty(operation) else {
        return false;
    };
    if let Some(parent) = path.parent() {
        if fs::create_dir_all(parent).is_err() {
            return false;
        }
    }
    fs::write(path, text).is_ok()
}

/// Hours at the measured draw. Absent until both numbers are real and positive.
pub fn measured_hours(dir: &Path) -> Option<f32> {
    let text = fs::read_to_string(dir.join("endurance.json")).ok()?;
    let measured: Measured = serde_json::from_str(&text).ok()?;
    if measured.pack_wh <= 0.0 || measured.draw_w <= 0.0 {
        return None;
    }
    Some((measured.pack_wh / measured.draw_w) as f32)
}

pub fn endurance_label(dir: &Path) -> String {
    match measured_hours(dir) {
        Some(hours) => format!("{hours:.1} h"),
        None => "—".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_roundtrip() {
        let dir = std::env::temp_dir().join(format!("rvn-store-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("operation.json");
        let operation = Operation {
            waypoints: Vec::new(),
            track: vec![(-27.47, 153.02)],
            selected_mark: "WP-01".into(),
            map_lat: -27.47,
            map_lon: 153.02,
            map_zoom: 14.0,
            map_follow: false,
            radio_mhz: 433.5,
            bookmarks: vec![Bookmark {
                name: "camp".into(),
                mhz: 433.5,
            }],
            mesh_messages: Vec::new(),
            night: true,
        };
        assert!(save(&path, &operation));
        let loaded = load(&path).expect("operation");
        assert_eq!(loaded.track, operation.track);
        assert_eq!(loaded.bookmarks[0].name, "camp");
        assert!(loaded.night);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn endurance_needs_both_numbers() {
        let dir = std::env::temp_dir().join(format!("rvn-endurance-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        assert!(measured_hours(&dir).is_none());
        fs::write(dir.join("endurance.json"), r#"{"pack_wh": 0, "draw_w": 8}"#).unwrap();
        assert!(measured_hours(&dir).is_none());
        fs::write(dir.join("endurance.json"), r#"{"pack_wh": 85.0, "draw_w": 8.5}"#).unwrap();
        let hours = measured_hours(&dir).expect("hours");
        assert!((hours - 10.0).abs() < 0.01, "{hours}");
        let _ = fs::remove_dir_all(&dir);
    }
}
