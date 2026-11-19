//! The IRs page: the pedal's User IRs, uploads, NAM conversion, and the local IR / NAM library.

use super::*;

impl App {
    /// Pick .nam captures (then a folder, except on Android), write a User IR .wav for each,
    /// and show what happened.
    pub(super) fn convert_nam_files(&mut self) {
        let id = crate::platform::FileDialog::new().set_title("Pick NAM captures").add_filter("NAM captures", &["nam"]).open_files();
        self.start_pick(id, picks::Pick::NamCaptures);
    }

    /// Write a User IR .wav into `out` for each capture, and show what happened.
    pub(super) fn convert_nam(&mut self, files: Vec<PathBuf>, out: PathBuf) {
        let opts = crate::nam::Options { use_level: self.settings.nam_use_level, use_loudness: self.settings.nam_use_loudness };
        let mut lines = Vec::new();
        let mut failed = 0;
        for f in &files {
            match crate::nam::convert_file(f, &out, opts) {
                Ok(line) => lines.push(format!("✔ {line}")),
                Err(e) => {
                    failed += 1;
                    lines.push(format!("✖ {}: {e}", f.file_name().and_then(|s| s.to_str()).unwrap_or("?")));
                }
            }
        }
        let ok = files.len() - failed;
        lines.push(String::new());
        lines.push(format!(
            "Saved in {}. Import each .wav into a User IR slot with HX Edit, then set the NAM block's Model knob to that slot.",
            out.display()
        ));
        self.note(if failed == 0 { LogLevel::Good } else { LogLevel::Bad }, format!("Converted {ok} of {} captures", files.len()));
        self.unit_report = Some((format!("NAM captures: {ok} converted, {failed} refused"), lines, failed > 0 && ok == 0));
    }

    /// Pick .wav IRs or .nam captures and ask before writing them into slots: the
    /// selected slots when there are as many, otherwise empty slots from the first selected one.
    pub(super) fn upload_irs(&mut self) {
        let id = crate::platform::FileDialog::new()
            .set_title("Pick IRs or NAM captures to send to the pedal")
            .set_directory(self.tone_dir())
            .add_filter("IRs and NAM captures", &["wav", "nam"])
            .open_files();
        self.start_pick(id, picks::Pick::UploadIrs);
    }

    /// Ask before writing these .wav / .nam files into the pedal's IR slots.
    pub(super) fn upload_ir_files(&mut self, files: Vec<PathBuf>) {
        let opts = self.nam_opts();
        let mut loaded = Vec::new();
        let mut problems = Vec::new();
        for f in &files {
            let name = f.file_name().and_then(|s| s.to_str()).unwrap_or("?").to_owned();
            match crate::irs::load_file(f, opts) {
                Ok(l) => loaded.push((name, l)),
                Err(e) => problems.push(format!("✖ {name}: {e}")),
            }
        }
        if loaded.is_empty() {
            self.unit_report = Some(("No IRs to send".into(), problems, true));
            return;
        }
        let used: std::collections::BTreeMap<i64, String> = self.irs.clone().unwrap_or_default().into_iter().collect();
        let sel: Vec<i64> = self.ir_sel.iter().copied().collect();
        let targets: Vec<i64> = if sel.len() == loaded.len() {
            sel.clone()
        } else {
            let from = sel.first().copied().unwrap_or(0);
            (from..crate::session::IR_SLOTS).filter(|s| !used.contains_key(s)).take(loaded.len()).collect()
        };
        // Each file gets a slot the person can change before anything is written.
        let mut rows = Vec::new();
        for (i, (file, l)) in loaded.into_iter().enumerate() {
            // A .wav can be either: the first samples of a converted NAM capture say so.
            let nam = crate::irs::is_nam_capture(&l.audio);
            let slot = targets.get(i).copied().unwrap_or(0);
            rows.push(IrPlanRow { file, name: l.name, audio: l.audio, slot, nam, notes: l.notes.join(", ") });
        }
        self.ir_plan = Some(IrPlan { rows, problems, firmware_ok: crate::nam::enabled() });
    }

