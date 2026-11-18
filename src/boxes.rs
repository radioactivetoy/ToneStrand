//! ToneStrand Boxes: finding them on the network, pairing, and remembering tokens.

use std::collections::BTreeMap;
use std::sync::mpsc::Sender;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// mDNS service types a box announces.
/// (`_helrigbox` and `_tonerelay` are the Helrig Box and plain ToneRelay builds of the same firmware.)
pub const SERVICES: [&str; 3] = ["_tonestrandbox._tcp.local.", "_helrigbox._tcp.local.", "_tonerelay._tcp.local."];
/// How long the box keeps a pairing window open.
pub const PAIR_WINDOW_SECS: u64 = 30;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct BoxInfo {
    pub name: String,
    pub host: String,
    pub id: String,
}

#[derive(Clone, Debug, Default)]
pub struct RelayStatus {
    pub relay_only: bool,
    pub client: bool,
    pub pedal: Option<String>,
    pub id: String,
    /// The box is in direct mode (BOOT held 2 to 4 s): its own network is up and it is not on a router.
    pub direct: bool,
}

/// Fields missing from an older file take their value from `Default` (the struct-level
/// `#[serde(default)]`), so a new field only needs a value there.
#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Token per box, keyed by the box id (or its address when it has none).
    pub tokens: BTreeMap<String, String>,
    pub saved: Vec<BoxInfo>,
    pub library: Option<String>,
    /// Where HX Edit's resource files are, when not in the usual place.
    pub hx_edit: Option<String>,
    /// The unit picked when no pedal is connected (its device number).
    pub unit: Option<i64>,
    /// Ask before writing to or clearing a slot on the pedal.
    pub confirm_pedal: bool,
    /// Ask before deleting a preset file from the library.
    pub confirm_delete: bool,
    /// Open the sample preset when the app starts. Switched off after the first start.
    pub show_sample: bool,
    /// The sample preset has been opened once (so it is not opened again unless asked for).
    pub sample_seen: bool,
    /// The "Getting started" card was dismissed, or a first connection made.
    pub hide_getting_started: bool,
    /// Start on the library page instead of the editor.
    pub start_in_library: bool,
    /// Look for ToneStrand Boxes on the network automatically.
    pub discover_boxes: bool,
    /// Size of everything in the window (1.0 = normal).
    pub ui_scale: f32,
    /// Show the real gear each effect is based on instead of the pedal's names.
    pub real_names: bool,
    /// The pedal runs the community NAM A2 Lite firmware (HX Stomp, XL, HX Effects).
    pub nam_lite: bool,
    /// Converter options: keep the capture's input level and loudness.
    pub nam_use_level: bool,
    pub nam_use_loudness: bool,
    /// Where converted captures were saved last.
    pub nam_out_dir: Option<String>,
    /// The IR and NAM library folder (default: "IRs and NAM" in the preset library).
    pub tone_library: Option<String>,
    /// The accent colour (0xRRGGBB).
    pub accent: u32,
    /// "dark", "light" or "system" (follow the computer).
    pub theme: String,
    /// The window as it was left: position and size (points), when not maximized.
    pub window: Option<[f32; 4]>,
    /// The window was maximized when it was closed (the usual: also the start-up default).
    pub window_maximized: bool,
    /// The last preset files opened, newest first (at most 10).
    pub recent: Vec<String>,
    /// Open the files that were open when the app last closed.
    pub reopen_tabs: bool,
    /// The saved preset files that were open when the app last closed.
    pub last_tabs: Vec<String>,
    /// Connect again by itself when the USB pedal comes back after being unplugged.
    pub auto_reconnect: bool,
    /// Units whose "what is checked" notice has been shown (their device numbers).
    pub units_seen: Vec<i64>,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            tokens: BTreeMap::new(),
            saved: Vec::new(),
            library: None,
            hx_edit: None,
            unit: None,
            confirm_pedal: true,
            confirm_delete: true,
            show_sample: true,
            sample_seen: false,
            hide_getting_started: false,
            start_in_library: false,
            discover_boxes: true,
            ui_scale: 1.0,
            real_names: false,
            nam_lite: false,
            nam_use_level: true,
            nam_use_loudness: true,
            nam_out_dir: None,
            tone_library: None,
            accent: 0x2ec4b6,
            theme: "dark".into(),
            window: None,
            window_maximized: true,
            recent: Vec::new(),
            reopen_tabs: false,
            last_tabs: Vec::new(),
            auto_reconnect: true,
            units_seen: Vec::new(),
        }
    }
}

