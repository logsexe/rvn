//! Meshtastic serial adapter.
//! The radio is opened only after the GPS probe has had one pass, and only on a
//! USB port that is not already claimed. Text leaves the radio when the operator
//! has armed TX and this adapter has finished the want-config handshake.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serialport::SerialPort;

use crate::core::hardware::{MeshPeer, MeshStatus, Readiness};

use super::ports::{is_timeout, port_label, usb_serial_ports, PortClaims};
use super::proto::{
    decode_from_radio, encode_heartbeat, encode_text_packet, encode_want_config, frame,
    remember_packet, Framer, RadioMessage,
};
use super::MeshInbound;

struct Heard {
    id: String,
    short_name: String,
    lat: Option<f64>,
    lon: Option<f64>,
}

struct MeshInner {
    status: MeshStatus,
    inbox: VecDeque<MeshInbound>,
    can_send: bool,
    my_node: Option<u32>,
    nodes: HashMap<u32, Heard>,
    seen: VecDeque<u32>,
    last_rx: Option<Instant>,
    configured: bool,
}

impl MeshInner {
    fn fresh() -> Self {
        Self {
            status: MeshStatus::default(),
            inbox: VecDeque::new(),
            can_send: false,
            my_node: None,
            nodes: HashMap::new(),
            seen: VecDeque::new(),
            last_rx: None,
            configured: false,
        }
    }

    fn publish(&mut self) {
        let peers = peer_list(self.my_node, &self.nodes);
        self.status.fixes = mesh_fixes(self.my_node, &self.nodes);
        self.status.peers = peers;
        self.status.nodes_heard = self.status.peers.len() as u32;
        self.status.node_id = self
            .my_node
            .map(node_label)
            .unwrap_or_else(|| "—".into());
        self.status.last_rx = self.last_rx.map(|at| {
            let secs = at.elapsed().as_secs();
            if secs < 2 {
                "live".into()
            } else {
                format!("{secs}s ago")
            }
        });
        self.status.region = "—".into();
        self.status.readiness = if !self.configured {
            Readiness::Degraded
        } else if self
            .last_rx
            .map(|at| at.elapsed() < Duration::from_secs(60))
            .unwrap_or(false)
        {
            Readiness::Active
        } else {
            Readiness::Ready
        };
        self.can_send = self.configured && self.my_node.unwrap_or(0) != 0;
    }
}

enum MeshCmd {
    Send(String),
}

struct OpenMesh {
    port: Box<dyn SerialPort>,
    prefetch: Vec<u8>,
    nonce: u32,
}

pub struct MeshAdapter {
    inner: Arc<Mutex<MeshInner>>,
    tx: Mutex<Option<Sender<MeshCmd>>>,
}

impl MeshAdapter {
    pub fn start(claims: Arc<PortClaims>, gate: Arc<AtomicBool>) -> Self {
        let inner = Arc::new(Mutex::new(MeshInner::fresh()));
        let (tx, rx) = mpsc::channel();
        let worker = inner.clone();
        if let Err(err) = std::thread::Builder::new()
            .name("rvn-mesh".into())
            .spawn(move || mesh_loop(worker, claims, gate, rx))
        {
            tracing::warn!("MESH thread: {err}");
        }
        Self {
            inner,
            tx: Mutex::new(Some(tx)),
        }
    }

    pub fn snapshot(&self) -> MeshStatus {
        let mut inner = self.inner.lock().unwrap();
        if inner.status.readiness != Readiness::NotPresent {
            inner.publish();
        }
        inner.status.clone()
    }

    pub fn take_inbox(&self) -> Vec<MeshInbound> {
        let mut inner = self.inner.lock().unwrap();
        inner.inbox.drain(..).collect()
    }

    pub fn send_text(&self, text: &str) -> bool {
        let text = text.trim();
        if text.is_empty() || text.len() > 200 {
            return false;
        }
        let ready = self.inner.lock().unwrap().can_send;
        if !ready {
            return false;
        }
        self.tx
            .lock()
            .unwrap()
            .as_ref()
            .map(|tx| tx.send(MeshCmd::Send(text.to_string())).is_ok())
            .unwrap_or(false)
    }
}

