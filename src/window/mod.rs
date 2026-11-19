//! The window: connections on the left, the pedal's presets and the local
//! library below them, and open presets as tabs in the middle.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, CornerRadius, RichText, Stroke, Vec2};

use crate::boxes::{self, BoxInfo, PairPoll, RelayStatus, Settings};
use crate::catalog::Catalog;
use crate::device::{self, Cmd, Event, Link, Purpose, Target};
use crate::devpreset::DevicePreset;
use crate::editors;
use crate::hlx;
use crate::library::{self, Library};
use crate::proto::slot_label;
use crate::theme::{self, *};
use crate::wire::{self, UsbPedal};
use std::sync::Arc;

mod backups;
mod bars;
mod compare_window;
mod connection;
mod irs;
mod library_page;
mod refresh;
mod restore_all;
mod settings;
mod tests_page;

// ----------------------------------------------------------------- documents ---

#[derive(Clone)]
enum DocKind {
    /// A JSON preset file on this computer.
    Hlx { json: serde_json::Value },
    /// The pedal's own document: from a slot, the loaded preset, or a file.
    Device { preset: DevicePreset, origin: Origin },
}

impl DocKind {
    /// The open preset as the trait both kinds share (saving, tempo, path meters).
    fn doc(&self) -> &dyn editors::PresetDoc {
        match self {
            DocKind::Hlx { json } => json,
            DocKind::Device { preset, .. } => preset,
        }
    }

