//! A fake pedal behind the `Wire` trait, for testing `Session` without hardware: it
//! answers channel handshakes, reassembles each channel's byte stream, replies to
//! requests through a script, and can send "still there?" pings or fail a transfer.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::msgpack::Value;
use crate::proto::*;
use crate::session::{Session, SessionError};
use crate::wire::{Wire, TIMEOUT};

/// How the fake pedal answers one request.
pub enum Reply {
    Ok(Value),
    /// Status 255 with this error code.
    Refuse(i64),
    /// No answer at all.
    Silent,
}

type Script = Box<dyn FnMut(i64, &Value) -> Reply + Send>;

/// What the test can look at and steer while the session owns the wire.
#[derive(Default)]
pub struct Shared {
    /// Every request the session made: (opcode, args).
    pub requests: Vec<(i64, Value)>,
    /// Every frame the session sent: (pedal-side channel, message type, data bytes).
    pub frames: Vec<(u16, u16, usize)>,
    /// Allow this many more data frames, then fail every send (None: never fail).
    pub sends_left: Option<usize>,
    /// Transfers waiting to be read by the session.
    pub inbox: VecDeque<Vec<u8>>,
}

pub struct FakePedal {
    shared: Arc<Mutex<Shared>>,
    streams: HashMap<u16, StreamReader>,
    seq: HashMap<u16, u16>,
    script: Script,
}

impl FakePedal {
    pub fn new(script: impl FnMut(i64, &Value) -> Reply + Send + 'static) -> (FakePedal, Arc<Mutex<Shared>>) {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let fake = FakePedal { shared: shared.clone(), streams: HashMap::new(), seq: HashMap::new(), script: Box::new(script) };
        (fake, shared)
    }

    fn next_seq(&mut self, ch: u16) -> u16 {
        let s = self.seq.entry(ch).or_insert(0);
        *s = s.wrapping_add(1);
        *s
    }

    /// A frame from the pedal's node `ch` to the computer's node `host`.
    fn frame(&mut self, ch: u16, host: u16, msg_type: u16, body: &[u8]) -> Vec<u8> {
        let mut payload = Vec::new();
        let seq = self.next_seq(ch);
        ChannelHeader { seq, msg_type, ack: 0x1000 }.encode(&mut payload);
        payload.extend_from_slice(body);
        Frame::new(host, ch, payload).encode()
    }

    /// Put a message into the stream from pedal to computer, in frames.
    fn reply(&mut self, ch: u16, host: u16, service: u16, v: &Value) {
        let body = v.encode();
        let mut stream = Vec::new();
        stream.extend_from_slice(&2u16.to_le_bytes());
        stream.extend_from_slice(&service.to_le_bytes());
        stream.extend_from_slice(&(body.len() as u32).to_le_bytes());
        stream.extend_from_slice(&body);
        for chunk in stream.chunks(1024) {
            let f = self.frame(ch, host, MSG_DATA, chunk);
            self.shared.lock().unwrap().inbox.push_back(f);
        }
    }

    fn handle(&mut self, f: Frame) {
        let Some((hdr, body)) = ChannelHeader::decode(&f.payload) else { return };
        let (ch, host) = (f.dst, f.src);
        self.shared.lock().unwrap().frames.push((ch, hdr.msg_type, if hdr.has_data() { body.len() } else { 0 }));
        if f.flags == FLAG_HANDSHAKE || hdr.msg_type == MSG_HELLO {
            let ack = self.frame(ch, host, MSG_ACK, &[]);
            self.shared.lock().unwrap().inbox.push_back(ack);
            return;
        }
        if !hdr.has_data() || body.is_empty() {
            return; // an acknowledgement
        }
        // The service number rides in each message's stream prefix (bytes 2..4).
        let reader = self.streams.entry(ch).or_default();
        reader.push(body);
        for msg in reader.take_messages() {
            let (Some(txn), Some(opcode)) = (msg.get(key::TXN).and_then(Value::as_i64), msg.get(key::OPCODE).and_then(Value::as_i64)) else {
                // Opening a service: just confirm.
                let ack = self.frame(ch, host, MSG_ACK, &[]);
                self.shared.lock().unwrap().inbox.push_back(ack);
                continue;
            };
            let args = msg.get(key::ARGS).cloned().unwrap_or(Value::Nil);
            self.shared.lock().unwrap().requests.push((opcode, args.clone()));
            let service = service_for(opcode);
            match (self.script)(opcode, &args) {
                Reply::Ok(result) => {
                    let v = Value::map(vec![(key::TXN, Value::int(txn)), (key::STATUS, Value::int(0)), (key::RESULT, result)]);
                    self.reply(ch, host, service, &v);
                }
                Reply::Refuse(code) => {
                    let v = Value::map(vec![
                        (key::TXN, Value::int(txn)),
                        (key::STATUS, Value::int(255)),
                        (key::RESULT, Value::map(vec![(key::ERROR_CODE, Value::int(code))])),
                    ]);
                    self.reply(ch, host, service, &v);
                }
                Reply::Silent => {}
            }
        }
    }

