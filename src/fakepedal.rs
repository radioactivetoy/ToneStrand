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
            let (Some(txn), Some(opcode)) = (msg.get(key::TXN).and_then(Value::as_i64), msg.get(key::OPCODE).and_then(Value::as_i64))
            else {
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

// ------------------------------------------------- copying, moving and swapping slots ---

use crate::device::{slot_op, Event, SlotOp};
use crate::msgpack::AUTO;

type Slots = Arc<Mutex<HashMap<i64, (String, Vec<u8>)>>>;

/// A pedal document whose content depends on `tag`.
fn tagged_document(tag: &str) -> Vec<u8> {
    let mut pairs = Vec::new();
    for k in [0i64, 1, 2, 3, 4, 5, 6, 7, 10] {
        pairs.push((Value::int(k), Value::map(vec![(1, Value::str(&format!("{tag}{}", "x".repeat(100))))])));
    }
    let mut blob = Value::str("l6-helix\0").encode();
    blob.extend(Value::Bin(vec![0; 48], 0xc4).encode());
    blob.extend(Value::Map(pairs, 0x80).encode());
    crate::devpreset::DevicePreset::parse(&blob).unwrap().encode()
}

/// A pedal that really stores presets in eight slots (slot index -> name and document).
/// Writes to `corrupt` come back changed, as a failing flash would.
fn pedal_with(slots: &[(i64, &str, Vec<u8>)], corrupt: Option<i64>) -> (Session, Slots) {
    let store: Slots = Arc::new(Mutex::new(slots.iter().map(|(i, n, d)| (*i, (n.to_string(), d.clone()))).collect::<HashMap<_, _>>()));
    let st = store.clone();
    let (s, _) = open(move |opcode, args| {
        let index = args.get(key::PRESET_INDEX).and_then(Value::as_i64).unwrap_or(-1);
        match opcode {
            op::LIST_PRESETS => {
                let names: Vec<Value> = (0..8)
                    .map(|i| {
                        let name = st.lock().unwrap().get(&i).map(|(n, _)| n.clone()).unwrap_or_default();
                        Value::Map(vec![(Value::int(i), Value::map(vec![(key::NAME, Value::str(&format!("{name}\0")))]))], AUTO)
                    })
                    .collect();
                Reply::Ok(Value::Array(names, AUTO))
            }
            op::FETCH_PRESET => Reply::Ok(Value::Bin(st.lock().unwrap().get(&index).map(|(_, d)| d.clone()).unwrap_or_default(), 0xc4)),
            op::WRITE_SLOT_NAMED => {
                let name = args.get(key::NAME).and_then(Value::as_str).unwrap_or("").trim_end_matches('\0').to_owned();
                let mut doc = args.get(key::DOCUMENT).and_then(Value::as_bin).unwrap_or(&[]).to_vec();
                if corrupt == Some(index) {
                    let last = doc.len() - 1;
                    doc[last] = b'y';
                }
                st.lock().unwrap().insert(index, (name, doc));
                Reply::Ok(Value::Nil)
            }
            op::CLEAR_SLOT => {
                st.lock().unwrap().remove(&index);
                Reply::Ok(Value::Nil)
            }
            _ => friendly(opcode, args),
        }
    });
    (s, store)
}

fn scratch_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("tonestrand-slotop-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

/// The documents kept in a backup folder.
fn kept(dir: &std::path::Path) -> Vec<Vec<u8>> {
    std::fs::read_dir(dir).map(|rd| rd.flatten().filter_map(|e| std::fs::read(e.path()).ok()).collect()).unwrap_or_default()
}

fn quiet(_: Event) {}

#[test]
fn copy_leaves_the_source_and_keeps_the_preset_it_replaces() {
    let (a, b) = (tagged_document("A"), tagged_document("B"));
    let (mut s, slots) = pedal_with(&[(1, "Alpha", a.clone()), (2, "Beta", b.clone())], None);
    let dir = scratch_dir("copy");
    slot_op(&mut s, &quiet, 0, 1, 2, SlotOp::Copy, &dir).unwrap();
    let st = slots.lock().unwrap();
    assert_eq!(st[&2], ("Alpha".to_owned(), a.clone()));
    assert_eq!(st[&1], ("Alpha".to_owned(), a));
    assert_eq!(kept(&dir), vec![b], "the replaced preset is in the backup folder");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn move_empties_the_source_and_keeps_it() {
    let a = tagged_document("A");
    let (mut s, slots) = pedal_with(&[(1, "Alpha", a.clone())], None);
    let dir = scratch_dir("move");
    slot_op(&mut s, &quiet, 0, 1, 3, SlotOp::Move, &dir).unwrap();
    let st = slots.lock().unwrap();
    assert_eq!(st[&3], ("Alpha".to_owned(), a.clone()));
    assert!(!st.contains_key(&1));
    assert_eq!(kept(&dir), vec![a], "the source is kept before it is emptied (the empty target needs nothing)");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn swap_exchanges_the_two_and_keeps_both() {
    let (a, b) = (tagged_document("A"), tagged_document("B"));
    let (mut s, slots) = pedal_with(&[(1, "Alpha", a.clone()), (2, "Beta", b.clone())], None);
    let dir = scratch_dir("swap");
    slot_op(&mut s, &quiet, 0, 1, 2, SlotOp::Swap, &dir).unwrap();
    let st = slots.lock().unwrap();
    assert_eq!(st[&1], ("Beta".to_owned(), b.clone()));
    assert_eq!(st[&2], ("Alpha".to_owned(), a.clone()));
    let mut got = kept(&dir);
    got.sort();
    let mut want = vec![a, b];
    want.sort();
    assert_eq!(got, want);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn swapping_with_an_empty_slot_is_a_move() {
    let a = tagged_document("A");
    let (mut s, slots) = pedal_with(&[(1, "Alpha", a.clone())], None);
    let dir = scratch_dir("swap-empty");
    slot_op(&mut s, &quiet, 0, 1, 5, SlotOp::Swap, &dir).unwrap();
    let st = slots.lock().unwrap();
    assert_eq!(st[&5], ("Alpha".to_owned(), a));
    assert!(!st.contains_key(&1));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_bad_read_back_stops_before_the_source_is_touched() {
    let (a, b) = (tagged_document("A"), tagged_document("B"));
    let (mut s, slots) = pedal_with(&[(1, "Alpha", a.clone()), (2, "Beta", b.clone())], Some(2));
    let dir = scratch_dir("corrupt");
    let e = slot_op(&mut s, &quiet, 0, 1, 2, SlotOp::Move, &dir).unwrap_err();
    assert!(e.contains("reads back different"), "{e}");
    let st = slots.lock().unwrap();
    assert_eq!(st[&1], ("Alpha".to_owned(), a), "the source was not emptied");
    assert!(kept(&dir).contains(&b), "the preset that was overwritten is in the backup folder");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_empty_source_or_the_same_slot_is_refused_without_writing() {
    let (mut s, slots) = pedal_with(&[(1, "Alpha", tagged_document("A"))], None);
    let dir = scratch_dir("refuse");
    assert!(slot_op(&mut s, &quiet, 0, 4, 1, SlotOp::Copy, &dir).unwrap_err().contains("empty"));
    assert!(slot_op(&mut s, &quiet, 0, 1, 1, SlotOp::Swap, &dir).is_err());
    assert_eq!(slots.lock().unwrap().len(), 1);
    assert!(kept(&dir).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_snapshot_copy_uses_the_pedals_copy_and_paste_on_the_current_snapshot() {
    let (mut s, shared) = open(friendly);
    s.copy_snapshot(0, 2, "LEAD").unwrap();
    let reqs: Vec<(i64, i64)> = shared
        .lock()
        .unwrap()
        .requests
        .iter()
        .filter(|(o, _)| [op::SELECT_SNAPSHOT, op::COPY_SNAPSHOT, op::PASTE_SNAPSHOT, op::RENAME_SNAPSHOT].contains(o))
        .map(|(o, a)| (*o, a.get(key::SNAPSHOT).and_then(Value::as_i64).unwrap_or(-1)))
        .collect();
    // Switch to the source, copy, switch to the destination, paste, then put its name back.
    assert_eq!(
        reqs,
        vec![(op::SELECT_SNAPSHOT, 0), (op::COPY_SNAPSHOT, 0), (op::SELECT_SNAPSHOT, 2), (op::PASTE_SNAPSHOT, 2), (op::RENAME_SNAPSHOT, 2)]
    );
}

#[test]
fn an_edit_buffer_write_sends_the_document_with_op_21_and_nothing_else_is_saved() {
    let (mut s, shared) = open(friendly);
    let doc = big_document();
    s.write_edit_buffer(doc.clone()).unwrap();
    let reqs = shared.lock().unwrap().requests.clone();
    let sent: Vec<&(i64, Value)> = reqs.iter().filter(|(o, _)| *o == op::WRITE_EDIT_BUFFER).collect();
    assert_eq!(sent.len(), 1);
    assert!(matches!(sent[0].1.get(key::DOCUMENT), Some(Value::Bin(b, _)) if *b == doc));
    assert!(!reqs.iter().any(|(o, _)| [op::WRITE_SLOT_NAMED, op::SAVE_PRESET].contains(o)), "no slot is written, nothing saved");
    // Something that isn't a pedal document is refused before it is sent.
    assert!(s.write_edit_buffer(b"nonsense".to_vec()).is_err());
    assert_eq!(shared.lock().unwrap().requests.iter().filter(|(o, _)| *o == op::WRITE_EDIT_BUFFER).count(), 1);
}