fn mesh_loop(
    slot: Arc<Mutex<MeshInner>>,
    claims: Arc<PortClaims>,
    gate: Arc<AtomicBool>,
    rx: Receiver<MeshCmd>,
) {
    while !gate.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(200));
    }
    loop {
        match find_mesh(&claims) {
            Some((name, open)) => {
                let label = port_label(&name);
                tracing::info!("MESH linked on {name} ({label})");
                {
                    let mut inner = slot.lock().unwrap();
                    inner.status.readiness = Readiness::Degraded;
                    inner.status.port = name.clone();
                    inner.status.label = label;
                }
                run_mesh(&slot, &rx, open);
                claims.release(&name);
                *slot.lock().unwrap() = MeshInner::fresh();
                tracing::info!("MESH lost {name}");
                std::thread::sleep(Duration::from_secs(3));
            }
            None => std::thread::sleep(Duration::from_secs(3)),
        }
    }
}

fn find_mesh(claims: &PortClaims) -> Option<(String, OpenMesh)> {
    for name in usb_serial_ports() {
        if !claims.try_claim(&name) {
            continue;
        }
        if let Some(open) = probe_mesh(&name) {
            return Some((name, open));
        }
        claims.release(&name);
    }
    None
}

fn probe_mesh(name: &str) -> Option<OpenMesh> {
    let mut port = serialport::new(name, 115_200)
        .timeout(Duration::from_millis(200))
        .open()
        .ok()?;
    let mut acc = Vec::new();
    if read_window(&mut *port, &mut acc, Duration::from_millis(300)) == Window::Nmea {
        return None;
    }
    let nonce = want_nonce();
    if !acc.windows(2).any(|pair| pair == [0x94, 0xC3]) {
        let hello = frame(&encode_want_config(nonce));
        if port.write_all(&hello).is_err() {
            return None;
        }
        let _ = port.flush();
        if read_window(&mut *port, &mut acc, Duration::from_millis(1200)) == Window::Nmea {
            return None;
        }
    }
    if acc.windows(2).any(|pair| pair == [0x94, 0xC3]) {
        Some(OpenMesh {
            port,
            prefetch: acc,
            nonce,
        })
    } else {
        None
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Window {
    Nmea,
    Quiet,
}

fn read_window(port: &mut dyn SerialPort, acc: &mut Vec<u8>, budget: Duration) -> Window {
    let start = Instant::now();
    let mut buf = [0u8; 256];
    while start.elapsed() < budget {
        match port.read(&mut buf) {
            Ok(n) if n > 0 => {
                acc.extend_from_slice(&buf[..n]);
                if acc.windows(2).any(|pair| pair == [b'$', b'G'] || pair == [b'$', b'B']) {
                    return Window::Nmea;
                }
            }
            Ok(_) => {}
            Err(err) if is_timeout(&err) => {}
            Err(_) => return Window::Quiet,
        }
    }
    Window::Quiet
}

fn want_nonce() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.subsec_nanos() | 1)
        .unwrap_or(1)
}

fn run_mesh(slot: &Mutex<MeshInner>, rx: &Receiver<MeshCmd>, mut open: OpenMesh) {
    let mut framer = Framer::default();
    for payload in framer.push(&open.prefetch) {
        apply_payload(slot, &payload, open.nonce);
    }
    let mut scratch = [0u8; 512];
    let mut last_heartbeat = Instant::now();
    let mut last_want = Instant::now();
    let mut heartbeat_nonce = 1u32;
    let mut next_id = 1u32;

    loop {
        while let Ok(MeshCmd::Send(text)) = rx.try_recv() {
            let from = {
                let inner = slot.lock().unwrap();
                if inner.can_send {
                    inner.my_node
                } else {
                    None
                }
            };
            if let Some(from) = from {
                let packet = frame(&encode_text_packet(from, next_id, &text));
                next_id = next_id.wrapping_add(1).max(1);
                if !write_all(&mut *open.port, &packet) {
                    return;
                }
            }
        }

        if last_heartbeat.elapsed() >= Duration::from_secs(10) {
            heartbeat_nonce = heartbeat_nonce.wrapping_add(1);
            let beat = frame(&encode_heartbeat(heartbeat_nonce));
            if !write_all(&mut *open.port, &beat) {
                return;
            }
            last_heartbeat = Instant::now();
        }

        let need_config = !slot.lock().unwrap().configured;
        if need_config && last_want.elapsed() >= Duration::from_secs(4) {
            let hello = frame(&encode_want_config(open.nonce));
            if !write_all(&mut *open.port, &hello) {
                return;
            }
            last_want = Instant::now();
        }

        match open.port.read(&mut scratch) {
            Ok(0) => {}
            Ok(n) => {
                for payload in framer.push(&scratch[..n]) {
                    apply_payload(slot, &payload, open.nonce);
                }
            }
            Err(err) if is_timeout(&err) => {}
            Err(err) => {
                tracing::warn!("MESH read: {err}");
                return;
            }
        }
    }
}