    /// The window that sends files to the pedal: each file with its slot (changeable),
    /// what it replaces, and a warning before sending NAM captures.
    pub(super) fn ir_plan_window(&mut self, ctx: &egui::Context) {
        let Some(mut plan) = self.ir_plan.take() else { return };
        let used: std::collections::BTreeMap<i64, String> = self.irs.clone().unwrap_or_default().into_iter().collect();
        let mut result = None;
        egui::Window::new("Send to the pedal's IR slots")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_min_width(560.0);
                ui.label(RichText::new("Pick the slot each file goes into.").color(muted()));
                ui.add_space(6.0);
                let slots: Vec<i64> = plan.rows.iter().map(|r| r.slot).collect();
                egui::Grid::new("ir-plan").num_columns(4).spacing([10.0, 6.0]).show(ui, |ui| {
                    for (i, row) in plan.rows.iter_mut().enumerate() {
                        let tex = if row.nam { crate::icons::capture(ui.ctx()) } else { crate::icons::ir(ui.ctx()) };
                        ui.horizontal(|ui| {
                            if let Some(t) = tex {
                                ui.add(egui::Image::new(&t).fit_to_exact_size(Vec2::splat(22.0)));
                            }
                            ui.label(RichText::new(if row.nam { "NAM capture" } else { "IR" }).small().color(if row.nam {
                                pink()
                            } else {
                                blue()
                            }));
                        });
                        ui.label(RichText::new(&row.name).strong()).on_hover_text(format!("{}\n{}", row.file, row.notes));
                        let label = |s: i64| match used.get(&s) {
                            Some(n) => format!("{} · {n}", s + 1),
                            None => format!("{} · empty", s + 1),
                        };
                        egui::ComboBox::from_id_salt(("plan-slot", i)).selected_text(label(row.slot)).width(220.0).height(360.0).show_ui(
                            ui,
                            |ui| {
                                for s in 0..crate::session::IR_SLOTS {
                                    let col = if used.contains_key(&s) { orange() } else { ink() };
                                    ui.selectable_value(&mut row.slot, s, RichText::new(label(s)).color(col));
                                }
                            },
                        );
                        let twice = slots.iter().filter(|s| **s == row.slot).count() > 1;
                        if twice {
                            ui.label(RichText::new("same slot twice").small().color(bad()));
                        } else if let Some(n) = used.get(&row.slot) {
                            ui.label(RichText::new(format!("replaces \"{n}\"")).small().color(orange()));
                        } else {
                            ui.label("");
                        }
                        ui.end_row();
                    }
                });
                if plan.rows.iter().any(|r| r.nam) {
                    ui.add_space(8.0);
                    theme::card(ui, |ui| {
                        ui.label(
                            RichText::new("NAM captures only work with the NAM A2 Lite firmware on the pedal.").strong().color(orange()),
                        );
                        ui.label(
                            RichText::new("Without it the pedal refuses them, and the transfer can hang and drop the connection.")
                                .small()
                                .color(muted()),
                        );
                        ui.checkbox(&mut plan.firmware_ok, "My pedal has the NAM A2 Lite firmware");
                    });
                }
                for p in &plan.problems {
                    ui.label(RichText::new(p).small().color(bad()));
                }
                ui.add_space(8.0);
                let dup = (0..plan.rows.len()).any(|i| plan.rows.iter().skip(i + 1).any(|r| r.slot == plan.rows[i].slot));
                let sendable = plan.rows.iter().filter(|r| !r.nam || plan.firmware_ok).count();
                ui.horizontal(|ui| {
                    let label = if sendable == plan.rows.len() {
                        format!("Send {sendable}")
                    } else {
                        format!("Send {sendable} (skip the captures)")
                    };
                    if ui
                        .add_enabled(
                            !dup && sendable > 0,
                            egui::Button::new(RichText::new(label).strong()).fill(accent().gamma_multiply(0.6)),
                        )
                        .clicked()
                    {
                        result = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        result = Some(false);
                    }
                });
            });
        match result {
            Some(true) => {
                let mut items = Vec::new();
                for r in plan.rows.into_iter().filter(|r| !r.nam || plan.firmware_ok) {
                    self.ir_kinds.insert((r.slot, r.name.clone()), r.nam);
                    items.push((r.slot, r.name, r.audio));
                }
                self.send(Cmd::ImportIrs { items, backup: self.backup_dir() });
            }
            Some(false) => {}
            None => self.ir_plan = Some(plan),
        }
    }

    pub(super) fn irs_page(&mut self, ctx: &egui::Context) {
        if self.irs_local {
            return self.tones_page(ctx);
        }
        let connected = self.connected.is_some() && self.link.is_some();
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(bg()).inner_margin(egui::Margin::same(18)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if theme::segmented(ui, &["On the pedal", "On this computer"], 0) == Some(1) {
                        self.irs_local = true;
                        self.scan_tones();
                        // Sending from the library needs the pedal's IR list (to find free slots).
                        if connected && self.irs.is_none() {
                            self.send(Cmd::ListIrs);
                        }
                    }
                    ui.add_space(8.0);
                    ui.label(RichText::new("Pedal IRs").size(24.0).strong());
                    theme::help(
                        ui,
                        "The pedal's 128 User IR slots. Send .wav impulse responses (any rate or depth; they're made 48 kHz mono, 2,048 samples), \
                         or NAM captures (.nam) for the NAM A2 Lite firmware, which load as IRs too. Download saves slots as .wav files, \
                         so a NAM capture comes back as the .wav the NAM block reads.",
                    );
                    if let Some(l) = &self.irs {
                        ui.label(RichText::new(format!("{} of 128 slots in use", l.len())).color(muted()));
                    }
                });
                ui.add_space(6.0);
                if !connected {
                    theme::card(ui, |ui| {
                        ui.label(RichText::new("Connect to a pedal to see and change its IRs.").color(muted()));
                        ui.add_space(4.0);
                        if ui.button("Convert .nam captures to .wav files...").clicked() {
                            self.convert_nam_files();
                        }
                    });
                    return;
                }
                let used: std::collections::BTreeMap<i64, String> = self.irs.clone().unwrap_or_default().into_iter().collect();
                let sel: Vec<i64> = self.ir_sel.iter().copied().collect();
                let busy = self.busy.is_some();
                let mut cmd: Option<Cmd> = None;
                let mut upload = false;
                ui.horizontal_wrapped(|ui| {
                    if ui.button("⟳ Read list").clicked() {
                        cmd = Some(Cmd::ListIrs);
                    }
                    ui.add_enabled_ui(!busy && self.irs.is_some(), |ui| {
                        if theme::primary_button(ui, "⬆ Send IRs or NAM captures...").on_hover_text("Goes into the selected slots, or the next empty ones").clicked() {
                            upload = true;
                        }
                        let picked: Vec<(i64, String)> = sel.iter().filter_map(|s| used.get(s).map(|n| (*s, n.clone()))).collect();
                        let into = self.tones.folder();
                        let into_label = format!("Saved into your IR library: {}", into.display());
                        if ui.add_enabled(!picked.is_empty(), egui::Button::new(format!("⬇ Download selected ({})", picked.len()))).on_hover_text(&into_label).clicked() {
                            cmd = Some(Cmd::ExportIrs { slots: picked.clone(), folder: into.clone() });
                        }
                        if ui.add_enabled(!used.is_empty(), egui::Button::new("⬇ Download all")).on_hover_text(&into_label).clicked() {
                            cmd = Some(Cmd::ExportIrs { slots: used.iter().map(|(s, n)| (*s, n.clone())).collect(), folder: into.join("From pedal") });
                        }
                        let one = (picked.len() == 1).then(|| picked[0].clone());
                        if ui.add_enabled(one.is_some(), egui::Button::new("Rename")).clicked() {
                            self.ir_rename = one.clone();
                        }
                        if ui.add_enabled(!picked.is_empty(), egui::Button::new(RichText::new("Remove").color(bad()))).clicked() {
                            let names: Vec<String> = picked.iter().map(|(s, n)| format!("{} \"{n}\"", s + 1)).collect();
                            self.confirm = Some((format!("Remove {} IR{} from the pedal? ({})\nPresets that use them will have an empty IR.", picked.len(), if picked.len() == 1 { "" } else { "s" }, names.join(", ")), Cmd::DeleteIrs { slots: picked.iter().map(|p| p.0).collect(), backup: self.backup_dir() }));
                        }
                    });
                    if !sel.is_empty() && ui.small_button("Clear selection").clicked() {
                        self.ir_sel.clear();
                    }
                });
                ui.add_space(8.0);
                if self.irs.is_none() {
                    ui.label(RichText::new(if busy { "Reading..." } else { "Press \"Read list\" to see the pedal's IRs." }).color(muted()));
                } else {
                    let multi = ui.input(|i| i.modifiers.ctrl || i.modifiers.command || i.modifiers.shift);
                    let backup = self.backup_dir();
                    // IRs and NAM captures share the pedal's slots but are different things: show them apart.
                    let kind_of = |slot: i64| -> Option<Option<bool>> { used.get(&slot).map(|n| self.ir_kinds.get(&(slot, n.clone())).copied()) };
                    let n_nam = used.keys().filter(|s| kind_of(**s) == Some(Some(true))).count();
                    let n_ir = used.len() - n_nam;
                    let n_empty = crate::session::IR_SLOTS as usize - used.len();
                    ui.horizontal(|ui| {
                        let labels = [
                            "All slots".to_owned(),
                            format!("IRs ({n_ir})"),
                            format!("NAM captures ({n_nam})"),
                            format!("Empty ({n_empty})"),
                        ];
                        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
                        if let Some(v) = theme::segmented(ui, &refs, self.ir_view) {
                            self.ir_view = v;
                        }
                        if used.keys().any(|s| kind_of(*s) == Some(None)) {
                            ui.label(RichText::new("sorting...").small().color(muted()));
                        }
                    });
                    ui.add_space(6.0);
                    let view = self.ir_view;
                    let shown: Vec<i64> = (0..crate::session::IR_SLOTS)
                        .filter(|s| match (view, kind_of(*s)) {
                            (0, _) => true,
                            (1, Some(k)) => k != Some(true),
                            (2, Some(k)) => k == Some(true),
                            (3, None) => true,
                            _ => false,
                        })
                        .collect();
                    if shown.is_empty() {
                        ui.label(RichText::new(match view {
                            1 => "No IRs on the pedal.",
                            2 => "No NAM captures on the pedal.",
                            _ => "No empty slots.",
                        }).color(muted()));
                    }
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        let cols = ((ui.available_width() / 300.0).floor() as usize).clamp(1, 4);
                        egui::Grid::new("ir-grid").num_columns(cols).spacing([10.0, 4.0]).show(ui, |ui| {
                            for (n_shown, slot) in shown.iter().copied().enumerate() {
                                let name = used.get(&slot);
                                let on = self.ir_sel.contains(&slot);
                                let nam = kind_of(slot) == Some(Some(true));
                                // One row: the slot number, an IR or capture icon, the name, all from the left.
                                let (rect, r) = ui.allocate_exact_size(Vec2::new(280.0, 26.0), egui::Sense::click());
                                let p = ui.painter();
                                let bg = if on { accent().gamma_multiply(0.25) } else if r.hovered() { card_hi() } else { Color32::TRANSPARENT };
                                p.rect_filled(rect, egui::CornerRadius::same(5), bg);
                                if on {
                                    p.rect_stroke(rect, egui::CornerRadius::same(5), egui::Stroke::new(1.0_f32, accent()), egui::StrokeKind::Inside);
                                }
                                let mono = egui::FontId::monospace(13.0);
                                p.text(rect.left_center() + Vec2::new(6.0, 0.0), egui::Align2::LEFT_CENTER, format!("{:>3}", slot + 1), mono.clone(), muted());
                                let icon_rect = egui::Rect::from_center_size(rect.left_center() + Vec2::new(50.0, 0.0), Vec2::splat(22.0));
                                match name {
                                    Some(n) => {
                                        let tex = if nam { crate::icons::capture(ui.ctx()) } else { crate::icons::ir(ui.ctx()) };
                                        if let Some(t) = tex {
                                            p.image(t.id(), icon_rect, egui::Rect::from_min_max(egui::Pos2::ZERO, egui::Pos2::new(1.0, 1.0)), Color32::WHITE);
                                        }
                                        let galley = p.layout_no_wrap(n.clone(), egui::FontId::proportional(14.0), if nam { pink() } else { ink() });
                                        let at = rect.left_center() + Vec2::new(68.0, -galley.size().y / 2.0);
                                        p.with_clip_rect(rect.shrink(2.0)).galley(at, galley, ink());
                                    }
                                    None => {
                                        p.text(rect.left_center() + Vec2::new(68.0, 0.0), egui::Align2::LEFT_CENTER, "empty", egui::FontId::proportional(13.0), muted().linear_multiply(0.7));
                                    }
                                }
                                let r = r
                                    .on_hover_text(match (name.is_some(), nam) {
                                        (true, true) => "A NAM capture (for the NAM A2 Lite block)",
                                        (true, false) => "An impulse response (for a cab / IR block)",
                                        _ => "Empty slot",
                                    });
                                if r.clicked() {
                                    if multi {
                                        if !self.ir_sel.remove(&slot) {
                                            self.ir_sel.insert(slot);
                                        }
                                    } else if on && self.ir_sel.len() == 1 {
                                        self.ir_sel.clear();
                                    } else {
                                        self.ir_sel.clear();
                                        self.ir_sel.insert(slot);
                                    }
                                }
                                if let Some(n) = name {
                                    r.context_menu(|ui| {
                                        let into = self.tones.folder();
                                        if ui.button("Download to the library").clicked() {
                                            cmd = Some(Cmd::ExportIrs { slots: vec![(slot, n.clone())], folder: into });
                                            ui.close_menu();
                                        }
                                        if ui.button("Rename...").clicked() {
                                            self.ir_rename = Some((slot, n.clone()));
                                            ui.close_menu();
                                        }
                                        ui.separator();
                                        if ui.button(RichText::new("Remove").color(bad())).clicked() {
                                            self.confirm = Some((
                                                format!("Remove {} \"{n}\" from the pedal?\nPresets that use it will have an empty IR.", slot + 1),
                                                Cmd::DeleteIrs { slots: vec![slot], backup: backup.clone() },
                                            ));
                                            ui.close_menu();
                                        }
                                    });
                                } else {
                                    r.context_menu(|ui| {
                                        if ui.button("Send an IR or capture to this slot...").clicked() {
                                            self.ir_sel.clear();
                                            self.ir_sel.insert(slot);
                                            upload = true;
                                            ui.close_menu();
                                        }
                                    });
                                }
                                if (n_shown + 1) % cols == 0 {
                                    ui.end_row();
                                }
                            }
                        });
                    });
                }
                if upload {
                    self.upload_irs();
                }
                if let Some(c) = cmd {
                    self.send(c);
                }
            });
        // Renaming one IR.
        if let Some((slot, mut name)) = self.ir_rename.take() {
            let mut result = None;
            egui::Window::new("Rename IR").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(
                ctx,
                |ui| {
                    ui.label(format!("Slot {}", slot + 1));
                    let r = ui.add(egui::TextEdit::singleline(&mut name).char_limit(crate::session::IR_NAME_MAX).desired_width(260.0));
                    ui.horizontal(|ui| theme::name_hint(ui, &name, crate::session::IR_NAME_MAX));
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    ui.horizontal(|ui| {
                        if theme::primary_button(ui, "Rename").clicked() || enter {
                            result = Some(true);
                        }
                        if ui.button("Cancel").clicked() {
                            result = Some(false);
                        }
                    });
                },
            );
            match result {
                Some(true) => {
                    let n = crate::irs::clean_name(&name);
                    if !n.is_empty() {
                        self.send(Cmd::RenameIr { slot, name: n });
                    }
                }
                Some(false) => {}
                None => self.ir_rename = Some((slot, name)),
            }
        }
    }

    /// The IR and NAM library on this computer (the IRs page's second view).
    pub(super) fn tones_page(&mut self, ctx: &egui::Context) {
        let connected = self.connected.is_some() && self.link.is_some();
        // Connected but the IR list not read yet (connected while this view was open): read it once.
        if connected && self.irs.is_none() && self.busy.is_none() && !self.tones_list_asked {
            self.tones_list_asked = true;
            self.send(Cmd::ListIrs);
        }
        if !connected {
            self.tones_list_asked = false;
        }
        let can_send = connected && self.irs.is_some() && self.busy.is_none();
        let opts = self.nam_opts();
        let mut back = false;
        let actions = egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(bg()).inner_margin(egui::Margin::same(18)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if theme::segmented(ui, &["On the pedal", "On this computer"], 1) == Some(0) {
                        back = true;
                    }
                    ui.add_space(8.0);
                    ui.label(RichText::new("IR and NAM library").size(24.0).strong());
                    theme::help(
                        ui,
                        "IRs (.wav) and NAM captures (.nam) kept on this computer, in real folders. Each capture shows whether it has been converted \
                         to the .wav the NAM A2 Lite block loads (written beside it as \"<name> NAM.wav\"). Select files and send them to the pedal; \
                         IRs downloaded from the pedal land in the selected folder here.",
                    );
                    let n_nam = self.tones.entries.iter().filter(|e| e.kind == crate::tonelib::Kind::Nam).count();
                    let n_conv = self.tones.entries.iter().filter(|e| e.converted.is_some()).count();
                    ui.label(RichText::new(format!("{} files · {n_conv} of {n_nam} captures converted", self.tones.entries.len())).color(muted()));
                    if ui.small_button("⟳").on_hover_text("Look again").clicked() {
                        self.tones.scan(opts);
                    }
                });
                ui.add_space(8.0);
                self.tones.ui(ui, can_send, opts)
            })
            .inner;
        for a in actions {
            match a {
                crate::tonelib::Action::SendToPedal(files) => self.upload_ir_files(files),
                crate::tonelib::Action::Import => {
                    let id = crate::platform::FileDialog::new().add_filter("IRs and NAM captures", &["wav", "nam"]).open_files();
                    self.start_pick(id, picks::Pick::ImportTones);
                }
                crate::tonelib::Action::Note(ok, text) => self.note(if ok { LogLevel::Good } else { LogLevel::Bad }, text),
            }
        }
        if back {
            self.irs_local = false;
            if self.irs.is_none() && self.connected.is_some() && self.link.is_some() {
                self.send(Cmd::ListIrs);
            }
        }
    }
}
