//! The IR and NAM library on this computer: impulse responses (.wav) and NAM
//! captures (.nam) in real folders, each capture shown with its converted
//! .wav (the file the NAM A2 Lite block loads) or as not converted yet.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use eframe::egui::{self, RichText, Vec2};

use crate::folders::{self, FolderEvent, FolderPane};
use crate::theme::{self, *};

#[derive(Clone, PartialEq)]
pub enum Kind {
    /// A plain impulse response.
    Ir,
    /// A NAM capture (.nam), with its converted .wav when there is one.
    Nam,
    /// A converted NAM capture whose .nam isn't here.
    NamWav,
}

#[derive(Clone)]
pub struct Entry {
    pub kind: Kind,
    pub path: PathBuf,
    pub name: String,
    pub folder: String,
    /// For a capture: its converted .wav, when it exists.
    pub converted: Option<PathBuf>,
    /// For a capture: why it can't be converted (not an A2 Lite capture, another rate...).
    pub problem: Option<String>,
}

pub enum Action {
    /// Send these files to the pedal's IR slots.
    SendToPedal(Vec<PathBuf>),
    /// Something was written: tell the person.
    Note(bool, String),
}

#[derive(Default)]
pub struct ToneLib {
    pub root: PathBuf,
    pub entries: Vec<Entry>,
    pub folders: Vec<String>,
    pub current: Option<String>,
    pub search: String,
    pub selected: BTreeSet<PathBuf>,
    /// Which kind of file is shown: 0 both, 1 IRs, 2 NAM captures.
    pub view: usize,
    pane: FolderPane,
    confirm_delete: Option<Vec<PathBuf>>,
}

/// The converted file a capture gets: "<name> NAM.wav" beside it (as the converter writes).
pub fn converted_path(nam: &Path) -> PathBuf {
    let stem = nam.file_stem().and_then(|s| s.to_str()).unwrap_or("capture");
    nam.with_file_name(format!("{stem} NAM.wav"))
}

impl ToneLib {
    pub fn new(root: PathBuf) -> ToneLib {
        ToneLib { root, ..Default::default() }
    }

    pub fn folder(&self) -> PathBuf {
        match self.current.as_deref() {
            Some(f) if !f.is_empty() => self.root.join(f),
            _ => self.root.clone(),
        }
    }

    pub fn scan(&mut self, opts: crate::nam::Options) {
        let _ = std::fs::create_dir_all(&self.root);
        self.folders = folders::scan(&self.root, &[], 4);
        let mut dirs = vec![self.root.clone()];
        dirs.extend(self.folders.iter().map(|f| self.root.join(f)));
        let mut entries = Vec::new();
        for dir in dirs {
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            let files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect();
            let ext = |p: &Path| p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            let nams: Vec<&PathBuf> = files.iter().filter(|p| ext(p) == "nam").collect();
            let paired: Vec<PathBuf> = nams.iter().map(|n| converted_path(n)).collect();
            for p in &files {
                let name = p.file_stem().and_then(|s| s.to_str()).unwrap_or("?").to_owned();
                let folder = folders::folder_of(&self.root, p);
                match ext(p).as_str() {
                    "nam" => {
                        let c = converted_path(p);
                        let problem = std::fs::read_to_string(p)
                            .map_err(|e| e.to_string())
                            .and_then(|t| crate::nam::convert(&t, opts).map(|_| ()))
                            .err();
                        entries.push(Entry {
                            kind: Kind::Nam,
                            path: p.clone(),
                            name,
                            folder,
                            converted: c.is_file().then_some(c),
                            problem,
                        });
                    }
                    "wav" if !paired.contains(p) => {
                        let nam_wav = std::fs::read(p)
                            .ok()
                            .and_then(|b| crate::irs::wav_to_audio(&b).ok())
                            .is_some_and(|(a, _)| crate::irs::is_nam_capture(&a));
                        entries.push(Entry {
                            kind: if nam_wav { Kind::NamWav } else { Kind::Ir },
                            path: p.clone(),
                            name,
                            folder,
                            converted: None,
                            problem: None,
                        });
                    }
                    _ => {}
                }
            }
        }
        entries.sort_by_key(|e| (e.folder.to_lowercase(), e.name.to_lowercase()));
        self.selected.retain(|p| entries.iter().any(|e| &e.path == p));
        self.entries = entries;
        if self.current.as_ref().is_some_and(|c| !c.is_empty() && !self.folders.contains(c)) {
            self.current = None;
        }
    }

