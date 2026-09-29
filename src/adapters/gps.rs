//! USB GNSS reader. Scans USB serial ports, locks the baud that emits NMEA, and
//! publishes a GpsStatus. A missing receiver stays NotPresent.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serialport::SerialPort;

use crate::core::hardware::{GpsFix, GpsStatus, Readiness};

use super::nmea::{apply_sentence, NmeaState};
use super::ports::{is_timeout, usb_serial_ports, PortClaims};

struct GpsInner {
    status: GpsStatus,
    last_sentence: Option<Instant>,
    open: bool,
}

impl GpsInner {
    fn publish(&mut self) {
        self.status.age_ms = self
            .last_sentence
            .map(|at| at.elapsed().as_millis() as u64);
        self.status.readiness = if !self.open {
            Readiness::NotPresent
        } else {
            match self.last_sentence {
                Some(at) if at.elapsed() > Duration::from_secs(5) => Readiness::Degraded,
                Some(_) => match self.status.fix {
                    GpsFix::Fix2D | GpsFix::Fix3D => Readiness::Active,
                    _ => Readiness::Ready,
                },
                None => Readiness::Degraded,
            }
        };
    }
}

pub struct GpsAdapter {
    inner: Arc<Mutex<GpsInner>>,
}

impl GpsAdapter {
    pub fn start(claims: Arc<PortClaims>, gate: Arc<AtomicBool>) -> Self {
        let inner = Arc::new(Mutex::new(GpsInner {
            status: GpsStatus::default(),
            last_sentence: None,
            open: false,
        }));
        let worker = inner.clone();
        if let Err(err) = std::thread::Builder::new()
            .name("rvn-gps".into())
            .spawn(move || gps_loop(worker, claims, gate))
        {
            tracing::warn!("GPS thread: {err}");
        }
        Self { inner }
    }

    pub fn snapshot(&self) -> GpsStatus {
        let mut inner = self.inner.lock().unwrap();
        inner.publish();
        inner.status.clone()
    }
}

fn gps_loop(slot: Arc<Mutex<GpsInner>>, claims: Arc<PortClaims>, gate: Arc<AtomicBool>) {
    loop {
        match find_gps(&claims) {
            Some((name, mut port)) => {
                gate.store(true, Ordering::SeqCst);
                tracing::info!("GPS linked on {name}");
                {
                    let mut inner = slot.lock().unwrap();
                    inner.open = true;
                    inner.publish();
                }
                read_gps(&mut *port, &slot);
                claims.release(&name);
                let mut inner = slot.lock().unwrap();
                *inner = GpsInner {
                    status: GpsStatus::default(),
                    last_sentence: None,
                    open: false,
                };
                tracing::info!("GPS lost {name}");
            }
            None => {
                gate.store(true, Ordering::SeqCst);
                std::thread::sleep(Duration::from_secs(3));
            }
        }
    }
}

fn find_gps(claims: &PortClaims) -> Option<(String, Box<dyn SerialPort>)> {
    for name in usb_serial_ports() {
        if !claims.try_claim(&name) {
            continue;
        }
        if let Some(port) = probe_nmea(&name) {
            return Some((name, port));
        }
        claims.release(&name);
    }
    None
}

fn probe_nmea(name: &str) -> Option<Box<dyn SerialPort>> {
    const BAUD: [u32; 4] = [9_600, 115_200, 4_800, 38_400];
    for baud in BAUD {
        let Ok(mut port) = serialport::new(name, baud)
            .timeout(Duration::from_millis(150))
            .open()
        else {
            return None;
        };
        if looks_like_nmea(&mut *port) {
            return Some(port);
        }
    }
    None
}

fn looks_like_nmea(port: &mut dyn SerialPort) -> bool {
    let start = Instant::now();
    let mut buf = [0u8; 256];
    let mut acc = String::new();
    while start.elapsed() < Duration::from_millis(600) {
        match port.read(&mut buf) {
            Ok(n) if n > 0 => {
                acc.push_str(&String::from_utf8_lossy(&buf[..n]));
                if acc.len() > 2048 {
                    let drop_to = acc.len() - 1024;
                    acc.drain(..drop_to);
                }
                if acc.contains("$G") || acc.contains("$B") {
                    return true;
                }
            }
            Ok(_) => {}
            Err(err) if is_timeout(&err) => {}
            Err(_) => return false,
        }
    }
    false
}

fn read_gps(port: &mut dyn SerialPort, slot: &Mutex<GpsInner>) {
    let mut buf = [0u8; 256];
    let mut acc = String::new();
    let mut nmea = NmeaState::default();
    loop {
        match port.read(&mut buf) {
            Ok(n) if n > 0 => {
                acc.push_str(&String::from_utf8_lossy(&buf[..n]));
                while let Some(idx) = acc.find('\n') {
                    let line: String = acc.drain(..=idx).collect();
                    if apply_sentence(&mut nmea, &line) {
                        let mut inner = slot.lock().unwrap();
                        inner.open = true;
                        inner.last_sentence = Some(Instant::now());
                        inner.status.fix = nmea.fix;
                        inner.status.satellites = nmea.satellites;
                        inner.status.latitude = nmea.latitude;
                        inner.status.longitude = nmea.longitude;
                        inner.status.altitude_m = nmea.altitude_m;
                        inner.status.speed_kmh = nmea.speed_kmh;
                        inner.status.course_deg = nmea.course_deg;
                        inner.status.hdop = nmea.hdop;
                        inner.publish();
                    }
                }
                if acc.len() > 4096 {
                    acc.clear();
                }
            }
            Ok(_) => {}
            Err(err) if is_timeout(&err) => {
                slot.lock().unwrap().publish();
            }
            Err(err) => {
                tracing::warn!("GPS read: {err}");
                return;
            }
        }
    }
}