fn settings_path() -> Option<std::path::PathBuf> {
    Some(dirs::config_dir()?.join("ToneStrand").join("settings.json"))
}

/// The last failed settings save, for the window to show (see [`take_save_error`]).
static SAVE_ERROR: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// A settings save that failed since the last call, if any.
pub fn take_save_error() -> Option<String> {
    SAVE_ERROR.lock().ok().and_then(|mut e| e.take())
}

impl Settings {
    pub fn load() -> Settings {
        Self::load_reporting().0
    }

    /// The settings, plus a note for the person when the file couldn't be used. An
    /// unreadable file is moved aside (never silently overwritten by the next save).
    pub fn load_reporting() -> (Settings, Option<String>) {
        let Some(p) = settings_path() else { return (Settings::default(), None) };
        Self::load_from(&p)
    }

    fn load_from(p: &std::path::Path) -> (Settings, Option<String>) {
        let text = match std::fs::read_to_string(p) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Settings::default(), None),
            Err(e) => return (Settings::default(), Some(set_aside(p, &format!("couldn't be read ({e})")))),
        };
        match serde_json::from_str(&text) {
            Ok(s) => (s, None),
            Err(e) => (Settings::default(), Some(set_aside(p, &format!("couldn't be understood ({e})")))),
        }
    }

    pub fn save(&self) {
        let Some(p) = settings_path() else { return };
        let result = (|| {
            if let Some(dir) = p.parent() {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            let s = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
            crate::files::write_atomic(&p, s.as_bytes()).map_err(|e| e.to_string())
        })();
        if let Err(e) = result {
            if let Ok(mut slot) = SAVE_ERROR.lock() {
                *slot = Some(format!("Could not save settings to {} ({e})", p.display()));
            }
        }
    }
    pub fn token_for(&self, b: &BoxInfo) -> Option<String> {
        self.tokens.get(&box_key(b)).cloned()
    }
}

/// Move a settings file that can't be used out of the way, keeping it for the person.
fn set_aside(p: &std::path::Path, why: &str) -> String {
    let dir = p.parent().unwrap_or(std::path::Path::new("."));
    let kept = crate::files::unique_path(dir, "settings.json.bad");
    match std::fs::rename(p, &kept) {
        Ok(_) => format!("Your settings file {why}; started with default settings. The old file was kept as {}", kept.display()),
        Err(e) => format!("Your settings file {why} and couldn't be moved aside ({e}); started with default settings"),
    }
}

pub fn box_key(b: &BoxInfo) -> String {
    if b.id.is_empty() {
        b.host.clone()
    } else {
        b.id.clone()
    }
}

/// Browse the network for boxes; each one found is sent once per address.
pub fn start_discovery(tx: Sender<BoxInfo>) -> Option<mdns_sd::ServiceDaemon> {
    let daemon = mdns_sd::ServiceDaemon::new().ok()?;
    for service in SERVICES {
        let Ok(rx) = daemon.browse(service) else { continue };
        let tx = tx.clone();
        std::thread::spawn(move || {
            while let Ok(event) = rx.recv() {
                if let mdns_sd::ServiceEvent::ServiceResolved(info) = event {
                    let id = info.get_property_val_str("id").unwrap_or("").to_owned();
                    let instance = info.get_fullname().split("._").next().unwrap_or("ToneStrand Box").to_owned();
                    let addr = info
                        .get_addresses()
                        .iter()
                        .find(|a| a.is_ipv4())
                        .or_else(|| info.get_addresses().iter().next())
                        .map(|a| a.to_string());
                    if let Some(host) = addr {
                        if tx.send(BoxInfo { name: instance, host, id }).is_err() {
                            return;
                        }
                    }
                }
            }
        });
    }
    Some(daemon)
}

/// A box address as it goes in a URL or `host:port`: IPv6 addresses need brackets.
pub fn url_host(host: &str) -> String {
    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V6(v6)) => format!("[{v6}]"),
        _ => host.to_owned(),
    }
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(3)).timeout(Duration::from_secs(5)).build()
}

