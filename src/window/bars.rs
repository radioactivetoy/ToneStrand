//! The top bars, the status bar and the left panel.

use super::*;

impl App {
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

    /// "Open file..." and the list of recent files beside it (for a right-to-left bar: the
    /// list comes out to the left of the button).
    pub(super) fn open_controls(&mut self, ui: &mut egui::Ui) {
        let recent = self.settings.recent.clone();
        let mut open = None;
        let mut clear = false;
        let mut compare = false;
        let mut dialog = false;
        ui.menu_button("Open", |ui| {
            if ui.button("Open file...  (Ctrl+O)").on_hover_text("Open a preset file").clicked() {
                dialog = true;
                ui.close_menu();
            }
            if ui
                .button("Compare two presets...")
                .on_hover_text("See what differs between two presets: effects, places, on/off and knob values")
                .clicked()
            {
                compare = true;
                ui.close_menu();
            }
            if !recent.is_empty() {
                ui.separator();
                ui.label(RichText::new("Recent").small().color(muted()));
                for p in &recent {
                    let path = Path::new(p);
                    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or(p);
                    if ui.button(name).on_hover_text(p).clicked() {
                        open = Some(p.clone());
                        ui.close_menu();
                    }
                }
                ui.separator();
                if ui.button("Clear this list").clicked() {
                    clear = true;
                    ui.close_menu();
                }
            }
        })
        .response
        .on_hover_text("Open a preset file, or one you opened lately (Ctrl+O)");
        if dialog {
            self.open_file_dialog();
        }
        if compare {
            self.open_compare();
        }
        if let Some(p) = open {
            let path = PathBuf::from(&p);
            if path.is_file() {
                self.view = View::Editor;
                self.open_file(&path);
            } else {
                self.settings.recent.retain(|r| *r != p);
                self.settings.save();
                self.note(LogLevel::Bad, format!("{p} is not there any more"));
            }
        }
        if clear {
            self.settings.recent.clear();
            self.settings.save();
        }
    }

    pub(super) fn open_file_dialog(&mut self) {
        if let Some(p) = rfd::FileDialog::new().set_directory(&self.library_dir).add_filter("Presets", &["hlx", "hxpreset"]).pick_file() {
            self.open_file(&p);
        }
    }

