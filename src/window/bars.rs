//! The top bars (Classic and Studio), the status bar and the left panel.

use super::*;

impl App {
    pub(super) fn top_bar(&mut self, ctx: &egui::Context) {
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
                        (Some(n), _) => theme::pill(ui, n, GOOD),
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
    pub(super) fn auto_pick(&mut self) {
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
    pub(super) fn new_preset(&mut self) {
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

    pub(super) fn open_file_dialog(&mut self) {
        if let Some(p) = rfd::FileDialog::new().set_directory(&self.library_dir).add_filter("Presets", &["hlx", "hxpreset"]).pick_file() {
            self.open_file(&p);
        }
    }

    /// The Studio look's top bar: the app and its pages, the open preset's name, undo and
    /// save, its snapshots, BPM and DSP; under it a slim row with the library and pedal
    /// actions, the accent colour, the unit and connection, New preset and Open file.
    pub(super) fn studio_top_bar(&mut self, ctx: &egui::Context) {
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
    pub(super) fn studio_preset_bar(&mut self, ui: &mut egui::Ui, i: usize) {
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
    pub(super) fn studio_strip(&mut self, ui: &mut egui::Ui, i: usize) {
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
    pub(super) fn studio_meters(&mut self, ui: &mut egui::Ui, i: usize) {
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

    pub(super) fn status_bar(&mut self, ctx: &egui::Context) {
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

    pub(super) fn left_panel(&mut self, ctx: &egui::Context) {
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
}