    fn doc_mut(&mut self) -> &mut dyn editors::PresetDoc {
        match self {
            DocKind::Hlx { json } => json,
            DocKind::Device { preset, .. } => preset,
        }
    }
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
    box_tx: std::sync::mpsc::Sender<BoxInfo>,
    box_lookup_at: Option<Instant>,
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
    /// Where the preset list on the left was drawn last frame (F2 renames what the pointer is over).
    left_rect: egui::Rect,
    /// The window as last seen, to keep for next time: (x, y, width, height), maximized.
    window_seen: Option<([f32; 4], bool)>,
    /// Problems logged since the Activity log was last opened (marks the button red).
    errors_unseen: usize,
    /// The keyboard shortcuts window is open (F1).
    show_shortcuts: bool,
    /// A unit connected for the first time: shows what has been checked on it.
    unit_notice: Option<i64>,
    /// A preset dropped on another slot, waiting for "copy, swap or move" (from, to).
    slot_drop: Option<(usize, usize)>,
    /// When a live change was last sent to the pedal (shows "sent" for a moment).
    last_live_send: Option<Instant>,
    /// Live copies of the preset still due from the pedal (one per edit sent that gets one back), and when the
    /// last such edit was sent; only the newest copy is shown.
    refresh: refresh::RefreshGuard,
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
    /// The "Restore from a backup" window, when open.
    restore: Option<backups::RestoreState>,
    restore_all: Option<restore_all::RestoreAllState>,
    compare: Option<compare_window::CompareState>,
    /// What the current (or last) link was made to: a USB pedal or a ToneStrand Box.
    last_target: Option<Target>,
    /// A link that dropped and is being made again: tries so far and when the next one is due.
    reconnect: Option<connection::Reconnect>,
    /// The person pressed Disconnect: never reconnect by itself after that.
    user_disconnected: bool,
    /// "Not now" was clicked on the card that explains HX Edit is missing.
    catalog_card_hidden: bool,
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
        // TONESTRAND_THEME=light|dark|system overrides the setting for one run (for pictures).
        let theme_choice = std::env::var("TONESTRAND_THEME").unwrap_or_else(|_| settings.theme.clone());
        theme::set_light(theme::wants_light(&theme_choice, &cc.egui_ctx));
        theme::apply(&cc.egui_ctx);
        let (tx, box_rx) = channel();
        let tx_local = tx.clone();
        let mdns = if settings.discover_boxes { boxes::start_discovery(tx) } else { None };
        cc.egui_ctx.set_zoom_factor(settings.ui_scale.clamp(0.75, 1.5));
        crate::realnames::set(settings.real_names || std::env::var("TONESTRAND_REAL_NAMES").is_ok_and(|v| v == "1"));
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
            box_tx: tx_local.clone(),
            box_lookup_at: None,
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
            left_rect: egui::Rect::NOTHING,
            window_seen: None,
            errors_unseen: 0,
            show_shortcuts: false,
            unit_notice: None,
            slot_drop: None,
            last_live_send: None,
            refresh: refresh::RefreshGuard::default(),
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
            restore: None,
            restore_all: None,
            compare: None,
            last_target: None,
            reconnect: None,
            user_disconnected: false,
            catalog_card_hidden: false,
        };
        app.catalog = crate::catalog::load_best(app.settings.hx_edit.as_deref()).map(Arc::new);
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

        // The sample preset is for the first start; after that only if it is asked for again
        // in Settings. (Pictures of the pages (--screenshot) always have it, and change nothing.)
        let open_sample = app.settings.show_sample || app.screenshot.is_some();
        if !app.settings.sample_seen && app.screenshot.is_none() {
            app.settings.sample_seen = true;
            app.settings.show_sample = false;
            app.settings.save();
        }
        if open_sample {
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
                    // TONESTRAND_SELECT=n picks the n-th place of the chain (0 is the input) for a picture.
                    if let Some(n) = std::env::var("TONESTRAND_SELECT").ok().and_then(|v| v.parse::<usize>().ok()) {
                        if let Some(d) = app.docs.last_mut() {
                            d.selected = Some(n);
                        }
                    }
                }
            }
            Ok("library") => app.view = View::Library,
            _ => {}
        }
        app.scan_library();
        if app.settings.reopen_tabs && app.screenshot.is_none() {
            for p in app.settings.last_tabs.clone() {
                let p = PathBuf::from(p);
                if p.is_file() {
                    app.open_file(&p);
                }
            }
        }
        app.note(LogLevel::Info, "Ready. Plug in a pedal or pick a ToneStrand Box to start.");
        if let Some(n) = settings_note {
            app.note(LogLevel::Bad, n);
        }
        app
    }

    fn note(&mut self, level: LogLevel, text: impl Into<String>) {
        if level == LogLevel::Bad {
            self.errors_unseen += 1;
        }
        self.log.push_back((level, text.into()));
        while self.log.len() > 200 {
            self.log.pop_front();
        }
    }

    fn send(&mut self, cmd: Cmd) {
        if let Some(l) = &self.link {
            if matches!(
                cmd,
                Cmd::SetParam { .. }
                    | Cmd::SetEnabled { .. }
                    | Cmd::SetModel { .. }
                    | Cmd::AddBlock { .. }
                    | Cmd::ReplaceBlock { .. }
                    | Cmd::ClearBlock { .. }
                    | Cmd::MoveBlock { .. }
                    | Cmd::Snapshot(_)
            ) {
                self.last_live_send = Some(Instant::now());
            }
            if cmd.refreshes_live() {
                self.refresh.sent(Instant::now());
            }
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
            match std::fs::read_to_string(path).map_err(|e| e.to_string()).and_then(|s| crate::hlx::parse_text(&s)) {
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
        self.docs.push(Doc {
            title,
            name,
            path: Some(path.to_owned()),
            kind,
            dirty: false,
            selected: None,
            example: false,
            undo: Vec::new(),
            redo: Vec::new(),
            last_edit: None,
        });
        self.active = self.docs.len() - 1;
        self.remember_recent(path);
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

    /// Put a file at the top of the recent list (at most 10, no repeats).
    fn remember_recent(&mut self, path: &Path) {
        if self.screenshot.is_some() {
            return;
        }
        let p = path.to_string_lossy().into_owned();
        self.settings.recent.retain(|r| *r != p);
        self.settings.recent.insert(0, p);
        self.settings.recent.truncate(10);
        self.settings.save();
    }

    /// The saved preset files that are open now, for "reopen last session's tabs". Live and
    /// pedal-slot documents have no file and are left out.
    fn open_file_paths(&self) -> Vec<String> {
        self.docs.iter().filter_map(|d| d.path.as_ref()).map(|p| p.to_string_lossy().into_owned()).collect()
    }

    fn save_doc(&mut self, i: usize, save_as: bool) {
        let lib = self.library_dir.clone();
        let Some(doc) = self.docs.get_mut(i) else { return };
        let (ext, bytes) = (doc.kind.doc().file_ext(), doc.kind.doc().encode());
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

    fn tabs(&mut self, ui: &mut egui::Ui) {
        let mut close = None;
        let middle = ui.input(|i| i.pointer.button_clicked(egui::PointerButton::Middle));
        let font = egui::TextStyle::Body.resolve(ui.style());
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(3.0, 3.0);
            for (i, d) in self.docs.iter().enumerate() {
                let active = i == self.active;
                let galley = ui.painter().layout_no_wrap(d.title.clone(), font.clone(), ink());
                let dot = if d.dirty { 12.0 } else { 0.0 };
                let (rect, resp) =
                    ui.allocate_exact_size(Vec2::new(10.0 + dot + galley.size().x + 6.0 + 16.0 + 5.0, 26.0), egui::Sense::click());
                // The close cross sits at the right end; it is its own target.
                let x_rect = egui::Rect::from_center_size(egui::pos2(rect.right() - 5.0 - 8.0, rect.center().y), Vec2::splat(16.0));
                let x_resp = ui.interact(x_rect, resp.id.with("close"), egui::Sense::click());
                crate::theme::focus_ring(ui, &resp, 7);
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &d.title));
                x_resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Close {}", d.title)));
                let hot = resp.hovered() || x_resp.hovered();
                let p = ui.painter();
                let fill = if active {
                    card_hi()
                } else if hot {
                    card_bg()
                } else {
                    Color32::TRANSPARENT
                };
                p.rect_filled(rect, CornerRadius::same(7), fill);
                if active {
                    // A thin accent underline, inside the pill's lower edge.
                    let under = egui::Rect::from_min_max(
                        egui::pos2(rect.left() + 8.0, rect.bottom() - 2.0),
                        egui::pos2(rect.right() - 8.0, rect.bottom() - 1.0),
                    );
                    p.rect_filled(under, CornerRadius::same(1), accent());
                }
                let mut x = rect.left() + 10.0;
                if d.dirty {
                    // Unsaved changes: a small painted dot.
                    p.circle_filled(egui::pos2(x + 3.0, rect.center().y), 3.0, orange());
                    x += dot;
                }
                let col = if active {
                    accent()
                } else if hot {
                    ink()
                } else {
                    muted()
                };
                p.galley(egui::pos2(x, rect.center().y - galley.size().y / 2.0), galley, col);
                if active || hot {
                    if x_resp.hovered() {
                        p.rect_filled(x_rect, CornerRadius::same(4), line());
                    }
                    let c = if x_resp.hovered() { ink() } else { muted() };
                    let m = x_rect.center();
                    let s = Stroke::new(1.4_f32, c);
                    p.line_segment([m + Vec2::new(-3.0, -3.0), m + Vec2::new(3.0, 3.0)], s);
                    p.line_segment([m + Vec2::new(-3.0, 3.0), m + Vec2::new(3.0, -3.0)], s);
                }
                let x_resp = x_resp.on_hover_text("Close");
                if x_resp.clicked() {
                    close = Some(i);
                } else if resp.clicked() && !x_resp.hovered() {
                    self.active = i;
                }
                // A middle click anywhere on the tab closes it.
                if middle && hot {
                    close = Some(i);
                }
            }
        });
        if let Some(i) = close {
            self.request_close_tab(i);
        }
    }

    /// Window-wide keys: Ctrl+W closes the tab, Ctrl+O opens a file, Ctrl+N starts a new
    /// preset, Ctrl+Tab / Ctrl+Shift+Tab move between tabs. (Ctrl+S, Ctrl+Z and Ctrl+Y are
    /// handled with the preset's bar.) They are left alone while a text field has the keyboard
    /// or a question is showing.
    fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.wants_keyboard_input() || self.close_ask.is_some() || self.confirm.is_some() || self.unit_report.is_some() {
            return;
        }
        let ctrl = egui::Modifiers::COMMAND;
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F1)) {
            self.show_shortcuts = !self.show_shortcuts;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F2)) {
            self.rename_here(ctx);
        }
        let (close, open, new, prev, next) = ctx.input_mut(|i| {
            (
                i.consume_key(ctrl, egui::Key::W),
                i.consume_key(ctrl, egui::Key::O),
                i.consume_key(ctrl, egui::Key::N),
                // The shifted one first: Ctrl+Tab alone must not match it.
                i.consume_key(ctrl | egui::Modifiers::SHIFT, egui::Key::Tab),
                i.consume_key(ctrl, egui::Key::Tab),
            )
        });
        if open {
            self.view = View::Editor;
            self.open_file_dialog();
        }
        if new {
            self.view = View::Editor;
            self.new_preset();
        }
        if self.view != View::Editor || self.docs.is_empty() {
            return;
        }
        let n = self.docs.len();
        let here = self.active.min(n - 1);
        if close {
            self.request_close_tab(here);
        } else if prev {
            self.active = (here + n - 1) % n;
        } else if next {
            self.active = (here + 1) % n;
        }
    }

    /// The list of keyboard shortcuts (F1, or the Shortcuts button in the status bar).
    fn shortcuts_window(&mut self, ctx: &egui::Context) {
        if !self.show_shortcuts {
            return;
        }
        let mut open = true;
        egui::Window::new("Keyboard shortcuts")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                let sections: [(&str, &[(&str, &str)]); 4] = [
                    (
                        "Presets",
                        &[
                            ("Ctrl+O", "Open a preset file"),
                            ("Ctrl+N", "New preset"),
                            ("Ctrl+S", "Save"),
                            ("Ctrl+W", "Close the tab"),
                            ("Ctrl+Tab / Ctrl+Shift+Tab", "Next / previous tab"),
                            ("Ctrl+Z / Ctrl+Y", "Undo / redo (not on the live preset)"),
                            ("F2", "Rename: the preset, or the pedal preset or IR you point at"),
                        ],
                    ),
                    (
                        "The chain (click a block first)",
                        &[
                            ("Arrow keys", "Pick the next block"),
                            ("Space", "Switch the block on or off"),
                            ("Enter", "Change the effect"),
                            ("Delete", "Remove the block"),
                        ],
                    ),
                    (
                        "Only the keyboard",
                        &[
                            ("Tab / Shift+Tab", "Move to the next / previous control; a ring shows where you are"),
                            ("Enter or Space", "Press the control that has the ring (a block, a button, a trigger)"),
                            ("Knobs: arrows", "Change the value (Shift = fine); Page Up / Down big steps"),
                            ("Knobs: Home / End", "Lowest / highest value; Delete or Backspace for the default"),
                        ],
                    ),
                    ("Anywhere", &[("F1", "This list"), ("Ctrl+Shift+T", "The hidden Tests page")]),
                ];
                for (title, rows) in sections {
                    ui.label(RichText::new(title).small().strong().color(muted()));
                    egui::Grid::new(("shortcuts", title)).num_columns(2).min_col_width(210.0).spacing([18.0, 4.0]).show(ui, |ui| {
                        for (keys, what) in rows {
                            ui.label(RichText::new(*keys).monospace().color(accent()));
                            ui.label(*what);
                            ui.end_row();
                        }
                    });
                    ui.add_space(8.0);
                }
            });
        if !open {
            self.show_shortcuts = false;
        }
    }

    /// F2: rename what is at hand. On the IRs page, the one IR picked on the pedal; in the
    /// editor, the chosen pedal preset when the pointer is over the preset list, else the
    /// open preset (its name box gets the focus, with the name selected).
    fn rename_here(&mut self, ctx: &egui::Context) {
        match self.view {
            View::Irs if !self.irs_local && self.ir_sel.len() == 1 => {
                let slot = *self.ir_sel.iter().next().expect("one picked");
                if let Some(name) = self.irs.as_ref().and_then(|l| l.iter().find(|(s, _)| *s == slot)).map(|(_, n)| n.clone()) {
                    self.ir_rename = Some((slot, name));
                }
            }
            View::Editor => {
                let over_list = ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| self.left_rect.contains(p));
                let slot = self.chosen.filter(|_| self.connected.is_some() && over_list);
                if let Some(i) = slot {
                    self.rename = Some((self.setlist, i, self.presets.get(i).cloned().unwrap_or_default()));
                } else if !self.docs.is_empty() {
                    ctx.data_mut(|d| d.insert_temp(egui::Id::new("focus-doc-name"), true));
                }
            }
            _ => {}
        }
    }

    /// Files dropped onto the window: presets open as tabs, backups and setlists go into the
    /// preset library, IRs and captures into the IR library. While files hover, a hint shows.
    fn drop_files(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            let screen = ctx.screen_rect();
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("drop-hint")));
            painter.rect_filled(screen, CornerRadius::ZERO, Color32::from_black_alpha(215));
            let inner = screen.shrink(24.0);
            painter.rect_stroke(inner, CornerRadius::same(12), Stroke::new(2.0_f32, accent()), egui::StrokeKind::Inside);
            painter.text(
                inner.center() - Vec2::new(0.0, 14.0),
                egui::Align2::CENTER_CENTER,
                "Drop files here",
                egui::FontId::proportional(26.0),
                ink(),
            );
            painter.text(
                inner.center() + Vec2::new(0.0, 18.0),
                egui::Align2::CENTER_CENTER,
                "Presets open to edit; backups and setlists go to the library; IRs and captures go to the IR library",
                egui::FontId::proportional(14.0),
                muted(),
            );
        }
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        if dropped.is_empty() {
            return;
        }
        let mut presets = Vec::new();
        let mut backups = Vec::new();
        let mut tones = Vec::new();
        let mut skipped = Vec::new();
        for p in dropped {
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            match ext.as_str() {
                "hlx" | "hxpreset" => presets.push(p),
                "hxb" | "hls" => backups.push(p),
                "wav" | "nam" => tones.push(p),
                _ => skipped.push(p.file_name().and_then(|s| s.to_str()).unwrap_or("?").to_owned()),
            }
        }
        if !presets.is_empty() {
            self.view = View::Editor;
            for p in presets {
                self.open_file(&p);
            }
        }
        if !backups.is_empty() {
            self.import_files(backups);
        }
        if !tones.is_empty() {
            self.scan_tones();
            let dir = self.tones.folder();
            let _ = std::fs::create_dir_all(&dir);
            let n = tones.iter().filter(|f| crate::files::copy_into(f, &dir).is_ok()).count();
            let failed = tones.len() - n;
            if failed == 0 {
                self.note(LogLevel::Good, format!("Imported {n} file{} into the IR library", if n == 1 { "" } else { "s" }));
            } else {
                self.note(LogLevel::Bad, format!("Imported {n} of {} files into the IR library", tones.len()));
            }
            self.scan_tones();
        }
        if !skipped.is_empty() {
            self.note(LogLevel::Bad, format!("Not a preset, backup, IR or capture: {}", skipped.join(", ")));
        }
    }

    /// Close a tab, asking first when it has changes that aren't saved.
    fn request_close_tab(&mut self, i: usize) {
        if self.docs.get(i).is_some_and(Doc::needs_save) {
            self.close_ask = Some(CloseAsk::Tab(i));
        } else {
            self.close_tab(i);
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
        egui::Window::new(title).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            ui.label(&text);
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let save = if matches!(ask, CloseAsk::Quit) && unsaved.len() > 1 { "Save all" } else { "Save" };
                if theme::primary_button(ui, save).clicked() {
                    choice = Some(0);
                }
                let discard = if matches!(ask, CloseAsk::Quit) { "Quit without saving" } else { "Close without saving" };
                if ui.button(RichText::new(discard).color(bad())).clicked() {
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

    /// Choose the effects list again: HX Edit's files (custom folder or the usual place) when
    /// found and not older than the built-in list, else the list built into ToneStrand.
    fn reload_catalog(&mut self) {
        self.catalog = crate::catalog::load_best(self.settings.hx_edit.as_deref()).map(Arc::new);
        self.scan_library();
        let chosen = self.settings.hx_edit.clone();
        match self.catalog.as_ref().map(|c| c.source().clone()) {
            Some(crate::catalog::CatalogSource::HxEdit(dir)) => {
                self.note(LogLevel::Good, format!("Using HX Edit's effects list ({}).", dir.display()));
            }
            Some(crate::catalog::CatalogSource::Builtin { hx_edit_version }) => {
                let msg = match builtin_reason(chosen.as_deref()) {
                    BuiltinWhy::FolderHasNoFiles => {
                        format!("That folder has no HX Edit files; using the list built into ToneStrand (HX Edit {hx_edit_version}).")
                    }
                    BuiltinWhy::FolderOlder => format!(
                        "The list built into ToneStrand (HX Edit {hx_edit_version}) is newer than the one in that folder; using it."
                    ),
                    BuiltinWhy::NotInstalled => {
                        format!("HX Edit not found; using the list built into ToneStrand (HX Edit {hx_edit_version}).")
                    }
                    BuiltinWhy::InstalledOlder => {
                        format!("The list built into ToneStrand (HX Edit {hx_edit_version}) is newer than HX Edit's; using it.")
                    }
                };
                self.note(LogLevel::Good, msg);
            }
            None => self.note(LogLevel::Bad, "No effects list found."),
        }
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
                if let Some(key) = hlx::place_model(
                    json,
                    &dsp,
                    st.path,
                    st.position as i64,
                    st.hlx_key.as_deref(),
                    &hlx::NewModel { model: &hlx_model, stereo, params: &params, cab: cab_ref },
                ) {
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

        self.catalog_card(ctx);
        self.getting_started_card(ctx);
        if !self.docs.is_empty() {
            return self.studio_central(ctx);
        }
        egui::CentralPanel::default().frame(egui::Frame::new().fill(bg()).inner_margin(egui::Margin::same(14))).show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(80.0);
                if let Some(t) = theme::logo(ui.ctx(), theme::Logo::WithName) {
                    ui.add(egui::Image::new(&t).fit_to_exact_size(Vec2::new(360.0, 145.0)).tint(Color32::from_white_alpha(150)));
                    ui.add_space(16.0);
                }
                ui.label(RichText::new("No preset open").size(22.0).strong());
                ui.label(RichText::new("Open a file, start a new preset, or edit one from the pedal.").color(muted()));
            });
        });
    }

    /// A short "Getting started" card on the editor page until the person has connected a pedal
    /// once (or dismisses it): three steps, with what the program can already see.
    fn getting_started_card(&mut self, ctx: &egui::Context) {
        if self.settings.hide_getting_started || self.connected.is_some() || self.screenshot.is_some() {
            return;
        }
        let mut hide = false;
        let found_usb = self.usb.first().map(|p| p.product.clone());
        let found_box = self.boxes.first().map(|b| if b.name.is_empty() { b.host.clone() } else { b.name.clone() });
        egui::TopBottomPanel::top("getting-started")
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(bg()).inner_margin(egui::Margin { left: 14, right: 14, top: 10, bottom: 0 }))
            .show(ctx, |ui| {
                theme::card(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new("Getting started").size(16.0).strong());
                    let step = |ui: &mut egui::Ui, n: &str, text: &str, note: Option<(String, Color32)>| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(n).monospace().color(accent()));
                            ui.label(text);
                            if let Some((t, c)) = note {
                                theme::pill(ui, &t, c);
                            }
                        });
                    };
                    let hx =
                        if self.catalog.is_some() { ("HX Edit found".to_owned(), good()) } else { ("not found yet".to_owned(), orange()) };
                    step(ui, "1", "Install Line 6's HX Edit on this computer (ToneStrand reads the effect names from it).", Some(hx));
                    let seen = match (found_usb, found_box) {
                        (Some(p), _) => Some((format!("{p} on USB"), good())),
                        (None, Some(b)) => Some((format!("{b} on the network"), good())),
                        _ => None,
                    };
                    step(
                        ui,
                        "2",
                        "Plug the pedal into a USB port, or switch on a ToneStrand Box and pair it (Pair, then press BOOT on the box).",
                        seen,
                    );
                    step(ui, "3", "Click the pedal button at the top (\"not connected\"), then Connect.", None);
                    ui.add_space(2.0);
                    ui.horizontal(|ui| {
                        if ui.button("Got it").on_hover_text("Hide this card for good").clicked() {
                            hide = true;
                        }
                        ui.label(
                            RichText::new("You can also work on preset files without a pedal: New preset or Open.").small().color(muted()),
                        );
                    });
                });
            });
        if hide {
            self.settings.hide_getting_started = true;
            self.settings.save();
        }
    }

    /// Without HX Edit's files there are no effect names, ranges or pictures: say so on the
    /// editor page, with a way to point at its folder.
    fn catalog_card(&mut self, ctx: &egui::Context) {
        if self.catalog.is_some() || self.catalog_card_hidden {
            return;
        }
        let mut choose = false;
        let mut again = false;
        let mut hide = false;
        egui::TopBottomPanel::top("no-catalog")
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(bg()).inner_margin(egui::Margin { left: 14, right: 14, top: 10, bottom: 0 }))
            .show(ctx, |ui| {
                theme::card(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new("HX Edit was not found").size(16.0).strong().color(orange()));
                    ui.label(
                        RichText::new(
                            "ToneStrand takes effect names, knob names and ranges from the HX Edit program on this computer. \
                             Without it most of the editor is empty. Install HX Edit, or choose the folder it is in.",
                        )
                        .color(muted()),
                    );
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        if theme::primary_button(ui, "Choose HX Edit folder...").clicked() {
                            choose = true;
                        }
                        if ui.button("Look again").on_hover_text("Search the usual places again").clicked() {
                            again = true;
                        }
                        ui.hyperlink_to("Get HX Edit from Line 6", "https://line6.com/software/");
                        theme::help(ui, "The folder is the one with HX Edit's \"res\" folder inside it (on Windows, usually C:\\Program Files (x86)\\Line6\\HX Edit). You can change this later in Settings.");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("Not now").clicked() {
                                hide = true;
                            }
                        });
                    });
                });
            });
        if choose {
            if let Some(d) = rfd::FileDialog::new().pick_folder() {
                self.settings.hx_edit = Some(d.to_string_lossy().into_owned());
                self.settings.save();
                self.reload_catalog();
            }
        }
        if again {
            self.reload_catalog();
        }
        if hide {
            self.catalog_card_hidden = true;
        }
    }

    /// The editor: the open presets' tabs, the chain on a large canvas with
    /// the controllers under it, and the selected block's panel docked at the bottom.
    fn studio_central(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().frame(egui::Frame::new().fill(bg()).inner_margin(egui::Margin::ZERO)).show(ctx, |ui| {
            egui::TopBottomPanel::top("studio-tabs")
                .show_separator_line(false)
                .frame(egui::Frame::new().fill(bg()).inner_margin(egui::Margin { left: 14, right: 14, top: 6, bottom: 4 }))
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
                            ui.label(RichText::new(n).color(orange()));
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
                    // TONESTRAND_DOCK_TAB=1 starts on Controllers (for pictures).
                    let mut tab: usize =
                        ui.data(|d| d.get_temp(tab_id)).unwrap_or_else(|| std::env::var("TONESTRAND_DOCK_TAB").ok().and_then(|v| v.parse().ok()).unwrap_or(0));
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
    /// mode 1: the preset bar's first row (name, undo,
    /// save); 2: its second row (library and pedal actions).
    fn doc_bar(&mut self, ui: &mut egui::Ui, i: usize, mode: u8) {
        if i >= self.docs.len() {
            return;
        }
        let connected = self.connected.is_some();
        let chosen = self.chosen;
        let chosen_name = chosen.and_then(|c| self.presets.get(c).cloned()).unwrap_or_default();
        let setlist = self.setlist;
        let backup = self.backup_dir();
        let sent_recently = self.last_live_send.is_some_and(|t| t.elapsed() < Duration::from_millis(1500));
        let mut cmds: Vec<Cmd> = Vec::new();
        let mut save = None;
        let mut confirm = None;
        let mut to_library = false;
        let mut convert: Option<Option<i64>> = None; // Some(None) an .hlx copy, Some(Some(unit)) one made for that unit
        let unit_now = self.unit;
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
                    let r = ui.add(egui::TextEdit::singleline(&mut name).font(font).desired_width(w));
                    focus_if_asked(ui, &r, name.chars().count());
                    if r.has_focus() {
                        theme::name_hint(ui, &name, crate::session::PRESET_NAME_MAX);
                    }
                    if r.changed() {
                        hlx::set_name(json, &name);
                        doc.title = name;
                        doc.dirty = true;
                    }
                    if !compact {
                        theme::pill(ui, "HLX file", blue());
                    }
                }
                DocKind::Device { origin, .. } => {
                    let r = ui
                        .add(
                            egui::TextEdit::singleline(&mut doc.name)
                                .font(font)
                                .desired_width(w)
                                .char_limit(crate::session::PRESET_NAME_MAX),
                        )
                        .on_hover_text("The name it gets on the pedal when you write or save it");
                    focus_if_asked(ui, &r, doc.name.chars().count());
                    if r.has_focus() {
                        theme::name_hint(ui, &doc.name, crate::session::PRESET_NAME_MAX);
                    }
                    if r.changed() {
                        doc.dirty = true;
                    }
                    match origin {
                        Origin::Live { index, .. } => theme::pill(ui, &format!("LIVE · {}", slot_label((*index).max(0) as usize)), good()),
                        Origin::Slot { index, .. } => {
                            theme::pill(ui, &format!("from pedal · {}", slot_label((*index).max(0) as usize)), accent())
                        }
                        Origin::File => theme::pill(ui, "pedal preset file", orange()),
                    }
                }
            }
            if doc.example && !compact {
                theme::pill(ui, "example", muted());
            }
        };
        let save_buttons = |ui: &mut egui::Ui, studio: bool, save: &mut Option<bool>| {
            if studio {
                if ui.button("Save as...").clicked() {
                    *save = Some(true);
                }
                let primary = egui::Button::new(RichText::new("Save").strong().color(on_accent())).fill(accent_fill());
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
        // A pedal preset can become a .hlx (and from there be made for another unit).
        let convert_menu = |ui: &mut egui::Ui, doc: &Doc, convert: &mut Option<Option<i64>>| {
            if !matches!(doc.kind, DocKind::Device { .. }) {
                return;
            }
            ui.menu_button("Make .hlx", |ui| {
                if ui
                    .button("A .hlx copy of this preset")
                    .on_hover_text("A new tab with this preset as a .hlx file, for the unit it came from")
                    .clicked()
                {
                    *convert = Some(None);
                    ui.close_menu();
                }
                let name = crate::units::unit_name(unit_now);
                if ui
                    .button(format!("A .hlx made for the {name}"))
                    .on_hover_text("A new tab with this preset fitted to the unit set at the top, with a list of what had to change")
                    .clicked()
                {
                    *convert = Some(Some(unit_now));
                    ui.close_menu();
                }
            });
        };
        let undo_buttons = |ui: &mut egui::Ui, doc: &Doc, studio: bool, history: &mut Option<bool>| {
            let live_doc = doc.is_live();
            let undo_tip = if live_doc { "Not on the live preset: its changes are already on the pedal" } else { "Undo (Ctrl+Z)" };
            let (u, r) = if studio { ("⟲", "⟳") } else { ("Undo", "Redo") };
            if ui
                .add_enabled(!live_doc && !doc.undo.is_empty(), egui::Button::new(u))
                .on_hover_text(undo_tip)
                .on_disabled_hover_text(undo_tip)
                .clicked()
            {
                *history = Some(true);
            }
            if ui.add_enabled(!live_doc && !doc.redo.is_empty(), egui::Button::new(r)).on_hover_text("Redo (Ctrl+Y)").clicked() {
                *history = Some(false);
            }
            if !live_doc && history.is_none() {
                ui.input_mut(|inp| {
                    if inp.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::Z)
                        || inp.consume_key(egui::Modifiers::COMMAND, egui::Key::Y)
                    {
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
                    let btn = egui::Button::new(RichText::new("Save on pedal").strong().color(on_accent())).fill(accent_fill());
                    if ui.add_enabled(connected, btn).on_hover_text("Keep the live changes in this preset slot").clicked() {
                        cmds.push(Cmd::SaveCurrent { setlist: *sl, index: *index, name: name.clone() });
                        clean = true;
                    }
                    // Live edits have no undo: this goes back to what the pedal has stored.
                    if ui
                        .add_enabled(connected, egui::Button::new("Revert"))
                        .on_hover_text("Reload this preset from the pedal's memory and drop the live changes you haven't saved")
                        .clicked()
                    {
                        *confirm = Some((
                            format!(
                                "Reload {} from the pedal's memory? Live changes you haven't saved on the pedal are lost.",
                                slot_label((*index).max(0) as usize)
                            ),
                            Cmd::Select(*sl, *index),
                        ));
                    }
                }
                if let Origin::Slot { setlist: sl, index } = origin {
                    if ui.add_enabled(connected && doc.dirty, egui::Button::new("Write back to its slot")).clicked() {
                        *confirm = Some((
                            format!("Replace {} on the pedal with your edited version?", slot_label((*index).max(0) as usize)),
                            Cmd::WriteSlot {
                                setlist: *sl,
                                index: *index,
                                name: name.clone(),
                                doc: preset.encode(),
                                backup: backup.clone(),
                            },
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
                if ui
                    .add_enabled(connected && chosen.is_some(), egui::Button::new(label))
                    .on_hover_text(hover)
                    .on_disabled_hover_text(hover)
                    .clicked()
                {
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
                let other_unit = crate::snapshots::device(json) != crate::nam::unit();
                let hover = if !connected {
                    "Connect to a pedal first"
                } else if other_unit {
                    "This preset is for another unit: reopen it so it is converted to the connected pedal (or pick that unit at the top)"
                } else if chosen.is_none() {
                    "Pick a slot in the pedal list first"
                } else if cat.is_none() {
                    "Needs HX Edit's effect list (see Settings)"
                } else {
                    "Convert this preset to the pedal's format and write it into the picked slot (it's read back to check)"
                };
                if ui
                    .add_enabled(connected && !other_unit && chosen.is_some() && cat.is_some(), egui::Button::new(label))
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

        let live = doc.is_live();
        match mode {
            1 => {
                header(ui, doc, true);
                if live && sent_recently {
                    // A live change just went to the pedal (a failure shows in red in the status bar).
                    ui.label(RichText::new("sent").small().color(good()));
                    ui.ctx().request_repaint_after(Duration::from_millis(200));
                }
                undo_buttons(ui, doc, true, &mut history);
                convert_menu(ui, doc, &mut convert);
                if live {
                    // The pedal is where a live preset is kept ("Save on pedal"); the file and
                    // library copies are quieter.
                    ui.menu_button("File", |ui| {
                        if ui.button("Save as a file...").on_hover_text("Keep a copy of this preset as a file on this computer").clicked() {
                            save = Some(true);
                            ui.close_menu();
                        }
                        if ui.button("Save to library").on_hover_text("Keep a copy in your preset library on this computer").clicked() {
                            to_library = true;
                            ui.close_menu();
                        }
                    });
                } else {
                    save_buttons(ui, true, &mut save);
                }
            }
            _ => {
                if !live && ui.button("📚 Save to library").on_hover_text("Keep a copy in your preset library on this computer").clicked()
                {
                    to_library = true;
                }
                convert_menu(ui, doc, &mut convert);
                pedal_row(ui, doc, &mut cmds, &mut confirm);
            }
        }
        if save.is_none() && mode != 2 && ui.input_mut(|inp| inp.consume_key(egui::Modifiers::COMMAND, egui::Key::S)) {
            save = Some(false);
        }
        if let Some(target) = convert {
            self.convert_device_doc(i, target);
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

    /// The preset's chain and knobs. part 1: the chain and the controllers; 2: only the
    /// selected block's panel (docked at the bottom).
    fn editor(&mut self, ui: &mut egui::Ui, i: usize, part: u8) {
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
                if part == 3 {
                    crate::controllers::panel(ui, json, cat.as_deref(), &mut doc.dirty);
                }
            }
            DocKind::Device { preset, origin } => {
                let live = matches!(origin, Origin::Live { .. }) && connected;
                if part != 3 {
                    editors::device_editor(
                        ui,
                        editors::DeviceEdit { preset, live, cmds: &mut cmds, cat: cat.as_deref(), dirty: &mut doc.dirty },
                        &mut doc.selected,
                        picker,
                    );
                }
                if part == 3 {
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
            // The title is the first line of the card, not a bar across its top.
            egui::Window::new("unit-report")
                .title_bar(false)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.set_max_width(520.0);
                    ui.label(RichText::new(&title).size(17.0).strong().color(if refusal { bad() } else { ink() }));
                    ui.add_space(6.0);
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
            egui::Window::new("Rename preset").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(
                ctx,
                |ui| {
                    ui.label(format!("New name for {}:", slot_label(idx)));
                    let r = ui.add(egui::TextEdit::singleline(&mut name).char_limit(crate::session::PRESET_NAME_MAX));
                    ui.horizontal(|ui| theme::name_hint(ui, &name, crate::session::PRESET_NAME_MAX));
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
                },
            );
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
        self.restore_window(ctx);
        self.restore_all_window(ctx);
        self.compare_window(ctx);
        if let Some((text, cmd)) = self.confirm.take() {
            let mut result = None;
            egui::Window::new("Please confirm").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(
                ctx,
                |ui| {
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
                },
            );
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
        LogLevel::Good => good(),
        LogLevel::Bad => bad(),
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
    /// A picture is taken without the computer's mouse pointer: it would show tooltips and highlights.
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        if self.screenshot.is_some() {
            raw_input.events.retain(|e| {
                !matches!(e, egui::Event::PointerMoved(_) | egui::Event::PointerButton { .. } | egui::Event::MouseWheel { .. })
            });
            raw_input.events.push(egui::Event::PointerGone);
            raw_input.focused = true;
            // TONESTRAND_TABS=n presses Tab n times (for a picture of where the keyboard focus goes).
            if let Some(n) = std::env::var("TONESTRAND_TABS").ok().and_then(|v| v.parse::<u64>().ok()) {
                let frame = _ctx.cumulative_pass_nr();
                if frame >= 20 && frame < 20 + 2 * n && frame.is_multiple_of(2) {
                    for pressed in [true, false] {
                        raw_input.events.push(egui::Event::Key {
                            key: egui::Key::Tab,
                            physical_key: None,
                            pressed,
                            repeat: false,
                            modifiers: egui::Modifiers::NONE,
                        });
                    }
                }
            }
        }
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        theme::track_keyboard_nav(ctx);
        // Fill the monitor on start (asking at window creation isn't always honoured on Windows),
        // unless the window was left smaller last time: then it comes back as it was.
        let fill = self.settings.window.is_none() || self.settings.window_maximized;
        if self.screenshot.is_none() && fill {
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
        // Remember the window for next time (written when the program closes).
        if self.screenshot.is_none() {
            let (maximized, outer, inner) =
                ctx.input(|i| (i.viewport().maximized == Some(true), i.viewport().outer_rect, i.viewport().inner_rect));
            match (maximized, outer, inner) {
                (false, Some(o), Some(n)) => self.window_seen = Some(([o.min.x, o.min.y, n.width(), n.height()], false)),
                (true, ..) => {
                    let rect = self.window_seen.map(|(r, _)| r);
                    self.window_seen = rect.map(|r| (r, true)).or(Some(([0.0, 0.0, 1320.0, 820.0], true)));
                }
                _ => {}
            }
        }
        self.guard_quit(ctx);
        self.shortcuts(ctx);
        self.drop_files(ctx);
        self.poll(ctx);
        self.auto_pick();
        // The hidden Tests page.
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::T)) {
            self.view = if self.view == View::Tests { View::Editor } else { View::Tests };
        }
        self.studio_top_bar(ctx);
        self.status_bar(ctx);
        self.left_panel(ctx);
        self.central(ctx);
        self.dialogs(ctx);
        self.close_dialog(ctx);
        self.slot_drop_dialog(ctx);
        self.shortcuts_window(ctx);
        self.unit_notice_window(ctx);
        if let Some(cat) = self.catalog.clone() {
            if let Some((st, model, cab)) = editors::picker_window(ctx, &mut self.picker, &cat) {
                self.apply_pick(&cat, st, model, cab);
            }
        }
        self.screenshot_step(ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some((rect, maximized)) = self.window_seen.filter(|_| self.screenshot.is_none()) {
            self.settings.window = Some(rect);
            self.settings.window_maximized = maximized;
            self.settings.save();
        }
        if self.screenshot.is_none() {
            let tabs = self.open_file_paths();
            if tabs != self.settings.last_tabs {
                self.settings.last_tabs = tabs;
                self.settings.save();
            }
        }
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
/// After F2: give the preset's name box the focus with the whole name selected.
fn focus_if_asked(ui: &egui::Ui, r: &egui::Response, len: usize) {
    let flag = egui::Id::new("focus-doc-name");
    if !ui.data_mut(|d| d.remove_temp::<bool>(flag)).unwrap_or(false) {
        return;
    }
    r.request_focus();
    if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), r.id) {
        let all = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(len));
        state.cursor.set_char_range(Some(all));
        state.store(ui.ctx(), r.id);
    }
}

/// A sidebar section: a card under a small capital heading.
fn side_section<R>(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.add_space(4.0);
    ui.label(RichText::new(title).small().strong().color(muted()));
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
}

/// Why the built-in effects list is in use instead of HX Edit's.
pub(crate) enum BuiltinWhy {
    FolderHasNoFiles,
    FolderOlder,
    NotInstalled,
    InstalledOlder,
}

pub(crate) fn builtin_reason(custom: Option<&str>) -> BuiltinWhy {
    match custom {
        Some(f) => {
            let dir = std::path::Path::new(f);
            if dir.join("Helix.sym").is_file() || dir.join("res").join("Helix.sym").is_file() {
                BuiltinWhy::FolderOlder
            } else {
                BuiltinWhy::FolderHasNoFiles
            }
        }
        None if crate::catalog::find_dir(None).is_some() => BuiltinWhy::InstalledOlder,
        None => BuiltinWhy::NotInstalled,
    }
}
