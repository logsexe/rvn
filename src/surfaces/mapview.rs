//! Slippy map for NAV.
//! Tiles are fetched one at a time for the view on screen, then kept on disk.
//! OpenStreetMap's tile policy asks for a named agent, a cache, and no bulk download.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::Arc;
use std::time::{Duration, Instant};

const TILE: f64 = 256.0;
const UA: &str = "RVN/0.1 (field shell; https://github.com/logsexe/rvn)";

pub struct MapImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

struct Hone {
    started: Instant,
    from_z: f64,
    to_z: f64,
    duration: Duration,
}

pub struct MapView {
    lat: f64,
    lon: f64,
    zoom: f64,
    follow: bool,
    fix: Option<(f64, f64)>,
    had_fix: bool,
    hone: Option<Hone>,
    width: u32,
    height: u32,
    tiles: HashMap<(u8, u32, u32), Arc<[u8]>>,
    pending: HashMap<(u8, u32, u32), Instant>,
    cache: PathBuf,
    tx: Option<SyncSender<(u8, u32, u32)>>,
    dirty: bool,
    missing: usize,
    track: Vec<(f64, f64)>,
    marks: Vec<(f64, f64)>,
}

impl MapView {
    pub fn new() -> Self {
        Self::create(true)
    }

    pub fn offline() -> Self {
        Self::create(false)
    }

    fn create(online: bool) -> Self {
        let cache = cache_dir();
        let tx = if online {
            let (tx, rx) = mpsc::sync_channel(24);
            let dir = cache.clone();
            if std::thread::Builder::new()
                .name("rvn-map".into())
                .spawn(move || fetch_loop(rx, dir))
                .is_err()
            {
                None
            } else {
                Some(tx)
            }
        } else {
            None
        };
        Self {
            lat: -25.0,
            lon: 134.0,
            zoom: 3.2,
            follow: true,
            fix: None,
            had_fix: false,
            hone: None,
            width: 640,
            height: 360,
            tiles: HashMap::new(),
            pending: HashMap::new(),
            cache,
            tx,
            dirty: true,
            missing: 0,
            track: Vec::new(),
            marks: Vec::new(),
        }
    }

    /// The walked path and the dropped waypoints. Unchanged input does not redraw.
    pub fn set_marks(&mut self, track: &[(f64, f64)], marks: &[(f64, f64)]) {
        if self.track == track && self.marks == marks {
            return;
        }
        self.track.clear();
        self.track.extend_from_slice(track);
        self.marks.clear();
        self.marks.extend_from_slice(marks);
        self.dirty = true;
    }

    pub fn set_viewport(&mut self, width: f32, height: f32) {
        let width = width.round().clamp(64.0, 1920.0) as u32;
        let height = height.round().clamp(64.0, 1200.0) as u32;
        if width != self.width || height != self.height {
            self.width = width;
            self.height = height;
            self.dirty = true;
        }
    }

    pub fn note_fix(&mut self, lat: f64, lon: f64) {
        let lat = lat.clamp(-85.0, 85.0);
        let lon = wrap_lon(lon);
        let first = !self.had_fix;
        self.had_fix = true;
        self.fix = Some((lat, lon));
        if self.follow {
            if (self.lat - lat).abs() > 1e-7 || (self.lon - lon).abs() > 1e-7 {
                self.dirty = true;
            }
            self.lat = lat;
            self.lon = lon;
            if first {
                self.zoom = 2.8;
                self.hone = Some(Hone {
                    started: Instant::now(),
                    from_z: 2.8,
                    to_z: 15.0,
                    duration: Duration::from_millis(3400),
                });
                self.dirty = true;
            }
        }
    }

    pub fn pan(&mut self, dx: f64, dy: f64) {
        if dx.abs() + dy.abs() < 0.4 {
            return;
        }
        self.follow = false;
        self.hone = None;
        let cx = world_x(self.lon, self.zoom) * TILE - dx;
        let cy = world_y(self.lat, self.zoom) * TILE - dy;
        self.lon = wrap_lon(x_to_lon(cx / TILE, self.zoom));
        self.lat = y_to_lat(cy / TILE, self.zoom).clamp(-85.0, 85.0);
        self.dirty = true;
    }