fn read_json(r: Result<ureq::Response, ureq::Error>) -> Result<(u16, serde_json::Value), String> {
    match r {
        Ok(resp) => {
            let code = resp.status();
            let v = resp.into_json().map_err(|e| e.to_string())?;
            Ok((code, v))
        }
        Err(ureq::Error::Status(code, resp)) => {
            let v = resp.into_json().unwrap_or(serde_json::Value::Null);
            Ok((code, v))
        }
        Err(e) => Err(format!("the box did not answer: {e}")),
    }
}

/// `GET /api/relay`: is relay mode on, is a pedal attached, is someone using it.
pub fn relay_status(host: &str) -> Result<RelayStatus, String> {
    let (_, v) = read_json(agent().get(&format!("http://{}/api/relay", url_host(host))).call())?;
    let pedal = v.get("pedal").and_then(|p| {
        if p.get("present").and_then(|x| x.as_bool()) == Some(true) {
            Some(p.get("product").and_then(|x| x.as_str()).unwrap_or("pedal").trim().to_owned())
        } else {
            None
        }
    });
    Ok(RelayStatus {
        relay_only: v.get("relay_only").and_then(|x| x.as_bool()).unwrap_or(false),
        client: v.get("client").and_then(|x| x.as_bool()).unwrap_or(false),
        pedal,
        id: v.get("id").and_then(|x| x.as_str()).unwrap_or("").to_owned(),
        direct: v.get("direct").and_then(|x| x.as_bool()).unwrap_or(false),
    })
}

/// `POST /api/relay {"on": ...}` with the token.
pub fn set_relay(host: &str, token: &str, on: bool) -> Result<(), String> {
    let r = agent()
        .post(&format!("http://{}/api/relay", url_host(host)))
        .set("Authorization", &format!("Bearer {token}"))
        .set("Content-Type", "application/json")
        .send_string(&format!("{{\"on\":{on}}}"));
    let (code, v) = read_json(r)?;
    match code {
        200 => Ok(()),
        401 => Err("the box did not accept this computer's pairing; pair again".into()),
        409 => Err("the box is using the pedal itself; unplug the pedal, turn relay on, then plug it back".into()),
        _ => Err(format!("the box said {code}: {}", v.get("error").and_then(|e| e.as_str()).unwrap_or("?"))),
    }
}

/// `POST /api/pair`: open the 30 s window. Returns the nonce.
pub fn pair_start(host: &str) -> Result<String, String> {
    let r = agent().post(&format!("http://{}/api/pair", url_host(host))).send_string("");
    let (code, v) = read_json(r)?;
    if code == 409 {
        return Err("someone else is pairing with this box right now; try again in 30 seconds".into());
    }
    v.get("nonce")
        .and_then(|n| n.as_str())
        .map(|s| s.to_owned())
        .ok_or_else(|| "the box did not open a pairing window (its firmware may be too old)".into())
}

