//! The preset library: every preset kept on this computer, grouped into
//! folders (real folders inside the library folder, nested allowed; a setlist is a folder), searchable by name and by the
//! effects inside. Nothing leaves the computer.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use eframe::egui::{self, Color32, CornerRadius, RichText, Stroke, Vec2};

use crate::catalog::Catalog;
use crate::devpreset::{DevicePreset, Kind};
use crate::hlx;
use crate::theme::{self, *};

#[derive(Clone)]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    /// "" for presets at the top of the library, else the setlist's name.
    pub setlist: String,
    pub pedal_file: bool,
    pub blocks: Vec<(String, Color32)>,
    pub modified: Option<SystemTime>,
}

#[derive(Default)]
pub struct Library {
    pub root: PathBuf,
    pub entries: Vec<Entry>,
    pub setlists: Vec<String>,
    /// None shows every preset.
    pub current: Option<String>,
    pub search: String,
    pub grid: bool,
    pub folders_pane: crate::folders::FolderPane,
    /// Folders inside the library that hold other things (the IR / NAM library, self-test backups): left out.
    pub skip: Vec<PathBuf>,
}

pub enum Action {
    Open(PathBuf),
    SendToPedal(PathBuf),
    Delete(PathBuf),
    MoveTo(PathBuf, String),
    Import,
    /// Folders changed on disk: look again.
    Rescan,
}

fn is_preset(p: &Path) -> bool {
    matches!(
        p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(),
        Some("hlx") | Some("hxpreset")
    )
}

/// The effects in a preset file, with their colours, for the cards and the search.
fn blocks_of(path: &Path, pedal_file: bool, cat: Option<&Catalog>) -> (Option<String>, Vec<(String, Color32)>) {
    if pedal_file {
        let Ok(doc) = std::fs::read(path).map_err(|e| e.to_string()).and_then(|b| DevicePreset::parse(&b)) else {
            return (None, Vec::new());
        };
        let blocks = doc
            .slots()
            .into_iter()
            .filter(|s| s.kind == Kind::Block)
            .filter_map(|s| s.model)
            .map(|m| match cat {
                Some(c) => (c.model_name(m), c.category(m).map(|x| x.1).unwrap_or(MUTED)),
                None => (format!("Model {m}"), theme::number_colour(m)),
            })
            .collect();
        (None, blocks)
    } else {
        let Some(json) = std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) else {
            return (None, Vec::new());
        };
        let blocks = hlx::blocks(&json)
            .into_iter()
            .map(|b| match cat {
                Some(c) => (
                    c.info_for_symbol(&b.model).map(|i| crate::realnames::show(&i.name)).unwrap_or_else(|| hlx::pretty_model(&b.model)),
                    c.category_of_symbol(&b.model).map(|x| x.1).unwrap_or(MUTED),
                ),
                None => (hlx::pretty_model(&b.model), family_colour(hlx::family(&b.model))),
            })
            .collect();
        (Some(hlx::name(&json)), blocks)
    }
}

impl Library {
    pub fn new(root: PathBuf) -> Library {
        Library { root, grid: true, ..Default::default() }
    }

    pub fn folder(&self) -> PathBuf {
        match &self.current {
            Some(s) => self.root.join(s),
            None => self.root.clone(),
        }
    }

