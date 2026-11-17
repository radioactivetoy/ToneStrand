//! A logging pass-through between another editor and a Helrig Box, to see what that editor sends the
//! pedal (for example a Command Center change). Run on this computer as
//! `tonestrand --box-proxy <listen address:port> <box address> [folder for documents]`, then point the
//! other editor at this computer's address instead of the box's. Everything is forwarded byte for
//! byte, so the other editor keeps its own sign-in with the box; only what travels is read.
//!
//! What is printed: the HTTP request line of each connection (no headers, no query), and for the
//! relay's WebSocket the pedal's own messages: requests (opcode and arguments) and replies. Documents
//! (preset documents over about 1 KB) are saved to the folder, when given, so they can be read with
//! `--commands` or `--dump-doc`. Tokens and headers are never printed or stored.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::msgpack::Value;
use crate::proto::{key, ChannelHeader, Frame, StreamReader};

/// A short text for a message value: big blobs and long lists are summarised.
pub fn brief(v: &Value, depth: usize) -> String {
    match v {
        Value::Nil => "nil".into(),
        Value::Bool(b) => b.to_string(),
        Value::Int(n, _) => n.to_string(),
        Value::F32(f) => format!("{f}f"),
        Value::F64(f) => format!("{f}d"),
        Value::Str(s, _) => format!("{:?}", s.trim_end_matches('\0')),
        Value::Bin(b, _) => format!("bin({} bytes)", b.len()),
        Value::Array(a, _) if depth == 0 => format!("[{} items]", a.len()),
        Value::Array(a, _) => {
            let shown: Vec<String> = a.iter().take(16).map(|x| brief(x, depth - 1)).collect();
            let more = if a.len() > 16 { format!(", ...+{}", a.len() - 16) } else { String::new() };
            format!("[{}{more}]", shown.join(", "))
        }
        Value::Map(m, _) if depth == 0 => format!("{{{} keys}}", m.len()),
        Value::Map(m, _) => {
            format!("{{{}}}", m.iter().map(|(k, v)| format!("{}: {}", brief(k, 0), brief(v, depth - 1))).collect::<Vec<_>>().join(", "))
        }
        #[allow(unreachable_patterns)]
        _ => "?".into(),
    }
}

/// One direction of one connection: the HTTP start, then WebSocket frames, then pedal frames and messages.
struct Tap {
    /// "computer -> box" or "box -> computer".
    name: &'static str,
    from_computer: bool,
    started: Instant,
    out: Arc<Mutex<Out>>,
    buf: Vec<u8>,
    state: State,
    frag: Vec<u8>,
    frag_text: bool,
    leftover: Vec<u8>,
    readers: HashMap<u16, StreamReader>,
}

#[derive(PartialEq)]
enum State {
    Http,
    WebSocket,
    Plain,
}

/// Where lines and documents go.
struct Out {
    log: Option<std::fs::File>,
    docs: Option<PathBuf>,
    saved: usize,
}

impl Out {
    fn line(&mut self, text: &str) {
        println!("{text}");
        if let Some(f) = &mut self.log {
            let _ = writeln!(f, "{text}");
        }
    }
}

impl Tap {
    fn new(name: &'static str, from_computer: bool, started: Instant, out: Arc<Mutex<Out>>) -> Tap {
        Tap {
            name,
            from_computer,
            started,
            out,
            buf: Vec::new(),
            state: State::Http,
            frag: Vec::new(),
            frag_text: false,
            leftover: Vec::new(),
            readers: HashMap::new(),
        }
    }

    fn say(&self, text: &str) {
        let ms = self.started.elapsed().as_millis();
        if let Ok(mut o) = self.out.lock() {
            o.line(&format!("{:>7.3}s {} {text}", ms as f64 / 1000.0, if self.from_computer { "->" } else { "<-" }));
        }
    }

    fn feed(&mut self, data: &[u8]) {
        match self.state {
            State::Plain => {}
            State::Http => {
                self.buf.extend_from_slice(data);
                let Some(end) = self.buf.windows(4).position(|w| w == b"\r\n\r\n") else { return };
                let head = String::from_utf8_lossy(&self.buf[..end]).into_owned();
                let rest = self.buf[end + 4..].to_vec();
                self.buf.clear();
                let first = head.lines().next().unwrap_or("");
                // The request line without any query; the response's status line as it is.
                let first = if self.from_computer { first.split('?').next().unwrap_or("").to_owned() } else { first.to_owned() };
                let upgrade = head.to_ascii_lowercase().contains("upgrade: websocket") || head.contains(" 101 ");
                self.say(&format!("http: {first}"));
                if upgrade {
                    self.state = State::WebSocket;
                    self.ws(&rest);
                } else {
                    self.state = State::Plain;
                }
            }
            State::WebSocket => self.ws(data),
        }
    }