    pub fn zoom_steps(&mut self, steps: i32) {
        if steps == 0 {
            return;
        }
        self.follow = false;
        self.hone = None;
        self.zoom = (self.zoom + f64::from(steps)).clamp(2.0, 16.0);
        self.dirty = true;
    }

    pub fn recenter(&mut self) {
        let Some((lat, lon)) = self.fix else {
            return;
        };
        self.follow = true;
        self.lat = lat;
        self.lon = lon;
        let from = self.zoom;
        if from < 14.5 {
            self.hone = Some(Hone {
                started: Instant::now(),
                from_z: from,
                to_z: 15.0,
                duration: Duration::from_millis(1600),
            });
        }
        self.dirty = true;
    }

    pub fn tick(&mut self) {
        let Some(hone) = &self.hone else {
            return;
        };
        let t = hone.started.elapsed().as_secs_f64() / hone.duration.as_secs_f64();
        if t >= 1.0 {
            self.zoom = hone.to_z;
            self.hone = None;
        } else {
            let eased = 1.0 - (1.0 - t).powi(3);
            self.zoom = hone.from_z + (hone.to_z - hone.from_z) * eased;
        }
        self.dirty = true;
    }

    pub fn animating(&self) -> bool {
        self.hone.is_some()
    }

    pub fn following(&self) -> bool {
        self.follow && self.fix.is_some()
    }

    pub fn caption(&self) -> String {
        let zoom = format!("zoom {:.0}", self.zoom.round());
        let base = if self.fix.is_none() {
            format!("Waiting for a sky fix · {zoom}")
        } else if self.following() {
            format!("Following · {zoom}")
        } else {
            format!("Looking around · {zoom}")
        };
        if self.missing > 0 {
            format!("{base} · loading roads")
        } else {
            base
        }
    }

    pub fn render(&mut self) -> Option<MapImage> {
        self.pump();
        self.enqueue();
        if !self.dirty {
            return None;
        }
        self.dirty = false;
        Some(self.draw())
    }

    fn draw(&mut self) -> MapImage {
        let width = self.width.max(1);
        let height = self.height.max(1);
        let mut rgba = vec![0u8; width as usize * height as usize * 4];
        fill(&mut rgba, 12, 18, 24, 255);
        draw_graticule(&mut rgba, width, height, self.lat, self.lon, self.zoom);
        let z = self.zoom.floor().clamp(2.0, 16.0) as u8;
        let keys = visible_tiles(self.lat, self.lon, self.zoom, width, height, z);
        self.missing = 0;
        for (tx, ty) in keys {
            let n = 1i32 << z;
            if ty < 0 || ty >= n {
                continue;
            }
            let wrapped = (z, tx.rem_euclid(n) as u32, ty as u32);
            let screen = tile_origin(tx, ty, z, self.lat, self.lon, self.zoom, width, height);
            if let Some(px) = self.tiles.get(&wrapped) {
                blit(&mut rgba, width, height, px, screen.0, screen.1, screen.2);
            } else {
                self.missing += 1;
            }
        }
        draw_track(
            &mut rgba,
            width,
            height,
            self.zoom,
            self.lat,
            self.lon,
            &self.track,
        );
        for (lat, lon) in &self.marks {
            let (x, y) = project(*lat, *lon, self.zoom, self.lat, self.lon, width, height);
            if x < -8.0 || y < -8.0 || x >= f64::from(width) + 8.0 || y >= f64::from(height) + 8.0 {
                continue;
            }
            paint_dot(&mut rgba, width, height, x, y, 4, 251, 191, 36);
        }
        if let Some((lat, lon)) = self.fix {
            let (x, y) = project(lat, lon, self.zoom, self.lat, self.lon, width, height);
            let inside = x >= 0.0 && y >= 0.0 && x < f64::from(width) && y < f64::from(height);
            let mx = x.clamp(8.0, f64::from(width) - 9.0);
            let my = y.clamp(8.0, f64::from(height) - 9.0);
            if inside {
                paint_dot(&mut rgba, width, height, mx, my, 6, 94, 234, 212);
            } else {
                paint_dot(&mut rgba, width, height, mx, my, 5, 251, 191, 36);
            }
        }
        MapImage { width, height, rgba }
    }