    pub fn scan(&mut self, cat: Option<&Catalog>) {
        let mut entries = Vec::new();
        let setlists = crate::folders::scan(&self.root, &self.skip, 4);
        let mut visit = |dir: &Path, setlist: &str, entries: &mut Vec<Entry>| {
            let Ok(rd) = std::fs::read_dir(dir) else { return };
            for e in rd.flatten() {
                let p = e.path();
                if !p.is_file() || !is_preset(&p) {
                    continue;
                }
                let pedal_file = p.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("hxpreset"));
                let (name, blocks) = blocks_of(&p, pedal_file, cat);
                let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("Preset").to_owned();
                entries.push(Entry {
                    name: name.unwrap_or(stem),
                    setlist: setlist.to_owned(),
                    pedal_file,
                    blocks,
                    modified: e.metadata().ok().and_then(|m| m.modified().ok()),
                    path: p,
                });
            }
        };
        visit(&self.root, "", &mut entries);
        for f in &setlists {
            visit(&self.root.join(f), f, &mut entries);
        }
        entries.sort_by_key(|e| e.name.to_lowercase());
        self.entries = entries;
        self.setlists = setlists;
        if self.current.as_ref().is_some_and(|c| !c.is_empty() && !self.setlists.contains(c)) {
            self.current = None;
        }
    }
    fn matches(&self, e: &Entry) -> bool {
        if let Some(c) = &self.current {
            if &e.setlist != c {
                return false;
            }
        }
        let q = self.search.trim().to_lowercase();
        q.is_empty()
            || e.name.to_lowercase().contains(&q)
            || e.setlist.to_lowercase().contains(&q)
            || e.blocks.iter().any(|(b, _)| b.to_lowercase().contains(&q))
    }

    /// The whole library page.
    pub fn ui(&mut self, ui: &mut egui::Ui, can_send: bool) -> Vec<Action> {
        let mut actions = Vec::new();
        ui.horizontal(|ui| {
            ui.label(RichText::new("Your library").size(24.0).strong());
            ui.label(RichText::new("Presets kept on this computer").color(MUTED));
            if !can_send {
                ui.label(RichText::new("· To send presets to the pedal, connect and pick a slot in the pedal list").small().color(MUTED));
            }
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let w = ui.available_width() - 250.0;
            ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("🔍 Search names, folders or effects...").desired_width(w.max(200.0)));
            if ui.button("Import files...").clicked() {
                actions.push(Action::Import);
            }
            if ui.selectable_label(self.grid, "▦ Grid").clicked() {
                self.grid = true;
            }
            if ui.selectable_label(!self.grid, "☰ List").clicked() {
                self.grid = false;
            }
        });
        ui.add_space(10.0);
        ui.horizontal_top(|ui| {
            // Folders (a setlist is a folder: the HX Stomp has no setlists of its own).
            ui.vertical(|ui| {
                ui.set_width(230.0);
                theme::section_help(
                    ui,
                    "Folders",
                    "Every folder here is a real folder on your disk, inside your library folder. Use them as setlists, gigs, bands or tunings. \
                     Drag presets onto a folder to move them; right-click a folder to add one inside it, rename it or show it in Explorer.",
                );
                ui.add_space(4.0);
                let entries = &self.entries;
                let count = |f: Option<&str>| match f {
                    None => entries.len(),
                    Some(f) => entries.iter().filter(|e| e.setlist == f).count(),
                };
                let mut events = Vec::new();
                let folders = self.setlists.clone();
                crate::folders::pane(ui, &mut self.folders_pane, &self.root, &folders, &mut self.current, &count, "All presets", &mut events);
                for ev in events {
                    match ev {
                        crate::folders::FolderEvent::Dropped(files, to) => {
                            for f in files {
                                actions.push(Action::MoveTo(f, to.clone()));
                            }
                        }
                        crate::folders::FolderEvent::Changed => actions.push(Action::Rescan),
                    }
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.small_button("📁 Show library folder").clicked() {
                        crate::folders::open_in_explorer(&self.root);
                    }
                    theme::help(ui, &format!("{}\n\nChange it in Settings.", self.root.to_string_lossy()));
                });
            });            ui.separator();
            // Presets.
            ui.vertical(|ui| {
                let shown: Vec<Entry> = self.entries.iter().filter(|e| self.matches(e)).cloned().collect();
                let title = match self.current.as_deref() { None => "All presets".to_owned(), Some("") => "Top folder".to_owned(), Some(f) => f.to_owned() };
                ui.horizontal(|ui| {
                    ui.label(RichText::new(title).strong());
                    ui.label(RichText::new(format!("{} presets", shown.len())).color(MUTED));
                });
                ui.add_space(6.0);
                if shown.is_empty() {
                    theme::card(ui, |ui| {
                        ui.vertical_centered(|ui| {
                            ui.add_space(20.0);
                            ui.label(RichText::new("Nothing here yet").size(17.0).strong());
                            ui.label(RichText::new("Open a preset in the editor and choose \"Save to library\", or import files.").color(MUTED));
                            ui.add_space(20.0);
                        });
                    });
                    return;
                }
                egui::ScrollArea::vertical().id_salt("library").auto_shrink([false, false]).show(ui, |ui| {
                    if self.grid {
                        // Rows of cards that fit the width (each card is 250 + 24 margin wide).
                        let per_row = ((ui.available_width() + 10.0) / (274.0 + 10.0)).floor().max(1.0) as usize;
                        for row in shown.chunks(per_row) {
                            ui.horizontal_top(|ui| {
                                ui.spacing_mut().item_spacing = Vec2::splat(10.0);
                                for e in row {
                                    self.card(ui, e, can_send, &mut actions);
                                }
                            });
                            ui.add_space(10.0);
                        }
                    } else {
                        for e in &shown {
                            self.row(ui, e, can_send, &mut actions);
                        }
                    }
                });
            });
        });
        actions
    }

    fn menu(&self, ui: &mut egui::Ui, e: &Entry, can_send: bool, actions: &mut Vec<Action>) {
        if ui.button("Open in editor").clicked() {
            actions.push(Action::Open(e.path.clone()));
            ui.close_menu();
        }
        let send = ui.add_enabled(can_send, egui::Button::new("Send to the picked pedal slot"));
        let send = send
            .on_hover_text(if e.pedal_file { "Write this preset into the slot picked in the pedal list" } else { "Converted to the pedal's format, then written into the slot picked in the pedal list" })
            .on_disabled_hover_text("Connect and pick a slot in the pedal list first");
        if send.clicked() {
            actions.push(Action::SendToPedal(e.path.clone()));
            ui.close_menu();
        }
        ui.menu_button("Move to folder", |ui| {
            if ui.button("Top folder").clicked() {
                actions.push(Action::MoveTo(e.path.clone(), String::new()));
                ui.close_menu();
            }
            for s in &self.setlists {
                if ui.button(s).clicked() {
                    actions.push(Action::MoveTo(e.path.clone(), s.clone()));
                    ui.close_menu();
                }
            }
        });
        ui.separator();
        if ui.button(RichText::new("Delete...").color(BAD)).clicked() {
            actions.push(Action::Delete(e.path.clone()));
            ui.close_menu();
        }
    }

    fn card(&self, ui: &mut egui::Ui, e: &Entry, can_send: bool, actions: &mut Vec<Action>) {
        let r = egui::Frame::new()
            .fill(card_bg())
            .stroke(Stroke::new(1.0_f32, line()))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| {
                ui.set_width(250.0);
                ui.set_height(118.0);
                // Stack the card's contents, whatever layout the grid around it uses.
                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if e.pedal_file {
                            theme::pill(ui, "PEDAL", ORANGE);
                        } else {
                            theme::pill(ui, "HLX", BLUE);
                        }
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.add(egui::Label::new(RichText::new(&e.name).size(15.0).strong()).truncate());
                        });
                    });
                });
                if !e.setlist.is_empty() {
                    let leaf = e.setlist.rsplit('/').next().unwrap_or(&e.setlist);
                    ui.add(egui::Label::new(RichText::new(format!("📁 {leaf}")).small().color(accent())).truncate()).on_hover_text(&e.setlist);
                }
                ui.add_space(4.0);
                // A strip of the effects, in their colours.
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(3.0, 3.0);
                    for (_, col) in e.blocks.iter().take(12) {
                        let (r, _) = ui.allocate_exact_size(Vec2::new(16.0, 8.0), egui::Sense::hover());
                        ui.painter().rect_filled(r, CornerRadius::same(2), *col);
                    }
                });
                let names: Vec<&str> = e.blocks.iter().take(4).map(|b| b.0.as_str()).collect();
                ui.add(egui::Label::new(RichText::new(names.join(" · ")).small().color(MUTED)).wrap());
                });
            });
        let resp = r.response.interact(egui::Sense::click_and_drag());
        resp.dnd_set_drag_payload(vec![e.path.clone()]);
        if resp.double_clicked() {
            actions.push(Action::Open(e.path.clone()));
        }
        resp.on_hover_text("Double-click to open, drag onto a folder to move, right-click for more")
            .context_menu(|ui| self.menu(ui, e, can_send, actions));
    }

    fn row(&self, ui: &mut egui::Ui, e: &Entry, can_send: bool, actions: &mut Vec<Action>) {
        let r = ui.horizontal(|ui| {
            if e.pedal_file {
                theme::pill(ui, "PEDAL", ORANGE);
            } else {
                theme::pill(ui, "HLX", BLUE);
            }
            ui.label(RichText::new(&e.name).strong());
            if !e.setlist.is_empty() {
                ui.label(RichText::new(&e.setlist).small().color(accent()));
            }
            let names: Vec<&str> = e.blocks.iter().take(6).map(|b| b.0.as_str()).collect();
            ui.label(RichText::new(names.join(" · ")).small().color(MUTED));
        });
        let resp = r.response.interact(egui::Sense::click_and_drag());
        resp.dnd_set_drag_payload(vec![e.path.clone()]);
        if resp.double_clicked() {
            actions.push(Action::Open(e.path.clone()));
        }
        resp.context_menu(|ui| self.menu(ui, e, can_send, actions));
        ui.separator();
    }
}
