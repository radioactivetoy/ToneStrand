//! The hidden Tests page (Ctrl+Shift+T): the hardware self-test.

use super::*;

impl App {
    /// The hidden Tests page: a careful, self-restoring run of every pedal operation.
    pub(super) fn tests_page(&mut self, ctx: &egui::Context) {
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
                    theme::pill(ui, "HIDDEN · Ctrl+Shift+T", orange());
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
                        ui.label(RichText::new("Connect to a pedal first.").color(muted()));
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
                                        if let Some(l) = &self.link {
                                            let _ = l.tx.send(Cmd::ListIrs);
                                        }
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
                        ui.label(RichText::new("✔ Always (reading only): backup of the setlist and the test IR, double reads, preset list, IR list, one IR download.").small().color(muted()));
                        ui.checkbox(&mut self.tests.preset_writes, "Preset writes: write a test preset into the slot, verify, rename, verify, restore, verify");
                        ui.checkbox(&mut self.tests.live_edit, "Live edit: set one knob of the loaded preset to the value it already has, verify, reload the preset (drops unsaved changes on the pedal)");
                        ui.checkbox(&mut self.tests.ir_writes, "IR writes: upload a test IR, verify name and audio, rename, verify, restore, verify");
                        let writes = self.tests.preset_writes || self.tests.live_edit || self.tests.ir_writes;
                        if writes {
                            ui.checkbox(&mut self.tests.understood, RichText::new("I understand the chosen slots are changed for a moment and then restored from the backup").color(orange()));
                        }
                        let folder = self.tests.folder.clone().unwrap_or_else(|| self.library_dir.join("Self-test backups"));
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("Backups go to {}", folder.display())).small().color(muted()));
                            if ui.small_button("Change...").clicked() {
                                if let Some(d) = rfd::FileDialog::new().set_directory(&self.library_dir).pick_folder() {
                                    self.tests.folder = Some(d);
                                }
                            }
                        });
                        ui.add_space(6.0);
                        let ready = self.tests.preset_slot.is_some() && self.tests.ir_slot.is_some() && (!writes || self.tests.understood) && self.busy.is_none();
                        if ui.add_enabled(ready, egui::Button::new(RichText::new("▶ Run the self-test").strong().color(on_accent())).fill(accent())).clicked() {
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
                            ui.label(RichText::new(self.busy.clone().unwrap_or_default()).small().color(muted()));
                        }
                        match self.tests.done {
                            Some(0) => theme::pill(ui, "ALL PASSED", good()),
                            Some(n) => theme::pill(ui, &format!("{n} FAILED"), bad()),
                            None => {}
                        }
                    });
                    if self.tests.log.is_empty() {
                        ui.label(RichText::new("Nothing run yet.").color(muted()));
                    }
                    egui::ScrollArea::vertical().auto_shrink([false, true]).max_height(ui.available_height() - 8.0).stick_to_bottom(true).show(ui, |ui| {
                        for (name, ok, detail) in &self.tests.log {
                            ui.horizontal_wrapped(|ui| {
                                let (mark, col) = match ok {
                                    Some(true) => ("✔", good()),
                                    Some(false) => ("✖", bad()),
                                    None => ("•", muted()),
                                };
                                ui.label(RichText::new(mark).strong().color(col));
                                ui.label(RichText::new(name).strong());
                                ui.label(RichText::new(detail).color(if *ok == Some(false) { bad() } else { muted() }));
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
}
