//! "Compare two presets": pick two open presets (or files) and see what differs.

use super::*;
use crate::compare::{self, Change, Line, Summary};

/// One side of the comparison: a name for it and what it holds.
struct Pick {
    label: String,
    summary: Summary,
}

pub(super) struct CompareState {
    a: Option<Pick>,
    b: Option<Pick>,
}

impl App {
    /// The open preset in tab `i`, read for comparing.
    fn pick_tab(&self, i: usize, cat: &Catalog) -> Option<Pick> {
        let d = self.docs.get(i)?;
        let summary = match &d.kind {
            DocKind::Hlx { json } => compare::from_hlx(json, cat),
            DocKind::Device { preset, .. } => compare::from_device(preset, cat),
        };
        Some(Pick { label: d.title.clone(), summary })
    }

    /// A preset file, read for comparing.
    fn pick_file(&mut self, path: &Path, cat: &Catalog) -> Option<Pick> {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        let summary = if ext == "hxpreset" {
            std::fs::read(path).map_err(|e| e.to_string()).and_then(|b| DevicePreset::parse(&b)).map(|p| compare::from_device(&p, cat))
        } else {
            std::fs::read_to_string(path).map_err(|e| e.to_string()).and_then(|t| crate::hlx::parse_text(&t)).and_then(|j| {
                if j.pointer("/data/tone").is_some() {
                    Ok(compare::from_hlx(&j, cat))
                } else {
                    Err("not a Helix preset".into())
                }
            })
        };
        match summary {
            Ok(summary) => Some(Pick { label: path.file_stem().and_then(|s| s.to_str()).unwrap_or("file").to_owned(), summary }),
            Err(e) => {
                self.note(LogLevel::Bad, format!("Cannot read {}: {e}", path.display()));
                None
            }
        }
    }

    /// A file picked for one side of the compare window.
    pub(super) fn set_compare_side(&mut self, side: usize, path: &Path, cat: &Catalog) {
        let picked = self.pick_file(path, cat);
        if let (Some(st), Some(picked)) = (self.compare.as_mut(), picked) {
            *(if side == 0 { &mut st.a } else { &mut st.b }) = Some(picked);
        }
    }

    pub(super) fn open_compare(&mut self) {
        let Some(cat) = self.catalog.clone() else {
            return self.note(LogLevel::Bad, "Comparing needs HX Edit's effect list (see Settings).");
        };
        let a = self.pick_tab(self.active, &cat);
        let b = (0..self.docs.len()).find(|i| *i != self.active).and_then(|i| self.pick_tab(i, &cat));
        self.compare = Some(CompareState { a, b });
    }

    pub(super) fn compare_window(&mut self, ctx: &egui::Context) {
        let Some(mut st) = self.compare.take() else { return };
        let Some(cat) = self.catalog.clone() else { return };
        let mut open = true;
        let (mut new_a, mut new_b): (Option<Pick>, Option<Pick>) = (None, None);
        let mut file_for: Option<usize> = None;
        let titles: Vec<String> = self.docs.iter().map(|d| d.title.clone()).collect();
        egui::Window::new("Compare two presets")
            .open(&mut open)
            .collapsible(false)
            .default_width(600.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    for side in 0..2 {
                        let pick = if side == 0 { &st.a } else { &st.b };
                        ui.vertical(|ui| {
                            ui.label(RichText::new(if side == 0 { "From" } else { "To" }).small().color(muted()));
                            egui::ComboBox::from_id_salt(("compare-side", side))
                                .width(250.0)
                                .selected_text(pick.as_ref().map(|p| p.label.clone()).unwrap_or_else(|| "Pick a preset".into()))
                                .show_ui(ui, |ui| {
                                    for (i, t) in titles.iter().enumerate() {
                                        if ui.selectable_label(false, t).clicked() {
                                            *(if side == 0 { &mut new_a } else { &mut new_b }) = self.pick_tab(i, &cat);
                                        }
                                    }
                                    ui.separator();
                                    if ui.button("A file...").clicked() {
                                        file_for = Some(side);
                                        ui.close_menu();
                                    }
                                });
                        });
                    }
                });
                ui.add_space(6.0);
                match (&st.a, &st.b) {
                    (Some(a), Some(b)) => {
                        let lines: Vec<Line> = compare::diff(&a.summary, &b.summary);
                        if lines.is_empty() {
                            ui.label(
                                RichText::new("These two are the same: the same effects, places, on/off and knob values.").color(good()),
                            );
                        } else {
                            ui.label(
                                RichText::new(format!("{} difference{}", lines.len(), if lines.len() == 1 { "" } else { "s" })).strong(),
                            );
                        }
                        egui::ScrollArea::vertical().max_height(380.0).auto_shrink([true, true]).show(ui, |ui| {
                            for l in &lines {
                                let (mark, colour) = match l.change {
                                    Change::Added => ("+", good()),
                                    Change::Removed => ("−", bad()),
                                    Change::Changed => ("~", orange()),
                                    Change::Note => ("•", muted()),
                                };
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(RichText::new(mark).monospace().strong().color(colour));
                                    ui.label(&l.text);
                                });
                            }
                        });
                    }
                    _ => {
                        ui.label(RichText::new("Pick two presets: open tabs, or files.").color(muted()));
                    }
                }
            });
        if let Some(side) = file_for {
            let id =
                crate::platform::FileDialog::new().set_directory(&self.library_dir).add_filter("Presets", &["hlx", "hxpreset"]).open_file();
            self.start_pick(id, picks::Pick::ComparePick(side));
        }
        if let Some(na) = new_a {
            st.a = Some(na);
        }
        if let Some(nb) = new_b {
            st.b = Some(nb);
        }
        if open {
            self.compare = Some(st);
        }
    }
}
