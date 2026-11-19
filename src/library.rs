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
    /// The preset's snapshot names, searched too.
    pub snapshots: Vec<String>,
    /// Tags the person gave it (kept in a small file in the library folder, by file name).
    pub tags: Vec<String>,
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
    /// How presets are ordered: 0 by name, 1 by date modified (newest first).
    pub sort: usize,
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
    /// Give a preset these tags.
    SetTags(PathBuf, Vec<String>),
}

fn is_preset(p: &Path) -> bool {
    matches!(p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(), Some("hlx") | Some("hxpreset"))
}

/// The effects in a preset file, with their colours, for the cards and the search.
/// What a preset file shows in the library: its name (for a .hlx), its effects and its snapshot names.
type Scanned = (Option<String>, Vec<(String, Color32)>, Vec<String>);

fn blocks_of(path: &Path, pedal_file: bool, cat: Option<&Catalog>) -> Scanned {
    if pedal_file {
        let Ok(doc) = std::fs::read(path).map_err(|e| e.to_string()).and_then(|b| DevicePreset::parse(&b)) else {
            return (None, Vec::new(), Vec::new());
        };
        let blocks = doc
            .slots()
            .into_iter()
            .filter(|s| s.kind == Kind::Block)
            .filter_map(|s| s.model)
            .map(|m| match cat {
                Some(c) => (c.model_name(m), c.category(m).map(|x| theme::rgb(x.1)).unwrap_or(muted())),
                None => (format!("Model {m}"), theme::number_colour(m)),
            })
            .collect();
        (None, blocks, doc.snapshot_names())
    } else {
        let Some(json) = std::fs::read_to_string(path).ok().and_then(|t| crate::hlx::parse_text(&t).ok()) else {
            return (None, Vec::new(), Vec::new());
        };
        let blocks = hlx::blocks(&json)
            .into_iter()
            .map(|b| match cat {
                Some(c) => (
                    c.info_for_symbol(&b.model).map(|i| crate::realnames::show(&i.name)).unwrap_or_else(|| hlx::pretty_model(&b.model)),
                    c.category_of_symbol(&b.model).map(|x| theme::rgb(x.1)).unwrap_or(muted()),
                ),
                None => (hlx::pretty_model(&b.model), family_colour(hlx::family(&b.model))),
            })
            .collect();
        let snaps = crate::snapshots::list(&json).into_iter().map(|(_, n)| n).collect();
        (Some(hlx::name(&json)), blocks, snaps)
    }
}

/// The key a preset's tags are kept under: its file name, so they follow it when it moves to another folder.
fn tag_key(p: &Path) -> String {
    p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_lowercase()
}

/// A tag as typed, tidied: no leading `#`, no surrounding spaces, at most 24 characters.
fn clean_tag(t: &str) -> String {
    t.trim().trim_start_matches('#').trim().chars().take(24).collect()
}

impl Library {
    pub fn new(root: PathBuf) -> Library {
        Library { root, grid: true, ..Default::default() }
    }

    fn tags_path(&self) -> PathBuf {
        self.root.join(".tonestrand-tags.json")
    }

