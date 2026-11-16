//! The window: connections on the left, the pedal's presets and the local
//! library below them, and open presets as tabs in the middle.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, CornerRadius, RichText, Stroke, Vec2};

use crate::boxes::{self, BoxInfo, PairPoll, RelayStatus, Settings};
use crate::device::{self, Cmd, Event, Link, Purpose, Target};
use crate::catalog::Catalog;
use crate::devpreset::DevicePreset;
use crate::editors;
use crate::library::{self, Library};
use std::sync::Arc;
use crate::hlx;
use crate::proto::slot_label;
use crate::theme::{self, *};
use crate::wire::{self, UsbPedal};

// ----------------------------------------------------------------- documents ---

#[derive(Clone)]
enum DocKind {
    /// A JSON preset file on this computer.
    Hlx { json: serde_json::Value },
    /// The pedal's own document: from a slot, the loaded preset, or a file.
    Device { preset: DevicePreset, origin: Origin },
}

#[derive(Clone)]
enum Origin {
    Live { setlist: i64, index: i64 },
    Slot { setlist: i64, index: i64 },
    File,
}

struct Doc {
    title: String,
    /// The preset's name on the pedal, for device documents.
    name: String,
    path: Option<PathBuf>,
    kind: DocKind,
    dirty: bool,
    selected: Option<usize>,
    example: bool,
    /// Earlier states, for undo (newest last), and undone ones, for redo.
    undo: Vec<DocKind>,
    redo: Vec<DocKind>,
    /// When the last edit happened, to group a drag into one step.
    last_edit: Option<Instant>,
}

impl Doc {
    fn is_live(&self) -> bool {
        matches!(self.kind, DocKind::Device { origin: Origin::Live { .. }, .. })
    }

    /// Closing it would lose edits. A live preset's edits are already on the pedal (its
    /// edit buffer), so closing its tab loses nothing.
    fn needs_save(&self) -> bool {
        self.dirty && !self.is_live()
    }
}

/// What is waiting for "save your changes?" to be answered.
enum CloseAsk {
    Tab(usize),
    Quit,
}

// --------------------------------------------------------------------- state ---

#[derive(Clone, PartialEq)]
enum Pick {
    Usb(usize),
    Box(usize),
}

struct Pairing {
    host: String,
    key: String,
    rx: Receiver<Result<String, String>>,
    started: Instant,
}

#[derive(PartialEq, Clone, Copy)]
enum View {
    Editor,
    Library,
    Irs,
    Settings,
    /// Hidden: Ctrl+Shift+T.
    Tests,
}

#[derive(PartialEq, Clone, Copy)]
enum LogLevel {
    Info,
    Good,
    Bad,
}

pub struct App {
    settings: Settings,
    usb: Vec<UsbPedal>,
    usb_scanned: Option<Instant>,
    boxes: Vec<BoxInfo>,
    box_rx: Receiver<BoxInfo>,
    _mdns: Option<mdns_sd::ServiceDaemon>,
    pick: Option<Pick>,
    /// The person picked a pedal or box themselves (otherwise one is picked for them).
    pick_manual: bool,
    status_rx: Option<(String, Receiver<Result<RelayStatus, String>>)>,
    status: Option<(String, Result<RelayStatus, String>)>,
    pairing: Option<Pairing>,
    add_box_text: String,

    link: Option<Link>,
    connected: Option<String>,
    connecting: bool,
    busy: Option<String>,

    setlists: Vec<String>,
    setlist: i64,
    presets: Vec<String>,
    current: Option<(i64, i64, String)>,
    chosen: Option<usize>,
    filter: String,
    rename: Option<(i64, usize, String)>,
    confirm: Option<(String, Cmd)>,

    library_dir: PathBuf,
    library: Vec<PathBuf>,

    docs: Vec<Doc>,
    active: usize,

    log: VecDeque<(LogLevel, String)>,
    screenshot: Option<(PathBuf, u32, bool)>,

    catalog: Option<Arc<Catalog>>,
    picker: Option<editors::PickerState>,
    view: View,
    lib: Library,
    pending_delete: Option<PathBuf>,
    close_ask: Option<CloseAsk>,
    /// The person chose to quit even with unsaved changes.
    quit_ok: bool,
    last_error: Option<String>,
    /// The unit presets are made and converted for: detected from the pedal,
    /// or picked by the person when none is connected.
    unit: i64,
    /// A conversion report to show: title, lines, and whether it is a refusal.
    unit_report: Option<(String, Vec<String>, bool)>,
    /// The pedal's User IRs in use (slot from 0, name), once read.
    irs: Option<Vec<(i64, String)>>,
    ir_sel: std::collections::BTreeSet<i64>,
    /// Which pedal slots hold NAM captures, by (slot, name).
    ir_kinds: std::collections::HashMap<(i64, String), bool>,
    /// The pedal IR view: 0 all slots, 1 IRs, 2 NAM captures, 3 empty.
    ir_view: usize,
    ir_rename: Option<(i64, String)>,
    /// Files about to be sent to IR slots, waiting for the person to pick slots.
    ir_plan: Option<IrPlan>,
    tests: TestsState,
    /// The IR and NAM library on this computer.
    tones: crate::tonelib::ToneLib,
    /// The IRs page shows this computer's library instead of the pedal.
    irs_local: bool,
    /// The IR list was asked for from the library view.
    tones_list_asked: bool,
}

