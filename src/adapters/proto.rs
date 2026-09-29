//! Minimal Meshtastic serial protobuf.
//! Only the PhoneAPI messages this shell sends and reads: want-config, heartbeat,
//! text packets, node info, and config-complete. The framing is the 0x94 0xC3 header.

use std::collections::VecDeque;

pub const TEXT_MESSAGE_APP: u32 = 1;
pub const POSITION_APP: u32 = 3;
pub const BROADCAST: u32 = 0xFFFF_FFFF;
const MAX_FRAME: usize = 1024;

pub fn frame(payload: &[u8]) -> Vec<u8> {
    let len = payload.len() as u16;
    let mut out = Vec::with_capacity(4 + payload.len());
    out.push(0x94);
    out.push(0xC3);
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(payload);
    out
}

pub fn encode_want_config(id: u32) -> Vec<u8> {
    let mut msg = Vec::new();
    write_varint_field(&mut msg, 3, id as u64);
    msg
}

pub fn encode_heartbeat(nonce: u32) -> Vec<u8> {
    let mut heartbeat = Vec::new();
    write_varint_field(&mut heartbeat, 1, nonce as u64);
    let mut msg = Vec::new();
    write_len_field(&mut msg, 7, &heartbeat);
    msg
}

/// A broadcast text on the primary channel, hop limit 3.
pub fn encode_text_packet(from: u32, packet_id: u32, text: &str) -> Vec<u8> {
    let mut data = Vec::new();
    write_varint_field(&mut data, 1, TEXT_MESSAGE_APP as u64);
    write_len_field(&mut data, 2, text.as_bytes());

    let mut packet = Vec::new();
    write_fixed32_field(&mut packet, 1, from);
    write_fixed32_field(&mut packet, 2, BROADCAST);
    write_len_field(&mut packet, 4, &data);
    write_fixed32_field(&mut packet, 6, packet_id);
    write_varint_field(&mut packet, 9, 3);

    let mut msg = Vec::new();
    write_len_field(&mut msg, 1, &packet);
    msg
}

#[derive(Debug, Clone, PartialEq)]
pub enum RadioMessage {
    MyNode(u32),
    Node {
        num: u32,
        id: String,
        long_name: String,
        short_name: String,
        lat: Option<f64>,
        lon: Option<f64>,
    },
    ConfigComplete(u32),
    Rebooted,
    Text { from: u32, packet_id: u32, body: String },
    Position { from: u32, lat: f64, lon: f64 },
}

pub fn decode_from_radio(buf: &[u8]) -> Option<RadioMessage> {
    let fields = read_fields(buf)?;
    if let Some(bytes) = field_bytes(&fields, 3) {
        let inner = read_fields(bytes)?;
        let num = field_varint(&inner, 1)? as u32;
        if num != 0 {
            return Some(RadioMessage::MyNode(num));
        }
    }
    if let Some(bytes) = field_bytes(&fields, 4) {
        return decode_node(bytes);
    }
    if let Some(id) = field_varint(&fields, 7) {
        return Some(RadioMessage::ConfigComplete(id as u32));
    }
    if field_varint(&fields, 8).unwrap_or(0) != 0 {
        return Some(RadioMessage::Rebooted);
    }
    if let Some(bytes) = field_bytes(&fields, 2) {
        return decode_packet(bytes);
    }
    None
}

fn decode_node(buf: &[u8]) -> Option<RadioMessage> {
    let fields = read_fields(buf)?;
    let num = field_varint(&fields, 1)? as u32;
    let mut id = String::new();
    let mut long_name = String::new();
    let mut short_name = String::new();
    if let Some(user) = field_bytes(&fields, 2) {
        if let Some(user_fields) = read_fields(user) {
            id = field_string(&user_fields, 1);
            long_name = field_string(&user_fields, 2);
            short_name = field_string(&user_fields, 3);
        }
    }
    let position = field_bytes(&fields, 3).and_then(position_of);
    Some(RadioMessage::Node {
        num,
        id,
        long_name,
        short_name,
        lat: position.map(|fix| fix.0),
        lon: position.map(|fix| fix.1),
    })
}

fn decode_packet(buf: &[u8]) -> Option<RadioMessage> {
    let fields = read_fields(buf)?;
    let from = field_fixed32(&fields, 1).unwrap_or(0);
    let packet_id = field_fixed32(&fields, 6).unwrap_or(0);
    let data = field_bytes(&fields, 4)?;
    let data_fields = read_fields(data)?;
    let port = field_varint(&data_fields, 1)? as u32;
    let payload = field_bytes(&data_fields, 2).unwrap_or(b"");
    match port {
        TEXT_MESSAGE_APP => {
            let body = String::from_utf8_lossy(payload).trim_matches('\0').to_string();
            if body.is_empty() {
                return None;
            }
            Some(RadioMessage::Text {
                from,
                packet_id,
                body,
            })
        }
        POSITION_APP => {
            let (lat, lon) = position_of(payload)?;
            Some(RadioMessage::Position { from, lat, lon })
        }
        _ => None,
    }
}