    fn enqueue(&mut self) {
        let z = self.zoom.floor().clamp(2.0, 16.0) as u8;
        let wanted = visible_tiles(self.lat, self.lon, self.zoom, self.width, self.height, z);
        let mut budget = 4;
        let n = 1i32 << z;
        for (tx, ty) in wanted {
            if budget == 0 {
                break;
            }
            if ty < 0 || ty >= n {
                continue;
            }
            if self.consider(z, tx.rem_euclid(n) as u32, ty as u32) {
                budget -= 1;
            }
        }
        if self.hone.is_some() {
            let street = 1i32 << 15;
            for (tx, ty) in neighborhood(self.lat, self.lon, 15) {
                if budget == 0 {
                    break;
                }
                if ty < 0 || ty >= street {
                    continue;
                }
                if self.consider(15, tx.rem_euclid(street) as u32, ty as u32) {
                    budget -= 1;
                }
            }
        }
        if self.tiles.len() > 120 {
            let before = self.tiles.len();
            self.tiles.retain(|key, _| {
                key.0.saturating_add(2) >= z && key.0 <= z.saturating_add(1)
            });
            if self.tiles.len() != before {
                self.dirty = true;
            }
        }
    }

    fn consider(&mut self, z: u8, x: u32, y: u32) -> bool {
        let key = (z, x, y);
        if self.tiles.contains_key(&key) || self.pending.contains_key(&key) {
            return false;
        }
        let path = tile_path(&self.cache, z, x, y);
        if path.exists() {
            if let Some(px) = decode_tile(&path) {
                self.tiles.insert(key, px);
                self.dirty = true;
            }
            return false;
        }
        let Some(tx) = &self.tx else {
            return false;
        };
        if tx.try_send(key).is_ok() {
            self.pending.insert(key, Instant::now());
            return true;
        }
        false
    }

    fn pump(&mut self) {
        let ready: Vec<_> = self
            .pending
            .iter()
            .filter(|(key, _)| tile_path(&self.cache, key.0, key.1, key.2).exists())
            .map(|(key, _)| *key)
            .collect();
        for key in ready {
            self.pending.remove(&key);
            let path = tile_path(&self.cache, key.0, key.1, key.2);
            if let Some(px) = decode_tile(&path) {
                self.tiles.insert(key, px);
                self.dirty = true;
            }
        }
        let stale: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, at)| at.elapsed() > Duration::from_secs(20))
            .map(|(key, _)| *key)
            .collect();
        for key in stale {
            self.pending.remove(&key);
        }
    }
}

fn neighborhood(lat: f64, lon: f64, z: u8) -> Vec<(i32, i32)> {
    let tx = world_x(lon, f64::from(z)).floor() as i32;
    let ty = world_y(lat, f64::from(z)).floor() as i32;
    let mut out = Vec::new();
    for dy in -1..=1 {
        for dx in -1..=1 {
            out.push((tx + dx, ty + dy));
        }
    }
    out
}

fn visible_tiles(lat: f64, lon: f64, zoom: f64, width: u32, height: u32, z: u8) -> Vec<(i32, i32)> {
    let scale = 2f64.powf(zoom - f64::from(z));
    let cx = world_x(lon, zoom) * TILE;
    let cy = world_y(lat, zoom) * TILE;
    let left = (cx - f64::from(width) / 2.0) / TILE / scale;
    let right = (cx + f64::from(width) / 2.0) / TILE / scale;
    let top = (cy - f64::from(height) / 2.0) / TILE / scale;
    let bottom = (cy + f64::from(height) / 2.0) / TILE / scale;
    let mut out = Vec::new();
    let x0 = left.floor() as i32 - 1;
    let x1 = right.ceil() as i32 + 1;
    let y0 = top.floor() as i32 - 1;
    let y1 = bottom.ceil() as i32 + 1;
    for ty in y0..=y1 {
        for tx in x0..=x1 {
            out.push((tx, ty));
        }
    }
    out
}

fn tile_origin(
    tx: i32,
    ty: i32,
    z: u8,
    lat: f64,
    lon: f64,
    zoom: f64,
    width: u32,
    height: u32,
) -> (f64, f64, f64) {
    let scale = 2f64.powf(zoom - f64::from(z));
    let size = TILE * scale;
    let cx = world_x(lon, zoom) * TILE;
    let cy = world_y(lat, zoom) * TILE;
    let origin_x = f64::from(tx) * size;
    let origin_y = f64::from(ty) * size;
    let screen_x = f64::from(width) / 2.0 + (origin_x - cx);
    let screen_y = f64::from(height) / 2.0 + (origin_y - cy);
    (screen_x, screen_y, size)
}

