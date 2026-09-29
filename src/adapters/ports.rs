//! Shared claim list so the GPS probe and the mesh probe do not open the same COM port.

use std::collections::HashSet;
use std::sync::Mutex;

use crate::core::hardware::{Attachment, GpsStatus, MeshStatus, RadioStatus, Readiness};

pub struct UsbSerial {
    pub name: String,
    pub label: String,
}

#[derive(Default)]
pub struct PortClaims {
    names: Mutex<HashSet<String>>,
}

impl PortClaims {
    pub fn try_claim(&self, name: &str) -> bool {
        let mut names = self.names.lock().unwrap();
        if names.contains(name) {
            false
        } else {
            names.insert(name.to_string());
            true
        }
    }

    pub fn release(&self, name: &str) {
        self.names.lock().unwrap().remove(name);
    }
}

pub fn usb_serial_ports() -> Vec<String> {
    usb_serial_devices().into_iter().map(|dev| dev.name).collect()
}

pub fn usb_serial_devices() -> Vec<UsbSerial> {
    match serialport::available_ports() {
        Ok(ports) => ports
            .into_iter()
            .filter_map(|port| match port.port_type {
                serialport::SerialPortType::UsbPort(info) => {
                    let label = info
                        .product
                        .or(info.manufacturer)
                        .filter(|text| !text.trim().is_empty())
                        .unwrap_or_else(|| "USB serial".into());
                    Some(UsbSerial {
                        name: port.port_name,
                        label,
                    })
                }
                _ => None,
            })
            .collect(),
        Err(err) => {
            tracing::debug!("serial scan: {err}");
            Vec::new()
        }
    }
}

pub fn port_label(name: &str) -> String {
    usb_serial_devices()
        .into_iter()
        .find(|dev| dev.name == name)
        .map(|dev| dev.label)
        .unwrap_or_else(|| "USB serial".into())
}

/// Assigned panels first, then USB serial ports the shell has not claimed for one.
pub fn attachments(gps: &GpsStatus, radio: &RadioStatus, mesh: &MeshStatus) -> Vec<Attachment> {
    let mut rows = Vec::new();
    let mut used = Vec::new();
    if gps.readiness != Readiness::NotPresent && gps.device != "—" {
        used.push(gps.device.clone());
        rows.push(Attachment {
            panel: "NAV".into(),
            text: format!("Using {} for NAV", describe(&gps.device, &gps.label)),
        });
    }
    if radio.readiness != Readiness::NotPresent && radio.device != "—" {
        rows.push(Attachment {
            panel: "RADIO".into(),
            text: format!("Using {} for RADIO", radio.device),
        });
    }
    if mesh.readiness != Readiness::NotPresent && mesh.port != "—" {
        used.push(mesh.port.clone());
        rows.push(Attachment {
            panel: "MESH".into(),
            text: format!("Using {} for MESH", describe(&mesh.port, &mesh.label)),
        });
    }
    for dev in usb_serial_devices() {
        if used.iter().any(|name| name == &dev.name) {
            continue;
        }
        rows.push(Attachment {
            panel: "USB".into(),
            text: format!("{} · {} is plugged in", dev.name, dev.label),
        });
    }
    rows
}

fn describe(port: &str, label: &str) -> String {
    if label.is_empty() || label == "—" {
        port.to_string()
    } else {
        format!("{port} · {label}")
    }
}

pub fn is_timeout(err: &std::io::Error) -> bool {
    if err.kind() == std::io::ErrorKind::TimedOut {
        return true;
    }
    let msg = err.to_string().to_ascii_lowercase();
    msg.contains("timed out") || msg.contains("timeout")
}