/// Meshtastic Position: latitude_i and longitude_i are degrees times 1e7.
fn position_of(buf: &[u8]) -> Option<(f64, f64)> {
    let fields = read_fields(buf)?;
    let lat_i = field_fixed32(&fields, 1)? as i32;
    let lon_i = field_fixed32(&fields, 2)? as i32;
    if lat_i == 0 && lon_i == 0 {
        return None;
    }
    let lat = f64::from(lat_i) / 1e7;
    let lon = f64::from(lon_i) / 1e7;
    if !(-85.0..=85.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return None;
    }
    Some((lat, lon))
}

#[derive(Debug, Default)]
pub struct Framer {
    buf: Vec<u8>,
}

impl Framer {
    pub fn push(&mut self, data: &[u8]) -> Vec<Vec<u8>> {
        self.buf.extend_from_slice(data);
        let mut out = Vec::new();
        loop {
            let Some(start) = find_magic(&self.buf) else {
                if self.buf.len() > 1 {
                    let keep = usize::from(self.buf.last() == Some(&0x94));
                    self.buf.drain(..self.buf.len() - keep);
                }
                break;
            };
            if start > 0 {
                self.buf.drain(..start);
            }
            if self.buf.len() < 4 {
                break;
            }
            let len = u16::from_be_bytes([self.buf[2], self.buf[3]]) as usize;
            if len > MAX_FRAME {
                self.buf.drain(..1);
                continue;
            }
            if self.buf.len() < 4 + len {
                break;
            }
            out.push(self.buf[4..4 + len].to_vec());
            self.buf.drain(..4 + len);
        }
        out
    }
}

fn find_magic(buf: &[u8]) -> Option<usize> {
    buf.windows(2).position(|pair| pair == [0x94, 0xC3])
}

struct Field<'a> {
    num: u32,
    val: FieldVal<'a>,
}

enum FieldVal<'a> {
    Var(u64),
    Fixed32(u32),
    Bytes(&'a [u8]),
}

fn read_fields(buf: &[u8]) -> Option<Vec<Field<'_>>> {
    let mut cursor = 0;
    let mut fields = Vec::new();
    while cursor < buf.len() {
        let (key, next) = read_varint_at(buf, cursor)?;
        cursor = next;
        let num = (key >> 3) as u32;
        let wire = (key & 0x7) as u32;
        match wire {
            0 => {
                let (value, next) = read_varint_at(buf, cursor)?;
                cursor = next;
                fields.push(Field {
                    num,
                    val: FieldVal::Var(value),
                });
            }
            2 => {
                let (len, next) = read_varint_at(buf, cursor)?;
                cursor = next;
                let len = len as usize;
                if len > buf.len() - cursor {
                    return None;
                }
                fields.push(Field {
                    num,
                    val: FieldVal::Bytes(&buf[cursor..cursor + len]),
                });
                cursor += len;
            }
            5 => {
                if cursor + 4 > buf.len() {
                    return None;
                }
                let value = u32::from_le_bytes(buf[cursor..cursor + 4].try_into().ok()?);
                cursor += 4;
                fields.push(Field {
                    num,
                    val: FieldVal::Fixed32(value),
                });
            }
            1 => {
                if cursor + 8 > buf.len() {
                    return None;
                }
                cursor += 8;
            }
            _ => return None,
        }
    }
    Some(fields)
}

fn read_varint_at(buf: &[u8], mut cursor: usize) -> Option<(u64, usize)> {
    let mut out = 0u64;
    let mut shift = 0;
    loop {
        if shift > 63 || cursor >= buf.len() {
            return None;
        }
        let byte = buf[cursor];
        cursor += 1;
        out |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some((out, cursor));
        }
        shift += 7;
    }
}

fn field_varint(fields: &[Field<'_>], num: u32) -> Option<u64> {
    fields.iter().find_map(|field| match field {
        Field { num: n, val: FieldVal::Var(value) } if *n == num => Some(*value),
        _ => None,
    })
}

fn field_fixed32(fields: &[Field<'_>], num: u32) -> Option<u32> {
    fields.iter().find_map(|field| match field {
        Field { num: n, val: FieldVal::Fixed32(value) } if *n == num => Some(*value),
        _ => None,
    })
}

fn field_bytes<'a>(fields: &[Field<'a>], num: u32) -> Option<&'a [u8]> {
    fields.iter().find_map(|field| match field {
        Field { num: n, val: FieldVal::Bytes(value) } if *n == num => Some(*value),
        _ => None,
    })
}