    /// Read WebSocket frames (client frames are masked) and hand each completed message on.
    fn ws(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
        loop {
            let b = &self.buf;
            if b.len() < 2 {
                return;
            }
            let (fin, rsv1, opcode) = (b[0] & 0x80 != 0, b[0] & 0x40 != 0, b[0] & 0x0f);
            let masked = b[1] & 0x80 != 0;
            let mut len = (b[1] & 0x7f) as usize;
            let mut at = 2;
            if len == 126 {
                if b.len() < 4 {
                    return;
                }
                len = u16::from_be_bytes([b[2], b[3]]) as usize;
                at = 4;
            } else if len == 127 {
                if b.len() < 10 {
                    return;
                }
                len = u64::from_be_bytes(b[2..10].try_into().unwrap_or([0; 8])) as usize;
                at = 10;
            }
            let mask: Option<[u8; 4]> = masked.then(|| b.get(at..at + 4).and_then(|m| m.try_into().ok())).flatten();
            if masked {
                at += 4;
            }
            if b.len() < at + len {
                return;
            }
            let mut payload = b[at..at + len].to_vec();
            if let Some(m) = mask {
                for (i, x) in payload.iter_mut().enumerate() {
                    *x ^= m[i % 4];
                }
            }
            self.buf.drain(..at + len);
            if rsv1 {
                self.say("a compressed WebSocket frame: not readable here");
                continue;
            }
            match opcode {
                0x1 | 0x2 => {
                    self.frag = payload;
                    self.frag_text = opcode == 0x1;
                }
                0x0 => self.frag.extend_from_slice(&payload),
                0x8 => {
                    self.say("close");
                    continue;
                }
                _ => continue, // ping, pong
            }
            if fin {
                let msg = std::mem::take(&mut self.frag);
                if self.frag_text {
                    let t = String::from_utf8_lossy(&msg);
                    // The box's own chatter: pings, pongs, hello, errors.
                    if !t.contains("\"ping\"") && !t.contains("\"pong\"") {
                        self.say(&format!("text: {}", t.chars().take(200).collect::<String>()));
                    }
                } else {
                    self.transfer(&msg);
                }
            }
        }
    }

    /// One USB transfer: frames of the pedal's channels, reassembled into request/reply messages.
    fn transfer(&mut self, data: &[u8]) {
        self.leftover.extend_from_slice(data);
        let buf = std::mem::take(&mut self.leftover);
        let (frames, rest) = Frame::decode_stream(&buf);
        self.leftover = rest;
        for f in frames {
            let Some((hdr, body)) = ChannelHeader::decode(&f.payload) else { continue };
            if !hdr.has_data() || body.is_empty() {
                continue;
            }
            let channel = if self.from_computer { f.dst } else { f.src };
            let reader = self.readers.entry(channel).or_default();
            reader.push(body);
            for msg in reader.take_messages() {
                self.message(&msg);
            }
        }
    }

    fn message(&mut self, msg: &Value) {
        let txn = msg.get(key::TXN).and_then(Value::as_i64);
        if let (Some(op), true) = (msg.get(key::OPCODE).and_then(Value::as_i64), self.from_computer) {
            let args = msg.get(key::ARGS).cloned().unwrap_or(Value::Nil);
            self.say(&format!("request txn {} op {op}: {}", txn.unwrap_or(-1), brief(&args, 4)));
            self.save_documents(&args, &format!("op{op} txn{}", txn.unwrap_or(-1)));
        } else if let Some(status) = msg.get(key::STATUS).and_then(Value::as_i64) {
            let result = msg.get(key::RESULT).cloned().unwrap_or(Value::Nil);
            self.say(&format!("reply txn {} status {status}: {}", txn.unwrap_or(-1), brief(&result, 3)));
            self.save_documents(&result, &format!("reply txn{}", txn.unwrap_or(-1)));
        } else if let Some(ev) = msg.get(key::EVENT).and_then(Value::as_i64) {
            self.say(&format!("event {ev}: {}", brief(msg.get(key::EVENT_ARGS).unwrap_or(&Value::Nil), 2)));
        } else {
            self.say(&format!("message: {}", brief(msg, 3)));
        }
    }

    /// Save any preset document (a large blob) found in a message's arguments or result.
    fn save_documents(&self, v: &Value, what: &str) {
        let blobs: Vec<&Vec<u8>> = match v {
            Value::Map(m, _) => m.iter().filter_map(|(_, x)| if let Value::Bin(b, _) = x { Some(b) } else { None }).collect(),
            Value::Bin(b, _) => vec![b],
            _ => Vec::new(),
        };
        for b in blobs.into_iter().filter(|b| b.len() > 1000) {
            let Ok(mut o) = self.out.lock() else { return };
            let Some(dir) = o.docs.clone() else { continue };
            o.saved += 1;
            let path = dir.join(format!("{:03} {what}.hxpreset", o.saved));
            match std::fs::write(&path, b) {
                Ok(_) => {
                    let n = path.display().to_string();
                    o.line(&format!("        saved a {} byte document: {n}", b.len()));
                }
                Err(e) => o.line(&format!("        could not save a document: {e}")),
            }
        }
    }
}