    /// The top bar: the app and its pages, the open preset's name, undo and
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
                        (Some(n), _) => (good(), n.clone()),
                        (None, true) => (orange(), "Connecting...".to_owned()),
                        _ => (muted(), format!("{} · not connected", crate::units::unit_name(self.unit))),
                    };
                    // A status light (painted: the font has no round bullet), then the menu.
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(12.0), egui::Sense::hover());
                    ui.painter().circle_filled(rect.center(), 4.5, dot);
                    let label = RichText::new(text.clone()).color(if self.connected.is_some() { ink() } else { muted() });
                    let r = ui.menu_button(label, |ui| {
                        ui.set_min_width(340.0);
                        self.devices_card(ui, ctx);
                    });
                    r.response.on_hover_text("Connect to the pedal (USB or Helrig Box)");
                    // The pill only when there is room left for it and the controls on the right.
                    if let (Some((_, idx, name)), true) = (&self.current, ui.available_width() > 620.0) {
                        ui.add_space(8.0);
                        theme::pill(ui, &format!("playing {}  {}", slot_label((*idx).max(0) as usize), name), accent());
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        self.open_controls(ui);
                        if ui
                            .button("New preset")
                            .on_hover_text(format!("An empty preset for the {} (Ctrl+N)", crate::units::unit_name(self.unit)))
                            .clicked()
                        {
                            self.new_preset();
                        }
                        let mut pick = None;
                        let locked = self.connected.is_some();
                        if locked {
                            // Connected: the pedal decides, so plain text, not a greyed-out menu.
                            ui.label(RichText::new(crate::units::unit_name(self.unit)).strong())
                                .on_hover_text("Detected from the connected pedal");
                        } else {
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
                                .on_hover_text("New presets are made for this unit and opened presets are converted to it");
                        }
                        if let Some(d) = pick {
                            self.set_unit(d);
                        }
                        ui.label(RichText::new("Unit").color(muted()));
                    });
                });
            });
    }

    /// The open preset's own bar, under its tab: name, undo, save, library,
    /// pedal actions, snapshots, BPM and DSP.
    pub(super) fn studio_preset_bar(&mut self, ui: &mut egui::Ui, i: usize) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            self.doc_bar(ui, i, 1);
            ui.add(egui::Separator::default().vertical().spacing(10.0));
            self.doc_bar(ui, i, 2);
            // The snapshot chips are one block the row cannot split: start a new row when
            // they would run past the edge (a live preset has the extra "Save on pedal").
            let room = ui.ctx().screen_rect().right() - 14.0 - ui.cursor().min.x;
            if room < 300.0 {
                ui.end_row();
            } else {
                ui.add(egui::Separator::default().vertical().spacing(10.0));
            }
            self.studio_strip(ui, i);
            ui.add(egui::Separator::default().vertical().spacing(10.0));
            self.studio_meters(ui, i);
        });
    }

    /// The top bar's snapshot chips for the open preset.
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

    /// BPM and the DSP meter of the open preset (right side of the top bar).
    pub(super) fn studio_meters(&mut self, ui: &mut egui::Ui, i: usize) {
        let cat = self.catalog.clone();
        let Some(doc) = self.docs.get_mut(i) else { return };
        let before = doc.kind.clone();
        // Each path: blocks used and its DSP, stacked one above the other when there are two.
        let mut paths = doc.kind.doc().path_meters(cat.as_deref());
        // A second pedal path with nothing in it isn't shown.
        if paths.len() > 1 && matches!(doc.kind, DocKind::Device { .. }) {
            paths.retain(|p| p.0 > 0 || p.2.is_some_and(|d| d > 0.5));
            if paths.is_empty() {
                paths.push((0, 8, None));
            }
        }
        let bpm = doc.kind.doc().tempo();
        if let Some(mut t) = bpm {
            ui.label(RichText::new("BPM").color(muted()));
            if ui.add(egui::DragValue::new(&mut t).range(30.0..=240.0).speed(0.5).fixed_decimals(1)).changed() {
                doc.kind.doc_mut().set_tempo(t);
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
                    ui.label(RichText::new(label).small().strong().color(muted()));
                    let bcol = if count >= max { orange() } else { muted() };
                    ui.label(RichText::new(format!("{count}/{max} blocks")).small().monospace().color(bcol));
                    if let Some(d) = used {
                        let col = if *d > 100.0 {
                            bad()
                        } else if *d > 85.0 {
                            orange()
                        } else {
                            accent()
                        };
                        ui.label(RichText::new("DSP").small().color(muted())).on_hover_text("Estimated DSP use of this path");
                        let (rect, _) = ui.allocate_exact_size(Vec2::new(80.0, if n > 1 { 5.0 } else { 6.0 }), egui::Sense::hover());
                        ui.painter().rect_filled(rect, CornerRadius::same(3), card_hi());
                        let fill = egui::Rect::from_min_size(
                            rect.min,
                            Vec2::new(rect.width() * (*d / 100.0).clamp(0.0, 1.0) as f32, rect.height()),
                        );
                        ui.painter().rect_filled(fill, CornerRadius::same(3), col);
                        ui.label(RichText::new(format!("{d:.0}%")).small().monospace().strong().color(col));
                    }
                });
            }
        });
        self.track_history(i, before, None);
    }

    pub(super) fn status_bar(&mut self, ctx: &egui::Context) {
        let mut copied = false;
        let mut save = false;
        let mut report: Option<bool> = None;
        let mut seen = false;
        let mut help = false;
        let unseen = self.errors_unseen;
        egui::TopBottomPanel::bottom("status").frame(egui::Frame::new().fill(bg()).inner_margin(egui::Margin::symmetric(14, 6))).show(
            ctx,
            |ui| {
                ui.horizontal(|ui| {
                    if let Some(b) = &self.busy {
                        ui.add(egui::Spinner::new().size(14.0).color(accent()));
                        ui.label(RichText::new(b).color(accent()));
                        ui.separator();
                    }
                    if let Some((lvl, msg)) = self.log.back() {
                        ui.label(RichText::new(msg).color(level_colour(*lvl, muted())));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Problems since the log was last opened turn the button red, so they are not missed.
                        let label = if unseen > 0 {
                            RichText::new(format!("Activity ({unseen} problem{})", if unseen == 1 { "" } else { "s" })).color(bad())
                        } else {
                            RichText::new("Activity")
                        };
                        ui.menu_button(label, |ui| {
                            seen = true;
                            ui.set_min_width(440.0);
                            ui.horizontal(|ui| {
                                if ui.small_button("Copy").on_hover_text("Copy the log, for a bug report").clicked() {
                                    ui.ctx().copy_text(self.log_text());
                                    copied = true;
                                    ui.close_menu();
                                }
                                if ui.small_button("Save...").on_hover_text("Save the log as a text file").clicked() {
                                    save = true;
                                    ui.close_menu();
                                }
                                if ui
                                    .small_button("Problem report")
                                    .on_hover_text(
                                        "Save a folder with the log, the version, the unit and what is checked on it (in Downloads)",
                                    )
                                    .clicked()
                                {
                                    report = Some(false);
                                    ui.close_menu();
                                }
                                if ui
                                    .small_button("Report with preset")
                                    .on_hover_text("The same, with the open preset as a file in the folder")
                                    .clicked()
                                {
                                    report = Some(true);
                                    ui.close_menu();
                                }
                            });
                            egui::ScrollArea::vertical().max_height(320.0).stick_to_bottom(true).show(ui, |ui| {
                                for (lvl, msg) in &self.log {
                                    ui.label(RichText::new(msg).color(level_colour(*lvl, ink())));
                                }
                            });
                        });
                        if ui.button("Shortcuts").on_hover_text("The keyboard shortcuts (F1)").clicked() {
                            help = true;
                        }
                    });
                });
            },
        );
        if seen {
            self.errors_unseen = 0;
        }
        if help {
            self.show_shortcuts = !self.show_shortcuts;
        }
        if copied {
            self.note(LogLevel::Good, "Log copied");
        }
        if let Some(with_preset) = report {
            self.save_report(with_preset);
        }
        if save {
            self.save_log();
        }
    }

    /// The activity log as plain text, with the program version and the pedal, for a bug report.
    fn log_text(&self) -> String {
        let mut out =
            format!("ToneStrand {}\nPedal: {}\n\n", env!("CARGO_PKG_VERSION"), self.connected.as_deref().unwrap_or("not connected"));
        for (_, msg) in &self.log {
            out.push_str(msg);
            out.push('\n');
        }
        out
    }

    /// The first time a unit is connected: what has been tried on it and how to report a problem.
    pub(super) fn unit_notice_window(&mut self, ctx: &egui::Context) {
        let Some(device) = self.unit_notice else { return };
        let mut open = true;
        let mut report = false;
        egui::Window::new(format!("Connected to {}", crate::units::unit_name(device)))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_max_width(460.0);
                if crate::support::checked_unit(device) {
                    ui.label("This unit has been tried on a real pedal. Everything below was checked on it:");
                } else {
                    ui.label(
                        RichText::new("This unit has not been tried on a real pedal yet.").color(orange()).strong(),
                    );
                    ui.label(
                        "Everything for it is built from the documented layouts and from the HX Stomp's behaviour. It should work, but check what you write to the pedal, and keep a backup.",
                    );
                }
                ui.add_space(6.0);
                for a in crate::support::areas(device) {
                    let (mark, colour) = match a.level {
                        crate::support::Level::Checked => ("✔", good()),
                        crate::support::Level::Documented => ("○", muted()),
                    };
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(mark).color(colour));
                        ui.label(a.name);
                    });
                }
                ui.add_space(8.0);
                ui.label(
                    RichText::new("If something differs on your unit, save a problem report (Activity menu, bottom right) and send it with what you did.")
                        .small()
                        .color(muted()),
                );
                ui.horizontal(|ui| {
                    if ui.button("Save a problem report...").clicked() {
                        report = true;
                    }
                });
            });
        if report {
            self.save_report(false);
        }
        if !open {
            self.unit_notice = None;
        }
    }

    /// A folder with a report for a problem: the log, the version, the unit and what has been checked on it, and,
    /// when asked, the open preset. Saved in the Downloads folder (or the library); nothing is sent anywhere.
    pub(super) fn save_report(&mut self, with_preset: bool) {
        let base = dirs::download_dir().unwrap_or_else(|| self.library_dir.clone());
        let dir = base.join(format!("ToneStrand report {}", crate::support::stamp(std::time::SystemTime::now())));
        let mut text = format!(
            "ToneStrand {}\nComputer: {} {}\nUnit set up for: {}\nPedal: {}\n\n{}\n",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH,
            crate::units::unit_name(self.unit),
            self.connected.as_deref().unwrap_or("not connected"),
            crate::support::summary(self.unit),
        );
        text.push_str("Activity log:\n");
        for (_, msg) in &self.log {
            text.push_str(msg);
            text.push('\n');
        }
        let result = (|| -> Result<Vec<String>, String> {
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let mut files = vec!["report.txt".to_owned()];
            crate::files::write_atomic(&dir.join("report.txt"), text.as_bytes()).map_err(|e| e.to_string())?;
            if with_preset {
                if let Some(d) = self.docs.get(self.active) {
                    match &d.kind {
                        DocKind::Device { preset, .. } => {
                            crate::files::write_atomic(&dir.join("preset.hxpreset"), &preset.encode()).map_err(|e| e.to_string())?;
                            files.push("preset.hxpreset".into());
                        }
                        DocKind::Hlx { json } => {
                            let s = serde_json::to_string_pretty(json).map_err(|e| e.to_string())?;
                            crate::files::write_atomic(&dir.join("preset.hlx"), s.as_bytes()).map_err(|e| e.to_string())?;
                            files.push("preset.hlx".into());
                        }
                    }
                }
            }
            Ok(files)
        })();
        match result {
            Ok(files) => self.note(LogLevel::Good, format!("Saved a problem report ({}) in {}", files.join(", "), dir.display())),
            Err(e) => self.note(LogLevel::Bad, format!("Could not save the report: {e}")),
        }
    }

    /// "Save..." in the Activity list: the log as a text file.
    fn save_log(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_directory(&self.library_dir)
            .set_file_name("ToneStrand log.txt")
            .add_filter("Text", &["txt"])
            .save_file()
        else {
            return;
        };
        match crate::files::write_atomic(&path, self.log_text().as_bytes()) {
            Ok(_) => self.note(LogLevel::Good, format!("Saved the log to {}", path.display())),
            Err(e) => self.note(LogLevel::Bad, format!("Could not save the log: {e}")),
        }
    }

    pub(super) fn left_panel(&mut self, ctx: &egui::Context) {
        // The pedal's preset list is for the editor, or whenever a pedal is connected. Library,
        // IRs and Settings without a pedal have no use for a panel that only says "connect".
        if self.connected.is_none() && self.view != View::Editor {
            self.left_rect = egui::Rect::NOTHING;
            return;
        }
        let shown = egui::SidePanel::left("left")
            .resizable(true)
            .default_width(330.0)
            .min_width(280.0)
            .frame(egui::Frame::new().fill(panel()).inner_margin(egui::Margin::symmetric(10, 10)).stroke(Stroke::new(1.0_f32, line())))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    // Connecting lives in the top bar; this side is for presets.
                    ui.add_space(8.0);
                    self.pedal_card(ui);
                });
            });
        self.left_rect = shown.response.rect;
    }
}
