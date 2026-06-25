//! WebSocket protocol (RFC 6455): frame parser, encoder, and handshake

#![allow(dead_code)]

use crate::base64;
use crate::sha1;

const WS_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Opcode {
    Continuation = 0x0,
    Text = 0x1,
    Binary = 0x2,
    Close = 0x8,
    Ping = 0x9,
    Pong = 0xA,
}

impl Opcode {
    fn from_u8(val: u8) -> Option<Self> {
        match val {
            0x0 => Some(Opcode::Continuation),
            0x1 => Some(Opcode::Text),
            0x2 => Some(Opcode::Binary),
            0x8 => Some(Opcode::Close),
            0x9 => Some(Opcode::Ping),
            0xA => Some(Opcode::Pong),
            _ => None,
        }
    }
}

pub struct WsFrame {
    pub fin: bool,
    pub opcode: Opcode,
    pub payload: Vec<u8>,
}

/// Compute the Sec-WebSocket-Accept value from the client's key
pub fn compute_accept_key(client_key: &str) -> String {
    let combined = format!("{}{}", client_key, WS_GUID);
    let hash = sha1::sha1(combined.as_bytes());
    base64::encode(&hash)
}

pub enum ReadResult {
    Frame(WsFrame),
    NeedMore,
    Closed(Option<u16>),
}

/// Attempt to parse a WebSocket frame from the beginning of a buffer
pub fn read_frame(buf: &[u8]) -> Result<ReadResult, String> {
    if buf.len() < 2 {
        return Ok(ReadResult::NeedMore);
    }

    let b0 = buf[0];
    let b1 = buf[1];

    let fin = (b0 & 0x80) != 0;
    let opcode_val = b0 & 0x0F;
    let opcode = Opcode::from_u8(opcode_val)
        .ok_or_else(|| format!("Invalid opcode: 0x{:x}", opcode_val))?;

    let masked = (b1 & 0x80) != 0;
    let payload_len = (b1 & 0x7F) as usize;

    let (extended_len_bytes, actual_len) = match payload_len {
        0..=125 => (0, payload_len),
        126 => {
            if buf.len() < 4 { return Ok(ReadResult::NeedMore); }
            (2, u16::from_be_bytes([buf[2], buf[3]]) as usize)
        }
        127 => {
            if buf.len() < 10 { return Ok(ReadResult::NeedMore); }
            (8, u64::from_be_bytes([
                buf[2], buf[3], buf[4], buf[5],
                buf[6], buf[7], buf[8], buf[9],
            ]) as usize)
        }
        _ => unreachable!(),
    };

    let mask_len = if masked { 4 } else { 0 };
    let header_len = 2 + extended_len_bytes + mask_len;
    let total_len = header_len + actual_len;

    if buf.len() < total_len {
        return Ok(ReadResult::NeedMore);
    }

    let mask_key = if masked {
        Some(&buf[2 + extended_len_bytes..2 + extended_len_bytes + 4])
    } else {
        None
    };

    let raw_payload = &buf[header_len..total_len];
    let payload = if let Some(key) = mask_key {
        let mut data = raw_payload.to_vec();
        for (i, byte) in data.iter_mut().enumerate() {
            *byte ^= key[i % 4];
        }
        data
    } else {
        raw_payload.to_vec()
    };

    if opcode == Opcode::Close {
        let status_code = if payload.len() >= 2 {
            Some(u16::from_be_bytes([payload[0], payload[1]]))
        } else {
            None
        };
        return Ok(ReadResult::Closed(status_code));
    }

    Ok(ReadResult::Frame(WsFrame {
        fin,
        opcode,
        payload,
    }))
}

/// Get the number of bytes consumed by the frame at the beginning of buf
pub fn frame_len(buf: &[u8]) -> Result<usize, String> {
    if buf.len() < 2 {
        return Err("Buffer too short".to_string());
    }

    let b1 = buf[1];
    let masked = (b1 & 0x80) != 0;
    let payload_len = (b1 & 0x7F) as usize;

    let (extended_len_bytes, actual_len) = match payload_len {
        0..=125 => (0, payload_len),
        126 => {
            if buf.len() < 4 { return Err("Incomplete extended length".to_string()); }
            (2, u16::from_be_bytes([buf[2], buf[3]]) as usize)
        }
        127 => {
            if buf.len() < 10 { return Err("Incomplete extended length".to_string()); }
            (8, u64::from_be_bytes([
                buf[2], buf[3], buf[4], buf[5],
                buf[6], buf[7], buf[8], buf[9],
            ]) as usize)
        }
        _ => unreachable!(),
    };

    let mask_len = if masked { 4 } else { 0 };
    Ok(2 + extended_len_bytes + mask_len + actual_len)
}

/// Encode a WebSocket frame for server-to-client transmission (no masking)
pub fn write_frame(opcode: Opcode, payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(payload.len() + 14);

    let first_byte = 0x80 | (opcode as u8);
    frame.push(first_byte);

    if payload.len() <= 125 {
        frame.push(payload.len() as u8);
    } else if payload.len() <= 65535 {
        frame.push(126);
        frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    } else {
        frame.push(127);
        frame.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    }

    frame.extend_from_slice(payload);
    frame
}

pub fn write_text_frame(text: &str) -> Vec<u8> {
    write_frame(Opcode::Text, text.as_bytes())
}

pub fn write_binary_frame(data: &[u8]) -> Vec<u8> {
    write_frame(Opcode::Binary, data)
}

pub fn write_ping_frame(data: &[u8]) -> Vec<u8> {
    write_frame(Opcode::Ping, data)
}

pub fn write_pong_frame(data: &[u8]) -> Vec<u8> {
    write_frame(Opcode::Pong, data)
}

/// Encode a close frame with optional status code
pub fn write_close_frame(status_code: Option<u16>) -> Vec<u8> {
    let mut payload = Vec::new();
    if let Some(code) = status_code {
        payload.extend_from_slice(&code.to_be_bytes());
    }
    write_frame(Opcode::Close, &payload)
}
