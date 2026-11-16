//! The pedal's USB message format: frames, channels, the byte stream inside a
//! channel, and the request/response messages carried in that stream.
//! docs/PROTOCOL.md describes the framing, channels and every opcode.

use crate::msgpack::{Decoder, Value};

/// Frame flag for ordinary traffic.
pub const FLAG_DATA: u8 = 0x18;
/// Frame flag for a channel's opening handshake.
pub const FLAG_HANDSHAKE: u8 = 0x28;

/// One bulk transfer: u24 LE length, flags, destination, source, payload padded to 4.
#[derive(Clone, Debug)]
pub struct Frame {
    pub flags: u8,
    pub dst: u16,
    pub src: u16,
    pub payload: Vec<u8>,
}

impl Frame {
    pub fn new(dst: u16, src: u16, payload: Vec<u8>) -> Frame {
        Frame { flags: FLAG_DATA, dst, src, payload }
    }

    pub fn encode(&self) -> Vec<u8> {
        let len = self.payload.len();
        let mut out = Vec::with_capacity(8 + len + 3);
        out.extend_from_slice(&(len as u32).to_le_bytes()[..3]);
        out.push(self.flags);
        out.extend_from_slice(&self.dst.to_le_bytes());
        out.extend_from_slice(&self.src.to_le_bytes());
        out.extend_from_slice(&self.payload);
        while out.len() % 4 != 0 {
            out.push(0);
        }
        out
    }

    /// Every whole frame in `buf`, plus the bytes of a frame that is not complete yet.
    pub fn decode_stream(buf: &[u8]) -> (Vec<Frame>, Vec<u8>) {
        let mut frames = Vec::new();
        let mut at = 0usize;
        while at + 8 <= buf.len() {
            if buf[at..].iter().all(|b| *b == 0) {
                return (frames, Vec::new());
            }
            let len = u32::from_le_bytes([buf[at], buf[at + 1], buf[at + 2], 0]) as usize;
            if len > 8192 {
                // Not a frame boundary: drop the rest rather than misread it.
                return (frames, Vec::new());
            }
            if at + 8 + len > buf.len() {
                return (frames, buf[at..].to_vec());
            }
            frames.push(Frame {
                flags: buf[at + 3],
                dst: u16::from_le_bytes([buf[at + 4], buf[at + 5]]),
                src: u16::from_le_bytes([buf[at + 6], buf[at + 7]]),
                payload: buf[at + 8..at + 8 + len].to_vec(),
            });
            let step = (8 + len).div_ceil(4) * 4;
            at = (at + step).min(buf.len());
        }
        let tail = &buf[at..];
        if tail.iter().all(|b| *b == 0) {
            (frames, Vec::new())
        } else {
            (frames, tail.to_vec())
        }
    }
}

/// Message types inside a channel. DATA is a flag: 0x0c is data+ack, 0x14 data+keepalive.
pub const MSG_HELLO: u16 = 0x02;
pub const MSG_DATA: u16 = 0x04;
pub const MSG_ACK: u16 = 0x08;

/// The 8 bytes that start every channel payload: seq (BE), type (BE), ack (LE).
#[derive(Clone, Copy, Debug)]
pub struct ChannelHeader {
    pub seq: u16,
    pub msg_type: u16,
    pub ack: u32,
}

impl ChannelHeader {
    pub fn decode(buf: &[u8]) -> Option<(ChannelHeader, &[u8])> {
        if buf.len() < 8 {
            return None;
        }
        Some((
            ChannelHeader {
                seq: u16::from_be_bytes([buf[0], buf[1]]),
                msg_type: u16::from_be_bytes([buf[2], buf[3]]),
                ack: u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]),
            },
            &buf[8..],
        ))
    }
    pub fn encode(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.seq.to_be_bytes());
        out.extend_from_slice(&self.msg_type.to_be_bytes());
        out.extend_from_slice(&self.ack.to_le_bytes());
    }
    pub fn has_data(&self) -> bool {
        self.msg_type & MSG_DATA != 0
    }
}

/// A channel: the pedal-side node and the computer-side node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChannelId {
    pub device: u16,
    pub host: u16,
}