fn write_all(port: &mut dyn SerialPort, bytes: &[u8]) -> bool {
    port.write_all(bytes).is_ok() && port.flush().is_ok()
}

fn apply_payload(slot: &Mutex<MeshInner>, payload: &[u8], nonce: u32) {
    let Some(message) = decode_from_radio(payload) else {
        return;
    };
    let mut inner = slot.lock().unwrap();
    match message {
        RadioMessage::MyNode(num) => {
            inner.my_node = Some(num);
            inner.nodes.entry(num).or_insert_with(|| Heard {
                id: node_label(num),
                short_name: String::new(),
                lat: None,
                lon: None,
            });
        }
        RadioMessage::Node {
            num,
            id,
            short_name,
            lat,
            lon,
            ..
        } => {
            let label = if id.starts_with('!') && !id.is_empty() {
                id
            } else {
                node_label(num)
            };
            let entry = inner.nodes.entry(num).or_insert_with(|| Heard {
                id: label.clone(),
                short_name: String::new(),
                lat: None,
                lon: None,
            });
            entry.id = label;
            if !short_name.is_empty() {
                entry.short_name = short_name;
            }
            if let (Some(lat), Some(lon)) = (lat, lon) {
                entry.lat = Some(lat);
                entry.lon = Some(lon);
            }
        }
        RadioMessage::ConfigComplete(id) if id == nonce => {
            inner.configured = true;
        }
        RadioMessage::ConfigComplete(_) => {}
        RadioMessage::Rebooted => {
            inner.configured = false;
            inner.can_send = false;
        }
        RadioMessage::Text {
            from,
            packet_id,
            body,
        } => {
            if inner.my_node == Some(from) {
                return;
            }
            if !remember_packet(&mut inner.seen, packet_id) {
                return;
            }
            let body: String = body.chars().take(240).collect();
            inner.inbox.push_back(MeshInbound {
                who: node_label(from),
                body,
            });
            while inner.inbox.len() > 40 {
                inner.inbox.pop_front();
            }
            inner.last_rx = Some(Instant::now());
        }
        RadioMessage::Position { from, lat, lon } => {
            if inner.my_node == Some(from) {
                inner.publish();
                return;
            }
            let entry = inner.nodes.entry(from).or_insert_with(|| Heard {
                id: node_label(from),
                short_name: String::new(),
                lat: None,
                lon: None,
            });
            entry.lat = Some(lat);
            entry.lon = Some(lon);
        }
    }
    inner.publish();
}

fn peer_list(my_node: Option<u32>, nodes: &HashMap<u32, Heard>) -> Vec<MeshPeer> {
    let mut peers = Vec::new();
    if let Some(num) = my_node {
        let id = nodes
            .get(&num)
            .map(|node| node.id.clone())
            .unwrap_or_else(|| node_label(num));
        peers.push(MeshPeer {
            id,
            role: "THIS NODE".into(),
            own: true,
        });
    }
    let mut rest: Vec<_> = nodes.keys().filter(|num| Some(**num) != my_node).copied().collect();
    rest.sort_unstable();
    for num in rest {
        let heard = &nodes[&num];
        let role = if heard.short_name.is_empty() {
            "PEER".into()
        } else {
            heard.short_name.clone()
        };
        peers.push(MeshPeer {
            id: heard.id.clone(),
            role,
            own: false,
        });
    }
    peers
}

fn mesh_fixes(my_node: Option<u32>, nodes: &HashMap<u32, Heard>) -> Vec<crate::core::hardware::MeshFix> {
    let mut fixes = Vec::new();
    for (num, heard) in nodes {
        if Some(*num) == my_node {
            continue;
        }
        let (Some(lat), Some(lon)) = (heard.lat, heard.lon) else {
            continue;
        };
        let name = if heard.short_name.is_empty() {
            heard.id.chars().take(4).collect()
        } else {
            heard.short_name.chars().take(4).collect()
        };
        fixes.push(crate::core::hardware::MeshFix { name, lat, lon });
    }
    fixes.sort_by(|a, b| a.name.cmp(&b.name));
    fixes
}

fn node_label(num: u32) -> String {
    format!("!{num:08x}")
}