pub fn world_x(lon: f64, zoom: f64) -> f64 {
    (lon + 180.0) / 360.0 * 2f64.powf(zoom)
}

pub fn world_y(lat: f64, zoom: f64) -> f64 {
    let lat = lat.clamp(-85.05112878, 85.05112878).to_radians();
    let merc = (lat.tan() + 1.0 / lat.cos()).ln();
    (1.0 - merc / std::f64::consts::PI) / 2.0 * 2f64.powf(zoom)
}

pub fn x_to_lon(x: f64, zoom: f64) -> f64 {
    x / 2f64.powf(zoom) * 360.0 - 180.0
}

pub fn y_to_lat(y: f64, zoom: f64) -> f64 {
    let merc = std::f64::consts::PI * (1.0 - 2.0 * y / 2f64.powf(zoom));
    merc.sinh().atan().to_degrees()
}

fn project(lat: f64, lon: f64, zoom: f64, center_lat: f64, center_lon: f64, width: u32, height: u32) -> (f64, f64) {
    let cx = world_x(center_lon, zoom) * TILE;
    let cy = world_y(center_lat, zoom) * TILE;
    let x = world_x(lon, zoom) * TILE;
    let y = world_y(lat, zoom) * TILE;
    (
        f64::from(width) / 2.0 + (x - cx),
        f64::from(height) / 2.0 + (y - cy),
    )
}

fn wrap_lon(lon: f64) -> f64 {
    let mut lon = (lon + 180.0) % 360.0;
    if lon < 0.0 {
        lon += 360.0;
    }
    lon - 180.0
}

fn fill(rgba: &mut [u8], r: u8, g: u8, b: u8, a: u8) {
    for px in rgba.chunks_exact_mut(4) {
        px[0] = r;
        px[1] = g;
        px[2] = b;
        px[3] = a;
    }
}

fn draw_graticule(rgba: &mut [u8], width: u32, height: u32, lat: f64, lon: f64, zoom: f64) {
    let step = if zoom < 4.0 {
        30.0
    } else if zoom < 7.0 {
        10.0
    } else if zoom < 10.0 {
        2.0
    } else if zoom < 13.0 {
        0.5
    } else {
        0.1
    };
    let (west, north) = unproject(0.0, 0.0, lat, lon, zoom, width, height);
    let (east, south) = unproject(f64::from(width), f64::from(height), lat, lon, zoom, width, height);
    let mut lon_line = (west / step).floor() * step;
    let lon_stop = east + step;
    while lon_line <= lon_stop {
        let (x1, y1) = project(north, lon_line, zoom, lat, lon, width, height);
        let (x2, y2) = project(south, lon_line, zoom, lat, lon, width, height);
        draw_line(rgba, width, height, x1, y1, x2, y2, 36, 48, 68);
        lon_line += step;
    }
    let mut lat_line = (south / step).floor() * step;
    let lat_stop = north + step;
    while lat_line <= lat_stop {
        let (x1, y1) = project(lat_line, west, zoom, lat, lon, width, height);
        let (x2, y2) = project(lat_line, east, zoom, lat, lon, width, height);
        draw_line(rgba, width, height, x1, y1, x2, y2, 36, 48, 68);
        lat_line += step;
    }
}

fn unproject(x: f64, y: f64, lat: f64, lon: f64, zoom: f64, width: u32, height: u32) -> (f64, f64) {
    let cx = world_x(lon, zoom) * TILE;
    let cy = world_y(lat, zoom) * TILE;
    let wx = (cx + (x - f64::from(width) / 2.0)) / TILE;
    let wy = (cy + (y - f64::from(height) / 2.0)) / TILE;
    (x_to_lon(wx, zoom), y_to_lat(wy, zoom))
}