impl ChannelId {
    pub const CONTROL: ChannelId = ChannelId { device: 0x1001, host: 0x03ef };
    pub const EVENTS: ChannelId = ChannelId { device: 0x1002, host: 0x03f0 };
    pub const DATA: ChannelId = ChannelId { device: 0x1080, host: 0x03ed };
}

/// Field keys of the request/response messages.
pub mod key {
    pub const OPCODE: i64 = 100;
    pub const ARGS: i64 = 101;
    pub const TXN: i64 = 102;
    pub const STATUS: i64 = 103;
    pub const RESULT: i64 = 104;
    pub const EVENT: i64 = 105;
    pub const EVENT_ARGS: i64 = 106;
    pub const SETLIST: i64 = 107;
    pub const PRESET_INDEX: i64 = 108;
    pub const NAME: i64 = 109;
    pub const DOCUMENT: i64 = 110;
    pub const ERROR_CODE: i64 = 111;
    pub const VALUE: i64 = 119;
    pub const BLOCK: i64 = 98;
    pub const PARAM_INDEX: i64 = 28;
    pub const PATH: i64 = 26;
    pub const COMMIT: i64 = 29;
    pub const ENABLED: i64 = 59;
    pub const SNAPSHOT: i64 = 92;
}

/// Operation codes.
/// Notification event numbers (events channel).
pub mod event {
    /// Both mean the loaded preset changed on the pedal itself.
    pub const PRESET_CHANGED: i64 = 20;
    pub const PRESET_CHANGED_ALT: i64 = 1;
}

pub mod op {
    pub const LIST_SETLISTS: i64 = 0;
    pub const LIST_PRESETS: i64 = 1;
    pub const FETCH_PRESET: i64 = 4;
    pub const RENAME_PRESET: i64 = 6;
    pub const WRITE_SLOT_NAMED: i64 = 8;
    pub const CLEAR_SLOT: i64 = 16;
    pub const SELECT_PRESET: i64 = 20;
    pub const READ_PRESET: i64 = 22;
    pub const PRESET_INFO: i64 = 23;
    pub const CLEAR_BLOCK: i64 = 28;
    pub const SET_PARAM: i64 = 30;
    pub const ADD_BLOCK: i64 = 39;
    pub const SET_MODEL: i64 = 40;
    pub const MOVE_BLOCK: i64 = 43;
    pub const SET_ROUTE: i64 = 42;
    pub const MOVE_SPLIT: i64 = 46;
    pub const MOVE_JOIN: i64 = 47;
    pub const SELECT_BLOCK: i64 = 78;
    pub const SET_ENABLED: i64 = 41;
    pub const SAVE_PRESET: i64 = 71;
    pub const SELECT_SNAPSHOT: i64 = 88;
    pub const RENAME_SNAPSHOT: i64 = 89;
    /// Copy the current snapshot, and paste what was copied into the current snapshot (name, block
    /// on/offs and knob values). The `{92}` argument is accepted but not used (tried on an HX Stomp).
    pub const COPY_SNAPSHOT: i64 = 90;
    pub const PASTE_SNAPSHOT: i64 = 91;
    pub const BROWSE_OPEN: i64 = 254;
    pub const BEGIN: i64 = 255;
    // User IRs (listIrs 13, irInfo 12, irAudio 11,
    // importIr 9, renameIr 10, deleteIr 15).
    // Controllers and footswitches.
    pub const ASSIGN_CONTROLLER: i64 = 37;
    pub const CONTROLLER_CC: i64 = 64;
    pub const CONTROLLER_MIN: i64 = 65;
    pub const CONTROLLER_MAX: i64 = 66;
    pub const ASSIGN_FOOTSWITCH: i64 = 56;
    pub const UNASSIGN_FOOTSWITCH: i64 = 57;
    pub const FOOTSWITCH_INDEX: i64 = 33;
    pub const IMPORT_IR: i64 = 9;
    pub const RENAME_IR: i64 = 10;
    pub const IR_AUDIO: i64 = 11;
    pub const IR_INFO: i64 = 12;
    pub const LIST_IRS: i64 = 13;
    pub const DELETE_IR: i64 = 15;
}

/// Transaction numbers start here on each channel.
pub const FIRST_TXN: i64 = 1000;