pub enum PairPoll {
    /// Seconds left in the pairing window (not shown yet).
    Waiting(#[allow(dead_code)] u64),
    Token(String),
    Closed,
}

/// `GET /api/pair?nonce=...`: has the BOOT button been pressed yet?
pub fn pair_poll(host: &str, nonce: &str) -> Result<PairPoll, String> {
    let r = agent().get(&format!("http://{}/api/pair?nonce={nonce}", url_host(host))).call();
    let (code, v) = read_json(r)?;
    if code == 403 {
        return Err("the box rejected the pairing request".into());
    }
    if let Some(t) = v.get("token").and_then(|t| t.as_str()) {
        return Ok(PairPoll::Token(t.to_owned()));
    }
    if v.get("pairing").and_then(|p| p.as_bool()) == Some(true) {
        return Ok(PairPoll::Waiting(v.get("seconds_left").and_then(|s| s.as_u64()).unwrap_or(0)));
    }
    Ok(PairPoll::Closed)
}

/// Ask the computer's own name lookup for the box's `.local` names, in case
/// the network search misses it (some VPN adapters swallow those questions).
pub fn lookup_local_names(tx: Sender<BoxInfo>) {
    std::thread::spawn(move || {
        // A box that has just joined the network, or a name lookup that is slow the first time, can miss the
        // first try, so ask again every few seconds for about a minute, until one answers.
        for attempt in 0..15 {
            if attempt > 0 {
                std::thread::sleep(Duration::from_secs(4));
            }
            let mut found = false;
            for (name, product) in
                [("tonestrandbox.local", "ToneStrand Box"), ("helrigbox.local", "Helrig Box"), ("tonerelay.local", "ToneRelay box")]
            {
                use std::net::ToSocketAddrs;
                let Ok(addrs) = (name, 80).to_socket_addrs() else { continue };
                for a in addrs.filter(|a| a.is_ipv4()) {
                    let host = a.ip().to_string();
                    if let Ok(st) = relay_status(&host) {
                        let label = if st.id.is_empty() { product.to_owned() } else { format!("{product} {}", st.id) };
                        found = true;
                        if tx.send(BoxInfo { name: label, host, id: st.id }).is_err() {
                            return;
                        }
                    }
                }
            }
            if found {
                return;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_broken_settings_file_is_kept_not_lost() {
        let dir = std::env::temp_dir().join(format!("tonestrand-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("settings.json");

        // Missing file: defaults, nothing to say.
        let (s, note) = Settings::load_from(&p);
        assert!(note.is_none() && s.confirm_pedal);

        // Unreadable JSON: defaults, a note, and the old file moved aside intact.
        std::fs::write(&p, b"{\"tokens\": {\"box\": \"secret\"}, \"ui_scale\": \"big\"}").unwrap();
        let (s, note) = Settings::load_from(&p);
        assert!(s.tokens.is_empty());
        assert!(note.unwrap().contains("settings.json.bad"));
        assert!(!p.exists());
        assert!(std::fs::read_to_string(dir.join("settings.json.bad")).unwrap().contains("secret"));

        // A good file still loads, with defaults for fields it doesn't have.
        std::fs::write(&p, b"{\"ui_scale\": 1.25}").unwrap();
        let (s, note) = Settings::load_from(&p);
        assert!(note.is_none());
        assert_eq!(s.ui_scale, 1.25);
        assert!(s.confirm_pedal);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod url_tests {
    #[test]
    fn ipv6_hosts_get_brackets() {
        use super::url_host;
        assert_eq!(url_host("192.168.1.20"), "192.168.1.20");
        assert_eq!(url_host("helrigbox.local"), "helrigbox.local");
        assert_eq!(url_host("fe80::1"), "[fe80::1]");
        let a: std::net::SocketAddr = format!("{}:80", url_host("fe80::1")).parse().unwrap();
        assert_eq!(a.port(), 80);
    }
}

/// The window as it was left last time (position and size, maximized), read without any side
/// effect: a damaged settings file is dealt with when the app loads its settings.
pub fn saved_window() -> Option<([f32; 4], bool)> {
    parse_window(&std::fs::read_to_string(settings_path()?).ok()?)
}

fn parse_window(text: &str) -> Option<([f32; 4], bool)> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    let a = v.get("window")?.as_array()?;
    let n: Vec<f32> = a.iter().filter_map(|x| x.as_f64()).map(|x| x as f32).collect();
    let rect: [f32; 4] = n.try_into().ok()?;
    let maximized = v.get("window_maximized").and_then(|m| m.as_bool()).unwrap_or(true);
    Some((rect, maximized))
}

#[cfg(test)]
mod window_tests {
    use super::parse_window;

    #[test]
    fn the_saved_window_is_read_when_it_is_whole() {
        let (r, max) = parse_window(r#"{"window": [10, 20, 1300.5, 800], "window_maximized": false, "ui_scale": 1.0}"#).unwrap();
        assert_eq!(r, [10.0, 20.0, 1300.5, 800.0]);
        assert!(!max);
        // Maximized is the default when the file doesn't say.
        assert!(parse_window(r#"{"window": [0, 0, 900, 600]}"#).unwrap().1);
        // Anything incomplete or damaged is simply "no saved window".
        assert!(parse_window(r#"{"window": [1, 2, 3]}"#).is_none());
        assert!(parse_window(r#"{"window": null}"#).is_none());
        assert!(parse_window("{ not json").is_none());
        assert!(parse_window("{}").is_none());
    }
}
