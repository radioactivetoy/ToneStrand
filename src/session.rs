//! A conversation with the pedal over any wire: open the channels, send
//! requests, acknowledge what arrives, and read replies and notifications.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use crate::msgpack::Value;
use crate::proto::*;
use crate::wire::{Wire, TIMEOUT};

/// Acknowledgements are this base plus the stream bytes received.
const ACK_BASE: u32 = 0x1000;
/// Handshake frame: the ack slot holds this, then these four bytes follow.
const HELLO_FIELD: u32 = 0x2100_0100;
const HELLO_TAIL: [u8; 4] = [0x00, 0x10, 0x00, 0x00];
/// Stream bytes per frame; the pedal chunks its own large transfers the same way.
const CHUNK: usize = 256;
const HANDSHAKE_WAIT: Duration = Duration::from_secs(2);
const REPLY_IDLE: Duration = Duration::from_secs(6);
const REPLY_MAX: Duration = Duration::from_secs(10);
const BETWEEN_CHUNKS: Duration = Duration::from_millis(80);
/// Pause after a flash write so commits never stack.
pub const SETTLE_FLASH: Duration = Duration::from_millis(750);
/// A User IR: 2,048 samples of 32-bit float at 48 kHz.
pub const IR_AUDIO_BYTES: usize = 8192;
pub const IR_SLOTS: i64 = 128;
pub const IR_NAME_MAX: usize = 26;
/// The key that carries an IR slot number.
const IR_SLOT: i64 = 112;

struct Channel {
    seq: u16,
    rx_bytes: u32,
    acked: u32,
    reader: StreamReader,
    txn: i64,
    /// The pedal asked "still there?" (a message of type 0x10 with no data).
    /// It must be answered with an acknowledgement, or after a few unanswered
    /// ones the pedal stops replying on that channel and every request times out.
    pinged: bool,
}

/// The pedal's "are you still there?" message on a channel (no data).
const MSG_PING: u16 = 0x10;

impl Channel {
    fn new() -> Channel {
        Channel { seq: 0, rx_bytes: 0, acked: 0, reader: StreamReader::default(), txn: FIRST_TXN, pinged: false }
    }
    fn ack(&self) -> u32 {
        ACK_BASE.wrapping_add(self.rx_bytes)
    }
}

#[derive(Debug)]
pub enum SessionError {
    Wire(String),
    Timeout,
    Refused(i64),
    Protocol(String),
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionError::Wire(e) => write!(f, "{e}"),
            SessionError::Timeout => write!(f, "the pedal did not answer in time"),
            SessionError::Refused(-302) => write!(f, "your pedal does not know this model (its firmware may be older)"),
            SessionError::Refused(-306) => write!(f, "it does not fit — this path does not have enough DSP left"),
            SessionError::Refused(-3) => write!(f, "the pedal has no such block or setting there"),
            SessionError::Refused(-46) => write!(f, "the pedal has no such snapshot"),
            SessionError::Refused(code) => write!(f, "the pedal refused the request (code {code})"),
            SessionError::Protocol(e) => write!(f, "{e}"),
        }
    }
}

pub type Result<T> = std::result::Result<T, SessionError>;

pub struct Session {
    wire: Box<dyn Wire>,
    channels: BTreeMap<u16, Channel>,
    leftover: Vec<u8>,
    notifications: Vec<(i64, Value)>,
    broken: Option<String>,
    browse_open: bool,
    /// Things worth telling the person that didn't stop an operation.
    warnings: Vec<String>,
}

impl Session {
    /// Warnings gathered since the last call.
    pub fn take_warnings(&mut self) -> Vec<String> {
        std::mem::take(&mut self.warnings)
    }

    /// Announce a flash write (op BEGIN). A refusal is passed on as a warning and the write
    /// goes ahead (as it always has); a link that doesn't answer stops it, since a write
    /// over that link could stop half way.
    fn begin_write(&mut self) -> Result<()> {
        match self.request(op::BEGIN, Value::empty_map()) {
            Ok(_) => Ok(()),
            Err(SessionError::Refused(code)) => {
                self.warnings.push(format!("the pedal answered the start of the write with code {code}; went ahead"));
                Ok(())
            }
            Err(e) => Err(SessionError::Protocol(format!("the pedal didn't answer the start of the write ({e}); nothing was written"))),
        }
    }

    pub fn open(wire: Box<dyn Wire>) -> Result<Session> {
        let mut s = Session {
            wire,
            channels: BTreeMap::new(),
            leftover: Vec::new(),
            notifications: Vec::new(),
            broken: None,
            browse_open: false,
            warnings: Vec::new(),
        };
        s.drain();
        // The order the pedal expects: control (service 5, then closed),
        // data (service 6), events (service 4).
        s.open_service(ChannelId::CONTROL, 5)?;
        s.close_service(ChannelId::CONTROL)?;
        s.open_service(ChannelId::DATA, 6)?;
        s.open_service(ChannelId::EVENTS, 4)?;
        // Prove the session answers before handing it over.
        s.preset_info()?;
        Ok(s)
    }