/// The hidden Tests page (Ctrl+Shift+T).
#[derive(Default)]
struct TestsState {
    preset_slot: Option<i64>,
    ir_slot: Option<i64>,
    preset_writes: bool,
    live_edit: bool,
    ir_writes: bool,
    understood: bool,
    running: bool,
    log: Vec<(String, Option<bool>, String)>,
    done: Option<usize>,
    folder: Option<PathBuf>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, screenshot: Option<PathBuf>) -> App {
        let (settings, settings_note) = Settings::load_reporting();
        theme::set_accent(settings.accent);
        theme::apply(&cc.egui_ctx);
        let (tx, box_rx) = channel();
        let tx_local = tx.clone();
        let mdns = if settings.discover_boxes { boxes::start_discovery(tx) } else { None };
        cc.egui_ctx.set_zoom_factor(settings.ui_scale.clamp(0.75, 1.5));
        crate::realnames::set(settings.real_names || std::env::var("TONESTRAND_REAL_NAMES").is_ok_and(|v| v == "1"));
        crate::chain::set_studio(settings.studio);
        let library_dir = std::env::var_os("TONESTRAND_LIBRARY")
            .map(PathBuf::from)
            .or_else(|| settings.library.clone().map(PathBuf::from))
            .unwrap_or_else(Self::default_library_dir);
        let boxes = settings.saved.clone();
        let mut app = App {
            settings,
            usb: Vec::new(),
            usb_scanned: None,
            boxes,
            box_rx,
            _mdns: mdns,
            pick: None,
            pick_manual: false,
            status_rx: None,
            status: None,
            pairing: None,
            add_box_text: String::new(),
            link: None,
            connected: None,
            connecting: false,
            busy: None,
            setlists: Vec::new(),
            setlist: 0,
            presets: Vec::new(),
            current: None,
            chosen: None,
            filter: String::new(),
            rename: None,
            confirm: None,
            library_dir,
            library: Vec::new(),
            docs: Vec::new(),
            active: 0,
            log: VecDeque::new(),
            screenshot: screenshot.map(|p| (p, 0, false)),
            catalog: None,
            picker: None,
            view: View::Editor,
            lib: Library::new(PathBuf::new()),
            pending_delete: None,
            close_ask: None,
            quit_ok: false,
            last_error: None,
            unit: 2162694,
            unit_report: None,
            irs: None,
            ir_sel: Default::default(),
            ir_kinds: Default::default(),
            ir_view: 0,
            ir_rename: None,
            ir_plan: None,
            tests: TestsState::default(),
            tones: crate::tonelib::ToneLib::default(),
            irs_local: false,
            tones_list_asked: false,
        };
        app.catalog = crate::catalog::find_dir(app.settings.hx_edit.as_deref())
            .and_then(|d| Catalog::load(&d).ok())
            .map(Arc::new);
        app.lib = Library::new(app.library_dir.clone());
        app.unit = app.settings.unit.unwrap_or(2162694);
        // Start for a given unit (used to check pictures for each pedal).
        if let Some(u) = std::env::var("TONESTRAND_UNIT").ok().and_then(|v| v.parse::<i64>().ok()) {
            app.unit = u;
        }
        crate::nam::set_enabled(app.settings.nam_lite || std::env::var("TONESTRAND_NAM").is_ok_and(|v| v == "1"));
        crate::nam::set_unit(app.unit);
        // The box by its .local name, in case the network search misses it.
        boxes::lookup_local_names(tx_local);

        if app.settings.show_sample {
            app.docs.push(Doc {
                title: "Sample Tone".into(),
                name: String::new(),
                path: None,
                kind: DocKind::Hlx { json: hlx::sample() },
                dirty: false,
                selected: Some(2),
                example: true,
                undo: Vec::new(),
                redo: Vec::new(),
                last_edit: None,
            });
            // The sample follows the unit picked last time.
            let _ = app.convert_doc(0);
            app.docs[0].dirty = false;
        }
        if app.settings.start_in_library {
            app.view = View::Library;
        }
        // Start on a given page (used to check each page's picture).
        match std::env::var("TONESTRAND_PAGE").as_deref() {
            Ok("settings") => app.view = View::Settings,
            Ok("irs") => app.view = View::Irs,
            Ok("tones") => {
                app.view = View::Irs;
                app.irs_local = true;
                app.scan_tones();
            }
            Ok("tests") => app.view = View::Tests,
            Ok("open") => {
                // For checks: open a preset file at start (TONESTRAND_OPEN).
                if let Some(p) = std::env::var_os("TONESTRAND_OPEN") {
                    app.open_file(&PathBuf::from(p));
                }
            }
            Ok("library") => app.view = View::Library,
            _ => {}
        }
        app.scan_library();
        app.note(LogLevel::Info, "Ready. Plug in a pedal or pick a Helrig Box to start.");
        if let Some(n) = settings_note {
            app.note(LogLevel::Bad, n);
        }
        app
    }

    fn note(&mut self, level: LogLevel, text: impl Into<String>) {
        self.log.push_back((level, text.into()));
        while self.log.len() > 200 {
            self.log.pop_front();
        }
    }

    fn send(&mut self, cmd: Cmd) {
        if let Some(l) = &self.link {
            let _ = l.tx.send(cmd);
        } else {
            self.note(LogLevel::Bad, "Not connected to a pedal.");
        }
    }

    /// Where the IR and NAM library lives.
    fn tone_dir(&self) -> PathBuf {
        // For checks: look at another folder without changing the person's setting.
        if let Some(d) = std::env::var_os("TONESTRAND_TONES") {
            return PathBuf::from(d);
        }
        self.settings.tone_library.clone().map(PathBuf::from).unwrap_or_else(|| self.library_dir.join("IRs and NAM"))
    }

    fn nam_opts(&self) -> crate::nam::Options {
        crate::nam::Options { use_level: self.settings.nam_use_level, use_loudness: self.settings.nam_use_loudness }
    }

    fn scan_tones(&mut self) {
        let dir = self.tone_dir();
        if self.tones.root != dir {
            self.tones = crate::tonelib::ToneLib::new(dir);
        }
        let opts = self.nam_opts();
        self.tones.scan(opts);
    }

    /// Where a slot or IR is kept before the pedal writes over it (sent with each write).
    fn backup_dir(&self) -> PathBuf {
        self.library_dir.join("Pedal backups")
    }

    fn scan_library(&mut self) {
        let cat = self.catalog.clone();
        self.lib.skip = vec![self.tone_dir(), self.library_dir.join("Self-test backups")];
        self.lib.scan(cat.as_deref());
        let mut files = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&self.library_dir) {
            for e in rd.flatten() {
                let p = e.path();
                let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
                if ext == "hlx" || ext == "hxpreset" {
                    files.push(p);
                }
            }
        }
        files.sort();
        self.library = files;
    }

    // ------------------------------------------------------------ background ---

    fn poll(&mut self, ctx: &egui::Context) {
        if let Some(e) = boxes::take_save_error() {
            self.note(LogLevel::Bad, e);
        }
        if self.usb_scanned.is_none_or(|t| t.elapsed() > Duration::from_secs(3)) {
            self.usb = wire::list_usb_pedals();
            self.usb_scanned = Some(Instant::now());
        }
        while let Ok(b) = self.box_rx.try_recv() {
            if let Some(existing) =
                self.boxes.iter_mut().find(|x| (!b.id.is_empty() && x.id == b.id) || x.host == b.host)
            {
                existing.host = b.host.clone();
                if existing.id.is_empty() {
                    existing.id = b.id.clone();
                }
            } else {
                self.note(LogLevel::Info, format!("Found {} at {}", b.name, b.host));
                self.boxes.push(b);
            }
        }
        if let Some((host, rx)) = &self.status_rx {
            if let Ok(r) = rx.try_recv() {
                let host = host.clone();
                // Learn the box id from its status, so its token is kept by id.
                if let Ok(st) = &r {
                    if let Some(b) = self.boxes.iter_mut().find(|b| b.host == host) {
                        if b.id.is_empty() && !st.id.is_empty() {
                            b.id = st.id.clone();
                        }
                    }
                }
                self.status = Some((host, r));
                self.status_rx = None;
            }
        }
        if let Some(p) = &self.pairing {
            if let Ok(r) = p.rx.try_recv() {
                let key = p.key.clone();
                let host = p.host.clone();
                self.pairing = None;
                match r {
                    Ok(token) => {
                        self.settings.tokens.insert(key, token);
                        if let Some(b) = self.boxes.iter().find(|b| b.host == host).cloned() {
                            self.settings.saved.retain(|s| s.host != b.host);
                            self.settings.saved.push(b);
                        }
                        self.settings.save();
                        self.note(LogLevel::Good, "Paired with the box. You can connect now.");
                    }
                    Err(e) => self.note(LogLevel::Bad, format!("Pairing failed: {e}")),
                }
            }
        }

        let mut events = Vec::new();
        if let Some(l) = &self.link {
            while let Ok(e) = l.rx.try_recv() {
                events.push(e);
            }
        }
        for e in events {
            self.on_event(e);
        }
        if self.link.is_some() || self.pairing.is_some() || self.status_rx.is_some() {
            ctx.request_repaint_after(Duration::from_millis(200));
        } else {
            ctx.request_repaint_after(Duration::from_secs(2));
        }
    }

    fn on_event(&mut self, e: Event) {
        match e {
            Event::Connected(name) => {
                self.last_error = None;
                self.view = View::Editor;
                self.connecting = false;
                self.busy = None;
                self.note(LogLevel::Good, format!("Connected to {name}"));
                // The connected pedal decides the unit.
                if let Some(u) = crate::snapshots::unit_for_product(&name) {
                    if u != self.unit {
                        self.note(LogLevel::Info, format!("Detected {}", crate::units::unit_name(u)));
                        self.set_unit(u);
                    }
                }
                self.connected = Some(name);
            }
            Event::Failed(why) => {
                self.connecting = false;
                self.busy = None;
                self.link = None;
                self.last_error = Some(why.clone());
                self.note(LogLevel::Bad, format!("Could not connect: {why}"));
            }
            Event::Disconnected(why) => {
                self.busy = None;
                self.link = None;
                self.connected = None;
                self.presets.clear();
                self.setlists.clear();
                self.irs = None;
                self.ir_sel.clear();
                self.ir_kinds.clear();
                self.current = None;
                self.chosen = None;
                for d in &mut self.docs {
                    if let DocKind::Device { origin, .. } = &mut d.kind {
                        if matches!(origin, Origin::Live { .. }) {
                            *origin = Origin::File;
                            d.title = format!("{} (offline)", d.title);
                        }
                    }
                }
                self.note(LogLevel::Info, why);
            }
            Event::Busy(m) => self.busy = Some(m),
            Event::Info(m) => self.note(LogLevel::Info, m),
            Event::Error(m) => {
                self.busy = None;
                self.note(LogLevel::Bad, m);
            }
            Event::Done(m) => {
                self.busy = None;
                self.note(LogLevel::Good, m);
            }
            Event::TestStep { name, ok, detail } => self.tests.log.push((name, ok, detail)),
            Event::TestDone { failures } => {
                self.tests.running = false;
                self.tests.done = Some(failures);
            }
            Event::Irs(list) => {
                crate::nam::set_ir_names(&list);
                // Slots not looked at yet (or holding something new) get sorted into IRs and NAM captures.
                let unknown: Vec<(i64, String)> = list.iter().filter(|(s, n)| !self.ir_kinds.contains_key(&(*s, n.clone()))).cloned().collect();
                self.ir_kinds.retain(|k, _| list.contains(k));
                crate::nam::set_slot_kinds(self.ir_kinds.iter().map(|((s, _), n)| (*s, *n)).collect());
                self.irs = Some(list);
                self.ir_sel.retain(|_| true);
                if !unknown.is_empty() {
                    if let Some(l) = &self.link {
                        let _ = l.tx.send(Cmd::IrKinds(unknown));
                    }
                }
            }
            Event::IrKinds(kinds) => {
                self.busy = None;
                for (s, n, nam) in kinds {
                    self.ir_kinds.insert((s, n), nam);
                }
                crate::nam::set_slot_kinds(self.ir_kinds.iter().map(|((s, _), n)| (*s, *n)).collect());
            }
            Event::Setlists(s) => self.setlists = s,
            Event::Presets(sl, p) => {
                self.setlist = sl;
                self.presets = p;
            }
            Event::Current(sl, idx, name) => self.current = Some((sl, idx, name)),
            Event::Document { setlist, index, live, purpose, blob } => {
                self.busy = None;
                let slot = index.max(0) as usize;
                let name = self.presets.get(slot).cloned().unwrap_or_default();
                let Some(blob) = blob else {
                    self.note(LogLevel::Info, format!("{} is empty", slot_label(slot)));
                    return;
                };
                if purpose == Purpose::SaveToFile {
                    let _ = std::fs::create_dir_all(&self.library_dir);
                    let default = format!("{} {}.hxpreset", slot_label(slot), device::sanitize(&name));
                    if let Some(p) = rfd::FileDialog::new()
                        .set_directory(&self.library_dir)
                        .set_file_name(&default)
                        .add_filter("Pedal preset", &["hxpreset"])
                        .save_file()
                    {
                        match crate::files::write_atomic(&p, &blob) {
                            Ok(_) => self.note(LogLevel::Good, format!("Saved {}", p.display())),
                            Err(e) => self.note(LogLevel::Bad, format!("Could not save: {e}")),
                        }
                        self.scan_library();
                    }
                    return;
                }
                match DevicePreset::parse(&blob) {
                    Ok(preset) => {
                        let origin =
                            if live { Origin::Live { setlist, index } } else { Origin::Slot { setlist, index } };
                        let label = slot_label(slot);
                        let title = if live { format!("Live · {label} {name}") } else { format!("{label} {name}") };
                        // There is one live tab: it follows whatever the pedal is playing.
                        let existing = if live {
                            self.docs.iter().position(|d| d.is_live())
                        } else {
                            self.docs.iter().position(|d| d.title == title)
                        };
                        if let Some(i) = existing {
                            let keep = self.docs[i].selected.filter(|_| self.docs[i].name == name);
                            self.docs[i].kind = DocKind::Device { preset, origin };
                            self.docs[i].dirty = false;
                            self.docs[i].title = title;
                            self.docs[i].name = name;
                            self.docs[i].selected = keep;
                            if live && !self.docs[self.active.min(self.docs.len() - 1)].is_live() && self.view == View::Editor {
                                // Do not pull the person away from another tab on a refresh.
                            } else {
                                self.active = i;
                            }
                        } else {
                            self.docs.push(Doc {
                                title,
                                name,
                                path: None,
                                kind: DocKind::Device { preset, origin },
                                dirty: false,
                                selected: None,
                                example: false,
                                undo: Vec::new(),
                                redo: Vec::new(),
                                last_edit: None,
                            });
                            self.active = self.docs.len() - 1;
                        }
                    }
                    Err(e) => self.note(LogLevel::Bad, e),
                }
            }
        }
    }

    // --------------------------------------------------------------- actions ---

    fn connect(&mut self, ctx: &egui::Context) {
        let Some(pick) = self.pick.clone() else { return };
        let target = match pick {
            Pick::Usb(i) => match self.usb.get(i) {
                Some(p) => Target::Usb { pid: p.pid },
                None => return,
            },
            Pick::Box(i) => {
                let Some(b) = self.boxes.get(i).cloned() else { return };
                let Some(token) = self.settings.token_for(&b) else {
                    self.note(LogLevel::Bad, "Pair with this box first.");
                    return;
                };
                Target::Box { host: b.host.clone(), token, label: b.name.clone() }
            }
        };
        let c = ctx.clone();
        self.link = Some(device::connect(target, move || c.request_repaint()));
        self.connecting = true;
    }

    fn disconnect(&mut self) {
        if let Some(l) = &self.link {
            let _ = l.tx.send(Cmd::Disconnect);
        }
    }

    fn check_box(&mut self, host: String) {
        let (tx, rx) = channel();
        let h = host.clone();
        std::thread::spawn(move || {
            let _ = tx.send(boxes::relay_status(&h));
        });
        self.status_rx = Some((host, rx));
    }

    fn start_pairing(&mut self, b: &BoxInfo) {
        let (tx, rx) = channel();
        let host = b.host.clone();
        std::thread::spawn(move || {
            let r = (|| {
                let nonce = boxes::pair_start(&host)?;
                let until = Instant::now() + Duration::from_secs(boxes::PAIR_WINDOW_SECS + 2);
                while Instant::now() < until {
                    std::thread::sleep(Duration::from_millis(800));
                    match boxes::pair_poll(&host, &nonce)? {
                        PairPoll::Token(t) => return Ok(t),
                        PairPoll::Waiting(_) => {}
                        PairPoll::Closed => {
                            return Err("the pairing window closed before the button was pressed".to_string())
                        }
                    }
                }
                Err("the button on the box was not pressed in time".to_string())
            })();
            let _ = tx.send(r);
        });
        self.pairing = Some(Pairing { host: b.host.clone(), key: boxes::box_key(b), rx, started: Instant::now() });
    }

    fn open_file(&mut self, path: &Path) {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        if let Some(i) = self.docs.iter().position(|d| d.path.as_deref() == Some(path)) {
            self.active = i;
            return;
        }
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Preset").to_owned();
        let kind = if ext == "hxpreset" {
            match std::fs::read(path).map_err(|e| e.to_string()).and_then(|b| DevicePreset::parse(&b)) {
                Ok(preset) => DocKind::Device { preset, origin: Origin::File },
                Err(e) => return self.note(LogLevel::Bad, format!("Could not open {}: {e}", path.display())),
            }
        } else {
            match std::fs::read_to_string(path)
                .map_err(|e| e.to_string())
                .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).map_err(|e| e.to_string()))
            {
                Ok(json) if json.pointer("/data/tone").is_some() => DocKind::Hlx { json },
                Ok(_) => return self.note(LogLevel::Bad, "That file is not a Helix preset (.hlx)."),
                Err(e) => return self.note(LogLevel::Bad, format!("Could not open {}: {e}", path.display())),
            }
        };
        let (title, name) = match &kind {
            DocKind::Hlx { json } => (hlx::name(json), String::new()),
            _ => {
                // Files saved from the pedal start with the slot label; drop it for the name.
                let name = match stem.split_once(' ') {
                    Some((a, b)) if a.len() == 3 && a.ends_with(['A', 'B', 'C']) => b.to_owned(),
                    _ => stem.clone(),
                };
                (stem.clone(), name)
            }
        };
        self.docs.push(Doc { title, name, path: Some(path.to_owned()), kind, dirty: false, selected: None, example: false, undo: Vec::new(), redo: Vec::new(), last_edit: None });
        self.active = self.docs.len() - 1;
        // A preset made for another unit is converted to this one, or explained.
        let i = self.active;
        let unit = crate::units::unit_name(self.unit);
        match self.convert_doc(i) {
            Some(Err(why)) => {
                let mut lines = vec![format!("It was made for another unit and cannot run on the {unit} as it is:")];
                lines.extend(why.into_iter().map(|w| format!("   • {w}")));
                lines.push("It is open unchanged; pick its own unit at the top to edit it as it is.".into());
                self.unit_report = Some((format!("\"{}\" does not fit the {unit}", self.docs[i].title), lines, true));
            }
            Some(Ok(lines)) if !lines.is_empty() => {
                self.unit_report = Some((format!("\"{}\" converted to the {unit}", self.docs[i].title), lines, false));
            }
            _ => {}
        }
    }

    fn save_doc(&mut self, i: usize, save_as: bool) {
        let lib = self.library_dir.clone();
        let Some(doc) = self.docs.get_mut(i) else { return };
        let (ext, bytes) = match &doc.kind {
            DocKind::Hlx { json } => ("hlx", serde_json::to_vec_pretty(json).unwrap_or_default()),
            DocKind::Device { preset, .. } => ("hxpreset", preset.encode()),
        };
        let path = if save_as || doc.path.is_none() {
            let _ = std::fs::create_dir_all(&lib);
            let base = if doc.name.is_empty() { doc.title.clone() } else { doc.name.clone() };
            rfd::FileDialog::new()
                .set_directory(&lib)
                .set_file_name(format!("{}.{ext}", device::sanitize(&base)))
                .add_filter("Preset", &[ext])
                .save_file()
        } else {
            doc.path.clone()
        };
        let Some(path) = path else { return };
        match crate::files::write_atomic(&path, &bytes) {
            Ok(_) => {
                doc.path = Some(path.clone());
                doc.dirty = false;
                doc.example = false;
                self.note(LogLevel::Good, format!("Saved {}", path.display()));
                self.scan_library();
            }
            Err(e) => self.note(LogLevel::Bad, format!("Could not save: {e}")),
        }
    }

    // -------------------------------------------------------------------- UI ---

    fn top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top")
            .frame(egui::Frame::new().fill(bg()).inner_margin(egui::Margin::symmetric(14, 10)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    // The ToneStrand mark, then the name.
                    if let Some(t) = theme::logo(ui.ctx(), theme::Logo::MarkSquare) {
                        ui.add(egui::Image::new(&t).fit_to_exact_size(Vec2::new(50.0, 27.0)));
                    }
                    ui.label(RichText::new("ToneStrand").size(19.0).strong());
                    ui.label(RichText::new("preset editor").color(MUTED));
                    ui.add_space(14.0);
                    let views = [View::Editor, View::Library, View::Irs, View::Settings];
                    let cur = views.iter().position(|v| *v == self.view).unwrap_or(usize::MAX);
                    if let Some(i) = theme::segmented(ui, &["Editor", "Library", "IRs", "⚙ Settings"], cur) {
                        self.view = views[i];
                        if self.view == View::Library {
                            self.scan_library();
                        }
                        if self.view == View::Irs && self.irs.is_none() && self.link.is_some() && self.connected.is_some() {
                            self.send(Cmd::ListIrs);
                        }
                    }

                    ui.add_space(16.0);
                    match (&self.connected, self.connecting) {
                        (Some(n), _) => theme::pill(ui, &format!("{n}"), GOOD),
                        (None, true) => theme::pill(ui, "Connecting...", ORANGE),
                        _ => theme::pill(ui, "Not connected", MUTED),
                    }
                    // The unit: detected from the connected pedal, or picked when offline.
                    ui.add_space(8.0);
                    ui.label(RichText::new("Unit").color(MUTED));
                    let mut pick = None;
                    let locked = self.connected.is_some();
                    ui.add_enabled_ui(!locked, |ui| {
                        egui::ComboBox::from_id_salt("app-unit")
                            .selected_text(RichText::new(crate::units::unit_name(self.unit)).strong())
                            .show_ui(ui, |ui| {
                                for (d, n) in crate::snapshots::UNITS {
                                    if ui.selectable_label(d == self.unit, n).clicked() && d != self.unit {
                                        pick = Some(d);
                                    }
                                }
                            })
                            .response
                            .on_hover_text("New presets are made for this unit and opened presets are converted to it")
                            .on_disabled_hover_text("Detected from the connected pedal");
                    });
                    if let Some(d) = pick {
                        self.set_unit(d);
                    }
                    if let Some((_, idx, name)) = &self.current {
                        theme::pill(ui, &format!("{}  {}", slot_label((*idx).max(0) as usize), name), accent());
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Open file...").clicked() {
                            if let Some(p) = rfd::FileDialog::new()
                                .set_directory(&self.library_dir)
                                .add_filter("Presets", &["hlx", "hxpreset"])
                                .pick_file()
                            {
                                self.open_file(&p);
                            }
                        }
                        // A new preset is made for the current unit.
                        let mut make: Option<i64> = None;
                        if ui
                            .button("New preset")
                            .on_hover_text(format!("An empty preset for the {}", crate::units::unit_name(self.unit)))
                            .clicked()
                        {
                            make = Some(self.unit);
                        }
                        if let Some(device) = make {
                            let json = hlx::blank_for("New Preset", device);
                            self.docs.push(Doc {
                                title: "New Preset".into(),
                                name: String::new(),
                                path: None,
                                kind: DocKind::Hlx { json },
                                dirty: true,
                                selected: None,
                                example: false,
                                undo: Vec::new(),
                                redo: Vec::new(),
                                last_edit: None,
                            });
                            self.active = self.docs.len() - 1;
                        }
                    });
                });
            });
    }

    /// Have something picked so Connect is one click away: a pedal on USB first, else the
    /// first paired Helrig Box, else the first box found. A pick the person made stays.
    fn auto_pick(&mut self) {
        if self.link.is_some() {
            return;
        }
        let valid = match &self.pick {
            Some(Pick::Usb(i)) => *i < self.usb.len(),
            Some(Pick::Box(i)) => *i < self.boxes.len(),
            None => false,
        };
        if self.pick_manual && valid {
            return;
        }
        let want = if !self.usb.is_empty() {
            Some(Pick::Usb(0))
        } else if let Some(i) = self.boxes.iter().position(|b| self.settings.token_for(b).is_some()) {
            Some(Pick::Box(i))
        } else if !self.boxes.is_empty() {
            Some(Pick::Box(0))
        } else {
            None
        };
        if want != self.pick {
            if let Some(Pick::Box(i)) = &want {
                if let Some(b) = self.boxes.get(*i) {
                    self.check_box(b.host.clone());
                }
            }
            self.pick = want;
            self.pick_manual = false;
        }
    }

    /// A new, empty preset for the current unit, in a new tab.
    fn new_preset(&mut self) {
        let json = hlx::blank_for("New Preset", self.unit);
        self.docs.push(Doc {
            title: "New Preset".into(),
            name: String::new(),
            path: None,
            kind: DocKind::Hlx { json },
            dirty: true,
            selected: None,
            example: false,
            undo: Vec::new(),
            redo: Vec::new(),
            last_edit: None,
        });
        self.active = self.docs.len() - 1;
    }

    fn open_file_dialog(&mut self) {
        if let Some(p) = rfd::FileDialog::new().set_directory(&self.library_dir).add_filter("Presets", &["hlx", "hxpreset"]).pick_file() {
            self.open_file(&p);
        }
    }

    /// The Studio look's top bar: the app and its pages, the open preset's name, undo and
    /// save, its snapshots, BPM and DSP; under it a slim row with the library and pedal
    /// actions, the accent colour, the unit and connection, New preset and Open file.
    fn studio_top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top-studio")
            .frame(egui::Frame::new().fill(panel()).inner_margin(egui::Margin::symmetric(12, 8)).stroke(Stroke::new(1.0_f32, line())))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if let Some(t) = theme::logo(ui.ctx(), theme::Logo::MarkSquare) {
                        ui.add(egui::Image::new(&t).fit_to_exact_size(Vec2::new(38.0, 21.0)));
                    }
                    ui.label(RichText::new("ToneStrand").size(17.0).strong());
                    ui.add_space(8.0);
                    let views = [View::Editor, View::Library, View::Irs, View::Settings];
                    let cur = views.iter().position(|v| *v == self.view).unwrap_or(usize::MAX);
                    if let Some(v) = theme::segmented(ui, &["Editor", "Library", "IRs", "⚙ Settings"], cur) {
                        self.view = views[v];
                        if self.view == View::Library {
                            self.scan_library();
                        }
                        if self.view == View::Irs && self.irs.is_none() && self.link.is_some() && self.connected.is_some() {
                            self.send(Cmd::ListIrs);
                        }
                    }
                    // The pedal: one click opens the connection options (USB, Helrig Boxes, pairing).
                    ui.add_space(8.0);
                    let (dot, text) = match (&self.connected, self.connecting) {
                        (Some(n), _) => (GOOD, n.clone()),
                        (None, true) => (ORANGE, "Connecting...".to_owned()),
                        _ => (MUTED, format!("{} · not connected", crate::units::unit_name(self.unit))),
                    };
                    // A status light (painted: the font has no round bullet), then the menu.
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(12.0), egui::Sense::hover());
                    ui.painter().circle_filled(rect.center(), 4.5, dot);
                    let label = RichText::new(text.clone()).color(if self.connected.is_some() { TEXT } else { MUTED });
                    let r = ui.menu_button(label, |ui| {
                        ui.set_min_width(340.0);
                        self.devices_card(ui, ctx);
                    });
                    r.response.on_hover_text("Connect to the pedal (USB or Helrig Box)");
                    if let Some((_, idx, name)) = &self.current {
                        ui.add_space(8.0);
                        theme::pill(ui, &format!("playing {}  {}", slot_label((*idx).max(0) as usize), name), accent());
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Open file...").clicked() {
                            self.open_file_dialog();
                        }
                        if ui.button("New preset").on_hover_text(format!("An empty preset for the {}", crate::units::unit_name(self.unit))).clicked() {
                            self.new_preset();
                        }
                        let mut pick = None;
                        let locked = self.connected.is_some();
                        ui.add_enabled_ui(!locked, |ui| {
                            egui::ComboBox::from_id_salt("app-unit")
                                .selected_text(RichText::new(crate::units::unit_name(self.unit)).strong())
                                .show_ui(ui, |ui| {
                                    for (d, n) in crate::snapshots::UNITS {
                                        if ui.selectable_label(d == self.unit, n).clicked() && d != self.unit {
                                            pick = Some(d);
                                        }
                                    }
                                })
                                .response
                                .on_hover_text("New presets are made for this unit and opened presets are converted to it")
                                .on_disabled_hover_text("Detected from the connected pedal");
                        });
                        if let Some(d) = pick {
                            self.set_unit(d);
                        }
                        ui.label(RichText::new("Unit").color(MUTED));
                        ui.add_space(10.0);
                        // The accent colour.
                        for (rgb, name) in theme::ACCENTS.iter().rev() {
                            let col = Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, *rgb as u8);
                            let (rect, r) = ui.allocate_exact_size(Vec2::splat(16.0), egui::Sense::click());
                            ui.painter().circle_filled(rect.center(), 6.5, col);
                            if theme::accent_rgb() == *rgb {
                                ui.painter().circle_stroke(rect.center(), 8.0, Stroke::new(1.5_f32, TEXT));
                            }
                            if r.on_hover_text(*name).clicked() {
                                theme::set_accent(*rgb);
                                theme::apply(ui.ctx());
                                self.settings.accent = *rgb;
                                self.settings.save();
                            }
                        }
                        ui.label(RichText::new("Accent").color(MUTED));
                    });
                });
            });
    }

    /// The open preset's own bar (Studio look), under its tab: name, undo, save, library,
    /// pedal actions, snapshots, BPM and DSP.
    fn studio_preset_bar(&mut self, ui: &mut egui::Ui, i: usize) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            self.doc_bar(ui, i, 1);
            ui.add(egui::Separator::default().vertical().spacing(10.0));
            self.doc_bar(ui, i, 2);
            ui.add(egui::Separator::default().vertical().spacing(10.0));
            self.studio_strip(ui, i);
            ui.add(egui::Separator::default().vertical().spacing(10.0));
            self.studio_meters(ui, i);
        });
    }

    /// The Studio top bar's snapshot chips for the open preset.
    fn studio_strip(&mut self, ui: &mut egui::Ui, i: usize) {
        let connected = self.connected.is_some();
        let Some(doc) = self.docs.get_mut(i) else { return };
        let before = doc.kind.clone();
        let mut cmds = Vec::new();
        match &mut doc.kind {
            DocKind::Hlx { json } => editors::studio_strip_hlx(ui, json, &mut doc.dirty),
            DocKind::Device { preset, origin } => {
                let live = matches!(origin, Origin::Live { .. }) && connected;
                editors::studio_strip_device(ui, preset, &mut doc.dirty, live, &mut cmds);
            }
        }
        for c in cmds {
            self.send(c);
        }
        self.track_history(i, before, None);
    }

    /// BPM and the DSP meter of the open preset (right side of the Studio top bar).
    fn studio_meters(&mut self, ui: &mut egui::Ui, i: usize) {
        let cat = self.catalog.clone();
        let Some(doc) = self.docs.get_mut(i) else { return };
        let before = doc.kind.clone();
        // Each path: blocks used and its DSP, stacked one above the other when there are two.
        let mut paths = match &doc.kind {
            DocKind::Hlx { json } => editors::hlx_path_meters(cat.as_deref(), json),
            DocKind::Device { preset, .. } => editors::device_path_meters(cat.as_deref(), preset),
        };
        // A second pedal path with nothing in it isn't shown.
        if paths.len() > 1 && matches!(doc.kind, DocKind::Device { .. }) {
            paths.retain(|p| p.0 > 0 || p.2.is_some_and(|d| d > 0.5));
            if paths.is_empty() {
                paths.push((0, 8, None));
            }
        }
        let bpm = match &doc.kind {
            DocKind::Hlx { json } => hlx::tempo(json),
            DocKind::Device { preset, .. } => preset.tempo(),
        };
        if let Some(mut t) = bpm {
            ui.label(RichText::new("BPM").color(MUTED));
            if ui.add(egui::DragValue::new(&mut t).range(30.0..=240.0).speed(0.5).fixed_decimals(1)).changed() {
                match &mut doc.kind {
                    DocKind::Hlx { json } => hlx::set_tempo(json, t),
                    DocKind::Device { preset, .. } => preset.set_tempo(t),
                }
                doc.dirty = true;
            }
            ui.add_space(10.0);
        }
        let n = paths.len();
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            for (pi, (count, max, used)) in paths.iter().enumerate() {
                ui.horizontal(|ui| {
                    let label = if n > 1 { format!("PATH {}", pi + 1) } else { "PATH 1".into() };
                    ui.label(RichText::new(label).small().strong().color(MUTED));
                    let bcol = if count >= max { ORANGE } else { MUTED };
                    ui.label(RichText::new(format!("{count}/{max} blocks")).small().monospace().color(bcol));
                    if let Some(d) = used {
                        let col = if *d > 100.0 { BAD } else if *d > 85.0 { ORANGE } else { accent() };
                        ui.label(RichText::new("DSP").small().color(MUTED)).on_hover_text("Estimated DSP use of this path");
                        let (rect, _) = ui.allocate_exact_size(Vec2::new(80.0, if n > 1 { 5.0 } else { 6.0 }), egui::Sense::hover());
                        ui.painter().rect_filled(rect, CornerRadius::same(3), card_hi());
                        let fill = egui::Rect::from_min_size(rect.min, Vec2::new(rect.width() * (*d / 100.0).clamp(0.0, 1.0) as f32, rect.height()));
                        ui.painter().rect_filled(fill, CornerRadius::same(3), col);
                        ui.label(RichText::new(format!("{d:.0}%")).small().monospace().strong().color(col));
                    }
                });
            }
        });
        self.track_history(i, before, None);
    }

    fn status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status")
            .frame(egui::Frame::new().fill(bg()).inner_margin(egui::Margin::symmetric(14, 6)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if let Some(b) = &self.busy {
                        ui.add(egui::Spinner::new().size(14.0).color(accent()));
                        ui.label(RichText::new(b).color(accent()));
                        ui.separator();
                    }
                    if let Some((lvl, msg)) = self.log.back() {
                        ui.label(RichText::new(msg).color(level_colour(*lvl, MUTED)));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.menu_button("Activity", |ui| {
                            ui.set_min_width(440.0);
                            egui::ScrollArea::vertical().max_height(320.0).stick_to_bottom(true).show(ui, |ui| {
                                for (lvl, msg) in &self.log {
                                    ui.label(RichText::new(msg).color(level_colour(*lvl, TEXT)));
                                }
                            });
                        });
                    });
                });
            });
    }

    fn left_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("left")
            .resizable(true)
            .default_width(330.0)
            .min_width(280.0)
            .frame(if crate::chain::studio() {
                egui::Frame::new().fill(panel()).inner_margin(egui::Margin::symmetric(10, 10)).stroke(Stroke::new(1.0_f32, line()))
            } else {
                egui::Frame::new().fill(panel()).inner_margin(egui::Margin::same(12))
            })
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    // In the Studio look connecting lives in the top bar; this side is for presets.
                    if !crate::chain::studio() {
                        self.devices_card(ui, ctx);
                    }
                    ui.add_space(8.0);
                    self.pedal_card(ui);
                });
            });
    }

    fn devices_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        side_section(ui, "Connect", "CONNECT", |ui| {

            ui.add_space(2.0);
            let locked = self.link.is_some();
            let studio = crate::chain::studio();
            // Studio look, connected: one line, the rest is in the DEVICE box above.
            if studio && locked {
                ui.horizontal(|ui| {
                    if ui.button("Disconnect").clicked() {
                        self.disconnect();
                    }
                    if self.connected.is_some() && ui.button("Refresh").on_hover_text("Read the preset list again").clicked() {
                        self.send(Cmd::Refresh);
                    }
                });
                return;
            }
            if !(studio && self.usb.is_empty()) {
                ui.label(RichText::new("USB").small().color(MUTED));
            }
            if self.usb.is_empty() && !studio {
                ui.label(RichText::new("No pedal plugged in").color(MUTED).italics());
            }
            for (i, p) in self.usb.clone().iter().enumerate() {
                let sel = self.pick == Some(Pick::Usb(i));
                if ui.add_enabled(!locked, egui::SelectableLabel::new(sel, format!("🎸  {}", p.product))).clicked() {
                    self.pick = Some(Pick::Usb(i));
                    self.pick_manual = true;
                }
            }
            ui.add_space(4.0);
            ui.label(RichText::new("Helrig Box (Wi-Fi)").small().color(MUTED));
            if self.boxes.is_empty() {
                ui.label(RichText::new("Looking for boxes on your network...").color(MUTED).italics());
            }
            for (i, b) in self.boxes.clone().iter().enumerate() {
                let sel = self.pick == Some(Pick::Box(i));
                let paired = self.settings.token_for(b).is_some();
                ui.horizontal(|ui| {
                    let text = format!("📡  {}", if b.name.is_empty() { &b.host } else { &b.name });
                    if ui.add_enabled(!locked, egui::SelectableLabel::new(sel, text)).clicked() {
                        self.pick = Some(Pick::Box(i));
                        self.pick_manual = true;
                        self.check_box(b.host.clone());
                    }
                    if paired {
                        theme::pill(ui, "paired", GOOD);
                    }
                });
            }
            // Adding a box by its address: tucked behind a small link in the Studio look.
            let addr_id = egui::Id::new("show-box-address");
            let show_addr = !studio || ui.data(|d| d.get_temp::<bool>(addr_id)).unwrap_or(false) || self.boxes.is_empty();
            if !show_addr && ui.small_button("+ Add a box by address").clicked() {
                ui.data_mut(|d| d.insert_temp(addr_id, true));
            }
            if show_addr {
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.add_box_text)
                        .hint_text("box address, e.g. 192.168.1.40")
                        .desired_width(170.0),
                );
                if ui.button("Add").clicked() && !self.add_box_text.trim().is_empty() {
                    let host = self.add_box_text.trim().to_owned();
                    if !self.boxes.iter().any(|b| b.host == host) {
                        self.boxes.push(BoxInfo { name: format!("Box at {host}"), host: host.clone(), id: String::new() });
                    }
                    if let Some(i) = self.boxes.iter().position(|b| b.host == host) {
                        self.pick = Some(Pick::Box(i));
                    }
                    self.check_box(host);
                    self.add_box_text.clear();
                }
            });
            }

            if let Some(Pick::Box(i)) = self.pick.clone() {
                if let Some(b) = self.boxes.get(i).cloned() {
                    ui.add_space(4.0);
                    if studio {
                        // The box's details (pedal attached, relay mode, pairing) folded away.
                        let paired = self.settings.token_for(&b).is_some();
                        egui::CollapsingHeader::new(RichText::new("Box details").small())
                            .id_salt(("box-details", b.host.clone()))
                            .default_open(!paired)
                            .show(ui, |ui| self.box_details(ui, &b, locked));
                    } else {
                        self.box_details(ui, &b, locked);
                    }
                }
            }

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if self.link.is_some() {
                    if ui.button("Disconnect").clicked() {
                        self.disconnect();
                    }
                    if self.connected.is_some() && ui.button("Refresh").clicked() {
                        self.send(Cmd::Refresh);
                    }
                } else {
                    let can = match &self.pick {
                        Some(Pick::Usb(_)) => true,
                        Some(Pick::Box(i)) => self.boxes.get(*i).is_some_and(|b| self.settings.token_for(b).is_some()),
                        None => false,
                    };
                    let btn = egui::Button::new(RichText::new("Connect").strong().color(Color32::from_rgb(8, 26, 24)))
                        .fill(accent());
                    if ui
                        .add_enabled(can, btn)
                        .on_disabled_hover_text("Pick a pedal, or a paired box, first")
                        .clicked()
                    {
                        self.connect(ctx);
                    }
                }
            });
            if let Some(Pick::Box(i)) = &self.pick {
                if self.link.is_none() && self.boxes.get(*i).is_some_and(|b| self.settings.token_for(b).is_none()) {
                    ui.label(RichText::new("This computer is not paired with the box yet: press Pair above, then the BOOT button on the box.").small().color(ORANGE));
                }
            }
            if let Some(e) = &self.last_error {
                ui.label(RichText::new(format!("Last try: {e}")).small().color(BAD));
            }
        });
    }


    fn box_details(&mut self, ui: &mut egui::Ui, b: &BoxInfo, locked: bool) {
        egui::Frame::new()
            .fill(card_hi())
            .corner_radius(CornerRadius::same(8))
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(format!("{}  ·  {}", b.name, b.host)).strong());
                let status = self.status.clone();
                match &status {
                    Some((h, Ok(st))) if *h == b.host => {
                        ui.label(match &st.pedal {
                            Some(p) => RichText::new(format!("Pedal attached: {p}")).color(GOOD),
                            None => RichText::new("No pedal attached to the box").color(ORANGE),
                        });
                        ui.label(
                            RichText::new(if st.relay_only { "Relay mode: on" } else { "Relay mode: off" })
                                .color(if st.relay_only { GOOD } else { ORANGE }),
                        );
                        if st.client && !locked {
                            ui.label(RichText::new("Another computer is using it right now").color(ORANGE));
                        }
                        if !st.relay_only {
                            if let Some(tok) = self.settings.token_for(b) {
                                if ui.button("Turn relay mode on").clicked() {
                                    match boxes::set_relay(&b.host, &tok, true) {
                                        Ok(_) => self.note(LogLevel::Good, "Relay mode is on."),
                                        Err(e) => self.note(LogLevel::Bad, e),
                                    }
                                    self.check_box(b.host.clone());
                                }
                            }
                        }
                    }
                    Some((h, Err(e))) if *h == b.host => {
                        ui.label(RichText::new(e).color(BAD));
                    }
                    _ => {
                        ui.horizontal(|ui| {
                            ui.add(egui::Spinner::new().size(12.0));
                            ui.label(RichText::new("Asking the box...").color(MUTED));
                        });
                    }
                }
                ui.horizontal_wrapped(|ui| {
                    if ui.small_button("Check again").clicked() {
                        self.check_box(b.host.clone());
                    }
                    if let Some(p) = &self.pairing {
                        let left = boxes::PAIR_WINDOW_SECS.saturating_sub(p.started.elapsed().as_secs());
                        ui.add(egui::Spinner::new().size(12.0).color(ORANGE));
                        ui.label(RichText::new(format!("Press BOOT on the box now ({left}s)")).color(ORANGE).strong());
                    } else if ui
                        .small_button(if self.settings.token_for(b).is_some() { "Pair again" } else { "Pair" })
                        .on_hover_text("Then press the BOOT button on the box once")
                        .clicked()
                    {
                        self.start_pairing(b);
                    }
                    if self.settings.saved.iter().any(|s| s.host == b.host) && ui.small_button("Forget").clicked() {
                        self.settings.saved.retain(|s| s.host != b.host);
                        self.settings.tokens.remove(&boxes::box_key(b));
                        self.settings.save();
                    }
                });
            });
    }

    fn pedal_card(&mut self, ui: &mut egui::Ui) {
        side_section(ui, "Presets on the pedal", "PRESETS", |ui| {

            if self.connected.is_none() {
                ui.label(RichText::new("Connect to a pedal to see and edit its presets.").color(MUTED));
                return;
            }
            if !self.setlists.is_empty() {
                let current = self.setlists.get(self.setlist as usize).cloned().unwrap_or_default();
                egui::ComboBox::from_id_salt("setlist")
                    .selected_text(current)
                    .width(ui.available_width() - 8.0)
                    .show_ui(ui, |ui| {
                        for (i, s) in self.setlists.clone().iter().enumerate() {
                            if ui.selectable_label(i as i64 == self.setlist, s).clicked() {
                                self.setlist = i as i64;
                                self.chosen = None;
                                self.send(Cmd::Presets(i as i64));
                            }
                        }
                    });
            }
            ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text("🔍 Filter presets").desired_width(f32::INFINITY));
            ui.horizontal_wrapped(|ui| {
                if ui
                    .button("Edit loaded preset (live)")
                    .on_hover_text("Changes are heard at once; Save on pedal keeps them")
                    .clicked()
                {
                    self.send(Cmd::ReadCurrent);
                }
                if ui.button("Back up all").on_hover_text("Save every preset of this setlist as files").clicked() {
                    if let Some(folder) = rfd::FileDialog::new().set_directory(&self.library_dir).pick_folder() {
                        let names = self.presets.clone();
                        self.send(Cmd::Backup { setlist: self.setlist, count: self.presets.len(), folder, names });
                    }
                }
            });
            ui.add_space(4.0);
            let current_idx = self.current.as_ref().filter(|c| c.0 == self.setlist).map(|c| c.1);
            let filter = self.filter.to_lowercase();
            let row_h = 24.0;
            let presets = self.presets.clone();
            let sl = self.setlist;
            egui::ScrollArea::vertical().id_salt("presets").max_height(if crate::chain::studio() { (ui.available_height() - 10.0).max(240.0) } else { 360.0 }).auto_shrink([false, true]).show(ui, |ui| {
                for (i, name) in presets.iter().enumerate() {
                    if !filter.is_empty() && !name.to_lowercase().contains(&filter) {
                        continue;
                    }
                    let is_current = current_idx == Some(i as i64);
                    let is_chosen = self.chosen == Some(i);
                    let (rect, resp) =
                        ui.allocate_exact_size(Vec2::new(ui.available_width(), row_h), egui::Sense::click());
                    let fill = if is_chosen {
                        accent().linear_multiply(0.22)
                    } else if resp.hovered() {
                        card_hi()
                    } else {
                        Color32::TRANSPARENT
                    };
                    let p = ui.painter();
                    p.rect_filled(rect, CornerRadius::same(5), fill);
                    if is_current {
                        p.rect_filled(egui::Rect::from_min_size(rect.min, Vec2::new(3.0, row_h)), CornerRadius::same(2), accent());
                    }
                    p.text(
                        rect.left_center() + Vec2::new(10.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        slot_label(i),
                        egui::FontId::monospace(12.5),
                        if is_current { accent() } else { MUTED },
                    );
                    let shown = if name.trim().is_empty() { "<empty>" } else { name.as_str() };
                    p.text(
                        rect.left_center() + Vec2::new(52.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        shown,
                        egui::FontId::proportional(14.0),
                        TEXT,
                    );
                    if resp.clicked() {
                        self.chosen = Some(i);
                    }
                    if resp.double_clicked() {
                        self.send(Cmd::Select(sl, i as i64));
                    }
                    resp.context_menu(|ui| {
                        if ui.button("Load on the pedal").clicked() {
                            self.send(Cmd::Select(sl, i as i64));
                            ui.close_menu();
                        }
                        if ui.button("Open in editor").clicked() {
                            self.send(Cmd::FetchSlot { setlist: sl, index: i as i64, purpose: Purpose::Edit });
                            ui.close_menu();
                        }
                        if ui.button("Save to a file...").clicked() {
                            self.send(Cmd::FetchSlot { setlist: sl, index: i as i64, purpose: Purpose::SaveToFile });
                            ui.close_menu();
                        }
                        if ui.button("Rename...").clicked() {
                            self.rename = Some((sl, i, name.clone()));
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.button(RichText::new("Clear slot").color(BAD)).clicked() {
                            self.confirm = Some((
                                format!("Clear {} \"{}\"? This empties the slot on the pedal.", slot_label(i), name),
                                Cmd::Clear { setlist: sl, index: i as i64 },
                            ));
                            ui.close_menu();
                        }
                    });
                }
            });
            if let Some(i) = self.chosen {
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(slot_label(i)).monospace().color(accent()));
                    if theme::primary_button(ui, "Load").clicked() {
                        self.send(Cmd::Select(sl, i as i64));
                    }
                    if ui.button("Edit").clicked() {
                        self.send(Cmd::FetchSlot { setlist: sl, index: i as i64, purpose: Purpose::Edit });
                    }
                    if ui.button("Rename").clicked() {
                        self.rename = Some((sl, i, self.presets.get(i).cloned().unwrap_or_default()));
                    }
                    if ui.button("To file").clicked() {
                        self.send(Cmd::FetchSlot { setlist: sl, index: i as i64, purpose: Purpose::SaveToFile });
                    }
                });
            }
        });
    }

    fn tabs(&mut self, ui: &mut egui::Ui) {
        let mut close = None;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for (i, d) in self.docs.iter().enumerate() {
                let active = i == self.active;
                egui::Frame::new()
                    .fill(if active { card_bg() } else { panel() })
                    .stroke(Stroke::new(1.0_f32, if active { accent().linear_multiply(0.6) } else { line() }))
                    .corner_radius(CornerRadius { nw: 8, ne: 8, sw: 0, se: 0 })
                    .inner_margin(egui::Margin::symmetric(10, 5))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let star = if d.dirty { " •" } else { "" };
                            let text = RichText::new(format!("{}{star}", d.title)).color(if active { TEXT } else { MUTED });
                            if ui.add(egui::Label::new(text).sense(egui::Sense::click())).clicked() {
                                self.active = i;
                            }
                            if ui
                                .add(egui::Label::new(RichText::new("x").small().color(MUTED)).sense(egui::Sense::click()))
                                .clicked()
                            {
                                close = Some(i);
                            }
                        });
                    });
            }
        });
        if let Some(i) = close {
            if self.docs.get(i).is_some_and(Doc::needs_save) {
                self.close_ask = Some(CloseAsk::Tab(i));
            } else {
                self.close_tab(i);
            }
        }
    }

    fn close_tab(&mut self, i: usize) {
        if i >= self.docs.len() {
            return;
        }
        self.docs.remove(i);
        // Keep the same tab shown when one to its left closes.
        if i < self.active {
            self.active -= 1;
        }
        if self.active >= self.docs.len() {
            self.active = self.docs.len().saturating_sub(1);
        }
    }

    /// The window's close button: ask first when presets have unsaved changes.
    fn guard_quit(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) && !self.quit_ok && self.docs.iter().any(Doc::needs_save) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.close_ask = Some(CloseAsk::Quit);
        }
    }

    /// "Save your changes?" for closing a tab or quitting.
    fn close_dialog(&mut self, ctx: &egui::Context) {
        let Some(ask) = self.close_ask.take() else { return };
        let unsaved: Vec<usize> = (0..self.docs.len()).filter(|&i| self.docs[i].needs_save()).collect();
        let (text, title) = match &ask {
            CloseAsk::Tab(i) => match self.docs.get(*i) {
                Some(d) if d.needs_save() => (format!("\"{}\" has changes that aren't saved.", d.title), "Close preset"),
                // Saved or gone since: nothing to ask.
                _ => return self.close_tab(*i),
            },
            CloseAsk::Quit => {
                if unsaved.is_empty() {
                    return;
                }
                let names: Vec<String> = unsaved.iter().map(|&i| format!("\"{}\"", self.docs[i].title)).collect();
                let text = if names.len() == 1 {
                    format!("{} has changes that aren't saved.", names[0])
                } else {
                    format!("These presets have changes that aren't saved: {}.", names.join(", "))
                };
                (text, "Quit ToneStrand")
            }
        };
        let mut choice = None;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(&text);
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let save = if matches!(ask, CloseAsk::Quit) && unsaved.len() > 1 { "Save all" } else { "Save" };
                    if theme::primary_button(ui, save).clicked() {
                        choice = Some(0);
                    }
                    let discard = if matches!(ask, CloseAsk::Quit) { "Quit without saving" } else { "Close without saving" };
                    if ui.button(RichText::new(discard).color(BAD)).clicked() {
                        choice = Some(1);
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some(2);
                    }
                });
            });
        match (choice, ask) {
            (None, ask) => self.close_ask = Some(ask),
            (Some(2), _) => {}
            (Some(0), CloseAsk::Tab(i)) => {
                self.save_doc(i, false);
                // Still unsaved means the save was cancelled or failed: keep the tab.
                if !self.docs.get(i).is_some_and(Doc::needs_save) {
                    self.close_tab(i);
                }
            }
            (Some(_), CloseAsk::Tab(i)) => self.close_tab(i),
            (Some(0), CloseAsk::Quit) => {
                for &i in &unsaved {
                    self.save_doc(i, false);
                }
                if !self.docs.iter().any(Doc::needs_save) {
                    self.quit_ok = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            (Some(_), CloseAsk::Quit) => {
                self.quit_ok = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    fn default_library_dir() -> PathBuf {
        let docs = dirs::document_dir().unwrap_or_else(|| PathBuf::from("."));
        docs.join("ToneStrand Presets")
    }

    /// Point the library at another folder (None = the usual one in Documents).
    fn set_library_dir(&mut self, dir: Option<PathBuf>) {
        let d = dir.clone().unwrap_or_else(Self::default_library_dir);
        let _ = std::fs::create_dir_all(&d);
        self.settings.library = dir.map(|d| d.to_string_lossy().into_owned());
        self.settings.save();
        self.library_dir = d.clone();
        self.lib = Library::new(d.clone());
        self.scan_library();
        self.note(LogLevel::Good, format!("Library folder: {}", d.display()));
    }

    /// Load the effects list from HX Edit's resources (custom folder or the usual place).
    fn reload_catalog(&mut self) {
        self.catalog = crate::catalog::find_dir(self.settings.hx_edit.as_deref())
            .and_then(|d| Catalog::load(&d).ok())
            .map(Arc::new);
        self.scan_library();
        if self.catalog.is_some() {
            self.note(LogLevel::Good, "Effects list loaded.");
        } else {
            self.note(LogLevel::Bad, "No effects list found in that folder.");
        }
    }

    /// Pick .nam captures and a folder, write a User IR .wav for each, and show what happened.
    fn convert_nam_files(&mut self) {
        let Some(files) = rfd::FileDialog::new().set_title("Pick NAM captures").add_filter("NAM captures", &["nam"]).pick_files() else { return };
        let start = self.settings.nam_out_dir.clone().map(PathBuf::from).unwrap_or_else(|| self.library_dir.clone());
        let Some(out) = rfd::FileDialog::new().set_title("Where to save the .wav files").set_directory(&start).pick_folder() else { return };
        self.settings.nam_out_dir = Some(out.to_string_lossy().into_owned());
        self.settings.save();
        let opts = crate::nam::Options { use_level: self.settings.nam_use_level, use_loudness: self.settings.nam_use_loudness };
        let mut lines = Vec::new();
        let mut failed = 0;
        for f in &files {
            match crate::nam::convert_file(f, &out, opts) {
                Ok(line) => lines.push(format!("✔ {line}")),
                Err(e) => {
                    failed += 1;
                    lines.push(format!("✖ {}: {e}", f.file_name().and_then(|s| s.to_str()).unwrap_or("?")));
                }
            }
        }
        let ok = files.len() - failed;
        lines.push(String::new());
        lines.push(format!("Saved in {}. Import each .wav into a User IR slot with HX Edit, then set the NAM block's Model knob to that slot.", out.display()));
        self.note(if failed == 0 { LogLevel::Good } else { LogLevel::Bad }, format!("Converted {ok} of {} captures", files.len()));
        self.unit_report = Some((format!("NAM captures: {ok} converted, {failed} refused"), lines, failed > 0 && ok == 0));
    }

    /// Pick .wav IRs or .nam captures and ask before writing them into slots: the
    /// selected slots when there are as many, otherwise empty slots from the first selected one.
    fn upload_irs(&mut self) {
        let Some(files) = rfd::FileDialog::new()
            .set_title("Pick IRs or NAM captures to send to the pedal")
            .set_directory(self.tone_dir())
            .add_filter("IRs and NAM captures", &["wav", "nam"])
            .pick_files()
        else {
            return;
        };
        self.upload_ir_files(files);
    }

    /// Ask before writing these .wav / .nam files into the pedal's IR slots.
    fn upload_ir_files(&mut self, files: Vec<PathBuf>) {
        let opts = self.nam_opts();
        let mut loaded = Vec::new();
        let mut problems = Vec::new();
        for f in &files {
            let name = f.file_name().and_then(|s| s.to_str()).unwrap_or("?").to_owned();
            match crate::irs::load_file(f, opts) {
                Ok(l) => loaded.push((name, l)),
                Err(e) => problems.push(format!("✖ {name}: {e}")),
            }
        }
        if loaded.is_empty() {
            self.unit_report = Some(("No IRs to send".into(), problems, true));
            return;
        }
        let used: std::collections::BTreeMap<i64, String> = self.irs.clone().unwrap_or_default().into_iter().collect();
        let sel: Vec<i64> = self.ir_sel.iter().copied().collect();
        let targets: Vec<i64> = if sel.len() == loaded.len() {
            sel.clone()
        } else {
            let from = sel.first().copied().unwrap_or(0);
            (from..crate::session::IR_SLOTS).filter(|s| !used.contains_key(s)).take(loaded.len()).collect()
        };
        // Each file gets a slot the person can change before anything is written.
        let mut rows = Vec::new();
        for (i, (file, l)) in loaded.into_iter().enumerate() {
            // A .wav can be either: the first samples of a converted NAM capture say so.
            let nam = crate::irs::is_nam_capture(&l.audio);
            let slot = targets.get(i).copied().unwrap_or(0);
            rows.push(IrPlanRow { file, name: l.name, audio: l.audio, slot, nam, notes: l.notes.join(", ") });
        }
        self.ir_plan = Some(IrPlan { rows, problems, firmware_ok: crate::nam::enabled() });
    }

    /// The window that sends files to the pedal: each file with its slot (changeable),
    /// what it replaces, and a warning before sending NAM captures.
    fn ir_plan_window(&mut self, ctx: &egui::Context) {
        let Some(mut plan) = self.ir_plan.take() else { return };
        let used: std::collections::BTreeMap<i64, String> = self.irs.clone().unwrap_or_default().into_iter().collect();
        let mut result = None;
        egui::Window::new("Send to the pedal's IR slots")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_min_width(560.0);
                ui.label(RichText::new("Pick the slot each file goes into.").color(MUTED));
                ui.add_space(6.0);
                let slots: Vec<i64> = plan.rows.iter().map(|r| r.slot).collect();
                egui::Grid::new("ir-plan").num_columns(4).spacing([10.0, 6.0]).show(ui, |ui| {
                    for (i, row) in plan.rows.iter_mut().enumerate() {
                        let tex = if row.nam { crate::icons::capture(ui.ctx()) } else { crate::icons::ir(ui.ctx()) };
                        ui.horizontal(|ui| {
                            if let Some(t) = tex {
                                ui.add(egui::Image::new(&t).fit_to_exact_size(Vec2::splat(22.0)));
                            }
                            ui.label(RichText::new(if row.nam { "NAM capture" } else { "IR" }).small().color(if row.nam { PINK } else { BLUE }));
                        });
                        ui.label(RichText::new(&row.name).strong()).on_hover_text(format!("{}\n{}", row.file, row.notes));
                        let label = |s: i64| match used.get(&s) {
                            Some(n) => format!("{} · {n}", s + 1),
                            None => format!("{} · empty", s + 1),
                        };
                        egui::ComboBox::from_id_salt(("plan-slot", i)).selected_text(label(row.slot)).width(220.0).height(360.0).show_ui(ui, |ui| {
                            for s in 0..crate::session::IR_SLOTS {
                                let col = if used.contains_key(&s) { ORANGE } else { TEXT };
                                ui.selectable_value(&mut row.slot, s, RichText::new(label(s)).color(col));
                            }
                        });
                        let twice = slots.iter().filter(|s| **s == row.slot).count() > 1;
                        if twice {
                            ui.label(RichText::new("same slot twice").small().color(BAD));
                        } else if let Some(n) = used.get(&row.slot) {
                            ui.label(RichText::new(format!("replaces \"{n}\"")).small().color(ORANGE));
                        } else {
                            ui.label("");
                        }
                        ui.end_row();
                    }
                });
                if plan.rows.iter().any(|r| r.nam) {
                    ui.add_space(8.0);
                    theme::card(ui, |ui| {
                        ui.label(RichText::new("NAM captures only work with the NAM A2 Lite firmware on the pedal.").strong().color(ORANGE));
                        ui.label(RichText::new("Without it the pedal refuses them, and the transfer can hang and drop the connection.").small().color(MUTED));
                        ui.checkbox(&mut plan.firmware_ok, "My pedal has the NAM A2 Lite firmware");
                    });
                }
                for p in &plan.problems {
                    ui.label(RichText::new(p).small().color(BAD));
                }
                ui.add_space(8.0);
                let dup = (0..plan.rows.len()).any(|i| plan.rows.iter().skip(i + 1).any(|r| r.slot == plan.rows[i].slot));
                let sendable = plan.rows.iter().filter(|r| !r.nam || plan.firmware_ok).count();
                ui.horizontal(|ui| {
                    let label = if sendable == plan.rows.len() { format!("Send {sendable}") } else { format!("Send {sendable} (skip the captures)") };
                    if ui.add_enabled(!dup && sendable > 0, egui::Button::new(RichText::new(label).strong()).fill(accent().gamma_multiply(0.6))).clicked() {
                        result = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        result = Some(false);
                    }
                });
            });
        match result {
            Some(true) => {
                let mut items = Vec::new();
                for r in plan.rows.into_iter().filter(|r| !r.nam || plan.firmware_ok) {
                    self.ir_kinds.insert((r.slot, r.name.clone()), r.nam);
                    items.push((r.slot, r.name, r.audio));
                }
                self.send(Cmd::ImportIrs { items, backup: self.backup_dir() });
            }
            Some(false) => {}
            None => self.ir_plan = Some(plan),
        }
    }

    fn irs_page(&mut self, ctx: &egui::Context) {
        if self.irs_local {
            return self.tones_page(ctx);
        }
        let connected = self.connected.is_some() && self.link.is_some();
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(bg()).inner_margin(egui::Margin::same(18)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if theme::segmented(ui, &["On the pedal", "On this computer"], 0) == Some(1) {
                        self.irs_local = true;
                        self.scan_tones();
                        // Sending from the library needs the pedal's IR list (to find free slots).
                        if connected && self.irs.is_none() {
                            self.send(Cmd::ListIrs);
                        }
                    }
                    ui.add_space(8.0);
                    ui.label(RichText::new("Pedal IRs").size(24.0).strong());
                    theme::help(
                        ui,
                        "The pedal's 128 User IR slots. Send .wav impulse responses (any rate or depth; they're made 48 kHz mono, 2,048 samples), \
                         or NAM captures (.nam) for the NAM A2 Lite firmware, which load as IRs too. Download saves slots as .wav files, \
                         so a NAM capture comes back as the .wav the NAM block reads.",
                    );
                    if let Some(l) = &self.irs {
                        ui.label(RichText::new(format!("{} of 128 slots in use", l.len())).color(MUTED));
                    }
                });
                ui.add_space(6.0);
                if !connected {
                    theme::card(ui, |ui| {
                        ui.label(RichText::new("Connect to a pedal to see and change its IRs.").color(MUTED));
                        ui.add_space(4.0);
                        if ui.button("Convert .nam captures to .wav files...").clicked() {
                            self.convert_nam_files();
                        }
                    });
                    return;
                }
                let used: std::collections::BTreeMap<i64, String> = self.irs.clone().unwrap_or_default().into_iter().collect();
                let sel: Vec<i64> = self.ir_sel.iter().copied().collect();
                let busy = self.busy.is_some();
                let mut cmd: Option<Cmd> = None;
                let mut upload = false;
                ui.horizontal_wrapped(|ui| {
                    if ui.button("⟳ Read list").clicked() {
                        cmd = Some(Cmd::ListIrs);
                    }
                    ui.add_enabled_ui(!busy && self.irs.is_some(), |ui| {
                        if theme::primary_button(ui, "⬆ Send IRs or NAM captures...").on_hover_text("Goes into the selected slots, or the next empty ones").clicked() {
                            upload = true;
                        }
                        let picked: Vec<(i64, String)> = sel.iter().filter_map(|s| used.get(s).map(|n| (*s, n.clone()))).collect();
                        let into = self.tones.folder();
                        let into_label = format!("Saved into your IR library: {}", into.display());
                        if ui.add_enabled(!picked.is_empty(), egui::Button::new(format!("⬇ Download selected ({})", picked.len()))).on_hover_text(&into_label).clicked() {
                            cmd = Some(Cmd::ExportIrs { slots: picked.clone(), folder: into.clone() });
                        }
                        if ui.add_enabled(!used.is_empty(), egui::Button::new("⬇ Download all")).on_hover_text(&into_label).clicked() {
                            cmd = Some(Cmd::ExportIrs { slots: used.iter().map(|(s, n)| (*s, n.clone())).collect(), folder: into.join("From pedal") });
                        }
                        let one = (picked.len() == 1).then(|| picked[0].clone());
                        if ui.add_enabled(one.is_some(), egui::Button::new("Rename")).clicked() {
                            self.ir_rename = one.clone();
                        }
                        if ui.add_enabled(!picked.is_empty(), egui::Button::new(RichText::new("Remove").color(BAD))).clicked() {
                            let names: Vec<String> = picked.iter().map(|(s, n)| format!("{} \"{n}\"", s + 1)).collect();
                            self.confirm = Some((format!("Remove {} IR{} from the pedal? ({})\nPresets that use them will have an empty IR.", picked.len(), if picked.len() == 1 { "" } else { "s" }, names.join(", ")), Cmd::DeleteIrs { slots: picked.iter().map(|p| p.0).collect(), backup: self.backup_dir() }));
                        }
                    });
                    if !sel.is_empty() && ui.small_button("Clear selection").clicked() {
                        self.ir_sel.clear();
                    }
                });
                ui.add_space(8.0);
                if self.irs.is_none() {
                    ui.label(RichText::new(if busy { "Reading..." } else { "Press \"Read list\" to see the pedal's IRs." }).color(MUTED));
                } else {
                    let multi = ui.input(|i| i.modifiers.ctrl || i.modifiers.command || i.modifiers.shift);
                    let backup = self.backup_dir();
                    // IRs and NAM captures share the pedal's slots but are different things: show them apart.
                    let kind_of = |slot: i64| -> Option<Option<bool>> { used.get(&slot).map(|n| self.ir_kinds.get(&(slot, n.clone())).copied()) };
                    let n_nam = used.keys().filter(|s| kind_of(**s) == Some(Some(true))).count();
                    let n_ir = used.len() - n_nam;
                    let n_empty = crate::session::IR_SLOTS as usize - used.len();
                    ui.horizontal(|ui| {
                        let labels = [
                            "All slots".to_owned(),
                            format!("IRs ({n_ir})"),
                            format!("NAM captures ({n_nam})"),
                            format!("Empty ({n_empty})"),
                        ];
                        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
                        if let Some(v) = theme::segmented(ui, &refs, self.ir_view) {
                            self.ir_view = v;
                        }
                        if used.keys().any(|s| kind_of(*s) == Some(None)) {
                            ui.label(RichText::new("sorting...").small().color(MUTED));
                        }
                    });
                    ui.add_space(6.0);
                    let view = self.ir_view;
                    let shown: Vec<i64> = (0..crate::session::IR_SLOTS)
                        .filter(|s| match (view, kind_of(*s)) {
                            (0, _) => true,
                            (1, Some(k)) => k != Some(true),
                            (2, Some(k)) => k == Some(true),
                            (3, None) => true,
                            _ => false,
                        })
                        .collect();
                    if shown.is_empty() {
                        ui.label(RichText::new(match view {
                            1 => "No IRs on the pedal.",
                            2 => "No NAM captures on the pedal.",
                            _ => "No empty slots.",
                        }).color(MUTED));
                    }
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        let cols = ((ui.available_width() / 300.0).floor() as usize).clamp(1, 4);
                        egui::Grid::new("ir-grid").num_columns(cols).spacing([10.0, 4.0]).show(ui, |ui| {
                            for (n_shown, slot) in shown.iter().copied().enumerate() {
                                let name = used.get(&slot);
                                let on = self.ir_sel.contains(&slot);
                                let nam = kind_of(slot) == Some(Some(true));
                                // One row: the slot number, an IR or capture icon, the name, all from the left.
                                let (rect, r) = ui.allocate_exact_size(Vec2::new(280.0, 26.0), egui::Sense::click());
                                let p = ui.painter();
                                let bg = if on { accent().gamma_multiply(0.25) } else if r.hovered() { card_hi() } else { Color32::TRANSPARENT };
                                p.rect_filled(rect, egui::CornerRadius::same(5), bg);
                                if on {
                                    p.rect_stroke(rect, egui::CornerRadius::same(5), egui::Stroke::new(1.0_f32, accent()), egui::StrokeKind::Inside);
                                }
                                let mono = egui::FontId::monospace(13.0);
                                p.text(rect.left_center() + Vec2::new(6.0, 0.0), egui::Align2::LEFT_CENTER, format!("{:>3}", slot + 1), mono.clone(), MUTED);
                                let icon_rect = egui::Rect::from_center_size(rect.left_center() + Vec2::new(50.0, 0.0), Vec2::splat(22.0));
                                match name {
                                    Some(n) => {
                                        let tex = if nam { crate::icons::capture(ui.ctx()) } else { crate::icons::ir(ui.ctx()) };
                                        if let Some(t) = tex {
                                            p.image(t.id(), icon_rect, egui::Rect::from_min_max(egui::Pos2::ZERO, egui::Pos2::new(1.0, 1.0)), Color32::WHITE);
                                        }
                                        let galley = p.layout_no_wrap(n.clone(), egui::FontId::proportional(14.0), if nam { PINK } else { TEXT });
                                        let at = rect.left_center() + Vec2::new(68.0, -galley.size().y / 2.0);
                                        p.with_clip_rect(rect.shrink(2.0)).galley(at, galley, TEXT);
                                    }
                                    None => {
                                        p.text(rect.left_center() + Vec2::new(68.0, 0.0), egui::Align2::LEFT_CENTER, "empty", egui::FontId::proportional(13.0), MUTED.linear_multiply(0.7));
                                    }
                                }
                                let r = r
                                    .on_hover_text(match (name.is_some(), nam) {
                                        (true, true) => "A NAM capture (for the NAM A2 Lite block)",
                                        (true, false) => "An impulse response (for a cab / IR block)",
                                        _ => "Empty slot",
                                    });
                                if r.clicked() {
                                    if multi {
                                        if !self.ir_sel.remove(&slot) {
                                            self.ir_sel.insert(slot);
                                        }
                                    } else if on && self.ir_sel.len() == 1 {
                                        self.ir_sel.clear();
                                    } else {
                                        self.ir_sel.clear();
                                        self.ir_sel.insert(slot);
                                    }
                                }
                                if let Some(n) = name {
                                    r.context_menu(|ui| {
                                        let into = self.tones.folder();
                                        if ui.button("Download to the library").clicked() {
                                            cmd = Some(Cmd::ExportIrs { slots: vec![(slot, n.clone())], folder: into });
                                            ui.close_menu();
                                        }
                                        if ui.button("Rename...").clicked() {
                                            self.ir_rename = Some((slot, n.clone()));
                                            ui.close_menu();
                                        }
                                        ui.separator();
                                        if ui.button(RichText::new("Remove").color(BAD)).clicked() {
                                            self.confirm = Some((
                                                format!("Remove {} \"{n}\" from the pedal?\nPresets that use it will have an empty IR.", slot + 1),
                                                Cmd::DeleteIrs { slots: vec![slot], backup: backup.clone() },
                                            ));
                                            ui.close_menu();
                                        }
                                    });
                                } else {
                                    r.context_menu(|ui| {
                                        if ui.button("Send an IR or capture to this slot...").clicked() {
                                            self.ir_sel.clear();
                                            self.ir_sel.insert(slot);
                                            upload = true;
                                            ui.close_menu();
                                        }
                                    });
                                }
                                if (n_shown + 1) % cols == 0 {
                                    ui.end_row();
                                }
                            }
                        });
                    });
                }
                if upload {
                    self.upload_irs();
                }
                if let Some(c) = cmd {
                    self.send(c);
                }
            });
        // Renaming one IR.
        if let Some((slot, mut name)) = self.ir_rename.take() {
            let mut result = None;
            egui::Window::new("Rename IR").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                ui.label(format!("Slot {}", slot + 1));
                let r = ui.add(egui::TextEdit::singleline(&mut name).char_limit(crate::session::IR_NAME_MAX).desired_width(260.0));
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.horizontal(|ui| {
                    if theme::primary_button(ui, "Rename").clicked() || enter {
                        result = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        result = Some(false);
                    }
                });
            });
            match result {
                Some(true) => {
                    let n = crate::irs::clean_name(&name);
                    if !n.is_empty() {
                        self.send(Cmd::RenameIr { slot, name: n });
                    }
                }
                Some(false) => {}
                None => self.ir_rename = Some((slot, name)),
            }
        }
    }

    /// The IR and NAM library on this computer (the IRs page's second view).
    fn tones_page(&mut self, ctx: &egui::Context) {
        let connected = self.connected.is_some() && self.link.is_some();
        // Connected but the IR list not read yet (connected while this view was open): read it once.
        if connected && self.irs.is_none() && self.busy.is_none() && !self.tones_list_asked {
            self.tones_list_asked = true;
            self.send(Cmd::ListIrs);
        }
        if !connected {
            self.tones_list_asked = false;
        }
        let can_send = connected && self.irs.is_some() && self.busy.is_none();
        let opts = self.nam_opts();
        let mut back = false;
        let actions = egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(bg()).inner_margin(egui::Margin::same(18)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if theme::segmented(ui, &["On the pedal", "On this computer"], 1) == Some(0) {
                        back = true;
                    }
                    ui.add_space(8.0);
                    ui.label(RichText::new("IR and NAM library").size(24.0).strong());
                    theme::help(
                        ui,
                        "IRs (.wav) and NAM captures (.nam) kept on this computer, in real folders. Each capture shows whether it has been converted \
                         to the .wav the NAM A2 Lite block loads (written beside it as \"<name> NAM.wav\"). Select files and send them to the pedal; \
                         IRs downloaded from the pedal land in the selected folder here.",
                    );
                    let n_nam = self.tones.entries.iter().filter(|e| e.kind == crate::tonelib::Kind::Nam).count();
                    let n_conv = self.tones.entries.iter().filter(|e| e.converted.is_some()).count();
                    ui.label(RichText::new(format!("{} files · {n_conv} of {n_nam} captures converted", self.tones.entries.len())).color(MUTED));
                    if ui.small_button("⟳").on_hover_text("Look again").clicked() {
                        self.tones.scan(opts);
                    }
                });
                ui.add_space(8.0);
                self.tones.ui(ui, can_send, opts)
            })
            .inner;
        for a in actions {
            match a {
                crate::tonelib::Action::SendToPedal(files) => self.upload_ir_files(files),
                crate::tonelib::Action::Note(ok, text) => self.note(if ok { LogLevel::Good } else { LogLevel::Bad }, text),
            }
        }
        if back {
            self.irs_local = false;
            if self.irs.is_none() && self.connected.is_some() && self.link.is_some() {
                self.send(Cmd::ListIrs);
            }
        }
    }

    /// The hidden Tests page: a careful, self-restoring run of every pedal operation.
    fn tests_page(&mut self, ctx: &egui::Context) {
        let connected = self.connected.is_some() && self.link.is_some();
        // Sensible defaults: the last preset slot that isn't loaded, and the highest empty IR slot.
        let loaded = self.current.as_ref().filter(|c| c.0 == self.setlist).map(|c| c.1);
        if self.tests.preset_slot.is_none() && !self.presets.is_empty() {
            self.tests.preset_slot = (0..self.presets.len() as i64).rev().find(|i| Some(*i) != loaded);
        }
        if self.tests.ir_slot.is_none() {
            if let Some(l) = &self.irs {
                self.tests.ir_slot = (0..crate::session::IR_SLOTS).rev().find(|s| !l.iter().any(|(x, _)| x == s));
            }
        }
        let mut start = false;
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(bg()).inner_margin(egui::Margin::same(18)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Pedal self-test").size(24.0).strong());
                    theme::pill(ui, "HIDDEN · Ctrl+Shift+T", ORANGE);
                    theme::help(
                        ui,
                        "Tests every operation this app uses, as safely as it can: it backs up the whole setlist and the test IR first, \
                         reads everything twice, writes only to the one preset slot and the one IR slot you pick, checks each step by reading it back, \
                         then puts both slots and the loaded preset back and checks that too. If a step fails it restores at once.",
                    );
                });
                ui.add_space(8.0);
                if !connected {
                    theme::card(ui, |ui| {
                        ui.label(RichText::new("Connect to a pedal first.").color(MUTED));
                    });
                    return;
                }
                let running = self.tests.running;
                theme::card(ui, |ui| {
                    ui.add_enabled_ui(!running, |ui| {
                        theme::section_help(ui, "Test slots", "Pick slots you don't mind being used for a moment: empty ones are best. The loaded preset can't be picked.");
                        ui.horizontal(|ui| {
                            ui.label("Preset slot");
                            let shown = self.tests.preset_slot.map(|i| format!("{}  {}", slot_label(i as usize), self.presets.get(i as usize).cloned().unwrap_or_default())).unwrap_or_else(|| "pick one".into());
                            egui::ComboBox::from_id_salt("test-preset").width(240.0).selected_text(shown).show_ui(ui, |ui| {
                                for (i, n) in self.presets.iter().enumerate() {
                                    let i = i as i64;
                                    let is_loaded = Some(i) == loaded;
                                    let text = format!("{}  {n}{}", slot_label(i as usize), if is_loaded { "  (loaded)" } else { "" });
                                    if ui.add_enabled(!is_loaded, egui::SelectableLabel::new(self.tests.preset_slot == Some(i), text)).clicked() {
                                        self.tests.preset_slot = Some(i);
                                    }
                                }
                            });
                            ui.add_space(16.0);
                            ui.label("IR slot");
                            match &self.irs {
                                None => {
                                    if ui.button("Read the IR list").clicked() {
                                        let _ = self.link.as_ref().map(|l| l.tx.send(Cmd::ListIrs));
                                    }
                                }
                                Some(l) => {
                                    let name_of = |s: i64| l.iter().find(|(x, _)| *x == s).map(|(_, n)| n.clone());
                                    let shown = self.tests.ir_slot.map(|s| format!("{}  {}", s + 1, name_of(s).unwrap_or_else(|| "empty".into()))).unwrap_or_else(|| "pick one".into());
                                    egui::ComboBox::from_id_salt("test-ir").width(240.0).selected_text(shown).show_ui(ui, |ui| {
                                        for s in 0..crate::session::IR_SLOTS {
                                            let text = format!("{}  {}", s + 1, name_of(s).unwrap_or_else(|| "empty".into()));
                                            if ui.selectable_label(self.tests.ir_slot == Some(s), text).clicked() {
                                                self.tests.ir_slot = Some(s);
                                            }
                                        }
                                    });
                                }
                            }
                        });
                        ui.add_space(6.0);
                        theme::section(ui, "What to test");
                        ui.label(RichText::new("✔ Always (reading only): backup of the setlist and the test IR, double reads, preset list, IR list, one IR download.").small().color(MUTED));
                        ui.checkbox(&mut self.tests.preset_writes, "Preset writes: write a test preset into the slot, verify, rename, verify, restore, verify");
                        ui.checkbox(&mut self.tests.live_edit, "Live edit: set one knob of the loaded preset to the value it already has, verify, reload the preset (drops unsaved changes on the pedal)");
                        ui.checkbox(&mut self.tests.ir_writes, "IR writes: upload a test IR, verify name and audio, rename, verify, restore, verify");
                        let writes = self.tests.preset_writes || self.tests.live_edit || self.tests.ir_writes;
                        if writes {
                            ui.checkbox(&mut self.tests.understood, RichText::new("I understand the chosen slots are changed for a moment and then restored from the backup").color(ORANGE));
                        }
                        let folder = self.tests.folder.clone().unwrap_or_else(|| self.library_dir.join("Self-test backups"));
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("Backups go to {}", folder.display())).small().color(MUTED));
                            if ui.small_button("Change...").clicked() {
                                if let Some(d) = rfd::FileDialog::new().set_directory(&self.library_dir).pick_folder() {
                                    self.tests.folder = Some(d);
                                }
                            }
                        });
                        ui.add_space(6.0);
                        let ready = self.tests.preset_slot.is_some() && self.tests.ir_slot.is_some() && (!writes || self.tests.understood) && self.busy.is_none();
                        if ui.add_enabled(ready, egui::Button::new(RichText::new("▶ Run the self-test").strong().color(Color32::from_rgb(8, 26, 24))).fill(accent())).clicked() {
                            start = true;
                        }
                    });
                });
                ui.add_space(10.0);
                // Results.
                theme::card(ui, |ui| {
                    ui.horizontal(|ui| {
                        theme::section(ui, "Results");
                        if self.tests.running {
                            ui.spinner();
                            ui.label(RichText::new(self.busy.clone().unwrap_or_default()).small().color(MUTED));
                        }
                        match self.tests.done {
                            Some(0) => theme::pill(ui, "ALL PASSED", GOOD),
                            Some(n) => theme::pill(ui, &format!("{n} FAILED"), BAD),
                            None => {}
                        }
                    });
                    if self.tests.log.is_empty() {
                        ui.label(RichText::new("Nothing run yet.").color(MUTED));
                    }
                    egui::ScrollArea::vertical().auto_shrink([false, true]).max_height(ui.available_height() - 8.0).stick_to_bottom(true).show(ui, |ui| {
                        for (name, ok, detail) in &self.tests.log {
                            ui.horizontal_wrapped(|ui| {
                                let (mark, col) = match ok {
                                    Some(true) => ("✔", GOOD),
                                    Some(false) => ("✖", BAD),
                                    None => ("•", MUTED),
                                };
                                ui.label(RichText::new(mark).strong().color(col));
                                ui.label(RichText::new(name).strong());
                                ui.label(RichText::new(detail).color(if *ok == Some(false) { BAD } else { MUTED }));
                            });
                        }
                    });
                });
            });
        if start {
            let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            let base = self.tests.folder.clone().unwrap_or_else(|| self.library_dir.join("Self-test backups"));
            let plan = crate::selftest::Plan {
                setlist: self.setlist,
                preset_slot: self.tests.preset_slot.unwrap_or(0),
                ir_slot: self.tests.ir_slot.unwrap_or(127),
                folder: base.join(format!("backup-{stamp}")),
                preset_writes: self.tests.preset_writes,
                live_edit: self.tests.live_edit,
                ir_writes: self.tests.ir_writes,
            };
            self.tests.log.clear();
            self.tests.done = None;
            self.tests.running = true;
            self.tests.understood = false;
            self.send(Cmd::SelfTest(plan));
        }
    }

    fn settings_page(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(bg()).inner_margin(egui::Margin::same(18)))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.set_max_width(760.0);
                    ui.label(RichText::new("Settings").size(22.0).strong());
                    ui.label(RichText::new("Changes are saved right away.").color(MUTED));
                    ui.add_space(12.0);

                    // ---- Library folder.
                    let mut new_dir: Option<Option<PathBuf>> = None;
                    theme::card(ui, |ui| {
                        theme::section_help(ui, "Preset library", "The folder where your presets and setlists are kept on this computer.");
                        ui.add_space(4.0);
                        egui::Frame::new().fill(bg()).corner_radius(CornerRadius::same(6)).inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(RichText::new(self.library_dir.to_string_lossy()).monospace());
                        });
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            if theme::primary_button(ui, "Choose folder...").clicked() {
                                if let Some(d) = rfd::FileDialog::new().set_directory(&self.library_dir).pick_folder() {
                                    new_dir = Some(Some(d));
                                }
                            }
                            if ui.button("Show in Explorer").clicked() {
                                let _ = std::fs::create_dir_all(&self.library_dir);
                                crate::folders::open_in_explorer(&self.library_dir);
                            }
                            if ui
                                .add_enabled(self.settings.library.is_some(), egui::Button::new("Use the default"))
                                .on_hover_text(Self::default_library_dir().to_string_lossy())
                                .clicked()
                            {
                                new_dir = Some(None);
                            }
                        });
                        let files = self.library.len();
                        ui.label(RichText::new(format!("{files} preset file{} in the top folder", if files == 1 { "" } else { "s" })).small().color(MUTED));
                    });
                    if let Some(d) = new_dir {
                        self.set_library_dir(d);
                    }
                    ui.add_space(10.0);

                    // ---- IR and NAM library.
                    let mut tone_change: Option<Option<String>> = None;
                    theme::card(ui, |ui| {
                        theme::section_help(ui, "IR and NAM library", "The folder where your IRs (.wav) and NAM captures (.nam) are kept, with their converted files. By default it's \"IRs and NAM\" inside your preset library; it's left out of the preset folders.");
                        egui::Frame::new().fill(bg()).corner_radius(CornerRadius::same(6)).inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(RichText::new(self.tone_dir().to_string_lossy()).monospace());
                        });
                        ui.horizontal(|ui| {
                            if ui.button("Choose folder...").clicked() {
                                if let Some(d) = rfd::FileDialog::new().set_directory(self.tone_dir()).pick_folder() {
                                    tone_change = Some(Some(d.to_string_lossy().into_owned()));
                                }
                            }
                            if ui.button("Show in Explorer").clicked() {
                                crate::folders::open_in_explorer(&self.tone_dir());
                            }
                            if ui.add_enabled(self.settings.tone_library.is_some(), egui::Button::new("Use the default")).clicked() {
                                tone_change = Some(None);
                            }
                        });
                    });
                    if let Some(t) = tone_change {
                        self.settings.tone_library = t;
                        self.settings.save();
                        self.scan_tones();
                        self.scan_library();
                    }
                    ui.add_space(10.0);

                    // ---- Effects list.
                    let mut reload = false;
                    theme::card(ui, |ui| {
                        theme::section_help(ui, "Effects list", "Effect and amp names, knob ranges and colours are read from HX Edit's resource folder.");
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            match &self.catalog {
                                Some(c) => theme::pill(ui, &format!("Loaded  •  {} models", c.model_count()), GOOD),
                                None => theme::pill(ui, "Not found", ORANGE),
                            }
                            let shown = self.settings.hx_edit.clone().unwrap_or_else(|| "Looking in the usual place".into());
                            ui.label(RichText::new(shown).small().color(MUTED));
                        });
                        ui.horizontal(|ui| {
                            if ui.button("Choose folder...").clicked() {
                                if let Some(d) = rfd::FileDialog::new().pick_folder() {
                                    self.settings.hx_edit = Some(d.to_string_lossy().into_owned());
                                    self.settings.save();
                                    reload = true;
                                }
                            }
                            if ui.add_enabled(self.settings.hx_edit.is_some(), egui::Button::new("Use the usual place")).clicked() {
                                self.settings.hx_edit = None;
                                self.settings.save();
                                reload = true;
                            }
                            if ui.button("Load again").clicked() {
                                reload = true;
                            }
                        });
                    });
                    if reload {
                        self.reload_catalog();
                    }
                    ui.add_space(10.0);

                    // ---- Unit.
                    let mut pick = None;
                    theme::card(ui, |ui| {
                        theme::section_help(ui, "Unit", "New presets are made for this unit when no pedal is connected. A connected pedal always decides.");
                        ui.add_enabled_ui(self.connected.is_none(), |ui| {
                            egui::ComboBox::from_id_salt("settings-unit")
                                .width(260.0)
                                .selected_text(crate::units::unit_name(self.unit))
                                .show_ui(ui, |ui| {
                                    for (d, n) in crate::snapshots::UNITS {
                                        if ui.selectable_label(d == self.unit, n).clicked() && d != self.unit {
                                            pick = Some(d);
                                        }
                                    }
                                });
                        });
                    });
                    if let Some(d) = pick {
                        self.set_unit(d);
                    }
                    ui.add_space(10.0);

                    // ---- NAM A2 Lite (optional community firmware).
                    let mut changed = false;
                    let mut convert = false;
                    theme::card(ui, |ui| {
                        theme::section_help(
                            ui,
                            "NAM captures (optional firmware)",
                            "For pedals running the community \"NAM A2 Lite\" firmware (HX Stomp, HX Stomp XL and HX Effects, firmware 3.80). \
                             That firmware plays Neural Amp Modeler captures in a block that takes the place of the mono Xenomorph Fuzz. \
                             It is unofficial: install it only if you accept the risk. This app doesn't install firmware.",
                        );
                        ui.horizontal(|ui| {
                            if ui.checkbox(&mut self.settings.nam_lite, "My pedal runs the NAM A2 Lite firmware").changed() {
                                crate::nam::set_enabled(self.settings.nam_lite);
                                changed = true;
                            }
                            theme::help(
                                ui,
                                "When on, the editor shows the block as \"NAM A2 Lite\" with its own knobs (Input, Treble, Output, Polarity, Bass, Mid, Model, Mid Freq, Q, High Cut). \
                                 On the HX Stomp and XL it is under Amp -> NAM; on the HX Effects under Distortion -> Mono. \
                                 Presets store it as the mono Xenomorph Fuzz, as the firmware does. Up to 3 per preset, about 23% of the DSP each.",
                            );
                        });
                        if self.settings.nam_lite && !crate::nam::UNITS.contains(&self.unit) {
                            ui.label(RichText::new(format!("Not shown for the {}: the firmware is for the HX Stomp, HX Stomp XL and HX Effects.", crate::units::unit_name(self.unit))).small().color(ORANGE));
                        }
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            if theme::primary_button(ui, "Convert .nam to IR...").clicked() {
                                convert = true;
                            }
                            theme::help(
                                ui,
                                "Turns NAM captures (3-channel \"A2 Lite\" size, 48 kHz) into .wav files. Import a .wav into a User IR slot with HX Edit, \
                                 then set the NAM block's Model knob to that slot's number. The .wav isn't a sound: it carries the capture.",
                            );
                            changed |= ui.checkbox(&mut self.settings.nam_use_level, "Match input level").on_hover_text("Use the capture's recorded input level, so Input 5.0 drives it as the capture did").changed();
                            changed |= ui.checkbox(&mut self.settings.nam_use_loudness, "Even out loudness").on_hover_text("Use the capture's loudness so captures play at about the same volume").changed();
                        });
                    });
                    if convert {
                        self.convert_nam_files();
                    }
                    ui.add_space(10.0);

                    // ---- Boxes.
                    theme::card(ui, |ui| {
                        theme::section_help(ui, "Helrig Boxes", "Boxes this computer is paired with. Forgetting one means pairing it again before you can connect.");
                        changed |= ui
                            .checkbox(&mut self.settings.discover_boxes, "Look for boxes on the network")
                            .on_hover_text("Takes effect the next time the app starts")
                            .changed();
                        ui.add_space(4.0);
                        if self.settings.saved.is_empty() && self.settings.tokens.is_empty() {
                            ui.label(RichText::new("No paired boxes yet. Pair one from the left panel.").color(MUTED));
                        }
                        let mut forget = None;
                        for (i, b) in self.settings.saved.iter().enumerate() {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&b.name).strong());
                                ui.label(RichText::new(&b.host).small().color(MUTED));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.small_button("Forget").clicked() {
                                        forget = Some(i);
                                    }
                                });
                            });
                        }
                        if let Some(i) = forget {
                            let b = self.settings.saved.remove(i);
                            self.settings.tokens.remove(&boxes::box_key(&b));
                            self.settings.tokens.remove(&b.host);
                            changed = true;
                        }
                        if !self.settings.tokens.is_empty() {
                            ui.add_space(4.0);
                            if ui.button(RichText::new("Forget all pairings").color(BAD)).clicked() {
                                self.settings.tokens.clear();
                                self.settings.saved.clear();
                                changed = true;
                                self.note(LogLevel::Info, "All box pairings forgotten.");
                            }
                        }
                    });
                    ui.add_space(10.0);

                    // ---- Behaviour.
                    theme::card(ui, |ui| {
                        theme::section(ui, "Behaviour");
                        changed |= ui.checkbox(&mut self.settings.confirm_pedal, "Ask before writing to or clearing a slot on the pedal").changed();
                        changed |= ui.checkbox(&mut self.settings.confirm_delete, "Ask before deleting a preset file").changed();
                        changed |= ui.checkbox(&mut self.settings.show_sample, "Open the sample preset at start").changed();
                        changed |= ui.checkbox(&mut self.settings.start_in_library, "Start on the library page").changed();
                    });
                    ui.add_space(10.0);

                    // ---- Look.
                    let mut names_changed = false;
                    theme::card(ui, |ui| {
                        theme::section(ui, "Display");
                        ui.horizontal(|ui| {
                            ui.label("Look");
                            for (label, studio) in [("Studio (new)", true), ("Classic", false)] {
                                if ui.selectable_label(self.settings.studio == studio, label).clicked() && self.settings.studio != studio {
                                    self.settings.studio = studio;
                                    crate::chain::set_studio(studio);
                                    changed = true;
                                }
                            }
                            theme::help(ui, "Studio is the new look: bigger, more colourful effect boxes with their icons. Classic goes back to the look before the redesign.");
                        });
                        ui.horizontal(|ui| {
                            ui.label("Effect names");
                            for (label, real) in [("Pedal names", false), ("Real gear names", true)] {
                                if ui.selectable_label(self.settings.real_names == real, label).clicked() && self.settings.real_names != real {
                                    self.settings.real_names = real;
                                    crate::realnames::set(real);
                                    names_changed = true;
                                }
                            }
                            theme::help(ui, "Real gear names show what each effect is modelled on, e.g. \"Klon Centaur\" instead of \"Minotaur\". Hover an effect in the picker to see the other name. Effects with no real-world model keep the pedal's name.");
                        });
                        ui.horizontal(|ui| {
                            ui.label("Size");
                            for (label, s) in [("Small", 0.9f32), ("Normal", 1.0), ("Large", 1.15), ("Extra large", 1.3)] {
                                if ui.selectable_label((self.settings.ui_scale - s).abs() < 0.01, label).clicked() {
                                    self.settings.ui_scale = s;
                                    ctx.set_zoom_factor(s);
                                    changed = true;
                                }
                            }
                        });
                    });
                    if names_changed {
                        // The library cards keep names from their last look.
                        self.scan_library();
                        changed = true;
                    }
                    if changed {
                        self.settings.save();
                    }
                    ui.add_space(10.0);
                    // About.
                    theme::card(ui, |ui| {
                        ui.vertical_centered(|ui| {
                            if let Some(t) = theme::logo(ui.ctx(), theme::Logo::WithName) {
                                ui.add(egui::Image::new(&t).fit_to_exact_size(Vec2::new(300.0, 121.0)));
                            }
                            ui.label(RichText::new(format!("ToneStrand {}", env!("CARGO_PKG_VERSION"))).color(MUTED));
                            ui.label(RichText::new("Preset editor for Helix-family pedals, over USB or a Helrig Box").small().color(MUTED));
                        });
                    });
                });
            });
    }

    /// Keep the undo history of a preset: an edit that starts after a short pause is a new
    /// step (so one knob drag is one step); `history` asks to undo (true) or redo (false).
    fn track_history(&mut self, i: usize, before: DocKind, history: Option<bool>) {
        let Some(doc) = self.docs.get_mut(i) else { return };
        if doc.is_live() {
            doc.undo.clear();
            doc.redo.clear();
            return;
        }
        match history {
            Some(true) => {
                if let Some(prev) = doc.undo.pop() {
                    doc.redo.push(std::mem::replace(&mut doc.kind, prev));
                    doc.dirty = true;
                    doc.last_edit = None;
                }
                return;
            }
            Some(false) => {
                if let Some(next) = doc.redo.pop() {
                    doc.undo.push(std::mem::replace(&mut doc.kind, next));
                    doc.dirty = true;
                    doc.last_edit = None;
                }
                return;
            }
            None => {}
        }
        let changed = match (&before, &doc.kind) {
            (DocKind::Hlx { json: a }, DocKind::Hlx { json: b }) => a != b,
            (DocKind::Device { preset: a, .. }, DocKind::Device { preset: b, .. }) => !a.same_as(b),
            _ => true,
        };
        if !changed {
            return;
        }
        if doc.last_edit.is_none_or(|t| t.elapsed() > Duration::from_millis(700)) {
            doc.undo.push(before);
            if doc.undo.len() > 100 {
                doc.undo.drain(..1);
            }
            doc.redo.clear();
        }
        doc.last_edit = Some(Instant::now());
    }

    /// Take the presets out of a device backup (.hxb) or setlist export (.hls) into the
    /// library: a folder named after the file, with a folder per setlist, one .hlx per preset.
    fn import_backup(&mut self, file: &Path, into: &Path) -> Result<(usize, Vec<String>), String> {
        let bytes = std::fs::read(file).map_err(|e| e.to_string())?;
        let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        let (setlists, irs) = if ext == "hxb" {
            let c = crate::backup::read_hxb(&bytes)?;
            (c.setlists, c.irs)
        } else {
            (vec![crate::backup::read_hls(&bytes)?], Vec::new())
        };
        let stem = device::sanitize(file.file_stem().and_then(|s| s.to_str()).unwrap_or("Backup"));
        let base = into.join(&stem);
        let mut n = 0;
        let mut lines = Vec::new();
        for s in &setlists {
            let dir = if setlists.len() == 1 && ext == "hls" { base.clone() } else { base.join(device::sanitize(&s.name)) };
            let _ = std::fs::create_dir_all(&dir);
            let mut written = 0;
            for p in &s.presets {
                let path = crate::files::unique_path(&dir, &format!("{} {}.hlx", slot_label(p.slot), device::sanitize(&p.name)));
                match crate::files::write_atomic(&path, &serde_json::to_vec_pretty(&p.hlx).unwrap_or_default()) {
                    Ok(_) => written += 1,
                    Err(e) => lines.push(format!("✖ {}: {e}", path.display())),
                }
            }
            n += written;
            let skipped = if s.skipped > 0 { format!(", {} unreadable slots skipped", s.skipped) } else { String::new() };
            lines.push(format!("✔ {} -> {stem}/{}: {written} presets{skipped}", file.file_name().and_then(|x| x.to_str()).unwrap_or("?"), s.name));
        }
        if !irs.is_empty() {
            // The backup's IRs go to the IR library, in a folder named after the backup.
            let ir_dir = self.tone_dir().join(&stem);
            let _ = std::fs::create_dir_all(&ir_dir);
            let written = irs
                .iter()
                .filter(|ir| crate::files::write_atomic(&crate::files::unique_path(&ir_dir, &crate::irs::file_name(ir.slot as i64, &ir.name)), &ir.wav).is_ok())
                .count();
            lines.push(format!("✔ {written} IRs -> IR library/{stem}"));
            self.scan_tones();
        }
        Ok((n, lines))
    }

    fn library_page(&mut self, ctx: &egui::Context) {
        let can_send = self.connected.is_some() && self.chosen.is_some();
        let actions = egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(bg()).inner_margin(egui::Margin::same(18)))
            .show(ctx, |ui| self.lib.ui(ui, can_send))
            .inner;
        for a in actions {
            match a {
                library::Action::Open(p) => {
                    self.open_file(&p);
                    self.view = View::Editor;
                }
                library::Action::SendToPedal(p) if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("hlx")) => {
                    let Some(slot) = self.chosen else { continue };
                    let Some(cat) = self.catalog.clone() else {
                        self.note(LogLevel::Bad, "Sending .hlx presets needs HX Edit's effect list (see Settings).");
                        continue;
                    };
                    let json = std::fs::read_to_string(&p).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str::<serde_json::Value>(&t).map_err(|e| e.to_string()));
                    let mut json = match json {
                        Ok(j) => j,
                        Err(e) => {
                            self.note(LogLevel::Bad, format!("Cannot read {}: {e}", p.display()));
                            continue;
                        }
                    };
                    // A preset made for another unit is converted to this pedal first.
                    if crate::snapshots::device(&json) != self.unit {
                        if let Err(why) = crate::units::convert(&mut json, self.unit, Some(&cat)) {
                            self.unit_report = Some((format!("This preset doesn't fit the {}", crate::units::unit_name(self.unit)), why, true));
                            continue;
                        }
                    }
                    let warning = dsp_warning(&cat, &json).map(|w| format!("\n\n{w}")).unwrap_or_default();
                    let name = crate::hlx::name(&json);
                    let now = self.presets.get(slot).cloned().unwrap_or_default();
                    self.confirm = Some((
                        format!("Write \"{name}\" into {} (now \"{now}\")?\n\nIt is converted from .hlx using the pedal's loaded preset as the template. The slot's current preset is backed up first.{warning}", slot_label(slot)),
                        Cmd::WriteHlx { setlist: self.setlist, index: slot as i64, name, json, cat, backup: self.backup_dir() },
                    ));
                }
                library::Action::SendToPedal(p) => {
                    let Some(slot) = self.chosen else { continue };
                    match std::fs::read(&p).map_err(|e| e.to_string()).and_then(|b| DevicePreset::parse(&b)) {
                        Ok(preset) => {
                            let doc = preset.encode();
                            let name = p.file_stem().and_then(|s| s.to_str()).unwrap_or("Preset");
                            let name = match name.split_once(' ') {
                                Some((a, b)) if a.len() == 3 && a.ends_with(['A', 'B', 'C']) => b.to_owned(),
                                _ => name.to_owned(),
                            };
                            let now = self.presets.get(slot).cloned().unwrap_or_default();
                            self.confirm = Some((
                                format!("Write \"{name}\" into {} (now \"{now}\")?", slot_label(slot)),
                                Cmd::WriteSlot { setlist: self.setlist, index: slot as i64, name, doc, backup: self.backup_dir() },
                            ));
                        }
                        Err(e) => self.note(LogLevel::Bad, format!("Cannot send {} to the pedal: {e}", p.display())),
                    }
                }
                library::Action::Delete(p) => {
                    if self.settings.confirm_delete {
                        self.pending_delete = Some(p);
                    } else {
                        match std::fs::remove_file(&p) {
                            Ok(_) => self.note(LogLevel::Good, "Deleted"),
                            Err(e) => self.note(LogLevel::Bad, format!("Could not delete: {e}")),
                        }
                        self.scan_library();
                    }
                }
                library::Action::MoveTo(p, setlist) => {
                    let dir = if setlist.is_empty() { self.lib.root.clone() } else { self.lib.root.join(&setlist) };
                    if p.parent() != Some(dir.as_path()) {
                        match crate::folders::move_into(&p, &dir) {
                            Ok(_) => self.note(LogLevel::Good, format!("Moved to {}", if setlist.is_empty() { "the top folder" } else { &setlist })),
                            Err(e) => self.note(LogLevel::Bad, format!("Could not move: {e}")),
                        }
                    }
                    self.scan_library();
                }
                library::Action::Rescan => self.scan_library(),
                library::Action::Import => {
                    if let Some(files) = rfd::FileDialog::new()
                        .add_filter("Presets, backups and setlists", &["hlx", "hxpreset", "hxb", "hls"])
                        .pick_files()
                    {
                        let dir = self.lib.folder();
                        let _ = std::fs::create_dir_all(&dir);
                        let mut n = 0;
                        let mut report = Vec::new();
                        for f in files {
                            let ext = f.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
                            if ext == "hxb" || ext == "hls" {
                                match self.import_backup(&f, &dir) {
                                    Ok((count, lines)) => {
                                        n += count;
                                        report.extend(lines);
                                    }
                                    Err(e) => report.push(format!("✖ {}: {e}", f.file_name().and_then(|s| s.to_str()).unwrap_or("?"))),
                                }
                            } else {
                                let shown = f.file_name().and_then(|s| s.to_str()).unwrap_or("?").to_owned();
                                match crate::files::copy_into(&f, &dir) {
                                    Ok(to) => {
                                        n += 1;
                                        if to.file_name() != f.file_name() {
                                            let new = to.file_name().and_then(|s| s.to_str()).unwrap_or("?");
                                            report.push(format!("✔ {shown} was already there; kept both (new copy: {new})"));
                                        }
                                    }
                                    Err(e) => report.push(format!("✖ {shown}: {e}")),
                                }
                            }
                        }
                        let failures = report.iter().filter(|l| l.starts_with('✖')).count();
                        let (level, summary) = match (n, failures) {
                            (_, 0) if n > 0 => (LogLevel::Good, format!("Imported {n} presets")),
                            (0, 0) => (LogLevel::Info, "Nothing was imported".to_owned()),
                            (_, f) => (LogLevel::Bad, format!("Imported {n} presets; {f} could not be imported (see the report)")),
                        };
                        self.note(level, summary);
                        if !report.is_empty() {
                            self.unit_report = Some((format!("Imported {n} presets"), report, false));
                        }
                        self.scan_library();
                    }
                }
            }
        }
        if let Some(p) = self.pending_delete.clone() {
            let mut result = None;
            egui::Window::new("Delete preset")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(format!("Delete {} from your library? This cannot be undone.", p.file_name().and_then(|s| s.to_str()).unwrap_or("")));
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new("Delete").color(BAD)).clicked() {
                            result = Some(true);
                        }
                        if ui.button("Cancel").clicked() {
                            result = Some(false);
                        }
                    });
                });
            if let Some(yes) = result {
                if yes {
                    match std::fs::remove_file(&p) {
                        Ok(_) => self.note(LogLevel::Good, "Deleted"),
                        Err(e) => self.note(LogLevel::Bad, format!("Could not delete: {e}")),
                    }
                    self.scan_library();
                }
                self.pending_delete = None;
            }
        }
    }

    /// Change the unit everything is made for, and convert the open .hlx presets to it.
    fn set_unit(&mut self, unit: i64) {
        self.unit = unit;
        crate::nam::set_unit(unit);
        if self.connected.is_none() {
            self.settings.unit = Some(unit);
            self.settings.save();
        }
        let mut refused = Vec::new();
        let mut adjusted = Vec::new();
        for i in 0..self.docs.len() {
            match self.convert_doc(i) {
                Some(Err(lines)) => refused.push((self.docs[i].title.clone(), lines)),
                Some(Ok(lines)) if lines.len() > 1 => adjusted.push((self.docs[i].title.clone(), lines)),
                _ => {}
            }
        }
        let name = crate::units::unit_name(unit);
        if !refused.is_empty() {
            let mut lines = Vec::new();
            for (title, why) in refused {
                lines.push(format!("\"{title}\" stays as it was:"));
                lines.extend(why.into_iter().map(|w| format!("   • {w}")));
            }
            self.unit_report = Some((format!("Some presets do not fit the {name}"), lines, true));
        } else if !adjusted.is_empty() {
            let mut lines = Vec::new();
            for (title, what) in adjusted {
                lines.push(format!("\"{title}\":"));
                lines.extend(what.into_iter().map(|w| format!("   • {w}")));
            }
            self.unit_report = Some((format!("Presets converted to the {name}"), lines, false));
        }
    }

    /// Convert one open .hlx preset to the current unit. None when there was nothing to do.
    fn convert_doc(&mut self, i: usize) -> Option<Result<Vec<String>, Vec<String>>> {
        let unit = self.unit;
        let cat = self.catalog.clone();
        let doc = self.docs.get_mut(i)?;
        let DocKind::Hlx { json } = &mut doc.kind else { return None };
        if crate::snapshots::device(json) == unit {
            return None;
        }
        let r = crate::units::convert(json, unit, cat.as_deref());
        if r.is_ok() {
            doc.dirty = true;
            doc.selected = None;
        }
        Some(r)
    }

    /// Put the effect picked in the "Add a block" window into the open preset.
    /// On a live preset it also goes to the pedal at once.
    fn apply_pick(&mut self, cat: &Catalog, st: editors::PickerState, model: i64, cab: Option<i64>) {
        let connected = self.connected.is_some();
        let i = self.active.min(self.docs.len().saturating_sub(1));
        let mut cmd = None;
        let Some(doc) = self.docs.get_mut(i) else { return };
        match &mut doc.kind {
            DocKind::Device { preset, origin } => {
                if matches!(origin, Origin::Live { .. }) && connected {
                    let old = preset.slots().into_iter().find(|s| s.path == st.path && s.position == st.position);
                    let old_looper = old.as_ref().and_then(|s| s.model).is_some_and(|m| cat.kind_code(m) == 22);
                    let new_looper = cat.kind_code(model) == 22;
                    let same_pairing = old.as_ref().is_some_and(|s| s.paired.is_some() == cab.is_some());
                    cmd = Some(if st.replacing && same_pairing && !old_looper && !new_looper {
                        Cmd::SetModel { block: st.block, model, paired: cab }
                    } else if st.replacing {
                        Cmd::ReplaceBlock { block: st.block, slot: crate::devpreset::block_slot(cat, model, cab, true, false) }
                    } else {
                        Cmd::AddBlock { block: st.block, body: crate::devpreset::block_slot(cat, model, cab, true, false) }
                    });
                }
                preset.place_model(cat, st.path, st.position, model, cab);
                doc.selected = Some(st.slot);
            }
            DocKind::Hlx { json } => {
                let Some(dsp) = st.hlx_dsp.clone() else { return };
                let Some((hlx_model, stereo)) = cat.hlx_model(model) else { return };
                let params = cat.default_values(model);
                let cab_model = cab.and_then(|c| cat.hlx_model(c).map(|(m, _)| (m, cat.default_values(c))));
                let cab_ref = cab_model.as_ref().map(|(m, p)| (m.as_str(), p.as_slice()));
                if let Some(key) = hlx::place_model(json, &dsp, st.path, st.position as i64, st.hlx_key.as_deref(), &hlx_model, stereo, &params, cab_ref) {
                    // Select the new block in the chain.
                    let shown: Vec<hlx::HlxBlock> = hlx::blocks(json);
                    doc.selected = shown.iter().filter(|b| !b.key.starts_with("cab")).position(|b| b.dsp == dsp && b.key == key);
                }
            }
        }
        doc.dirty = true;
        self.note(LogLevel::Good, format!("{} placed", cat.model_name(model)));
        if let Some(c) = cmd {
            self.send(c);
        }
    }

    /// Keep a copy of an open preset in the library (in the setlist being shown there).
    fn save_to_library(&mut self, i: usize) {
        let Some(doc) = self.docs.get(i) else { return };
        let (ext, bytes) = match &doc.kind {
            DocKind::Hlx { json } => ("hlx", serde_json::to_vec_pretty(json).unwrap_or_default()),
            DocKind::Device { preset, .. } => ("hxpreset", preset.encode()),
        };
        let base = device::sanitize(if doc.name.is_empty() { &doc.title } else { &doc.name });
        let dir = self.lib.folder();
        let _ = std::fs::create_dir_all(&dir);
        let path = crate::files::unique_path(&dir, &format!("{base}.{ext}"));
        match crate::files::write_atomic(&path, &bytes) {
            Ok(_) => self.note(LogLevel::Good, format!("Saved to your library as {}", path.file_name().and_then(|s| s.to_str()).unwrap_or(""))),
            Err(e) => self.note(LogLevel::Bad, format!("Could not save to the library: {e}")),
        }
        self.scan_library();
    }

    fn central(&mut self, ctx: &egui::Context) {
        if self.view == View::Library {
            return self.library_page(ctx);
        }
        if self.view == View::Settings {
            return self.settings_page(ctx);
        }
        if self.view == View::Irs {
            return self.irs_page(ctx);
        }
        if self.view == View::Tests {
            return self.tests_page(ctx);
        }

        if crate::chain::studio() && !self.docs.is_empty() {
            return self.studio_central(ctx);
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(bg()).inner_margin(egui::Margin::same(14)))
            .show(ctx, |ui| {
                if self.docs.is_empty() {
                    ui.vertical_centered(|ui| {
                        ui.add_space(80.0);
                        if let Some(t) = theme::logo(ui.ctx(), theme::Logo::WithName) {
                            ui.add(egui::Image::new(&t).fit_to_exact_size(Vec2::new(360.0, 145.0)).tint(Color32::from_white_alpha(150)));
                            ui.add_space(16.0);
                        }
                        ui.label(RichText::new("No preset open").size(22.0).strong());
                        ui.label(RichText::new("Open a file, start a new preset, or edit one from the pedal.").color(MUTED));
                    });
                    return;
                }
                self.tabs(ui);
                // The last tab may have just been closed: the empty screen shows next frame.
                if self.docs.is_empty() {
                    ui.ctx().request_repaint();
                    return;
                }
                let i = self.active.min(self.docs.len() - 1);
                egui::Frame::new()
                    .fill(card_bg())
                    .stroke(Stroke::new(1.0_f32, line()))
                    .corner_radius(CornerRadius { nw: 0, ne: 10, sw: 10, se: 10 })
                    .inner_margin(egui::Margin::same(16))
                    .show(ui, |ui| {
                        ui.set_min_size(ui.available_size());
                        egui::ScrollArea::vertical().id_salt("editor").auto_shrink([false, false]).show(ui, |ui| {
                            self.editor(ui, i, 0);
                        });
                    });
            });
    }

    /// The Studio look's editor: the open presets' tabs, the chain on a large canvas with
    /// the controllers under it, and the selected block's panel docked at the bottom.
    fn studio_central(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().frame(egui::Frame::new().fill(bg()).inner_margin(egui::Margin::ZERO)).show(ctx, |ui| {
            egui::TopBottomPanel::top("studio-tabs")
                .show_separator_line(false)
                .frame(egui::Frame::new().fill(bg()).inner_margin(egui::Margin { left: 14, right: 14, top: 8, bottom: 0 }))
                .show_inside(ui, |ui| self.tabs(ui));
            if self.docs.is_empty() {
                ui.ctx().request_repaint();
                return;
            }
            let i = self.active.min(self.docs.len() - 1);
            // The open preset's own bar, right under its tab.
            egui::TopBottomPanel::top("studio-preset-bar")
                .show_separator_line(true)
                .frame(egui::Frame::new().fill(card_bg()).inner_margin(egui::Margin::symmetric(14, 7)))
                .show_inside(ui, |ui| self.studio_preset_bar(ui, i));
            // The chain takes the height it needs (scrolling past about 60% of the window)...
            let canvas_max = (ui.available_height() * 0.6).max(220.0);
            // The chain's own height, measured last frame.
            let h_id = egui::Id::new(("studio-canvas-h", i));
            let chain_h: f32 = ui.data(|d| d.get_temp(h_id)).unwrap_or(260.0);
            egui::TopBottomPanel::top("studio-canvas")
                .resizable(false)
                .exact_height(chain_h.min(canvas_max) + 44.0)
                .show_separator_line(true)
                .frame(egui::Frame::new().fill(bg()).inner_margin(egui::Margin { left: 18, right: 18, top: 6, bottom: 10 }))
                .show_inside(ui, |ui| {
                    ui.horizontal(|ui| {
                        if let Some(n) = editors::current_note(ui.ctx()) {
                            ui.label(RichText::new(n).color(ORANGE));
                            ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            theme::help(
                                ui,
                                "Click the line between two effects to split there or add an effect, FX loop, send or return.\nClick an effect to edit it; click its light or press Space to switch it on or off.\nEnter changes the selected effect, Delete removes it, the arrow keys pick the next one.\nDrag an effect to move it; right-click for more.",
                            );
                        });
                    });
                    let out = egui::ScrollArea::vertical().id_salt("studio-canvas").auto_shrink([false, false]).show(ui, |ui| {
                        self.editor(ui, i, 1);
                    });
                    let h = out.content_size.y;
                    if (h - chain_h).abs() > 1.0 {
                        ui.data_mut(|d| d.insert_temp(h_id, h));
                        ui.ctx().request_repaint();
                    }
                });
            // ...and the block (or controllers) panel gets all the rest.
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(panel()).inner_margin(egui::Margin::symmetric(14, 8)))
                .show_inside(ui, |ui| {
                    let tab_id = egui::Id::new("studio-dock-tab");
                    let mut tab: usize = ui.data(|d| d.get_temp(tab_id)).unwrap_or(0);
                    ui.horizontal(|ui| {
                        if let Some(t) = theme::segmented(ui, &["Block", "Controllers"], tab) {
                            tab = t;
                        }
                        theme::help(ui, if tab == 0 { "Block: the knobs of the effect picked in the chain." } else { "Controllers: what footswitches, expression pedals, snapshots and MIDI change in this preset." });
                    });
                    ui.data_mut(|d| d.insert_temp(tab_id, tab));
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical().id_salt(("studio-dock-scroll", tab)).auto_shrink([false, false]).show(ui, |ui| {
                        self.editor(ui, i, if tab == 0 { 2 } else { 3 });
                    });
                });
        });
    }
    /// The preset's own controls: its name and kind, save / save as / library, undo / redo,
    /// and the pedal actions (save on pedal, write back, send to a slot).
    /// mode 0: the classic look (two lines inside the page); 1: Studio top bar (name, undo,
    /// save); 2: Studio second row (library and pedal actions).
    fn doc_bar(&mut self, ui: &mut egui::Ui, i: usize, mode: u8) {
        if i >= self.docs.len() {
            return;
        }
        let connected = self.connected.is_some();
        let chosen = self.chosen;
        let chosen_name = chosen.and_then(|c| self.presets.get(c).cloned()).unwrap_or_default();
        let setlist = self.setlist;
        let backup = self.backup_dir();
        let mut cmds: Vec<Cmd> = Vec::new();
        let mut save = None;
        let mut confirm = None;
        let mut to_library = false;
        let mut history: Option<bool> = None; // Some(true) undo, Some(false) redo
        let cat = self.catalog.clone();
        let doc = &mut self.docs[i];
        let before = doc.kind.clone();

        let header = |ui: &mut egui::Ui, doc: &mut Doc, compact: bool| {
            let w = if compact { 190.0 } else { 320.0 };
            let font = if compact { egui::TextStyle::Body } else { egui::TextStyle::Heading };
            match &mut doc.kind {
                DocKind::Hlx { json } => {
                    let mut name = hlx::name(json);
                    if ui.add(egui::TextEdit::singleline(&mut name).font(font).desired_width(w)).changed() {
                        hlx::set_name(json, &name);
                        doc.title = name;
                        doc.dirty = true;
                    }
                    if !compact {
                        theme::pill(ui, "HLX file", BLUE);
                    }
                }
                DocKind::Device { origin, .. } => {
                    if ui
                        .add(egui::TextEdit::singleline(&mut doc.name).font(font).desired_width(w).char_limit(16))
                        .on_hover_text("The name it gets on the pedal when you write or save it")
                        .changed()
                    {
                        doc.dirty = true;
                    }
                    match origin {
                        Origin::Live { index, .. } => theme::pill(ui, &format!("LIVE · {}", slot_label((*index).max(0) as usize)), GOOD),
                        Origin::Slot { index, .. } => theme::pill(ui, &format!("from pedal · {}", slot_label((*index).max(0) as usize)), accent()),
                        Origin::File => theme::pill(ui, "pedal preset file", ORANGE),
                    }
                }
            }
            if doc.example && !compact {
                theme::pill(ui, "example", MUTED);
            }
        };
        let save_buttons = |ui: &mut egui::Ui, studio: bool, save: &mut Option<bool>| {
            if studio {
                if ui.button("Save as...").clicked() {
                    *save = Some(true);
                }
                let primary = egui::Button::new(RichText::new("Save").strong().color(Color32::from_rgb(8, 22, 26))).fill(accent());
                if ui.add(primary).on_hover_text("Save this preset (Ctrl+S)").clicked() {
                    *save = Some(false);
                }
            } else {
                if ui.button("💾 Save").clicked() {
                    *save = Some(false);
                }
                if ui.button("Save as...").clicked() {
                    *save = Some(true);
                }
            }
        };
        let undo_buttons = |ui: &mut egui::Ui, doc: &Doc, studio: bool, history: &mut Option<bool>| {
            let live_doc = doc.is_live();
            let undo_tip = if live_doc { "Not on the live preset: its changes are already on the pedal" } else { "Undo (Ctrl+Z)" };
            let (u, r) = if studio { ("⟲", "⟳") } else { ("Undo", "Redo") };
            if ui.add_enabled(!live_doc && !doc.undo.is_empty(), egui::Button::new(u)).on_hover_text(undo_tip).on_disabled_hover_text(undo_tip).clicked() {
                *history = Some(true);
            }
            if ui.add_enabled(!live_doc && !doc.redo.is_empty(), egui::Button::new(r)).on_hover_text("Redo (Ctrl+Y)").clicked() {
                *history = Some(false);
            }
            if !live_doc && history.is_none() {
                ui.input_mut(|inp| {
                    if inp.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::Z) || inp.consume_key(egui::Modifiers::COMMAND, egui::Key::Y) {
                        *history = Some(false);
                    } else if inp.consume_key(egui::Modifiers::COMMAND, egui::Key::Z) {
                        *history = Some(true);
                    }
                });
            }
        };
        let pedal_row = |ui: &mut egui::Ui, doc: &mut Doc, cmds: &mut Vec<Cmd>, confirm: &mut Option<(String, Cmd)>| match &doc.kind {
            DocKind::Device { preset, origin } => {
                let name = doc.name.clone();
                let mut clean = false;
                if let Origin::Live { setlist: sl, index } = origin {
                    let btn = egui::Button::new(RichText::new("Save on pedal").strong().color(Color32::from_rgb(8, 26, 24))).fill(accent());
                    if ui.add_enabled(connected, btn).on_hover_text("Keep the live changes in this preset slot").clicked() {
                        cmds.push(Cmd::SaveCurrent { setlist: *sl, index: *index, name: name.clone() });
                        clean = true;
                    }
                }
                if let Origin::Slot { setlist: sl, index } = origin {
                    if ui.add_enabled(connected && doc.dirty, egui::Button::new("Write back to its slot")).clicked() {
                        *confirm = Some((
                            format!("Replace {} on the pedal with your edited version?", slot_label((*index).max(0) as usize)),
                            Cmd::WriteSlot { setlist: *sl, index: *index, name: name.clone(), doc: preset.encode(), backup: backup.clone() },
                        ));
                        clean = true;
                    }
                }
                let label = chosen.map(|c| format!("Send to {}", slot_label(c))).unwrap_or_else(|| "Send to pedal...".into());
                let hover = if !connected {
                    "Connect to a pedal first"
                } else if chosen.is_none() {
                    "Pick a slot in the pedal list first"
                } else {
                    "Write this preset into the picked slot"
                };
                if ui.add_enabled(connected && chosen.is_some(), egui::Button::new(label)).on_hover_text(hover).on_disabled_hover_text(hover).clicked() {
                    if let Some(c) = chosen {
                        *confirm = Some((
                            format!("Write \"{}\" into {} (now \"{}\")?", name, slot_label(c), chosen_name),
                            Cmd::WriteSlot { setlist, index: c as i64, name, doc: preset.encode(), backup: backup.clone() },
                        ));
                    }
                }
                if clean {
                    doc.dirty = false;
                }
            }
            DocKind::Hlx { json } => {
                let label = chosen.map(|c| format!("Send to {}", slot_label(c))).unwrap_or_else(|| "Send to pedal...".into());
                let hover = if !connected {
                    "Connect to a pedal first"
                } else if chosen.is_none() {
                    "Pick a slot in the pedal list first"
                } else if cat.is_none() {
                    "Needs HX Edit's effect list (see Settings)"
                } else {
                    "Convert this preset to the pedal's format and write it into the picked slot (it's read back to check)"
                };
                if ui
                    .add_enabled(connected && chosen.is_some() && cat.is_some(), egui::Button::new(label))
                    .on_hover_text(hover)
                    .on_disabled_hover_text(hover)
                    .clicked()
                {
                    if let (Some(c), Some(catalog)) = (chosen, cat.clone()) {
                        let name = crate::hlx::name(json);
                        let warning = dsp_warning(&catalog, json).map(|w| format!("\n\n{w}")).unwrap_or_default();
                        *confirm = Some((
                            format!(
                                "Write \"{name}\" into {} (now \"{chosen_name}\")?\n\nIt is converted from .hlx using the pedal's loaded preset as the template. The slot's current preset is backed up first.{warning}",
                                slot_label(c)
                            ),
                            Cmd::WriteHlx { setlist, index: c as i64, name, json: json.clone(), cat: catalog, backup: backup.clone() },
                        ));
                    }
                }
            }
        };

        match mode {
            1 => {
                header(ui, doc, true);
                undo_buttons(ui, doc, true, &mut history);
                save_buttons(ui, true, &mut save);
            }
            2 => {
                if ui.button("📚 Save to library").on_hover_text("Keep a copy in your preset library on this computer").clicked() {
                    to_library = true;
                }
                pedal_row(ui, doc, &mut cmds, &mut confirm);
            }
            _ => {
                ui.horizontal(|ui| header(ui, doc, false));
                if let Some(p) = &doc.path {
                    ui.label(RichText::new(p.to_string_lossy()).small().color(MUTED));
                }
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    save_buttons(ui, false, &mut save);
                    if ui.button("📚 Save to library").on_hover_text("Keep a copy in your preset library on this computer").clicked() {
                        to_library = true;
                    }
                    ui.add(egui::Separator::default().vertical().spacing(10.0));
                    undo_buttons(ui, doc, false, &mut history);
                    ui.add(egui::Separator::default().vertical().spacing(10.0));
                    pedal_row(ui, doc, &mut cmds, &mut confirm);
                });
            }
        }
        if save.is_none() && mode != 2 && ui.input_mut(|inp| inp.consume_key(egui::Modifiers::COMMAND, egui::Key::S)) {
            save = Some(false);
        }
        if to_library {
            self.save_to_library(i);
        }
        for c in cmds {
            self.send(c);
        }
        if let Some(c) = confirm {
            self.confirm = Some(c);
        }
        self.track_history(i, before, history);
        if let Some(as_) = save {
            self.save_doc(i, as_);
        }
    }

    /// The preset's chain and knobs. part 0: everything; 1: the chain and the controllers;
    /// 2: only the selected block's panel (the Studio look docks it at the bottom).
    fn editor(&mut self, ui: &mut egui::Ui, i: usize, part: u8) {
        if part == 0 && !crate::chain::studio() {
            self.doc_bar(ui, i, 0);
            ui.add_space(10.0);
        }
        if i >= self.docs.len() {
            return;
        }
        let connected = self.connected.is_some();
        let mut cmds: Vec<Cmd> = Vec::new();
        let cat = self.catalog.clone();
        let picker = &mut self.picker;
        let doc = &mut self.docs[i];
        // What the preset was before this frame's edits, for undo.
        let before = doc.kind.clone();
        editors::set_part(part);
        match &mut doc.kind {
            DocKind::Hlx { json } => {
                if part != 3 {
                    editors::hlx_editor(ui, json, &mut doc.selected, &mut doc.dirty, cat.as_deref(), picker);
                }
                if part == 0 || part == 3 {
                    if part == 0 {
                        ui.add_space(10.0);
                    }
                    crate::controllers::panel(ui, json, cat.as_deref(), &mut doc.dirty);
                }
            }
            DocKind::Device { preset, origin } => {
                let live = matches!(origin, Origin::Live { .. }) && connected;
                if part != 3 {
                    editors::device_editor(ui, preset, &mut doc.selected, &mut doc.dirty, live, &mut cmds, cat.as_deref(), picker);
                }
                if part == 0 || part == 3 {
                    if part == 0 {
                        ui.add_space(10.0);
                    }
                    editors::device_controllers(ui, preset, live, &mut cmds, cat.as_deref(), &mut doc.dirty);
                }
            }
        }
        editors::set_part(0);
        for c in cmds {
            self.send(c);
        }
        self.track_history(i, before, None);
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        // What converting presets to the unit did, or why it could not.
        if let Some((title, lines, refusal)) = self.unit_report.clone() {
            let mut close = false;
            egui::Window::new(RichText::new(&title).strong().color(if refusal { BAD } else { TEXT }))
                .id(egui::Id::new("unit-report"))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.set_max_width(520.0);
                    for l in &lines {
                        ui.label(l);
                    }
                    ui.add_space(8.0);
                    if theme::primary_button(ui, "OK").clicked() {
                        close = true;
                    }
                });
            if close {
                self.unit_report = None;
            }
        }
        if let Some((sl, idx, mut name)) = self.rename.take() {
            let mut keep = true;
            let mut go = false;
            egui::Window::new("Rename preset")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(format!("New name for {}:", slot_label(idx)));
                    let r = ui.add(egui::TextEdit::singleline(&mut name).char_limit(16));
                    ui.label(RichText::new("Up to 16 characters").small().color(MUTED));
                    ui.horizontal(|ui| {
                        if theme::primary_button(ui, "Rename").clicked()
                            || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                        {
                            go = true;
                        }
                        if ui.button("Cancel").clicked() {
                            keep = false;
                        }
                    });
                });
            if go {
                self.send(Cmd::Rename { setlist: sl, index: idx as i64, name });
            } else if keep {
                self.rename = Some((sl, idx, name));
            }
        }
        if !self.settings.confirm_pedal {
            if let Some((_, cmd)) = self.confirm.take() {
                self.send(cmd);
            }
        }
        self.ir_plan_window(ctx);
        if let Some((text, cmd)) = self.confirm.take() {
            let mut result = None;
            egui::Window::new("Please confirm")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(&text);
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if theme::primary_button(ui, "Yes, go ahead").clicked() {
                            result = Some(true);
                        }
                        if ui.button("Cancel").clicked() {
                            result = Some(false);
                        }
                    });
                });
            match result {
                Some(true) => self.send(cmd),
                Some(false) => {}
                None => self.confirm = Some((text, cmd)),
            }
        }
    }

    fn screenshot_step(&mut self, ctx: &egui::Context) {
        let Some((path, frames, requested)) = &mut self.screenshot else { return };
        *frames += 1;
        ctx.request_repaint();
        // Only once the window really has a size: reading a zero-sized (or minimised)
        // window's pixels crashes the graphics layer.
        let drawable = ctx.input(|i| {
            let v = i.viewport();
            v.minimized != Some(true) && v.inner_rect.is_some_and(|r| r.width() >= 200.0 && r.height() >= 200.0)
        }) && ctx.screen_rect().width() >= 200.0;
        if *frames >= 120 && !*requested && drawable {
            *requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        let path = path.clone();
        let too_long = *frames > 900;
        let shot = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = shot {
            if let Err(e) = save_png(&path, &image) {
                eprintln!("could not save the screenshot: {e}");
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if too_long {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

fn level_colour(l: LogLevel, info: Color32) -> Color32 {
    match l {
        LogLevel::Info => info,
        LogLevel::Good => GOOD,
        LogLevel::Bad => BAD,
    }
}

fn save_png(path: &Path, image: &egui::ColorImage) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
    }
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let [w, h] = image.size;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().map_err(|e| e.to_string())?;
    let mut data = Vec::with_capacity(w * h * 4);
    for c in &image.pixels {
        let [r, g, b, _] = c.to_srgba_unmultiplied();
        data.extend_from_slice(&[r, g, b, 255]);
    }
    writer.write_image_data(&data).map_err(|e| e.to_string())
}

/// A warning when a .hlx preset's estimated DSP is over what the pedal has. It's only an
/// estimate, so the person can still send it.
fn dsp_warning(cat: &Catalog, json: &serde_json::Value) -> Option<String> {
    editors::hlx_dsp_use(cat, json).into_iter().find(|(_, used)| *used > 100.5).map(|(dsp, used)| {
        let n = dsp.trim_start_matches("dsp").parse::<usize>().unwrap_or(0) + 1;
        format!(
            "⚠ Path {n} is estimated at about {used:.0}% of the DSP. The estimate can be off; if the pedal can't run it, it will refuse the preset or leave blocks out. You can still try."
        )
    })
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Fill the monitor on start (asking at window creation isn't always honoured on Windows).
        if self.screenshot.is_none() {
            let pass = ctx.cumulative_pass_nr();
            if pass == 3 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
            }
            // Still at the small first size a moment later: size it to the monitor ourselves.
            if pass == 20 {
                let (monitor, maximized) = ctx.input(|i| (i.viewport().monitor_size, i.viewport().maximized));
                let inner = ctx.screen_rect().size() * ctx.zoom_factor();
                if let Some(m) = monitor {
                    if inner.x < m.x * 0.85 || inner.y < m.y * 0.75 {
                        if maximized == Some(true) {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(false));
                        }
                        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(0.0, 0.0)));
                        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(m.x, m.y - 48.0)));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
                    }
                }
            }
            if pass < 25 {
                ctx.request_repaint();
            }
        }
        self.guard_quit(ctx);
        self.poll(ctx);
        self.auto_pick();
        // The hidden Tests page.
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::T)) {
            self.view = if self.view == View::Tests { View::Editor } else { View::Tests };
        }
        if crate::chain::studio() {
            self.studio_top_bar(ctx);
        } else {
            self.top_bar(ctx);
        }
        self.status_bar(ctx);
        self.left_panel(ctx);
        self.central(ctx);
        self.dialogs(ctx);
        self.close_dialog(ctx);
        if let Some(cat) = self.catalog.clone() {
            if let Some((st, model, cab)) = editors::picker_window(ctx, &mut self.picker, &cat) {
                self.apply_pick(&cat, st, model, cab);
            }
        }
        self.screenshot_step(ctx);

    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Let a write in progress finish (the thread takes Disconnect after its current
        // command) rather than cutting it off when the program exits.
        if let Some(link) = &mut self.link {
            link.shut_down(Duration::from_secs(8));
        }
    }
}


/// One file waiting to go into an IR slot.
struct IrPlanRow {
    file: String,
    name: String,
    audio: Vec<u8>,
    slot: i64,
    nam: bool,
    notes: String,
}

struct IrPlan {
    rows: Vec<IrPlanRow>,
    problems: Vec<String>,
    /// The person says their pedal runs the NAM A2 Lite firmware (needed for captures).
    firmware_ok: bool,
}
/// A sidebar section: a card with a title in the classic look; in the Studio look a flat
/// section under a small capital heading (as in the mockup).
fn side_section<R>(ui: &mut egui::Ui, title: &str, studio_title: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    if crate::chain::studio() {
        ui.add_space(4.0);
        ui.label(RichText::new(studio_title).small().strong().color(MUTED));
        ui.add_space(4.0);
        let r = egui::Frame::new()
            .fill(card_bg())
            .corner_radius(CornerRadius::same(8))
            .stroke(Stroke::new(1.0_f32, line()))
            .inner_margin(egui::Margin::same(10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                add(ui)
            })
            .inner;
        ui.add_space(6.0);
        r
    } else {
        theme::card(ui, |ui| {
            theme::section(ui, title);
            add(ui)
        })
    }
}
