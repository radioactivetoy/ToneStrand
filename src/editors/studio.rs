//! The Studio look's top-bar strips: snapshot chips, BPM and path meters.

use super::*;

/// One snapshot chip of the Studio top bar ("1 CLEAN"); `on` when it is the current one.
fn snap_chip(ui: &mut egui::Ui, n: usize, name: &str, on: bool) -> egui::Response {
    let label = if name.trim().is_empty() { format!("{n}") } else { format!("{n}  {}", name.trim()) };
    let text = RichText::new(label).monospace().size(12.5).color(if on { Color32::from_rgb(8, 22, 26) } else { TEXT });
    ui.add(egui::Button::new(text).fill(if on { accent() } else { Color32::TRANSPARENT }).stroke(Stroke::NONE).min_size(Vec2::new(0.0, 24.0)))
}

/// The Studio top bar's preset strip for a .hlx preset: BPM and the snapshot chips
/// (click to recall; right-click to rename, copy, paste, duplicate or delete; + adds one).
pub fn studio_strip_hlx(ui: &mut egui::Ui, json: &mut serde_json::Value, dirty: &mut bool) {
    let list = snapshots::list(json);
    let cur = snapshots::current(json);
    let fam = snapshots::family(json);
    let clip_id = egui::Id::new("tonestrand-snapshot-clipboard");
    let clip: Option<(String, serde_json::Value)> = ui.data(|d| d.get_temp(clip_id));
    let mut action: Option<(&str, usize)> = None;
    egui::Frame::new().fill(bg()).corner_radius(CornerRadius::same(7)).inner_margin(egui::Margin::same(2)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (i, name) in &list {
                let r = snap_chip(ui, i + 1, name, *i == cur).on_hover_text("Recall this snapshot. Right-click to rename, copy, paste, duplicate or delete.");
                if r.clicked() && *i != cur {
                    snapshots::select(json, *i);
                    *dirty = true;
                }
                r.context_menu(|ui| {
                    let mut n = name.clone();
                    ui.horizontal(|ui| {
                        ui.label("Name");
                        let r = ui.add(egui::TextEdit::singleline(&mut n).desired_width(110.0).char_limit(crate::session::SNAPSHOT_NAME_MAX));
                        theme::name_hint(ui, &n, crate::session::SNAPSHOT_NAME_MAX);
                        if r.changed() {
                            snapshots::rename(json, *i, &n);
                            *dirty = true;
                        }
                    });
                    ui.separator();
                    if ui.button("Copy").clicked() {
                        action = Some(("copy", *i));
                        ui.close_menu();
                    }
                    let paste = clip.as_ref().map(|(f, _)| format!("Paste {f} here")).unwrap_or_else(|| "Paste".into());
                    if ui.add_enabled(clip.is_some(), egui::Button::new(paste)).clicked() {
                        action = Some(("paste", *i));
                        ui.close_menu();
                    }
                    if ui.add_enabled(list.len() < fam.snapshot_count, egui::Button::new("Duplicate")).clicked() {
                        action = Some(("duplicate", *i));
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.add_enabled(list.len() > 1, egui::Button::new(RichText::new("Delete").color(BAD))).clicked() {
                        action = Some(("delete", *i));
                        ui.close_menu();
                    }
                });
            }
            if list.len() < fam.snapshot_count && ui.small_button("+").on_hover_text("Add a snapshot (a copy of the current one)").clicked() {
                if let Some(n) = snapshots::add(json) {
                    snapshots::select(json, n);
                    *dirty = true;
                }
            }
        });
    });
    match action {
        Some(("copy", i)) => {
            if let Some(v) = snapshots::copy(json, i) {
                let name = list.iter().find(|(x, _)| *x == i).map(|(_, n)| n.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| format!("Snapshot {}", i + 1));
                ui.data_mut(|d| d.insert_temp(clip_id, (name, v)));
            }
        }
        Some(("paste", i)) => {
            if let Some((_, v)) = &clip {
                snapshots::paste(json, i, v);
                *dirty = true;
            }
        }
        Some(("duplicate", i)) => {
            if i != cur {
                snapshots::select(json, i);
            }
            if let Some(n) = snapshots::add(json) {
                snapshots::select(json, n);
            }
            *dirty = true;
        }
        Some(("delete", i)) if snapshots::delete(json, i) => {
            *dirty = true;
        }
        _ => {}
    }
}

/// The Studio top bar's snapshot chips for a pedal preset: on the live preset a click
/// switches the pedal; right-click renames it or copies another snapshot into it.
pub fn studio_strip_device(ui: &mut egui::Ui, preset: &mut DevicePreset, dirty: &mut bool, live: bool, cmds: &mut Vec<Cmd>) {
    let names = preset.snapshot_names();
    if names.is_empty() {
        return;
    }
    let cur = preset.current_snapshot();
    egui::Frame::new().fill(bg()).corner_radius(CornerRadius::same(7)).inner_margin(egui::Margin::same(2)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (i, name) in names.iter().enumerate() {
                let tip = if live { "Switch the pedal to this snapshot. Right-click to rename it." } else { "Right-click to rename it or copy another snapshot into it." };
                let r = snap_chip(ui, i + 1, name, i == cur).on_hover_text(tip);
                if live && r.clicked() && i != cur {
                    cmds.push(Cmd::Snapshot(i as i64));
                }
                r.context_menu(|ui| {
                    let mut n = name.clone();
                    ui.horizontal(|ui| {
                        ui.label("Name");
                        let te = ui.add(egui::TextEdit::singleline(&mut n).desired_width(110.0).char_limit(crate::session::SNAPSHOT_NAME_MAX));
                        theme::name_hint(ui, &n, crate::session::SNAPSHOT_NAME_MAX);
                        if te.changed() {
                            preset.set_snapshot_name(i, &n);
                            *dirty = true;
                        }
                        if live && te.lost_focus() {
                            cmds.push(Cmd::RenameSnapshot { index: i as i64, name: n.clone() });
                        }
                    });
                    if !live {
                        ui.separator();
                        ui.label(RichText::new(format!("Make snapshot {} the same as", i + 1)).small().color(MUTED));
                        for j in (0..names.len()).filter(|j| *j != i) {
                            let from = names.get(j).filter(|s| !s.trim().is_empty()).cloned().unwrap_or_else(|| format!("Snapshot {}", j + 1));
                            if ui.button(format!("{}: {from}", j + 1)).clicked() {
                                if preset.copy_snapshot_states(j, i) {
                                    *dirty = true;
                                }
                                ui.close_menu();
                            }
                        }
                    }
                });
            }
        });
    });
}