    fn shown(&self) -> Vec<Entry> {
        let q = self.search.trim().to_lowercase();
        self.entries
            .iter()
            .filter(|e| self.current.as_ref().is_none_or(|c| &e.folder == c))
            .filter(|e| q.is_empty() || e.name.to_lowercase().contains(&q) || e.folder.to_lowercase().contains(&q))
            .filter(|e| match self.view {
                1 => e.kind == Kind::Ir,
                2 => e.kind != Kind::Ir,
                _ => true,
            })
            .cloned()
            .collect()
    }

    /// The files to send for an entry: a converted capture goes as its .wav, exactly.
    fn send_path(e: &Entry) -> PathBuf {
        e.converted.clone().unwrap_or_else(|| e.path.clone())
    }

    fn convert(&mut self, which: &[Entry], opts: crate::nam::Options) -> Vec<Action> {
        let mut ok = 0;
        let mut bad = Vec::new();
        for e in which.iter().filter(|e| e.kind == Kind::Nam) {
            let dir = e.path.parent().unwrap_or(&self.root).to_path_buf();
            match crate::nam::convert_file(&e.path, &dir, opts) {
                Ok(_) => ok += 1,
                Err(err) => bad.push(format!("{}: {err}", e.name)),
            }
        }
        self.scan(opts);
        let mut out = vec![Action::Note(bad.is_empty(), format!("Converted {ok} capture{}", if ok == 1 { "" } else { "s" }))];
        for b in bad {
            out.push(Action::Note(false, b));
        }
        out
    }