fn draw_track(
    rgba: &mut [u8],
    width: u32,
    height: u32,
    zoom: f64,
    lat: f64,
    lon: f64,
    track: &[(f64, f64)],
) {
    for pair in track.windows(2) {
        let (x1, y1) = project(pair[0].0, pair[0].1, zoom, lat, lon, width, height);
        let (x2, y2) = project(pair[1].0, pair[1].1, zoom, lat, lon, width, height);
        if (x2 - x1).hypot(y2 - y1) > 900.0 {
            continue;
        }
        draw_line(rgba, width, height, x1, y1 + 1.0, x2, y2 + 1.0, 6, 10, 14);
        draw_line(rgba, width, height, x1, y1, x2, y2, 45, 212, 191);
    }
}

fn draw_line(rgba: &mut [u8], width: u32, height: u32, x0: f64, y0: f64, x1: f64, y1: f64, r: u8, g: u8, b: u8) {
    let mut x0 = x0.round() as i32;
    let mut y0 = y0.round() as i32;
    let x1 = x1.round() as i32;
    let y1 = y1.round() as i32;
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    let w = width as i32;
    let h = height as i32;
    for _ in 0..2000 {
        if x0 >= 0 && y0 >= 0 && x0 < w && y0 < h {
            let i = (y0 as usize * width as usize + x0 as usize) * 4;
            rgba[i] = r;
            rgba[i + 1] = g;
            rgba[i + 2] = b;
            rgba[i + 3] = 255;
        }
        if x0 == x1 && y0 == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
    }
}

fn paint_dot(rgba: &mut [u8], width: u32, height: u32, cx: f64, cy: f64, radius: i32, r: u8, g: u8, b: u8) {
    let w = width as i32;
    let h = height as i32;
    let x0 = cx.round() as i32;
    let y0 = cy.round() as i32;
    for y in (y0 - radius - 1)..=(y0 + radius + 1) {
        for x in (x0 - radius - 1)..=(x0 + radius + 1) {
            if x < 0 || y < 0 || x >= w || y >= h {
                continue;
            }
            let d2 = (x - x0).pow(2) + (y - y0).pow(2);
            let i = (y as usize * width as usize + x as usize) * 4;
            if d2 <= radius * radius {
                rgba[i] = r;
                rgba[i + 1] = g;
                rgba[i + 2] = b;
                rgba[i + 3] = 255;
            } else if d2 <= (radius + 1) * (radius + 1) {
                rgba[i] = 8;
                rgba[i + 1] = 10;
                rgba[i + 2] = 14;
                rgba[i + 3] = 255;
            }
        }
    }
}

fn blit(dst: &mut [u8], dst_w: u32, dst_h: u32, tile: &[u8], dest_x: f64, dest_y: f64, dest_size: f64) {
    if tile.len() < 256 * 256 * 4 {
        return;
    }
    let size = dest_size.max(1.0);
    let x0 = dest_x.floor() as i32;
    let y0 = dest_y.floor() as i32;
    let x1 = (dest_x + size).ceil() as i32;
    let y1 = (dest_y + size).ceil() as i32;
    let dw = dst_w as i32;
    let dh = dst_h as i32;
    for y in y0.max(0)..y1.min(dh) {
        let ty = ((y as f64 - dest_y) / size * 256.0).clamp(0.0, 255.0) as usize;
        for x in x0.max(0)..x1.min(dw) {
            let tx = ((x as f64 - dest_x) / size * 256.0).clamp(0.0, 255.0) as usize;
            let si = (ty * 256 + tx) * 4;
            let di = (y as usize * dst_w as usize + x as usize) * 4;
            dst[di..di + 4].copy_from_slice(&tile[si..si + 4]);
        }
    }
}

fn cache_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
        return PathBuf::from(xdg).join("rvn").join("map");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".cache").join("rvn").join("map");
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        return PathBuf::from(local).join("rvn").join("map");
    }
    PathBuf::from("map-cache")
}

fn tile_path(cache: &Path, z: u8, x: u32, y: u32) -> PathBuf {
    cache.join(z.to_string()).join(x.to_string()).join(format!("{y}.png"))
}

fn decode_tile(path: &Path) -> Option<Arc<[u8]>> {
    let bytes = std::fs::read(path).ok()?;
    let image = image::load_from_memory(&bytes).ok()?.to_rgba8();
    if image.width() != 256 || image.height() != 256 {
        return None;
    }
    Some(Arc::from(image.into_raw().into_boxed_slice()))
}