fn field_string(fields: &[Field<'_>], num: u32) -> String {
    field_bytes(fields, num)
        .map(|bytes| String::from_utf8_lossy(bytes).trim_matches('\0').to_string())
        .unwrap_or_default()
}

fn write_varint(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
}

fn write_varint_field(out: &mut Vec<u8>, field: u32, value: u64) {
    write_varint(out, (u64::from(field) << 3) | 0);
    write_varint(out, value);
}

fn write_fixed32_field(out: &mut Vec<u8>, field: u32, value: u32) {
    write_varint(out, (u64::from(field) << 3) | 5);
    out.extend_from_slice(&value.to_le_bytes());
}

fn write_len_field(out: &mut Vec<u8>, field: u32, bytes: &[u8]) {
    write_varint(out, (u64::from(field) << 3) | 2);
    write_varint(out, bytes.len() as u64);
    out.extend_from_slice(bytes);
}

/// Remember recent packet ids so a repeated phone-API delivery is logged once.
pub fn remember_packet(seen: &mut VecDeque<u32>, id: u32) -> bool {
    if id == 0 {
        return true;
    }
    if seen.contains(&id) {
        return false;
    }
    seen.push_back(id);
    if seen.len() > 64 {
        seen.pop_front();
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn want_config_frame_matches_the_serial_header() {
        let payload = encode_want_config(1);
        assert_eq!(payload, vec![0x18, 0x01]);
        let framed = frame(&payload);
        assert_eq!(framed, vec![0x94, 0xC3, 0x00, 0x02, 0x18, 0x01]);
    }

    #[test]
    fn text_packet_is_a_broadcast_on_the_text_port() {
        let encoded = encode_text_packet(0xA1B2_C3D4, 0x0102_0304, "ridge");
        let to_radio = read_fields(&encoded).unwrap();
        let packet = field_bytes(&to_radio, 1).unwrap();
        let fields = read_fields(packet).unwrap();
        assert_eq!(field_fixed32(&fields, 1), Some(0xA1B2_C3D4));
        assert_eq!(field_fixed32(&fields, 2), Some(BROADCAST));
        assert_eq!(field_fixed32(&fields, 6), Some(0x0102_0304));
        assert_eq!(field_varint(&fields, 9), Some(3));
        let data = read_fields(field_bytes(&fields, 4).unwrap()).unwrap();
        assert_eq!(field_varint(&data, 1), Some(u64::from(TEXT_MESSAGE_APP)));
        assert_eq!(field_string(&data, 2), "ridge");
    }

    #[test]
    fn from_radio_node_and_text_decode() {
        let user = {
            let mut user = Vec::new();
            write_len_field(&mut user, 1, b"!ab");
            write_len_field(&mut user, 3, b"AB");
            user
        };
        let mut node = Vec::new();
        write_varint_field(&mut node, 1, 42);
        write_len_field(&mut node, 2, &user);
        let mut from_radio = Vec::new();
        write_len_field(&mut from_radio, 4, &node);
        match decode_from_radio(&from_radio) {
            Some(RadioMessage::Node { num, id, short_name, .. }) => {
                assert_eq!(num, 42);
                assert_eq!(id, "!ab");
                assert_eq!(short_name, "AB");
            }
            other => panic!("unexpected {other:?}"),
        }

        let text = encode_text_packet(7, 9, "holding");
        let to_radio = read_fields(&text).unwrap();
        let packet = field_bytes(&to_radio, 1).unwrap().to_vec();
        let mut wrapped = Vec::new();
        write_len_field(&mut wrapped, 2, &packet);
        match decode_from_radio(&wrapped) {
            Some(RadioMessage::Text { from, packet_id, body }) => {
                assert_eq!(from, 7);
                assert_eq!(packet_id, 9);
                assert_eq!(body, "holding");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn position_packet_decodes_degrees() {
        let lat_i = (-27.47_f64 * 1e7).round() as i32;
        let lon_i = (153.02_f64 * 1e7).round() as i32;
        let mut pos = Vec::new();
        write_fixed32_field(&mut pos, 1, lat_i as u32);
        write_fixed32_field(&mut pos, 2, lon_i as u32);
        let mut data = Vec::new();
        write_varint_field(&mut data, 1, u64::from(POSITION_APP));
        write_len_field(&mut data, 2, &pos);
        let mut packet = Vec::new();
        write_fixed32_field(&mut packet, 1, 11);
        write_len_field(&mut packet, 4, &data);
        let mut wrapped = Vec::new();
        write_len_field(&mut wrapped, 2, &packet);
        match decode_from_radio(&wrapped) {
            Some(RadioMessage::Position { from, lat, lon }) => {
                assert_eq!(from, 11);
                assert!((lat + 27.47).abs() < 1e-5, "{lat}");
                assert!((lon - 153.02).abs() < 1e-5, "{lon}");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn framer_skips_noise_and_splits_two_frames() {
        let payload = encode_want_config(1);
        let mut bytes = vec![0x00, 0x94, 0x11];
        bytes.extend(frame(&payload));
        bytes.extend(frame(&payload));
        let mut framer = Framer::default();
        let got = framer.push(&bytes);
        assert_eq!(got, vec![payload.clone(), payload]);
    }
}