/// The stream service a request rides on. Browse and slot storage requests
/// (setlists, preset lists, reading/writing/renaming/clearing slots, and the
/// session markers around them) ride on service 2; live edit-buffer requests
/// ride on service 6.
pub fn service_for(opcode: i64) -> u16 {
    match opcode {
        op::LIST_SETLISTS
        | op::LIST_PRESETS
        | op::FETCH_PRESET
        | op::RENAME_PRESET
        | op::WRITE_SLOT_NAMED
        | op::CLEAR_SLOT
        | op::BROWSE_OPEN
        | op::BEGIN
        // IR storage rides with the browse/storage requests (service 2).
        | op::IMPORT_IR
        | op::RENAME_IR
        | op::IR_AUDIO
        | op::IR_INFO
        | op::LIST_IRS
        | op::DELETE_IR => 2,
        _ => 6,
    }
}

/// A message read out of a channel's byte stream.
#[derive(Debug, Clone)]
pub enum Message {
    Response { txn: i64, status: i64, result: Value },
    Notification { event: i64, args: Value },
    Other,
}

impl Message {
    pub fn from_value(v: Value) -> Message {
        let txn = v.get(key::TXN).and_then(Value::as_i64);
        if let (Some(txn), Some(status)) = (txn, v.get(key::STATUS).and_then(Value::as_i64)) {
            return Message::Response { txn, status, result: v.get(key::RESULT).cloned().unwrap_or(Value::Nil) };
        }
        if let Some(event) = v.get(key::EVENT).and_then(Value::as_i64) {
            if txn.is_none() {
                return Message::Notification { event, args: v.get(key::EVENT_ARGS).cloned().unwrap_or(Value::Nil) };
            }
        }
        Message::Other
    }
}

/// Encode a request in the field order the pedal's editors use: txn, opcode, args.
pub fn request(txn: i64, opcode: i64, args: Value) -> Value {
    Value::map(vec![(key::TXN, Value::int(txn)), (key::OPCODE, Value::int(opcode)), (key::ARGS, args)])
}

/// Reassembles one channel's byte stream into messages. Each message has an
/// 8-byte prefix: origin (u16 LE, 1 from the computer), service (u16 LE), length (u32 LE).
#[derive(Default)]
pub struct StreamReader {
    buf: Vec<u8>,
}

impl StreamReader {
    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }
    pub fn clear(&mut self) {
        self.buf.clear();
    }
    pub fn take_messages(&mut self) -> Vec<Value> {
        let mut out = Vec::new();
        let mut at = 0usize;
        while self.buf.len() >= at + 8 {
            let len = u32::from_le_bytes([self.buf[at + 4], self.buf[at + 5], self.buf[at + 6], self.buf[at + 7]]) as usize;
            if len > 4 * 1024 * 1024 {
                self.buf.clear();
                return out;
            }
            if self.buf.len() < at + 8 + len {
                break;
            }
            match Decoder::new(&self.buf[at + 8..at + 8 + len]).value() {
                Ok(v) => out.push(v),
                Err(_) => {
                    self.buf.clear();
                    return out;
                }
            }
            at += 8 + len;
        }
        self.buf.drain(..at);
        out
    }
}

/// Front-panel label for a zero-based preset index: 0 is 01A, 7 is 03B.
pub fn slot_label(index: usize) -> String {
    format!("{:02}{}", index / 3 + 1, ['A', 'B', 'C'][index % 3])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_frame_round_trips() {
        // The first frame a client sends after claiming the interface.
        let raw = [0x0c, 0x00, 0x00, 0x28, 0x01, 0x10, 0xef, 0x03, 0x00, 0x00, 0x00, 0x02, 0x00, 0x01, 0x00, 0x21, 0x00, 0x10, 0x00, 0x00];
        let (frames, rest) = Frame::decode_stream(&raw);
        assert!(rest.is_empty());
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].dst, 0x1001);
        assert_eq!(frames[0].encode(), raw);
    }

    #[test]
    fn labels() {
        assert_eq!(slot_label(0), "01A");
        assert_eq!(slot_label(7), "03B");
        assert_eq!(slot_label(125), "42C");
    }
}
