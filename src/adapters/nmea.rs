//! NMEA-0183 sentence parser for a USB GNSS receiver.
//! GGA and RMC only. A checksum is enforced when the sentence carries one.

use crate::core::hardware::GpsFix;

#[derive(Debug, Clone, PartialEq)]
pub struct NmeaState {
    pub fix: GpsFix,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub altitude_m: Option<f32>,
    pub satellites: u8,
    pub hdop: Option<f32>,
    pub speed_kmh: Option<f32>,
    pub course_deg: Option<f32>,
    gga_fix: bool,
}

impl Default for NmeaState {
    fn default() -> Self {
        Self {
            fix: GpsFix::None,
            latitude: None,
            longitude: None,
            altitude_m: None,
            satellites: 0,
            hdop: None,
            speed_kmh: None,
            course_deg: None,
            gga_fix: false,
        }
    }
}

/// Apply one line. Returns true when the line was a GGA or RMC that passed its checksum.
pub fn apply_sentence(state: &mut NmeaState, line: &str) -> bool {
    let Some(body) = sentence_body(line.trim()) else {
        return false;
    };
    let fields: Vec<&str> = body.split(',').collect();
    if fields.is_empty() {
        return false;
    }
    match sentence_kind(fields[0]) {
        Some("GGA") => apply_gga(state, &fields),
        Some("RMC") => apply_rmc(state, &fields),
        _ => false,
    }
}

fn sentence_body(line: &str) -> Option<&str> {
    let rest = line.strip_prefix('$')?;
    if let Some((body, sum)) = rest.split_once('*') {
        let expect = u8::from_str_radix(sum.get(..2)?, 16).ok()?;
        if checksum(body) != expect {
            return None;
        }
        Some(body)
    } else {
        Some(rest)
    }
}

fn checksum(body: &str) -> u8 {
    body.bytes().fold(0u8, |acc, byte| acc ^ byte)
}

fn sentence_kind(talker: &str) -> Option<&'static str> {
    let kind = if talker.len() >= 5 { &talker[2..] } else { talker };
    match kind {
        "GGA" => Some("GGA"),
        "RMC" => Some("RMC"),
        _ => None,
    }
}

fn apply_gga(state: &mut NmeaState, fields: &[&str]) -> bool {
    if fields.len() < 10 {
        return false;
    }
    let quality: u8 = fields[6].parse().unwrap_or(0);
    state.satellites = fields[7].parse().unwrap_or(0);
    state.hdop = parse_f32(fields[8]);
    state.gga_fix = true;
    if quality == 0 {
        state.fix = GpsFix::None;
        return true;
    }
    if let Some((lat, lon)) = position(fields[2], fields[3], fields[4], fields[5]) {
        state.latitude = Some(lat);
        state.longitude = Some(lon);
    }
    state.altitude_m = parse_f32(fields[9]);
    state.fix = match quality {
        6 => GpsFix::DeadReckoning,
        1 | 2 if state.altitude_m.is_some() => GpsFix::Fix3D,
        1 | 2 => GpsFix::Fix2D,
        _ => GpsFix::Fix3D,
    };
    true
}

fn apply_rmc(state: &mut NmeaState, fields: &[&str]) -> bool {
    if fields.len() < 9 {
        return false;
    }
    if fields[2] != "A" {
        state.speed_kmh = None;
        state.course_deg = None;
        if !state.gga_fix {
            state.fix = GpsFix::None;
        }
        return true;
    }
    if let Some((lat, lon)) = position(fields[3], fields[4], fields[5], fields[6]) {
        state.latitude = Some(lat);
        state.longitude = Some(lon);
    }
    if let Some(knots) = parse_f32(fields[7]) {
        state.speed_kmh = Some(knots * 1.852);
    }
    state.course_deg = parse_f32(fields[8]);
    if !state.gga_fix || state.fix == GpsFix::None {
        state.fix = GpsFix::Fix2D;
    }
    true
}

fn parse_f32(raw: &str) -> Option<f32> {
    if raw.is_empty() {
        None
    } else {
        raw.parse().ok()
    }
}

fn position(lat: &str, ns: &str, lon: &str, ew: &str) -> Option<(f64, f64)> {
    Some((coord(lat, ns)?, coord(lon, ew)?))
}

fn coord(raw: &str, hemi: &str) -> Option<f64> {
    if raw.is_empty() {
        return None;
    }
    let value: f64 = raw.parse().ok()?;
    let degrees = (value / 100.0).floor();
    let minutes = value - degrees * 100.0;
    if !(0.0..60.0).contains(&minutes) {
        return None;
    }
    let mut decimal = degrees + minutes / 60.0;
    if hemi.eq_ignore_ascii_case("S") || hemi.eq_ignore_ascii_case("W") {
        decimal = -decimal;
    }
    Some(decimal)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_sum(body: &str) -> String {
        format!("${body}*{:02X}", checksum(body))
    }

    #[test]
    fn gga_parses_a_3d_fix() {
        let line = with_sum("GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,");
        let mut state = NmeaState::default();
        assert!(apply_sentence(&mut state, &line));
        assert_eq!(state.fix, GpsFix::Fix3D);
        assert_eq!(state.satellites, 8);
        assert_eq!(state.hdop, Some(0.9));
        let lat = state.latitude.unwrap();
        let lon = state.longitude.unwrap();
        assert!((lat - 48.1173).abs() < 1e-4, "{lat}");
        assert!((lon - 11.516666).abs() < 1e-4, "{lon}");
        assert_eq!(state.altitude_m, Some(545.4));
    }

    #[test]
    fn rmc_converts_knots_and_southern_hemisphere() {
        let line = with_sum("GNRMC,123519,A,2742.006,S,15301.266,E,022.4,084.4,230394,003.1,W");
        let mut state = NmeaState::default();
        assert!(apply_sentence(&mut state, &line));
        assert_eq!(state.fix, GpsFix::Fix2D);
        let lat = state.latitude.unwrap();
        assert!(lat < 0.0, "{lat}");
        assert!((lat + 27.7001).abs() < 1e-3, "{lat}");
        let speed = state.speed_kmh.unwrap();
        assert!((speed - 22.4 * 1.852).abs() < 1e-3, "{speed}");
        assert_eq!(state.course_deg, Some(84.4));
    }

    #[test]
    fn bad_checksum_is_ignored() {
        let mut state = NmeaState::default();
        assert!(!apply_sentence(
            &mut state,
            "$GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,*00"
        ));
        assert_eq!(state.fix, GpsFix::None);
        assert!(state.latitude.is_none());
    }

    #[test]
    fn void_gga_clears_the_fix_and_keeps_the_last_position() {
        let mut state = NmeaState::default();
        assert!(apply_sentence(
            &mut state,
            &with_sum("GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,")
        ));
        assert!(apply_sentence(
            &mut state,
            &with_sum("GPGGA,123520,,,,,,0,00,,,M,,M,,")
        ));
        assert_eq!(state.fix, GpsFix::None);
        assert!(state.latitude.is_some());
    }
}
