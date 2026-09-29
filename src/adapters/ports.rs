//! Shared claim list so the GPS probe and the mesh probe do not open the same COM port.

use std::collections::HashSet;
use std::sync::Mutex;

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
    match serialport::available_ports() {
        Ok(ports) => ports
            .into_iter()
            .filter(|port| matches!(port.port_type, serialport::SerialPortType::UsbPort(_)))
            .map(|port| port.port_name)
            .collect(),
        Err(err) => {
            tracing::debug!("serial scan: {err}");
            Vec::new()
        }
    }
}

pub fn is_timeout(err: &std::io::Error) -> bool {
    if err.kind() == std::io::ErrorKind::TimedOut {
        return true;
    }
    let msg = err.to_string().to_ascii_lowercase();
    msg.contains("timed out") || msg.contains("timeout")
}