/// Carry bytes one way, reading them as they pass.
fn pump(mut from: TcpStream, mut to: TcpStream, mut tap: Tap) {
    let mut buf = [0u8; 16 * 1024];
    loop {
        match from.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                tap.feed(&buf[..n]);
                if to.write_all(&buf[..n]).is_err() {
                    break;
                }
            }
        }
    }
    tap.say(&format!("{} ended", tap.name));
    let _ = to.shutdown(std::net::Shutdown::Both);
}

/// Listen, and for each connection open one to the box and pass everything through, reading it.
pub fn run(listen: &str, target: &str, docs: Option<PathBuf>, log: Option<PathBuf>) -> Result<(), String> {
    let listener = TcpListener::bind(listen).map_err(|e| format!("can't listen on {listen}: {e}"))?;
    if let Some(d) = &docs {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let log = log.map(|p| std::fs::File::create(p).map_err(|e| e.to_string())).transpose()?;
    let out = Arc::new(Mutex::new(Out { log, docs, saved: 0 }));
    println!("listening on {listen}, passing everything to {target}. Point the other editor at this address.");
    let started = Instant::now();
    for incoming in listener.incoming() {
        let Ok(client) = incoming else { continue };
        let target = target.to_owned();
        let out = out.clone();
        std::thread::spawn(move || {
            let peer = client.peer_addr().map(|a| a.to_string()).unwrap_or_default();
            let Ok(upstream) = TcpStream::connect(&target) else {
                eprintln!("connection from {peer}: the box at {target} did not answer");
                return;
            };
            let _ = client.set_nodelay(true);
            let _ = upstream.set_nodelay(true);
            if let Ok(mut o) = out.lock() {
                o.line(&format!("-- connection from {peer}"));
            }
            let (t_up, t_down) =
                (Tap::new("computer -> box", true, started, out.clone()), Tap::new("box -> computer", false, started, out.clone()));
            let (c2, u2) = (client.try_clone(), upstream.try_clone());
            let (Ok(c2), Ok(u2)) = (c2, u2) else { return };
            let up = std::thread::spawn(move || pump(client, upstream, t_up));
            pump(u2, c2, t_down);
            let _ = up.join();
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tap(from_computer: bool) -> (Tap, Arc<Mutex<Out>>) {
        let out = Arc::new(Mutex::new(Out { log: None, docs: None, saved: 0 }));
        (
            Tap {
                name: "t",
                from_computer,
                started: Instant::now(),
                out: out.clone(),
                buf: Vec::new(),
                state: State::Http,
                frag: Vec::new(),
                frag_text: false,
                leftover: Vec::new(),
                readers: HashMap::new(),
            },
            out,
        )
    }

    /// A masked client-to-server binary WebSocket frame.
    fn masked(payload: &[u8]) -> Vec<u8> {
        let key = [1u8, 2, 3, 4];
        let mut f = vec![0x82];
        if payload.len() < 126 {
            f.push(0x80 | payload.len() as u8);
        } else {
            f.push(0x80 | 126);
            f.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        }
        f.extend_from_slice(&key);
        f.extend(payload.iter().enumerate().map(|(i, b)| b ^ key[i % 4]));
        f
    }

    #[test]
    fn a_request_sent_through_the_relay_is_read_as_opcode_and_arguments() {
        use crate::proto::{request, MSG_DATA};
        // One request, as the session sends it: stream prefix, a data frame on the data channel.
        let body = request(1001, 21, Value::map(vec![(key::DOCUMENT, Value::Bin(vec![0; 5], 0xc4))])).encode();
        let mut stream = Vec::new();
        stream.extend_from_slice(&1u16.to_le_bytes());
        stream.extend_from_slice(&6u16.to_le_bytes());
        stream.extend_from_slice(&(body.len() as u32).to_le_bytes());
        stream.extend_from_slice(&body);
        let mut payload = Vec::new();
        ChannelHeader { seq: 3, msg_type: MSG_DATA, ack: 0 }.encode(&mut payload);
        payload.extend_from_slice(&stream);
        let transfer = Frame::new(0x1080, 0x03ed, payload).encode();

        let (mut t, out) = tap(true);
        t.feed(b"GET /raw HTTP/1.1\r\nHost: box\r\nUpgrade: websocket\r\nAuthorization: Bearer SECRET\r\n\r\n");
        t.feed(&masked(&transfer));
        // The frame arrives in two pieces, too.
        let again = masked(&transfer);
        t.feed(&again[..7]);
        t.feed(&again[7..]);
        // Nothing to read back from the log lines here: they were printed; check the state instead.
        assert!(t.state == State::WebSocket);
        assert!(t.leftover.is_empty() && t.buf.is_empty(), "everything was consumed");
        drop(out);
    }

    #[test]
    fn values_are_summarised_not_dumped() {
        let v = Value::map(vec![(110, Value::Bin(vec![0; 4000], 0xda)), (1, Value::Array((0..40).map(Value::int).collect(), 0x90))]);
        let s = brief(&v, 4);
        assert!(s.contains("bin(4000 bytes)") && s.contains("...+24"), "{s}");
    }
}
