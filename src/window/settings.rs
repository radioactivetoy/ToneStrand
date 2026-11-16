//! The Settings page.

use super::*;

impl App {
    pub(super) fn settings_page(&mut self, ctx: &egui::Context) {
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
                        ui.horizontal(|ui| {
                            changed |= ui.checkbox(&mut self.settings.auto_reconnect, "Connect again when a USB pedal is plugged back in").changed();
                            theme::help(ui, "After the pedal drops out (unplugged, or it stopped answering and then left the USB list), ToneStrand connects to it once when it reappears. It doesn't do this after you press Disconnect, or for a Helrig Box.");
                        });
                        ui.horizontal(|ui| {
                            changed |= ui.checkbox(&mut self.settings.reopen_tabs, "Reopen last session's presets at start").changed();
                            theme::help(ui, "Opens the preset files that were open when you last closed ToneStrand. Presets read from the pedal aren't included: they have no file.");
                        });
                    });
                    ui.add_space(10.0);

                    // ---- Look.
                    let mut names_changed = false;
                    theme::card(ui, |ui| {
                        theme::section(ui, "Display");
                        ui.horizontal(|ui| {
                            ui.label("Accent colour");
                            for (rgb, name) in theme::ACCENTS.iter() {
                                let col = egui::Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, *rgb as u8);
                                let (rect, r) = ui.allocate_exact_size(egui::Vec2::splat(20.0), egui::Sense::click());
                                ui.painter().circle_filled(rect.center(), 8.0, col);
                                if theme::accent_rgb() == *rgb {
                                    ui.painter().circle_stroke(rect.center(), 10.0, egui::Stroke::new(1.5_f32, theme::TEXT));
                                }
                                if r.on_hover_text(*name).clicked() && theme::accent_rgb() != *rgb {
                                    theme::set_accent(*rgb);
                                    theme::apply(ui.ctx());
                                    self.settings.accent = *rgb;
                                    changed = true;
                                }
                            }
                            theme::help(ui, "The colour of selected items, buttons and the signal chain's highlights.");
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
                            theme::help(ui, "Real gear names show what each effect is modelled on, e.g. \"Klon Centaur\" instead of \"Minotaur\". Hover an effect in the picker to see the other name. Effects with no real-world model keep the pedal's name. These names are trademarks of their owners, who have nothing to do with Line 6 or ToneStrand; they are used only to describe the sound.");
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
}