    /// Queue the pedal's "still there?" (type 0x10, no data) on a channel.
    pub fn ping(shared: &Arc<Mutex<Shared>>, ch: ChannelId) {
        let mut payload = Vec::new();
        ChannelHeader { seq: 99, msg_type: 0x10, ack: 0x1000 }.encode(&mut payload);
        shared.lock().unwrap().inbox.push_back(Frame::new(ch.host, ch.device, payload).encode());
    }
}

impl Wire for FakePedal {
    fn send(&mut self, bytes: &[u8]) -> Result<(), String> {
        let (frames, _) = Frame::decode_stream(bytes);
        for f in frames {
            let carries_data = ChannelHeader::decode(&f.payload).is_some_and(|(h, b)| h.has_data() && !b.is_empty());
            if carries_data {
                let mut s = self.shared.lock().unwrap();
                match s.sends_left {
                    Some(0) => return Err("USB write failed: unplugged".into()),
                    Some(n) => s.sends_left = Some(n - 1),
                    None => {}
                }
            }
            self.handle(f);
        }
        Ok(())
    }

    fn recv(&mut self, timeout: Duration) -> Result<Vec<u8>, String> {
        if let Some(t) = self.shared.lock().unwrap().inbox.pop_front() {
            return Ok(t);
        }
        std::thread::sleep(timeout.min(Duration::from_millis(2)));
        Err(TIMEOUT.into())
    }

    fn describe(&self) -> String {
        "fake pedal".into()
    }
}

/// A pedal that answers preset info, browsing and writes with plain success.
fn friendly(opcode: i64, _args: &Value) -> Reply {
    match opcode {
        op::PRESET_INFO => Reply::Ok(Value::map(vec![
            (key::SETLIST, Value::int(0)),
            (key::PRESET_INDEX, Value::int(4)),
            (key::NAME, Value::str("Clean Tone\0")),
        ])),
        _ => Reply::Ok(Value::Nil),
    }
}

fn open(script: impl FnMut(i64, &Value) -> Reply + Send + 'static) -> (Session, Arc<Mutex<Shared>>) {
    let (fake, shared) = FakePedal::new(script);
    let s = Session::open(Box::new(fake)).unwrap_or_else(|e| panic!("session didn't open: {e}"));
    (s, shared)
}

/// A pedal document big enough to need several 256-byte chunks.
fn big_document() -> Vec<u8> {
    let mut pairs = Vec::new();
    for k in [0i64, 1, 2, 3, 4, 5, 6, 7, 10] {
        pairs.push((Value::int(k), Value::map(vec![(1, Value::str(&"x".repeat(100)))])));
    }
    let mut blob = Value::str("l6-helix\0").encode();
    blob.extend(Value::Bin(vec![0; 48], 0xc4).encode());
    blob.extend(Value::Map(pairs, 0x80).encode());
    // Re-encode so the offset table is right.
    crate::devpreset::DevicePreset::parse(&blob).unwrap().encode()
}

#[test]
fn opens_and_reads_the_loaded_preset() {
    let (mut s, shared) = open(friendly);
    assert_eq!(s.preset_info().unwrap(), (0, 4, "Clean Tone".to_owned()));
    // The order the pedal expects: control opened then closed, data, events.
    let hellos: Vec<u16> = shared.lock().unwrap().frames.iter().filter(|(_, t, _)| *t == MSG_HELLO).map(|(c, _, _)| *c).collect();
    assert_eq!(hellos, vec![ChannelId::CONTROL.device, ChannelId::CONTROL.device, ChannelId::DATA.device, ChannelId::EVENTS.device]);
}

