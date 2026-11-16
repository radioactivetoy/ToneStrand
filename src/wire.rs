//! The two ways bytes reach the pedal: its USB bulk endpoints directly, or a
//! Helrig Box on the network that carries those same transfers over a WebSocket.

use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

pub trait Wire: Send {
    /// Send one bulk OUT transfer.
    fn send(&mut self, bytes: &[u8]) -> Result<(), String>;
    /// Receive the next bulk IN transfer, or Err("timeout") when none arrives in time.
    fn recv(&mut self, timeout: Duration) -> Result<Vec<u8>, String>;
    /// Called regularly while idle; the box wants a frame at least every 5 s.
    fn idle(&mut self) -> Result<(), String> {
        Ok(())
    }
    fn describe(&self) -> String;
    /// A link that needs gentler pacing of large transfers (the Helrig Box relays USB over Wi-Fi).
    fn slow(&self) -> bool {
        false
    }
}

pub const TIMEOUT: &str = "timeout";

// ---------------------------------------------------------------------- USB ---

/// Line 6's USB vendor id.
pub const LINE6_VID: u16 = 0x0e41;
const EP_OUT: u8 = 0x01;
const EP_IN: u8 = 0x81;

#[derive(Clone, Debug)]
pub struct UsbPedal {
    pub product: String,
    pub pid: u16,
    pub serial: String,
}

pub fn list_usb_pedals() -> Vec<UsbPedal> {
    let Ok(devices) = nusb::list_devices() else {
        return Vec::new();
    };
    devices
        .filter(|d| d.vendor_id() == LINE6_VID)
        .map(|d| UsbPedal {
            product: d
                .product_string()
                .map(|s| s.trim().to_owned())
                .unwrap_or_else(|| format!("Line 6 device {:04x}", d.product_id())),
            pid: d.product_id(),
            serial: d.serial_number().unwrap_or("").trim().to_owned(),
        })
        .collect()
}

pub struct UsbWire {
    name: String,
    _interface: nusb::Interface,
    out_q: nusb::transfer::Queue<Vec<u8>>,
    in_q: nusb::transfer::Queue<nusb::transfer::RequestBuffer>,
}

impl UsbWire {
    pub fn open(pid: u16) -> Result<UsbWire, String> {
        let info = nusb::list_devices()
            .map_err(|e| e.to_string())?
            .find(|d| d.vendor_id() == LINE6_VID && d.product_id() == pid)
            .ok_or("the pedal is no longer plugged in")?;
        let name = info.product_string().unwrap_or("Helix").trim().to_owned();
        let device = info
            .open()
            .map_err(|e| format!("could not open the pedal ({e}); is another editor running?"))?;
        // Claim, release and claim again: the release clears the pedal's
        // per-channel state left from an earlier connection.
        drop(device.claim_interface(0).map_err(|e| format!("could not claim the pedal: {e}"))?);
        let interface = device
            .claim_interface(0)
            .map_err(|e| format!("could not claim the pedal: {e}"))?;
        let _ = interface.clear_halt(EP_OUT);
        let _ = interface.clear_halt(EP_IN);
        let out_q = interface.bulk_out_queue(EP_OUT);
        let mut in_q = interface.bulk_in_queue(EP_IN);
        in_q.submit(nusb::transfer::RequestBuffer::new(512));
        Ok(UsbWire { name, _interface: interface, out_q, in_q })
    }
}

/// Poll a cancel-safe future once without blocking.
fn poll_now<F: std::future::Future>(f: F) -> Option<F::Output> {
    futures_lite::future::block_on(futures_lite::future::poll_once(f))
}