    /// The library view. `can_send`: a pedal is connected and its IR list read.
    pub fn ui(&mut self, ui: &mut egui::Ui, can_send: bool, opts: crate::nam::Options) -> Vec<Action> {
        let mut actions = Vec::new();
        let mut rescan = false;
        ui.horizontal_top(|ui| {
            // Folders.
            ui.vertical(|ui| {
                ui.set_width(230.0);
                theme::section_help(ui, "Folders", "Real folders on your disk, inside the IR and NAM library folder (set in Settings). Drag files onto a folder to move them.");
                ui.add_space(4.0);
                let entries = &self.entries;
                let count = |f: Option<&str>| match f {
                    None => entries.len(),
                    Some(f) => entries.iter().filter(|e| e.folder == f).count(),
                };
                let fl = self.folders.clone();
                let events = folders::pane(ui, &mut self.pane, &self.root, &fl, &mut self.current, &count, "Everything");
                for ev in events {
                    match ev {
                        FolderEvent::Dropped(files, to) => {
                            let dir = if to.is_empty() { self.root.clone() } else { self.root.join(&to) };
                            for f in files {
                                // A capture moves with its converted file.
                                let conv = converted_path(&f);
                                let _ = folders::move_into(&f, &dir);
                                if f.extension().is_some_and(|e| e.eq_ignore_ascii_case("nam")) && conv.is_file() {
                                    let _ = folders::move_into(&conv, &dir);
                                }
                            }
                            rescan = true;
                        }
                        FolderEvent::Changed => rescan = true,
                    }
                }
                ui.add_space(10.0);
                if ui.small_button("📁 Show library folder").clicked() {
                    folders::open_in_explorer(&self.root);
                }
            });
            ui.separator();
            // Files.
            ui.vertical(|ui| {
                let shown = self.shown();
                let picked: Vec<Entry> = shown.iter().filter(|e| self.selected.contains(&e.path)).cloned().collect();
                let unconverted: Vec<Entry> = shown.iter().filter(|e| e.kind == Kind::Nam && e.converted.is_none() && e.problem.is_none()).cloned().collect();
                ui.horizontal(|ui| {
                    let in_folder = |k: &dyn Fn(&Kind) -> bool| {
                        self.entries.iter().filter(|e| self.current.as_ref().is_none_or(|c| &e.folder == c) && k(&e.kind)).count()
                    };
                    let labels = [
                        "Both".to_owned(),
                        format!("IRs ({})", in_folder(&|k| *k == Kind::Ir)),
                        format!("NAM captures ({})", in_folder(&|k| *k != Kind::Ir)),
                    ];
                    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
                    if let Some(v) = theme::segmented(ui, &refs, self.view) {
                        self.view = v;
                        self.selected.clear();
                    }
                });
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("🔍 Search IRs and captures").desired_width(220.0));
                    if ui.button("Import files...").on_hover_text("Copy .wav IRs and .nam captures into the selected folder").clicked() {
                        if let Some(files) = rfd::FileDialog::new().add_filter("IRs and NAM captures", &["wav", "nam"]).pick_files() {
                            let dir = self.folder();
                            let _ = std::fs::create_dir_all(&dir);
                            let n = files.iter().filter(|f| crate::files::copy_into(f, &dir).is_ok()).count();
                            actions.push(Action::Note(true, format!("Imported {n} file{}", if n == 1 { "" } else { "s" })));
                            rescan = true;
                        }
                    }
                    let to_convert: Vec<Entry> = if picked.iter().any(|e| e.kind == Kind::Nam) { picked.iter().filter(|e| e.kind == Kind::Nam).cloned().collect() } else { unconverted.clone() };
                    let label = if picked.iter().any(|e| e.kind == Kind::Nam) { format!("Convert selected ({})", to_convert.len()) } else { format!("Convert all not converted ({})", to_convert.len()) };
                    if ui.add_enabled(!to_convert.is_empty(), egui::Button::new(label)).on_hover_text("Write the .wav each capture needs for the NAM A2 Lite block, beside it").clicked() {
                        actions.extend(self.convert(&to_convert, opts));
                    }
                    let send = ui.add_enabled(can_send && !picked.is_empty(), egui::Button::new(format!("⬆ Send selected to pedal ({})", picked.len())));
                    let send = send.on_disabled_hover_text(if can_send { "Select files first" } else { "Connect a pedal and read its IR list first (On the pedal)" });
                    if send.clicked() {
                        actions.push(Action::SendToPedal(picked.iter().map(Self::send_path).collect()));
                    }
                    if ui.add_enabled(!picked.is_empty(), egui::Button::new(RichText::new("Delete...").color(BAD))).clicked() {
                        let mut files: Vec<PathBuf> = picked.iter().map(|e| e.path.clone()).collect();
                        files.extend(picked.iter().filter_map(|e| e.converted.clone()));
                        self.confirm_delete = Some(files);
                    }
                    if !picked.is_empty() && ui.small_button("Clear selection").clicked() {
                        self.selected.clear();
                    }
                });
                ui.add_space(6.0);
                if shown.is_empty() {
                    theme::card(ui, |ui| {
                        ui.vertical_centered(|ui| {
                            ui.add_space(16.0);
                            ui.label(RichText::new("No IRs or captures here yet").size(16.0).strong());
                            ui.label(RichText::new("Import .wav IRs and .nam captures, or download IRs from the pedal into this library.").color(MUTED));
                            ui.add_space(16.0);
                        });
                    });
                    return;
                }
                let multi = ui.input(|i| i.modifiers.ctrl || i.modifiers.command || i.modifiers.shift);
                egui::ScrollArea::vertical().id_salt("tonelib").auto_shrink([false, false]).show(ui, |ui| {
                    for e in &shown {
                        let on = self.selected.contains(&e.path);
                        let r = egui::Frame::new()
                            .fill(if on { accent().linear_multiply(0.18) } else { card_bg() })
                            .corner_radius(egui::CornerRadius::same(6))
                            .inner_margin(egui::Margin::symmetric(10, 6))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    match e.kind {
                                        Kind::Ir => theme::pill(ui, "IR", BLUE),
                                        Kind::Nam => theme::pill(ui, "NAM", PINK),
                                        Kind::NamWav => theme::pill(ui, "NAM .wav", PINK),
                                    }
                                    ui.label(RichText::new(&e.name).strong());
                                    if self.current.is_none() && !e.folder.is_empty() {
                                        ui.label(RichText::new(&e.folder).small().color(accent()));
                                    }
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| match (&e.kind, &e.converted, &e.problem) {
                                        (Kind::Nam, _, Some(p)) => {
                                            theme::help(ui, p);
                                            theme::pill(ui, "can't convert", BAD);
                                        }
                                        (Kind::Nam, Some(c), _) => {
                                            theme::pill(ui, "✔ converted", GOOD);
                                            ui.label(RichText::new(c.file_name().and_then(|s| s.to_str()).unwrap_or("")).small().color(MUTED));
                                        }
                                        (Kind::Nam, None, None) => theme::pill(ui, "not converted", ORANGE),
                                        (Kind::NamWav, ..) => {
                                            ui.label(RichText::new("converted capture (its .nam isn't here)").small().color(MUTED));
                                        }
                                        _ => {}
                                    });
                                });
                            })
                            .response
                            .interact(egui::Sense::click_and_drag());
                        r.dnd_set_drag_payload(vec![e.path.clone()]);
                        if r.clicked() {
                            if multi {
                                if !self.selected.remove(&e.path) {
                                    self.selected.insert(e.path.clone());
                                }
                            } else if on && self.selected.len() == 1 {
                                self.selected.clear();
                            } else {
                                self.selected.clear();
                                self.selected.insert(e.path.clone());
                            }
                        }
                        r.context_menu(|ui| {
                            if ui.button("Show in Explorer").clicked() {
                                folders::open_in_explorer(e.path.parent().unwrap_or(&self.root));
                                ui.close_menu();
                            }
                            if e.kind == Kind::Nam && e.problem.is_none() && ui.button(if e.converted.is_some() { "Convert again" } else { "Convert" }).clicked() {
                                actions.extend(self.convert(std::slice::from_ref(e), opts));
                                ui.close_menu();
                            }
                        });
                        ui.add_space(3.0);
                    }
                    ui.add_space(4.0);
                    theme::help(ui, "Click to select, Ctrl+click for several, drag onto a folder to move.");
                });
            });
        });
        // Deleting files, after asking.
        if let Some(files) = self.confirm_delete.clone() {
            let mut result = None;
            egui::Window::new("Delete files").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO).show(
                ui.ctx(),
                |ui| {
                    let s = if files.len() == 1 { "" } else { "s" };
                    if crate::files::HAS_RECYCLE_BIN {
                        ui.label(format!("Move {} file{s} to the Recycle Bin?", files.len()));
                    } else {
                        ui.label(format!("Delete {} file{s} from your library? This can't be undone.", files.len()));
                    }
                    for f in files.iter().take(8) {
                        ui.label(RichText::new(f.file_name().and_then(|s| s.to_str()).unwrap_or("")).small().color(MUTED));
                    }
                    ui.horizontal(|ui| {
                        if ui
                            .button(RichText::new(if crate::files::HAS_RECYCLE_BIN { "Move to Recycle Bin" } else { "Delete" }).color(BAD))
                            .clicked()
                        {
                            result = Some(true);
                        }
                        if ui.button("Cancel").clicked() {
                            result = Some(false);
                        }
                    });
                },
            );
            if let Some(yes) = result {
                if yes {
                    let n = files.iter().filter(|f| crate::files::delete_file(f).is_ok()).count();
                    let verb = if crate::files::HAS_RECYCLE_BIN { "Moved to the Recycle Bin:" } else { "Deleted" };
                    actions.push(Action::Note(
                        n == files.len(),
                        format!("{verb} {n} of {} file{}", files.len(), if files.len() == 1 { "" } else { "s" }),
                    ));
                    rescan = true;
                }
                self.confirm_delete = None;
            }
        }
        if rescan {
            self.scan(opts);
        }
        actions
    }
}