    pub fn describe(&self) -> String {
        self.wire.describe()
    }

    fn drain(&mut self) {
        let give_up = Instant::now() + Duration::from_secs(3);
        let mut quiet = 0;
        while quiet < 3 && Instant::now() < give_up {
            match self.wire.recv(Duration::from_millis(150)) {
                Ok(_) => quiet = 0,
                Err(_) => quiet += 1,
            }
        }
        self.leftover.clear();
    }

    fn write(&mut self, f: &Frame) -> Result<()> {
        let bytes = f.encode();
        if debug() {
            eprintln!("OUT {}", hex(&bytes));
        }
        self.wire.send(&bytes).map_err(SessionError::Wire)
    }

    fn tick(&mut self, id: ChannelId) -> (u16, u32) {
        let ch = self.channels.entry(id.device).or_insert_with(Channel::new);
        let seq = ch.seq;
        ch.seq = ch.seq.wrapping_add(1);
        (seq, ch.ack())
    }

    /// Read one transfer and route its frames. Ok(true) when a frame arrived.
    fn read_once(&mut self, timeout: Duration) -> Result<bool> {
        let data = match self.wire.recv(timeout) {
            Ok(d) => d,
            Err(e) if e == TIMEOUT => return Ok(false),
            Err(e) => {
                self.broken = Some(e.clone());
                return Err(SessionError::Wire(e));
            }
        };
        if data.is_empty() && self.leftover.is_empty() {
            return Ok(false);
        }
        if debug() {
            eprintln!("IN  {}", hex(&data));
        }
        let mut buf = std::mem::take(&mut self.leftover);
        buf.extend_from_slice(&data);
        let (frames, rest) = Frame::decode_stream(&buf);
        self.leftover = rest;
        let any = !frames.is_empty();
        for f in frames {
            if let Some((hdr, body)) = ChannelHeader::decode(&f.payload) {
                if let Some(ch) = self.channels.get_mut(&f.src) {
                    if hdr.msg_type & MSG_PING != 0 {
                        ch.pinged = true;
                    }
                    if hdr.has_data() && !body.is_empty() {
                        ch.rx_bytes = ch.rx_bytes.wrapping_add(body.len() as u32);
                        ch.reader.push(body);
                    }
                }
            }
        }
        Ok(any)
    }