impl Wire for UsbWire {
    fn send(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.out_q.submit(bytes.to_vec());
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(c) = poll_now(self.out_q.next_complete()) {
                return c.status.map_err(|e| format!("USB write failed: {e}"));
            }
            if Instant::now() >= deadline {
                self.out_q.cancel_all();
                // Collect the cancelled transfer now, or the next send would take its
                // "cancelled" result for its own.
                let until = Instant::now() + Duration::from_millis(500);
                while self.out_q.pending() > 0 && Instant::now() < until {
                    if poll_now(self.out_q.next_complete()).is_none() {
                        std::thread::sleep(Duration::from_millis(1));
                    }
                }
                return Err("USB write timed out".into());
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn recv(&mut self, timeout: Duration) -> Result<Vec<u8>, String> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(c) = poll_now(self.in_q.next_complete()) {
                let data = c.data;
                // Always keep one read waiting, even after a failed one: an empty queue
                // would leave every later read with nothing to wait on.
                self.in_q.submit(nusb::transfer::RequestBuffer::new(512));
                if let Err(e) = c.status {
                    return Err(format!("USB read failed: {e}"));
                }
                return Ok(data);
            }
            if Instant::now() >= deadline {
                return Err(TIMEOUT.into());
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn describe(&self) -> String {
        format!("{} (USB)", self.name)
    }
}

// --------------------------------------------------------------- Helrig Box ---

pub struct BoxWire {
    name: String,
    ws: tungstenite::WebSocket<TcpStream>,
    last_sent: Instant,
    pub product: String,
}

impl BoxWire {
    /// Open `ws://<host>/raw` with the box's token and read its hello.
    pub fn open(host: &str, token: &str, label: &str) -> Result<BoxWire, String> {
        use tungstenite::client::IntoClientRequest;
        let url_host = crate::boxes::url_host(host);
        let addr: SocketAddr = format!("{url_host}:80")
            .to_socket_addrs()
            .map_err(|e| format!("could not find {host}: {e}"))?
            .next()
            .ok_or_else(|| format!("could not find {host}"))?;
        let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(4))
            .map_err(|e| format!("the box at {host} did not answer: {e}"))?;
        let _ = stream.set_nodelay(true);
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .map_err(|e| e.to_string())?;
        let mut req = format!("ws://{url_host}/raw")
            .into_client_request()
            .map_err(|e| e.to_string())?;
        req.headers_mut().insert(
            "Authorization",
            format!("Bearer {token}").parse().map_err(|_| "bad token")?,
        );
        let (mut ws, _) = tungstenite::client(req, stream)
            .map_err(|e| format!("the box refused the link: {e}"))?;

        // The hello comes first, before any transfer.
        let hello = loop {
            match ws.read() {
                Ok(tungstenite::Message::Text(t)) => break t.to_string(),
                Ok(tungstenite::Message::Close(_)) => return Err("the box closed the link".into()),
                Ok(_) => continue,
                Err(e) => return Err(format!("the box did not say hello: {e}")),
            }
        };
        let v: serde_json::Value = serde_json::from_str(&hello).map_err(|e| e.to_string())?;
        match v.get("t").and_then(|t| t.as_str()) {
            Some("hello") => {}
            Some("error") => {
                let code = v.get("code").and_then(|c| c.as_str()).unwrap_or("?");
                return Err(explain_box_error(code));
            }
            _ => return Err(format!("unexpected first message from the box: {hello}")),
        }
        let proto = v.get("proto").and_then(|p| p.as_i64()).unwrap_or(-1);
        if proto != 0 {
            return Err(format!("the box speaks relay protocol {proto}; this app knows 0"));
        }
        if v.get("pid").map_or(true, |p| p.is_null()) {
            return Err("no pedal is plugged into the box".into());
        }
        let product = v
            .get("product")
            .and_then(|p| p.as_str())
            .unwrap_or("Helix")
            .trim()
            .to_owned();
        Ok(BoxWire {
            name: format!("{product} via {label}"),
            ws,
            last_sent: Instant::now(),
            product,
        })
    }
}

pub fn explain_box_error(code: &str) -> String {
    match code {
        "relay_unavailable" => "relay mode is off on the box - turn it on in the box panel".into(),
        "busy" => "another computer is using the pedal through this box".into(),
        "box_session_open" => "the box is using the pedal itself; turn relay mode on and replug the pedal".into(),
        "unauthorized" => "the box did not accept this computer's pairing; pair again".into(),
        "origin_not_allowed" => "the box refused the connection's origin".into(),
        "oversize" => "a message was too big for the box".into(),
        "usb_error" => "the box lost USB contact with the pedal".into(),
        other => format!("the box reported: {other}"),
    }
}

impl Wire for BoxWire {
    fn send(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.last_sent = Instant::now();
        self.ws
            .send(tungstenite::Message::Binary(bytes.to_vec().into()))
            .map_err(|e| format!("lost the box: {e}"))
    }

    fn recv(&mut self, timeout: Duration) -> Result<Vec<u8>, String> {
        let t = timeout.max(Duration::from_millis(1));
        let _ = self.ws.get_ref().set_read_timeout(Some(t));
        loop {
            match self.ws.read() {
                Ok(tungstenite::Message::Binary(b)) => return Ok(b.to_vec()),
                Ok(tungstenite::Message::Text(text)) => {
                    let v: serde_json::Value =
                        serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
                    match v.get("t").and_then(|t| t.as_str()) {
                        Some("pong") => continue,
                        Some("pedal_detached") => return Err("the pedal was unplugged from the box".into()),
                        Some("closing") => return Err("the box closed the link (it heard nothing for too long)".into()),
                        Some("error") => {
                            let code = v.get("code").and_then(|c| c.as_str()).unwrap_or("?");
                            return Err(explain_box_error(code));
                        }
                        _ => continue,
                    }
                }
                Ok(tungstenite::Message::Close(_)) => return Err("the box closed the link".into()),
                Ok(_) => continue,
                Err(tungstenite::Error::Io(e))
                    if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) =>
                {
                    return Err(TIMEOUT.into())
                }
                Err(e) => return Err(format!("lost the box: {e}")),
            }
        }
    }

    fn idle(&mut self) -> Result<(), String> {
        // The box drops a client that sends nothing for 6 s; ping after 3 s of quiet.
        if self.last_sent.elapsed() > Duration::from_secs(3) {
            self.last_sent = Instant::now();
            self.ws
                .send(tungstenite::Message::Text(r#"{"t":"ping"}"#.into()))
                .map_err(|e| format!("lost the box: {e}"))?;
        }
        Ok(())
    }

    fn describe(&self) -> String {
        self.name.clone()
    }

    fn slow(&self) -> bool {
        true
    }
}

impl Drop for BoxWire {
    fn drop(&mut self) {
        let _ = self.ws.close(None);
        let _ = self.ws.flush();
    }
}