fn fetch_loop(rx: Receiver<(u8, u32, u32)>, cache: PathBuf) {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(8))
        .build();
    while let Ok((z, x, y)) = rx.recv() {
        let path = tile_path(&cache, z, x, y);
        if path.exists() {
            continue;
        }
        let url = format!("https://tile.openstreetmap.org/{z}/{x}/{y}.png");
        let fetched = agent
            .get(&url)
            .set("User-Agent", UA)
            .set("Accept", "image/png")
            .call();
        match fetched {
            Ok(response) if response.status() == 200 => {
                let mut bytes = Vec::new();
                if response.into_reader().take(800_000).read_to_end(&mut bytes).is_ok()
                    && bytes.starts_with(&[0x89, b'P', b'N', b'G'])
                {
                    if let Some(parent) = path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let _ = std::fs::write(&path, bytes);
                }
            }
            _ => {}
        }
        // One tile at a time. The tile server asks clients not to burst.
        std::thread::sleep(Duration::from_millis(600));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mercator_roundtrip() {
        let zoom = 12.0;
        let lon = x_to_lon(world_x(153.0211, zoom), zoom);
        let lat = y_to_lat(world_y(-27.4701, zoom), zoom);
        assert!((lon - 153.0211).abs() < 1e-6);
        assert!((lat + 27.4701).abs() < 1e-6);
    }

    #[test]
    fn center_projects_to_the_middle() {
        let (x, y) = project(-27.47, 153.02, 10.0, -27.47, 153.02, 400, 300);
        assert!((x - 200.0).abs() < 0.6, "{x}");
        assert!((y - 150.0).abs() < 0.6, "{y}");
    }

    #[test]
    fn pan_leaves_follow_and_shifts_west_when_dragged_right() {
        let mut map = MapView::offline();
        map.note_fix(-27.47, 153.02);
        map.hone = None;
        map.follow = true;
        map.lat = -27.47;
        map.lon = 153.02;
        map.zoom = 12.0;
        let before = map.lon;
        map.pan(180.0, 0.0);
        assert!(map.lon < before);
        assert!(!map.follow);
    }

    #[test]
    fn track_crosses_the_center() {
        let mut map = MapView::offline();
        map.set_viewport(200.0, 120.0);
        map.lat = -27.47;
        map.lon = 153.02;
        map.zoom = 8.0;
        map.follow = false;
        map.hone = None;
        map.set_marks(&[(-27.47, 152.5), (-27.47, 153.5)], &[]);
        let frame = map.render().expect("frame");
        let i = (frame.height / 2 * frame.width + frame.width / 2) as usize * 4;
        assert!(frame.rgba[i + 1] > 150, "g {}", frame.rgba[i + 1]);
    }

    #[test]
    fn waypoint_marks_its_place() {
        let mut map = MapView::offline();
        map.set_viewport(200.0, 120.0);
        map.lat = -27.47;
        map.lon = 153.02;
        map.zoom = 10.0;
        map.follow = false;
        map.hone = None;
        let mark = (-27.50, 153.05);
        map.set_marks(&[], &[mark]);
        let frame = map.render().expect("frame");
        let (x, y) = project(mark.0, mark.1, 10.0, -27.47, 153.02, frame.width, frame.height);
        let xi = x.round() as usize;
        let yi = y.round() as usize;
        assert!(xi < frame.width as usize && yi < frame.height as usize);
        let i = (yi * frame.width as usize + xi) * 4;
        assert!(frame.rgba[i] > 200, "r {}", frame.rgba[i]);
    }

    #[test]
    fn marker_sits_on_the_center_when_the_view_is_on_the_fix() {
        let mut map = MapView::offline();
        map.set_viewport(200.0, 120.0);
        map.note_fix(-27.47, 153.02);
        map.hone = None;
        map.follow = true;
        map.lat = -27.47;
        map.lon = 153.02;
        map.zoom = 8.0;
        map.dirty = true;
        let frame = map.render().expect("frame");
        let x = frame.width / 2;
        let y = frame.height / 2;
        let i = (y * frame.width + x) as usize * 4;
        assert!(frame.rgba[i + 1] > 180, "g {}", frame.rgba[i + 1]);
    }
}