    /// The tags kept in the library folder, by file name.
    fn load_tags(&self) -> std::collections::BTreeMap<String, Vec<String>> {
        std::fs::read_to_string(self.tags_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    /// Every tag in use, sorted.
    pub fn all_tags(&self) -> Vec<String> {
        let mut v: Vec<String> = self.entries.iter().flat_map(|e| e.tags.iter().cloned()).collect();
        v.sort_by_key(|t| t.to_lowercase());
        v.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        v
    }

    /// Give a preset these tags (an empty list takes them all away), keeping them in the library folder.
    pub fn set_tags(&mut self, path: &Path, tags: Vec<String>) -> Result<(), String> {
        let mut clean: Vec<String> = Vec::new();
        for t in tags.iter().map(|t| clean_tag(t)).filter(|t| !t.is_empty()) {
            if !clean.iter().any(|c| c.eq_ignore_ascii_case(&t)) {
                clean.push(t);
            }
        }
        let key = tag_key(path);
        let mut map = self.load_tags();
        if clean.is_empty() {
            map.remove(&key);
        } else {
            map.insert(key.clone(), clean.clone());
        }
        let text = serde_json::to_string_pretty(&map).map_err(|e| e.to_string())?;
        crate::files::write_atomic(&self.tags_path(), text.as_bytes()).map_err(|e| e.to_string())?;
        for e in self.entries.iter_mut().filter(|e| tag_key(&e.path) == key) {
            e.tags = clean.clone();
        }
        Ok(())
    }

    pub fn folder(&self) -> PathBuf {
        match &self.current {
            Some(s) => self.root.join(s),
            None => self.root.clone(),
        }
    }

    pub fn scan(&mut self, cat: Option<&Catalog>) {
        let mut entries = Vec::new();
        let tag_map = self.load_tags();
        let setlists = crate::folders::scan(&self.root, &self.skip, 4);
        let visit = |dir: &Path, setlist: &str, entries: &mut Vec<Entry>| {
            let Ok(rd) = std::fs::read_dir(dir) else { return };
            for e in rd.flatten() {
                let p = e.path();
                if !p.is_file() || !is_preset(&p) {
                    continue;
                }
                let pedal_file = p.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("hxpreset"));
                let (name, blocks, snapshots) = blocks_of(&p, pedal_file, cat);
                let tags = tag_map.get(&tag_key(&p)).cloned().unwrap_or_default();
                let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("Preset").to_owned();
                entries.push(Entry {
                    name: name.unwrap_or(stem),
                    setlist: setlist.to_owned(),
                    pedal_file,
                    blocks,
                    snapshots,
                    tags,
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
    /// Searching looks in every folder, whichever one is picked.
    fn searching(&self) -> bool {
        !self.search.trim().is_empty()
    }

    fn matches(&self, e: &Entry) -> bool {
        if let Some(c) = &self.current {
            if &e.setlist != c && !self.searching() {
                return false;
            }
        }
        let q = self.search.trim().to_lowercase();
        // "#tag" shows the presets with exactly that tag.
        if let Some(tag) = q.strip_prefix('#') {
            return e.tags.iter().any(|t| t.to_lowercase() == tag);
        }
        q.is_empty()
            || e.name.to_lowercase().contains(&q)
            || e.setlist.to_lowercase().contains(&q)
            || e.blocks.iter().any(|(b, _)| b.to_lowercase().contains(&q))
            || e.tags.iter().any(|t| t.to_lowercase().contains(&q))
            || e.snapshots.iter().any(|s| s.to_lowercase().contains(&q))
    }

    /// The whole library page.
    pub fn ui(&mut self, ui: &mut egui::Ui, can_send: bool) -> Vec<Action> {
        let mut actions = Vec::new();
        ui.horizontal(|ui| {
            ui.label(RichText::new("Your library").size(24.0).strong());
            ui.label(RichText::new("Presets kept on this computer").color(muted()));
            if !can_send {
                ui.label(RichText::new("· To send presets to the pedal, connect and pick a slot in the pedal list").small().color(muted()));
            }
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let w = ui.available_width() - 580.0;
            theme::search_box(ui, &mut self.search, "Search names, folders, effects, snapshots or #tags...", w.max(200.0));
            ui.label(RichText::new("Sort").small().color(muted()));
            if let Some(s) = theme::segmented(ui, &["Name", "Newest"], self.sort) {
                self.sort = s;
            }
            theme::help(ui, "Sort by name, or by when the file was last changed (newest first). Searching looks in every folder.");
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
        // The tags in use: one click shows the presets with that tag.
        let tags = self.all_tags();
        if !tags.is_empty() {
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Tags").small().color(muted()));
                for t in &tags {
                    let query = format!("#{t}");
                    let on = self.search.trim().eq_ignore_ascii_case(&query);
                    if ui.selectable_label(on, t).clicked() {
                        self.search = if on { String::new() } else { query };
                    }
                }
            });
        }
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
                let folders = self.setlists.clone();
                let events = crate::folders::pane(ui, &mut self.folders_pane, &self.root, &folders, &mut self.current, &count, "All presets");
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
                    if !cfg!(target_os = "android") && ui.small_button("Show library folder").clicked() {
                        crate::folders::open_in_explorer(&self.root);
                    }
                    theme::help(ui, &format!("{}\n\nChange it in Settings.", self.root.to_string_lossy()));
                });
            });            ui.separator();
            // Presets.
            ui.vertical(|ui| {
                let mut shown: Vec<Entry> = self.entries.iter().filter(|e| self.matches(e)).cloned().collect();
                if self.sort == 1 {
                    // Newest first; files with no date last; the name breaks ties.
                    shown.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
                }
                let title = if self.searching() {
                    "Search results (all folders)".to_owned()
                } else {
                    match self.current.as_deref() { None => "All presets".to_owned(), Some("") => "Top folder".to_owned(), Some(f) => f.to_owned() }
                };
                ui.horizontal(|ui| {
                    ui.label(RichText::new(title).strong());
                    ui.label(RichText::new(format!("{} presets", shown.len())).color(muted()));
                });
                ui.add_space(6.0);
                if shown.is_empty() {
                    theme::card(ui, |ui| {
                        ui.vertical_centered(|ui| {
                            ui.add_space(20.0);
                            ui.label(RichText::new("Nothing here yet").size(17.0).strong());
                            ui.label(RichText::new("Open a preset in the editor and choose \"Save to library\", or import files.").color(muted()));
                            ui.add_space(20.0);
                        });
                    });
                    return;
                }
                egui::ScrollArea::vertical().id_salt("library").auto_shrink([false, false]).drag_to_scroll(!theme::carrying(ui.ctx())).show(ui, |ui| {
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
            .on_hover_text(if e.pedal_file {
                "Write this preset into the slot picked in the pedal list"
            } else {
                "Converted to the pedal's format, then written into the slot picked in the pedal list"
            })
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
        ui.menu_button("Tags", |ui| {
            // Tags in use (tick one to give it to this preset or take it away), and a box for a new one.
            for t in self.all_tags() {
                let mut on = e.tags.iter().any(|x| x.eq_ignore_ascii_case(&t));
                if ui.checkbox(&mut on, &t).changed() {
                    let mut next: Vec<String> = e.tags.iter().filter(|x| !x.eq_ignore_ascii_case(&t)).cloned().collect();
                    if on {
                        next.push(t.clone());
                    }
                    actions.push(Action::SetTags(e.path.clone(), next));
                    ui.close_menu();
                }
            }
            let id = ui.id().with(("new-tag", &e.path));
            let mut text: String = ui.data_mut(|d| d.get_temp(id).unwrap_or_default());
            let resp = ui.add(egui::TextEdit::singleline(&mut text).hint_text("New tag, then Enter").desired_width(150.0));
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !clean_tag(&text).is_empty() {
                let mut next = e.tags.clone();
                next.push(text.clone());
                actions.push(Action::SetTags(e.path.clone(), next));
                text.clear();
                ui.close_menu();
            }
            ui.data_mut(|d| d.insert_temp(id, text));
        });
        ui.separator();
        if ui.button(RichText::new("Delete...").color(bad())).clicked() {
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
                ui.set_height(134.0);
                // Stack the card's contents, whatever layout the grid around it uses.
                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if e.pedal_file {
                                theme::pill(ui, "PEDAL", orange());
                            } else {
                                theme::pill(ui, "HLX", blue());
                            }
                            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                ui.add(egui::Label::new(RichText::new(&e.name).size(15.0).strong()).truncate());
                            });
                        });
                    });
                    if !e.setlist.is_empty() {
                        let leaf = e.setlist.rsplit('/').next().unwrap_or(&e.setlist);
                        ui.horizontal(|ui| {
                            theme::folder_icon(ui, accent());
                            ui.add(egui::Label::new(RichText::new(leaf).small().color(accent())).truncate()).on_hover_text(&e.setlist);
                        });
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
                    ui.add(egui::Label::new(RichText::new(names.join(" · ")).small().color(muted())).wrap());
                    if !e.tags.is_empty() {
                        let tags: Vec<String> = e.tags.iter().take(5).map(|t| format!("#{t}")).collect();
                        ui.add(egui::Label::new(RichText::new(tags.join("  ")).small().color(accent())).truncate());
                    }
                });
            });
        let resp = r.response.interact(theme::list_row_sense());
        crate::theme::focus_ring(ui, &resp, 10);
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                true,
                format!("{}, {}", e.name, if e.pedal_file { "pedal preset" } else { "hlx preset" }),
            )
        });
        theme::row_payload(&resp, vec![e.path.clone()]);
        if resp.double_clicked() {
            actions.push(Action::Open(e.path.clone()));
        }
        let resp = resp.on_hover_text(if theme::touch() {
            "Double-tap to open, press and hold for more, press, hold and move onto a folder to move"
        } else {
            "Double-click to open, drag onto a folder to move, right-click for more"
        });
        theme::row_menu(&resp, |ui| self.menu(ui, e, can_send, actions));
    }

    fn row(&self, ui: &mut egui::Ui, e: &Entry, can_send: bool, actions: &mut Vec<Action>) {
        let r = ui.horizontal(|ui| {
            if e.pedal_file {
                theme::pill(ui, "PEDAL", orange());
            } else {
                theme::pill(ui, "HLX", blue());
            }
            ui.label(RichText::new(&e.name).strong());
            if !e.setlist.is_empty() {
                ui.label(RichText::new(&e.setlist).small().color(accent()));
            }
            let names: Vec<&str> = e.blocks.iter().take(6).map(|b| b.0.as_str()).collect();
            ui.label(RichText::new(names.join(" · ")).small().color(muted()));
            if !e.tags.is_empty() {
                let tags: Vec<String> = e.tags.iter().take(5).map(|t| format!("#{t}")).collect();
                ui.label(RichText::new(tags.join("  ")).small().color(accent()));
            }
        });
        let resp = r.response.interact(theme::list_row_sense());
        crate::theme::focus_ring(ui, &resp, 10);
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                true,
                format!("{}, {}", e.name, if e.pedal_file { "pedal preset" } else { "hlx preset" }),
            )
        });
        theme::row_payload(&resp, vec![e.path.clone()]);
        if resp.double_clicked() {
            actions.push(Action::Open(e.path.clone()));
        }
        theme::row_menu(&resp, |ui| self.menu(ui, e, can_send, actions));
        ui.separator();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &Path, name: &str, snapshots: &[&str]) -> Entry {
        Entry {
            path: path.to_path_buf(),
            name: name.into(),
            setlist: String::new(),
            pedal_file: false,
            blocks: vec![("Scream 808".into(), Color32::WHITE)],
            modified: None,
            snapshots: snapshots.iter().map(|s| s.to_string()).collect(),
            tags: Vec::new(),
        }
    }

    #[test]
    fn tags_are_kept_in_the_library_folder_and_found_by_search() {
        let dir = std::env::temp_dir().join(format!("tonestrand-tags-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b) = (dir.join("Blues.hlx"), dir.join("Lead.hlx"));
        let mut lib = Library::new(dir.clone());
        lib.entries = vec![entry(&a, "Blues", &["Clean", "Crunch"]), entry(&b, "Lead", &["Solo"])];
        lib.set_tags(&a, vec!["#Gig".into(), " gig ".into(), "".into(), "Church".into()]).unwrap();
        assert_eq!(lib.entries[0].tags, vec!["Gig", "Church"], "tidied and without repeats");
        assert_eq!(lib.all_tags(), vec!["Church", "Gig"]);
        // Found by tag, by #tag and by snapshot name; "#" is exact.
        lib.search = "#gig".into();
        assert!(lib.matches(&lib.entries[0]) && !lib.matches(&lib.entries[1]));
        lib.search = "#gi".into();
        assert!(!lib.matches(&lib.entries[0]));
        lib.search = "church".into();
        assert!(lib.matches(&lib.entries[0]));
        lib.search = "solo".into();
        assert!(!lib.matches(&lib.entries[0]) && lib.matches(&lib.entries[1]));
        // They survive a new scan of the folder, and follow the file when it moves.
        assert_eq!(lib.load_tags().get("blues.hlx").map(Vec::len), Some(2));
        std::fs::create_dir_all(dir.join("Gigs")).unwrap();
        assert_eq!(tag_key(&dir.join("Gigs").join("Blues.hlx")), tag_key(&a));
        lib.set_tags(&a, Vec::new()).unwrap();
        assert!(lib.load_tags().is_empty() && lib.entries[0].tags.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