    fn wait_incoming(&mut self, budget: Duration) -> Result<bool> {
        let deadline = Instant::now() + budget;
        while Instant::now() < deadline {
            if self.read_once(Duration::from_millis(50))? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn open_service(&mut self, id: ChannelId, service: u16) -> Result<()> {
        self.channels.insert(id.device, Channel::new());
        let mut payload = Vec::new();
        ChannelHeader { seq: 0, msg_type: MSG_HELLO, ack: HELLO_FIELD }.encode(&mut payload);
        payload.extend_from_slice(&HELLO_TAIL);
        let mut hello = Frame::new(id.device, id.host, payload);
        hello.flags = FLAG_HANDSHAKE;
        self.write(&hello)?;
        if !self.wait_incoming(HANDSHAKE_WAIT)? {
            return Err(SessionError::Protocol(
                "the pedal did not answer the opening handshake - try unplugging it for a moment".into(),
            ));
        }
        if let Some(ch) = self.channels.get_mut(&id.device) {
            ch.reader.take_messages();
            ch.rx_bytes = 0;
            // The sequence jumps 0 -> 2 after the handshake.
            ch.seq = 2;
        }
        self.send_stream(id, service, &Value::int(service as i64))?;
        if !self.wait_incoming(HANDSHAKE_WAIT)? {
            return Err(SessionError::Protocol("the pedal did not confirm the channel".into()));
        }
        if let Some(ch) = self.channels.get_mut(&id.device) {
            ch.reader.take_messages();
        }
        self.ack_channel(id)
    }

    fn close_service(&mut self, id: ChannelId) -> Result<()> {
        let (seq, ack) = self.tick(id);
        let mut payload = Vec::new();
        ChannelHeader { seq, msg_type: MSG_HELLO, ack }.encode(&mut payload);
        self.write(&Frame::new(id.device, id.host, payload))?;
        self.wait_incoming(HANDSHAKE_WAIT)?;
        Ok(())
    }

    fn ack_channel(&mut self, id: ChannelId) -> Result<()> {
        let (seq, ack) = self.tick(id);
        if let Some(ch) = self.channels.get_mut(&id.device) {
            ch.acked = ch.rx_bytes;
            ch.pinged = false;
        }
        let mut payload = Vec::new();
        ChannelHeader { seq, msg_type: MSG_ACK, ack }.encode(&mut payload);
        self.write(&Frame::new(id.device, id.host, payload))
    }

    /// Acknowledge every channel except `busy` that has unacknowledged bytes.
    fn ack_others(&mut self, busy: Option<ChannelId>) -> Result<()> {
        for id in [ChannelId::CONTROL, ChannelId::EVENTS, ChannelId::DATA] {
            if Some(id) == busy {
                continue;
            }
            let behind = self.channels.get(&id.device).is_some_and(|c| c.rx_bytes != c.acked || c.pinged);
            if behind && id != ChannelId::CONTROL {
                self.ack_channel(id)?;
            }
        }
        Ok(())
    }

    fn send_stream(&mut self, id: ChannelId, service: u16, body: &Value) -> Result<()> {
        if let Some(why) = &self.broken {
            return Err(SessionError::Protocol(why.clone()));
        }
        let encoded = body.encode();
        let mut stream = Vec::with_capacity(encoded.len() + 8);
        stream.extend_from_slice(&1u16.to_le_bytes());
        stream.extend_from_slice(&service.to_le_bytes());
        stream.extend_from_slice(&(encoded.len() as u32).to_le_bytes());
        stream.extend_from_slice(&encoded);
        let chunks: Vec<&[u8]> = stream.chunks(CHUNK).collect();
        let n = chunks.len();
        for (i, chunk) in chunks.into_iter().enumerate() {
            let (seq, ack) = self.tick(id);
            let mut payload = Vec::with_capacity(8 + chunk.len());
            ChannelHeader { seq, msg_type: MSG_DATA, ack }.encode(&mut payload);
            payload.extend_from_slice(chunk);
            if let Err(e) = self.write(&Frame::new(id.device, id.host, payload)) {
                if i > 0 {
                    self.broken = Some(format!(
                        "a transfer stopped half way ({e}); power-cycle the pedal before reconnecting"
                    ));
                }
                return Err(e);
            }
            if i + 1 < n {
                // Keep reading (and acknowledging) for the whole gap; through the Box, a longer
                // gap so its relay to the pedal's USB never falls behind.
                let gap = if self.wire.slow() { BETWEEN_CHUNKS * 3 } else { BETWEEN_CHUNKS };
                let until = Instant::now() + gap;
                while Instant::now() < until {
                    if self.read_once(Duration::from_millis(15)).is_err() {
                        break;
                    }
                }
            }
        }
        Ok(())
    }

    /// Collect notifications from every channel that has whole messages.
    fn collect_notifications(&mut self, except: Option<ChannelId>) {
        for (dev, ch) in self.channels.iter_mut() {
            if Some(*dev) == except.map(|c| c.device) {
                continue;
            }
            for v in ch.reader.take_messages() {
                if let Message::Notification { event, args } = Message::from_value(v) {
                    self.notifications.push((event, args));
                }
            }
        }
    }

    /// Send a request on the data channel and wait for its reply: (status, result).
    pub fn request(&mut self, opcode: i64, args: Value) -> Result<(i64, Value)> {
        let id = ChannelId::DATA;
        // Take and acknowledge what the pedal has been holding first.
        let until = Instant::now() + Duration::from_millis(60);
        while Instant::now() < until {
            if !self.read_once(Duration::from_millis(5))? {
                break;
            }
        }
        self.collect_notifications(Some(id));
        self.ack_others(None)?;
        let behind = self.channels.get(&id.device).is_some_and(|c| c.rx_bytes != c.acked || c.pinged);
        if behind {
            self.ack_channel(id)?;
        }

        let txn = {
            let ch = self.channels.entry(id.device).or_insert_with(Channel::new);
            let t = ch.txn;
            ch.txn += 1;
            t
        };
        let service = service_for(opcode);
        if service != 2 {
            self.browse_open = false;
        }
        self.send_stream(id, service, &request(txn, opcode, args))?;

        let give_up = Instant::now() + REPLY_MAX;
        let mut idle_until = Instant::now() + REPLY_IDLE;
        while Instant::now() < give_up && Instant::now() < idle_until {
            let before = self.channels.get(&id.device).map_or(0, |c| c.rx_bytes);
            self.read_once(Duration::from_millis(30))?;
            let after = self.channels.get(&id.device).map_or(0, |c| c.rx_bytes);
            let pinged = self.channels.get(&id.device).is_some_and(|c| c.pinged);
            if after != before {
                idle_until = Instant::now() + REPLY_IDLE;
                // One packet in, one acknowledgement out: the pedal paces by it.
                self.ack_channel(id)?;
            } else if pinged {
                self.ack_channel(id)?;
            }
            let ready = self
                .channels
                .get_mut(&id.device)
                .map(|c| c.reader.take_messages())
                .unwrap_or_default();
            for v in ready {
                match Message::from_value(v) {
                    Message::Response { txn: t, status, result } if t == txn => {
                        if status == 255 {
                            let code = result.get(key::ERROR_CODE).and_then(Value::as_i64).unwrap_or(0);
                            return Err(SessionError::Refused(code));
                        }
                        return Ok((status, result));
                    }
                    Message::Notification { event, args } => self.notifications.push((event, args)),
                    _ => {}
                }
            }
            self.collect_notifications(Some(id));
            self.ack_others(Some(id))?;
        }
        if let Some(ch) = self.channels.get_mut(&id.device) {
            ch.reader.clear();
        }
        self.leftover.clear();
        Err(SessionError::Timeout)
    }

    /// Keep things moving while nothing is being asked: read, acknowledge,
    /// gather notifications, and keep the box link alive.
    pub fn idle(&mut self, budget: Duration) -> Result<Vec<(i64, Value)>> {
        let until = Instant::now() + budget;
        while Instant::now() < until {
            if !self.read_once(Duration::from_millis(20))? {
                break;
            }
            self.ack_others(None)?;
        }
        self.ack_others(None)?;
        self.collect_notifications(None);
        self.wire.idle().map_err(SessionError::Wire)?;
        Ok(std::mem::take(&mut self.notifications))
    }

    // ----------------------------------------------------------- operations ---

    /// (setlist, index, name) of the loaded preset.
    pub fn preset_info(&mut self) -> Result<(i64, i64, String)> {
        let (_, v) = self.request(op::PRESET_INFO, Value::Nil)?;
        Ok((
            v.get(key::SETLIST).and_then(Value::as_i64).unwrap_or(-1),
            v.get(key::PRESET_INDEX).and_then(Value::as_i64).unwrap_or(-1),
            v.get(key::NAME).and_then(Value::as_str).map(clean).unwrap_or_default(),
        ))
    }

    fn open_browse(&mut self) {
        if !self.browse_open {
            let _ = self.request(op::BROWSE_OPEN, Value::empty_map());
            let _ = self.request(op::LIST_SETLISTS, Value::Nil);
            self.browse_open = true;
        }
    }

    pub fn setlists(&mut self) -> Result<Vec<String>> {
        self.browse_open = false;
        let _ = self.request(op::BROWSE_OPEN, Value::empty_map());
        let (_, v) = self.request(op::LIST_SETLISTS, Value::Nil)?;
        self.browse_open = true;
        Ok(v.as_array()
            .map(|a| {
                a.iter()
                    .map(|e| match e {
                        Value::Map(pairs, _) => pairs
                            .first()
                            .and_then(|(_, v)| v.as_str())
                            .map(clean)
                            .unwrap_or_default(),
                        other => other.as_str().map(clean).unwrap_or_default(),
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    pub fn presets(&mut self, setlist: i64) -> Result<Vec<String>> {
        self.open_browse();
        let (_, v) = self.request(
            op::LIST_PRESETS,
            Value::map(vec![(key::SETLIST, Value::int(setlist)), (key::ARGS, Value::int(2))]),
        )?;
        let Some(entries) = v.as_array() else {
            return Err(SessionError::Protocol("the preset list was not a list".into()));
        };
        Ok(entries
            .iter()
            .map(|e| match e {
                Value::Map(pairs, _) => pairs
                    .first()
                    .and_then(|(_, v)| v.get(key::NAME))
                    .and_then(Value::as_str)
                    .map(clean)
                    .unwrap_or_default(),
                _ => String::new(),
            })
            .collect())
    }

    /// Load a preset and wait until the pedal reports it as the current one.
    pub fn select_preset(&mut self, setlist: i64, index: i64) -> Result<()> {
        self.request(
            op::SELECT_PRESET,
            Value::map(vec![(key::SETLIST, Value::int(setlist)), (key::PRESET_INDEX, Value::int(index))]),
        )?;
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let _ = self.idle(Duration::from_millis(40));
            if let Ok((current_list, current, _)) = self.preset_info() {
                // Some units report no setlist (-1); otherwise it must match too.
                if current == index && (current_list == setlist || current_list < 0) {
                    return Ok(());
                }
            }
            if Instant::now() >= deadline {
                return Err(SessionError::Protocol(format!(
                    "the pedal did not finish switching to preset {}",
                    slot_label(index.max(0) as usize)
                )));
            }
            std::thread::sleep(Duration::from_millis(150));
        }
    }

    /// The loaded preset's document (the edit buffer).
    pub fn read_current(&mut self) -> Result<Vec<u8>> {
        let (_, v) = self.request(op::READ_PRESET, Value::Nil)?;
        let bytes = v
            .as_bin()
            .map(|b| b.to_vec())
            .ok_or_else(|| SessionError::Protocol("the pedal sent no preset document".into()))?;
        crate::devpreset::DevicePreset::parse(&bytes).map_err(SessionError::Protocol)?;
        Ok(bytes)
    }

    /// Any slot's stored document, without loading it. None for an empty slot.
    pub fn fetch_slot(&mut self, setlist: i64, index: i64) -> Result<Option<Vec<u8>>> {
        self.open_browse();
        let (_, v) = self.request(
            op::FETCH_PRESET,
            Value::map(vec![
                (key::SETLIST, Value::int(setlist)),
                (key::PRESET_INDEX, Value::int(index)),
                (key::ARGS, Value::int(2)),
            ]),
        )?;
        let Some(bytes) = v.as_bin().map(|b| b.to_vec()).filter(|b| !b.is_empty()) else {
            return Ok(None);
        };
        crate::devpreset::DevicePreset::parse(&bytes).map_err(SessionError::Protocol)?;
        Ok(Some(bytes))
    }

    /// Write a document straight into a slot and name it. Returns the bytes sent, to compare
    /// with what reads back.
    pub fn write_slot(&mut self, setlist: i64, index: i64, name: &str, doc: Vec<u8>) -> Result<Vec<u8>> {
        let parsed = crate::devpreset::DevicePreset::parse(&doc).map_err(|e| {
            SessionError::Protocol(format!("refusing to write a non-pedal preset document to the unit ({e})"))
        })?;
        let doc = parsed.encode_checked().map_err(SessionError::Protocol)?;
        if doc.len() > u16::MAX as usize {
            return Err(SessionError::Protocol("the preset document is too large for a pedal slot".into()));
        }
        self.browse_open = false;
        self.open_browse();
        self.begin_write()?;
        if self.wire.slow() {
            let _ = self.idle(Duration::from_millis(400));
        }
        self.request(
            op::WRITE_SLOT_NAMED,
            Value::map(vec![
                (key::SETLIST, Value::int(setlist)),
                (key::PRESET_INDEX, Value::int(index)),
                (key::NAME, wire_name(name, PRESET_NAME_MAX)),
                (123, Value::Bool(false)),
                (124, Value::Bool(false)),
                (125, Value::int(0)),
                (key::DOCUMENT, Value::Bin(doc.clone(), 0xda)),
            ]),
        )?;
        self.browse_open = false;
        std::thread::sleep(SETTLE_FLASH);
        Ok(doc)
    }

    pub fn rename_slot(&mut self, setlist: i64, index: i64, name: &str) -> Result<()> {
        self.open_browse();
        self.request(
            op::RENAME_PRESET,
            Value::map(vec![
                (key::SETLIST, Value::int(setlist)),
                (key::PRESET_INDEX, Value::int(index)),
                (key::NAME, wire_name(name, PRESET_NAME_MAX)),
            ]),
        )?;
        self.browse_open = false;
        std::thread::sleep(SETTLE_FLASH);
        Ok(())
    }

    pub fn clear_slot(&mut self, setlist: i64, index: i64) -> Result<()> {
        self.open_browse();
        self.request(
            op::CLEAR_SLOT,
            Value::map(vec![(key::SETLIST, Value::int(setlist)), (key::PRESET_INDEX, Value::int(index))]),
        )?;
        self.browse_open = false;
        std::thread::sleep(SETTLE_FLASH);
        Ok(())
    }

    /// Commit the edit buffer to a slot.
    pub fn save_current(&mut self, setlist: i64, index: i64, name: &str) -> Result<()> {
        self.request(
            op::SAVE_PRESET,
            Value::map(vec![
                (key::SETLIST, Value::int(setlist)),
                (key::PRESET_INDEX, Value::int(index)),
                (key::NAME, wire_name(name, PRESET_NAME_MAX)),
            ]),
        )?;
        std::thread::sleep(SETTLE_FLASH);
        Ok(())
    }

    /// Change one parameter of the loaded preset, live.
    pub fn set_param(&mut self, block: i64, path: i64, index: i64, value: Value) -> Result<()> {
        let value = match value {
            Value::F64(f) => Value::F32(f as f32),
            other => other,
        };
        self.request(
            op::SET_PARAM,
            Value::map(vec![
                (key::BLOCK, Value::int(block)),
                (key::COMMIT, Value::Bool(true)),
                (key::PATH, Value::int(path)),
                (key::PARAM_INDEX, Value::int(index)),
                (key::VALUE, value),
            ]),
        )?;
        Ok(())
    }

    /// Switch one block of the loaded preset on or off, live.
    pub fn set_enabled(&mut self, block: i64, enabled: bool) -> Result<()> {
        self.request(
            op::SET_ENABLED,
            Value::map(vec![(key::BLOCK, Value::int(block)), (key::ENABLED, Value::Bool(enabled))]),
        )?;
        Ok(())
    }

    /// Move the pedal's editing cursor to a block; model changes need it there first.
    pub fn select_block(&mut self, block: i64) -> Result<()> {
        self.request(
            op::SELECT_BLOCK,
            Value::map(vec![(key::BLOCK, Value::int(block)), (key::PATH, Value::int(0))]),
        )?;
        Ok(())
    }

    /// Put a model (and optionally a paired cab) into a block of the loaded preset.
    pub fn set_model(&mut self, block: i64, model: i64, paired: Option<i64>) -> Result<()> {
        self.select_block(block)?;
        self.request(
            op::SET_MODEL,
            Value::map(vec![
                (key::BLOCK, Value::int(block)),
                (
                    key::OPCODE, // the model descriptor rides on key 100
                    Value::map(vec![
                        (23, Value::Bool(paired.is_some())),
                        (25, Value::int(model)),
                        (26, Value::int(paired.unwrap_or(-1))),
                    ]),
                ),
            ]),
        )?;
        Ok(())
    }

    /// Put a new block into an empty slot of the loaded preset (op 39):
    /// `{98: slot, 99: {19: 6|7, 20: block body without knob values}}`.
    pub fn add_block(&mut self, block: i64, slot: Value) -> Result<()> {
        self.request(
            op::ADD_BLOCK,
            Value::map(vec![
                (key::BLOCK, Value::int(block)),
                (99, slot),
            ]),
        )?;
        Ok(())
    }

    /// Change where an input listens (key 51) or an output sends (key 52), op 42.
    pub fn set_route(&mut self, block: i64, input: bool, value: i64) -> Result<()> {
        let k = if input { 51 } else { 52 };
        self.request(op::SET_ROUTE, Value::map(vec![(key::BLOCK, Value::int(block)), (k, Value::int(value))]))?;
        Ok(())
    }

    /// Move a block from one slot to another (op 43, `{75: from, 76: to}`).
    pub fn move_block(&mut self, from: i64, to: i64) -> Result<()> {
        self.request(op::MOVE_BLOCK, Value::map(vec![(75, Value::int(from)), (76, Value::int(to))]))?;
        Ok(())
    }

    /// Move the split point on a path (op 46, `{98: 20 * path + attach}`).
    pub fn move_split(&mut self, block: i64) -> Result<()> {
        self.request(op::MOVE_SPLIT, Value::map(vec![(key::BLOCK, Value::int(block))]))?;
        Ok(())
    }

    /// Move the merge point on a path (op 47, `{98: 20 * path + attach}`).
    pub fn move_join(&mut self, block: i64) -> Result<()> {
        self.request(op::MOVE_JOIN, Value::map(vec![(key::BLOCK, Value::int(block))]))?;
        Ok(())
    }

    /// Empty a block of the loaded preset.
    pub fn clear_block(&mut self, block: i64) -> Result<()> {
        self.select_block(block)?;
        self.request(op::CLEAR_BLOCK, Value::map(vec![(key::BLOCK, Value::int(block))]))?;
        Ok(())
    }

    pub fn select_snapshot(&mut self, index: i64) -> Result<()> {
        self.request(op::SELECT_SNAPSHOT, Value::map(vec![(key::SNAPSHOT, Value::int(index))]))?;
        Ok(())
    }

    // ------------------------------------------- controllers and footswitches ---
    // Live, on the loaded preset; requests
    // (assignControllerRequest, controllerCcRequest, controllerMin/Max, (un)assignFootswitchRequest).

    /// Put a knob on a controller (op 37): `{98 block, 29 true, 26 cab, 28 knob, 74 controller, 71 cc, 129 false}`.
    pub fn assign_controller(&mut self, block: i64, cab: bool, index: i64, controller: i64, cc: i64) -> Result<()> {
        self.request(
            op::ASSIGN_CONTROLLER,
            Value::map(vec![
                (key::BLOCK, Value::int(block)),
                (key::COMMIT, Value::Bool(true)),
                (key::PATH, Value::int(cab as i64)),
                (key::PARAM_INDEX, Value::int(index)),
                (74, Value::int(controller)),
                (71, Value::int(cc)),
                (129, Value::Bool(false)),
            ]),
        )?;
        Ok(())
    }

    /// Take a knob off its controller (op 37 with controller 0).
    pub fn unassign_controller(&mut self, block: i64, cab: bool, index: i64) -> Result<()> {
        self.assign_controller(block, cab, index, 0, 0)
    }

    /// A controller's lowest or highest value for a knob (ops 65 / 66).
    pub fn controller_bound(&mut self, max: bool, block: i64, cab: bool, index: i64, value: Value) -> Result<()> {
        let value = match value {
            Value::F64(f) => Value::F32(f as f32),
            other => other,
        };
        self.request(
            if max { op::CONTROLLER_MAX } else { op::CONTROLLER_MIN },
            Value::map(vec![
                (key::BLOCK, Value::int(block)),
                (key::COMMIT, Value::Bool(true)),
                (key::PATH, Value::int(cab as i64)),
                (key::PARAM_INDEX, Value::int(index)),
                (key::VALUE, value),
            ]),
        )?;
        Ok(())
    }

    /// The MIDI CC number of a knob's MIDI controller (op 64).
    pub fn controller_cc(&mut self, block: i64, cab: bool, index: i64, cc: i64) -> Result<()> {
        self.request(
            op::CONTROLLER_CC,
            Value::map(vec![
                (key::BLOCK, Value::int(block)),
                (key::COMMIT, Value::Bool(true)),
                (key::PATH, Value::int(cab as i64)),
                (key::PARAM_INDEX, Value::int(index)),
                (71, Value::int(cc)),
            ]),
        )?;
        Ok(())
    }

    /// Put a block's on/off on footswitch `fs` (1-based): op 56, then op 33 for that switch.
    pub fn assign_footswitch(&mut self, block: i64, fs: i64) -> Result<()> {
        self.request(op::ASSIGN_FOOTSWITCH, Value::map(vec![(key::BLOCK, Value::int(block)), (102, Value::int(fs - 1))]))?;
        self.request(op::FOOTSWITCH_INDEX, Value::map(vec![(102, Value::int(fs))]))?;
        Ok(())
    }

    /// Take a block's on/off off footswitch `fs` (op 57).
    pub fn unassign_footswitch(&mut self, block: i64, fs: i64) -> Result<()> {
        self.request(op::UNASSIGN_FOOTSWITCH, Value::map(vec![(key::BLOCK, Value::int(block)), (102, Value::int(fs - 1))]))?;
        Ok(())
    }

    // ------------------------------------------------------------ user IRs ---

    /// The pedal's User IR slots that hold something: (slot from 0, name).
    pub fn list_irs(&mut self) -> Result<Vec<(i64, String)>> {
        self.open_browse();
        let (_, v) = self.request(op::LIST_IRS, Value::map(vec![(key::ARGS, Value::int(2))]))?;
        let mut out: Vec<(i64, String)> = v
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|e| {
                        let slot = e.get(IR_SLOT).and_then(Value::as_i64)?;
                        let name = e.get(key::NAME).and_then(Value::as_str).map(clean).unwrap_or_default();
                        Some((slot, name))
                    })
                    .collect()
            })
            .unwrap_or_default();
        out.sort_by_key(|(s, _)| *s);
        Ok(out)
    }

    /// One User IR's 2,048 samples as the pedal stores them (8,192 bytes, 32-bit float).
    pub fn ir_audio(&mut self, slot: i64) -> Result<Vec<u8>> {
        self.open_browse();
        let _ = self.request(op::IR_INFO, Value::map(vec![(IR_SLOT, Value::int(slot))]));
        let (_, v) = self.request(op::IR_AUDIO, Value::map(vec![(IR_SLOT, Value::int(slot)), (key::ARGS, Value::int(2))]))?;
        let bytes = match &v {
            Value::Bin(b, _) => b.clone(),
            Value::Str(s, _) => s.as_bytes().to_vec(),
            _ => Vec::new(),
        };
        if bytes.len() != IR_AUDIO_BYTES {
            return Err(SessionError::Protocol(format!("the pedal sent no audio for IR slot {}", slot + 1)));
        }
        Ok(bytes)
    }

    /// Put an IR into a User IR slot (replacing what is there).
    pub fn import_ir(&mut self, slot: i64, name: &str, audio: &[u8]) -> Result<()> {
        if audio.len() != IR_AUDIO_BYTES {
            return Err(SessionError::Protocol(format!("an IR is {IR_AUDIO_BYTES} bytes, not {}", audio.len())));
        }
        if !(0..IR_SLOTS).contains(&slot) {
            return Err(SessionError::Protocol(format!("there is no IR slot {}", slot + 1)));
        }
        // The checksum: the sum of the audio's little-endian 32-bit words.
        let sum = audio.chunks_exact(4).fold(0u32, |a, w| a.wrapping_add(u32::from_le_bytes([w[0], w[1], w[2], w[3]])));
        self.browse_open = false;
        self.open_browse();
        self.begin_write()?;
        if self.wire.slow() {
            let _ = self.idle(Duration::from_millis(400));
        }
        self.request(
            op::IMPORT_IR,
            Value::map(vec![
                (IR_SLOT, Value::int(slot)),
                (113, Value::Int(sum as i64, 0xce)),
                (key::NAME, wire_name(name, IR_NAME_MAX)),
                (114, Value::int(1)),
                (115, Value::int(3)),
                (123, Value::Bool(false)),
                (124, Value::Bool(false)),
                (125, Value::int(0)),
                (key::DOCUMENT, Value::Bin(audio.to_vec(), 0xda)),
            ]),
        )?;
        self.browse_open = false;
        std::thread::sleep(SETTLE_FLASH);
        Ok(())
    }

    pub fn rename_ir(&mut self, slot: i64, name: &str) -> Result<()> {
        self.open_browse();
        self.request(op::RENAME_IR, Value::map(vec![(IR_SLOT, Value::int(slot)), (key::NAME, wire_name(name, IR_NAME_MAX))]))?;
        self.browse_open = false;
        std::thread::sleep(SETTLE_FLASH);
        Ok(())
    }

    pub fn delete_ir(&mut self, slot: i64) -> Result<()> {
        self.open_browse();
        self.request(op::DELETE_IR, Value::map(vec![(IR_SLOT, Value::int(slot))]))?;
        self.browse_open = false;
        std::thread::sleep(SETTLE_FLASH);
        Ok(())
    }

    pub fn rename_snapshot(&mut self, index: i64, name: &str) -> Result<()> {
        self.request(
            op::RENAME_SNAPSHOT,
            Value::map(vec![(key::SNAPSHOT, Value::int(index)), (key::NAME, wire_name(name, SNAPSHOT_NAME_MAX))]),
        )?;
        Ok(())
    }
}

/// Longest preset and snapshot names the pedal keeps (IRs: [`IR_NAME_MAX`]).
pub const PRESET_NAME_MAX: usize = 16;
pub const SNAPSHOT_NAME_MAX: usize = 10;

/// What a name becomes on the pedal: printable ASCII only (common accented letters lose
/// their accent rather than disappearing), at most `max_len` characters.
pub fn pedal_name(name: &str, max_len: usize) -> String {
    let mut s = String::new();
    for c in clean(name).chars() {
        let plain = match c {
            ' '..='~' => None,
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => Some("a"),
            'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' => Some("A"),
            'ç' => Some("c"),
            'Ç' => Some("C"),
            'è' | 'é' | 'ê' | 'ë' => Some("e"),
            'È' | 'É' | 'Ê' | 'Ë' => Some("E"),
            'ì' | 'í' | 'î' | 'ï' => Some("i"),
            'Ì' | 'Í' | 'Î' | 'Ï' => Some("I"),
            'ñ' => Some("n"),
            'Ñ' => Some("N"),
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' => Some("o"),
            'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' | 'Ø' => Some("O"),
            'ù' | 'ú' | 'û' | 'ü' => Some("u"),
            'Ù' | 'Ú' | 'Û' | 'Ü' => Some("U"),
            'ý' | 'ÿ' => Some("y"),
            'Ý' => Some("Y"),
            'ß' => Some("ss"),
            'æ' => Some("ae"),
            'Æ' => Some("AE"),
            'œ' => Some("oe"),
            'Œ' => Some("OE"),
            '‘' | '’' => Some("'"),
            '“' | '”' => Some("\""),
            '–' | '—' => Some("-"),
            _ => Some(""),
        };
        match plain {
            None => s.push(c),
            Some(p) => s.push_str(p),
        }
    }
    let mut s: String = s.chars().take(max_len).collect::<String>().trim_end().to_owned();
    if s.trim().is_empty() {
        s = "New Preset".chars().take(max_len).collect();
    }
    s
}

/// A note for the person when a name will read differently on the pedal.
pub fn name_note(name: &str, max_len: usize) -> Option<String> {
    let on_pedal = pedal_name(name, max_len);
    (on_pedal != clean(name)).then(|| {
        format!("On the pedal the name reads \"{on_pedal}\" (it keeps up to {max_len} plain letters, digits and symbols)")
    })
}

/// A name as the pedal takes it, with the trailing zero byte it expects.
fn wire_name(name: &str, max_len: usize) -> Value {
    let mut s = pedal_name(name, max_len);
    s.push('\0');
    Value::str(&s)
}

/// Set HX_DEBUG to print every transfer.
fn debug() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("HX_DEBUG").is_some())
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Names arrive padded with a zero byte and sometimes trailing spaces.
pub fn clean(s: &str) -> String {
    s.trim_end_matches('\0').trim_end().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_keep_letters_without_accents() {
        assert_eq!(pedal_name("Café Lead", PRESET_NAME_MAX), "Cafe Lead");
        assert_eq!(pedal_name("Señor Crunch Über Long", PRESET_NAME_MAX), "Senor Crunch Ube");
        assert_eq!(pedal_name("Ritmo ★", PRESET_NAME_MAX), "Ritmo");
        assert_eq!(pedal_name("   ", PRESET_NAME_MAX), "New Preset");
        assert_eq!(pedal_name("Clean\0", PRESET_NAME_MAX), "Clean");
        assert!(name_note("Clean", PRESET_NAME_MAX).is_none());
        assert!(name_note("Café", PRESET_NAME_MAX).unwrap().contains("\"Cafe\""));
        assert!(name_note("A very long preset name", PRESET_NAME_MAX).is_some());
    }
}
