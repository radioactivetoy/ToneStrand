//! Folders on disk, shared by the preset library and the IR / NAM library:
//! every folder is a real folder under the library's root (nested allowed),
//! and a "setlist" is just a folder, since the HX Stomp has no setlists of its own.

use std::path::{Path, PathBuf};

use eframe::egui::{self, RichText};

use crate::theme::{self, *};

/// Folders under `root`, as paths relative to it with '/' between levels, sorted,
/// at most `depth` levels deep, leaving out hidden folders and those in `skip`.
pub fn scan(root: &Path, skip: &[PathBuf], depth: usize) -> Vec<String> {
    fn walk(root: &Path, dir: &Path, skip: &[PathBuf], depth: usize, out: &mut Vec<String>) {
        if depth == 0 {
            return;
        }
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if !p.is_dir() || name.starts_with('.') || skip.iter().any(|s| same(&p, s)) {
                continue;
            }
            if let Ok(rel) = p.strip_prefix(root) {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
            walk(root, &p, skip, depth - 1, out);
        }
    }
    let mut out = Vec::new();
    walk(root, root, skip, depth, &mut out);
    out.sort_by_key(|s| s.to_lowercase());
    out
}

fn same(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// The folder a file sits in, relative to `root` ("" at the top).
pub fn folder_of(root: &Path, file: &Path) -> String {
    file.parent().and_then(|p| p.strip_prefix(root).ok()).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_default()
}

pub fn open_in_explorer(dir: &Path) {
    let _ = std::fs::create_dir_all(dir);
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("explorer").arg(dir).spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(dir).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
}

/// Move a file into a folder of the library, keeping its name (adding " (2)" if taken).
pub fn move_into(file: &Path, dir: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let name = file.file_name().ok_or("no file name")?.to_string_lossy().into_owned();
    if dir.join(&name) == file {
        return Ok(file.to_owned());
    }
    let to = crate::files::unique_path(dir, &name);
    std::fs::rename(file, &to).map_err(|e| e.to_string())?;
    Ok(to)
}

/// State of the folder list on the left of a library page.
#[derive(Default)]
pub struct FolderPane {
    pub new_name: String,
    /// Some(parent) while typing a new folder's name.
    pub adding: Option<String>,
    /// (folder, new name) while renaming.
    pub renaming: Option<(String, String)>,
    pub error: Option<String>,
}

/// What the person did in the folder list.
pub enum FolderEvent {
    /// Files dropped onto a folder ("" = the top).
    Dropped(Vec<PathBuf>, String),
    /// Folders were made, renamed or removed: look again.
    Changed,
}

/// The folder list: "All", then each folder (indented by depth) with a count,
/// a menu (new folder inside, rename, show in Explorer, remove when empty), and
/// files dropped on it. `current` is None for "All", Some("") for the top folder.
pub fn pane(
    ui: &mut egui::Ui,
    pane: &mut FolderPane,
    root: &Path,
    folders: &[String],
    current: &mut Option<String>,
    count: &dyn Fn(Option<&str>) -> usize,
    all_label: &str,
) -> Vec<FolderEvent> {
    let mut out = Vec::new();
    let events = &mut out;
    let row = |ui: &mut egui::Ui, label: String, selected: bool, depth: usize| -> egui::Response {
        ui.horizontal(|ui| {
            ui.add_space(depth as f32 * 14.0);
            ui.selectable_label(selected, label)
        })
        .inner
    };
    let drop_target = |r: &egui::Response, folder: &str, events: &mut Vec<FolderEvent>| {
        if r.dnd_hover_payload::<Vec<PathBuf>>().is_some() {
            ui_paint_drop(r);
        }
        if let Some(p) = r.dnd_release_payload::<Vec<PathBuf>>() {
            events.push(FolderEvent::Dropped((*p).clone(), folder.to_owned()));
        }
    };
    if row(ui, format!("{all_label}   {}", count(None)), current.is_none(), 0).clicked() {
        *current = None;
    }
    let top = row(ui, format!("📁 Top folder   {}", count(Some(""))), current.as_deref() == Some(""), 0);
    if top.clicked() {
        *current = Some(String::new());
    }
    drop_target(&top, "", events);
    top.context_menu(|ui| {
        if ui.button("New folder here").clicked() {
            pane.adding = Some(String::new());
            ui.close_menu();
        }
        if ui.button("Show in Explorer").clicked() {
            open_in_explorer(root);
            ui.close_menu();
        }
    });
    for f in folders {
        let depth = f.matches('/').count() + 1;
        let leaf = f.rsplit('/').next().unwrap_or(f);
        if pane.renaming.as_ref().is_some_and(|(r, _)| r == f) {
            let mut done = None;
            ui.horizontal(|ui| {
                ui.add_space(depth as f32 * 14.0);
                let (_, name) = pane.renaming.as_mut().unwrap();
                let r = ui.add(egui::TextEdit::singleline(name).desired_width(120.0));
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ui.small_button("✔").clicked() || enter {
                    done = Some(true);
                }
                if ui.small_button("✖").clicked() {
                    done = Some(false);
                }
            });
            match done {
                Some(true) => {
                    let new = crate::device::sanitize(&pane.renaming.as_ref().unwrap().1);
                    let from = root.join(f);
                    let to = from.parent().unwrap_or(root).join(&new);
                    match std::fs::rename(&from, &to) {
                        Ok(_) => {
                            let parent = f.rsplit_once('/').map(|(p, _)| format!("{p}/")).unwrap_or_default();
                            if current.as_deref() == Some(f.as_str()) {
                                *current = Some(format!("{parent}{new}"));
                            }
                            events.push(FolderEvent::Changed);
                        }
                        Err(e) => pane.error = Some(format!("Could not rename: {e}")),
                    }
                    pane.renaming = None;
                }
                Some(false) => pane.renaming = None,
                None => {}
            }
            continue;
        }
        let r = row(ui, format!("📁 {leaf}   {}", count(Some(f))), current.as_deref() == Some(f.as_str()), depth);
        if r.clicked() {
            *current = Some(f.clone());
        }
        drop_target(&r, f, events);
        r.context_menu(|ui| {
            if ui.button("New folder inside").clicked() {
                pane.adding = Some(f.clone());
                ui.close_menu();
            }
            if ui.button("Rename").clicked() {
                pane.renaming = Some((f.clone(), leaf.to_owned()));
                ui.close_menu();
            }
            if ui.button("Show in Explorer").clicked() {
                open_in_explorer(&root.join(f));
                ui.close_menu();
            }
            ui.separator();
            if ui.button(RichText::new("Remove (only when empty)").color(BAD)).clicked() {
                match std::fs::remove_dir(root.join(f)) {
                    Ok(_) => {
                        if current.as_deref() == Some(f.as_str()) {
                            *current = None;
                        }
                        events.push(FolderEvent::Changed);
                    }
                    Err(_) => pane.error = Some(format!("\"{leaf}\" isn't empty: move or delete what's in it first.")),
                }
                ui.close_menu();
            }
        });
    }
    ui.add_space(4.0);
    if let Some(parent) = pane.adding.clone() {
        let r = ui.add(egui::TextEdit::singleline(&mut pane.new_name).hint_text(if parent.is_empty() {
            "Folder name".to_owned()
        } else {
            format!("Folder in {parent}")
        }));
        r.request_focus();
        ui.horizontal(|ui| {
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if theme::primary_button(ui, "Create").clicked() || enter {
                let name = crate::device::sanitize(&pane.new_name);
                let rel = if parent.is_empty() { name } else { format!("{parent}/{name}") };
                match std::fs::create_dir_all(root.join(&rel)) {
                    Ok(_) => {
                        *current = Some(rel);
                        events.push(FolderEvent::Changed);
                    }
                    Err(e) => pane.error = Some(format!("Could not make the folder: {e}")),
                }
                pane.new_name.clear();
                pane.adding = None;
            }
            if ui.button("Cancel").clicked() {
                pane.adding = None;
            }
        });
    } else if ui.button("+ New folder").on_hover_text("Inside the selected folder").clicked() {
        pane.adding = Some(current.clone().unwrap_or_default());
    }
    if let Some(e) = pane.error.clone() {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(e).small().color(BAD));
            if ui.small_button("OK").clicked() {
                pane.error = None;
            }
        });
    }
    out
}

fn ui_paint_drop(r: &egui::Response) {
    r.ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("drop-hint"))).rect_stroke(
        r.rect.expand(2.0),
        egui::CornerRadius::same(6),
        egui::Stroke::new(2.0_f32, accent()),
        egui::StrokeKind::Outside,
    );
}