#[test]
fn a_refusal_carries_the_pedals_reason() {
    let (mut s, _) = open(|op, args| if op == op::SET_PARAM { Reply::Refuse(-306) } else { friendly(op, args) });
    let e = s.set_param(3, 0, 1, Value::F64(0.5)).unwrap_err();
    assert!(matches!(e, SessionError::Refused(-306)));
    assert!(e.to_string().contains("DSP"));
}

#[test]
fn no_answer_is_a_timeout_not_a_hang() {
    let (mut s, _) = open(|op, args| if op == op::SET_ENABLED { Reply::Silent } else { friendly(op, args) });
    let t = std::time::Instant::now();
    assert!(matches!(s.set_enabled(2, true), Err(SessionError::Timeout)));
    assert!(t.elapsed() < Duration::from_secs(5));
    // The session still works afterwards.
    assert!(s.preset_info().is_ok());
}

#[test]
fn a_slot_write_arrives_whole_after_begin() {
    let (mut s, shared) = open(friendly);
    let doc = big_document();
    let sent = s.write_slot(0, 7, "Café Lead", doc.clone()).unwrap();
    assert_eq!(sent, doc);
    let reqs = &shared.lock().unwrap().requests;
    let ops: Vec<i64> = reqs.iter().map(|(o, _)| *o).collect();
    let begin = ops.iter().position(|o| *o == op::BEGIN).expect("BEGIN sent");
    let write = ops.iter().position(|o| *o == op::WRITE_SLOT_NAMED).expect("write sent");
    assert!(begin < write);
    let args = &reqs[write].1;
    assert_eq!(args.get(key::DOCUMENT).and_then(Value::as_bin), Some(&doc[..]), "the document arrives byte for byte");
    assert_eq!(args.get(key::NAME).and_then(Value::as_str), Some("Cafe Lead\0"));
    assert_eq!(args.get(key::PRESET_INDEX).and_then(Value::as_i64), Some(7));
}

#[test]
fn a_refused_begin_is_a_warning_but_silence_stops_the_write() {
    let (mut s, _) = open(|op, args| if op == op::BEGIN { Reply::Refuse(-1) } else { friendly(op, args) });
    s.write_slot(0, 1, "A", big_document()).unwrap();
    assert_eq!(s.take_warnings().len(), 1);

    let (mut s, shared) = open(|op, args| if op == op::BEGIN { Reply::Silent } else { friendly(op, args) });
    assert!(s.write_slot(0, 1, "A", big_document()).is_err());
    assert!(!shared.lock().unwrap().requests.iter().any(|(o, _)| *o == op::WRITE_SLOT_NAMED), "nothing written");
}

#[test]
fn a_transfer_cut_half_way_stops_all_sending() {
    let (mut s, shared) = open(friendly);
    s.preset_info().unwrap();
    // write_slot first makes three one-frame requests (BROWSE_OPEN, LIST_SETLISTS, BEGIN);
    // let those and the document's first chunk through, then fail its second chunk.
    shared.lock().unwrap().sends_left = Some(3 + 1);
    let e = s.write_slot(0, 1, "A", big_document()).unwrap_err();
    assert!(e.to_string().contains("USB write failed"), "{e}");
    // From now on nothing more is sent: the pedal may be mid-transfer.
    shared.lock().unwrap().sends_left = None;
    let frames_before = shared.lock().unwrap().frames.len();
    let e = s.preset_info().unwrap_err();
    assert!(e.to_string().contains("power-cycle"), "{e}");
    assert_eq!(shared.lock().unwrap().frames.len(), frames_before, "no frame sent after the break");
}

#[test]
fn pings_get_acknowledged() {
    let (mut s, shared) = open(friendly);
    let n = shared.lock().unwrap().frames.len();
    FakePedal::ping(&shared, ChannelId::EVENTS);
    s.idle(Duration::from_millis(30)).unwrap();
    let acked = shared.lock().unwrap().frames[n..].iter().any(|(c, t, _)| *c == ChannelId::EVENTS.device && *t == MSG_ACK);
    assert!(acked, "the ping on the events channel must be answered");
}
