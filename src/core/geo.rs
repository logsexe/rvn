//! Small field-geometry helpers used by NAV.
//! Maidenhead is the grid shown on the position surface.

/// Six-character Maidenhead locator (field, square, subsquare).
pub fn maidenhead(lat: f64, lon: f64) -> String {
    let mut lon = (lon + 180.0).clamp(0.0, 359.999999);
    let mut lat = (lat + 90.0).clamp(0.0, 179.999999);

    let lon_field = (lon / 20.0).floor() as u8;
    let lat_field = (lat / 10.0).floor() as u8;
    lon -= f64::from(lon_field) * 20.0;
    lat -= f64::from(lat_field) * 10.0;

    let lon_square = (lon / 2.0).floor() as u8;
    let lat_square = lat.floor() as u8;
    lon -= f64::from(lon_square) * 2.0;
    lat -= f64::from(lat_square);

    let lon_sub = ((lon / 2.0) * 24.0).floor() as u8;
    let lat_sub = (lat * 24.0).floor() as u8;

    format!(
        "{}{}{}{}{}{}",
        field_char(lon_field),
        field_char(lat_field),
        char::from(b'0' + lon_square.min(9)),
        char::from(b'0' + lat_square.min(9)),
        sub_char(lon_sub),
        sub_char(lat_sub),
    )
}

fn field_char(n: u8) -> char {
    char::from(b'A' + n.min(17))
}

fn sub_char(n: u8) -> char {
    char::from(b'a' + n.min(23))
}

/// Great-circle distance in metres.
pub fn haversine_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const R: f64 = 6_371_000.0;
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let dp = (lat2 - lat1).to_radians();
    let dl = (lon2 - lon1).to_radians();
    let a = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * R * a.sqrt().asin()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equator_origin_is_jj00aa() {
        assert_eq!(maidenhead(0.0, 0.0), "JJ00aa");
    }

    #[test]
    fn brisbane_is_qg62() {
        let grid = maidenhead(-27.4701, 153.0211);
        assert_eq!(grid.len(), 6);
        assert!(grid.starts_with("QG62"), "{grid}");
    }

    #[test]
    fn same_point_is_zero() {
        assert!(haversine_m(-27.47, 153.02, -27.47, 153.02) < 0.01);
    }
}
