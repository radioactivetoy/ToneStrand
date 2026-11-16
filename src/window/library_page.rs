//! The Library page: presets on disk, importing backups, saving to the library.

use super::*;

impl App {
    /// Take the presets out of a device backup (.hxb) or setlist export (.hls) into the
    /// library: a folder named after the file, with a folder per setlist, one .hlx per preset.
    pub(super) fn import_backup(&mut self, file: &Path, into: &Path) -> Result<(usize, Vec<String>), String> {
        let bytes = std::fs::read(file).map_err(|e| e.to_string())?;
        let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        let (setlists, irs) = if ext == "hxb" {
            let c = crate::backup::read_hxb(&bytes)?;
            (c.setlists, c.irs)
        } else {
            (vec![crate::backup::read_hls(&bytes)?], Vec::new())
        };
        let stem = device::sanitize(file.file_stem().and_then(|s| s.to_str()).unwrap_or("Backup"));
        let base = into.join(&stem);
        let mut n = 0;
        let mut lines = Vec::new();
        for s in &setlists {
            let dir = if setlists.len() == 1 && ext == "hls" { base.clone() } else { base.join(device::sanitize(&s.name)) };
            let _ = std::fs::create_dir_all(&dir);
            let mut written = 0;
            for p in &s.presets {
                let path = crate::files::unique_path(&dir, &format!("{} {}.hlx", slot_label(p.slot), device::sanitize(&p.name)));
                match crate::files::write_atomic(&path, &serde_json::to_vec_pretty(&p.hlx).unwrap_or_default()) {
                    Ok(_) => written += 1,
                    Err(e) => lines.push(format!("✖ {}: {e}", path.display())),
                }
            }
            n += written;
            let skipped = if s.skipped > 0 { format!(", {} unreadable slots skipped", s.skipped) } else { String::new() };
            lines.push(format!("✔ {} -> {stem}/{}: {written} presets{skipped}", file.file_name().and_then(|x| x.to_str()).unwrap_or("?"), s.name));
        }
        if !irs.is_empty() {
            // The backup's IRs go to the IR library, in a folder named after the backup.
            let ir_dir = self.tone_dir().join(&stem);
            let _ = std::fs::create_dir_all(&ir_dir);
            let written = irs
                .iter()
                .filter(|ir| crate::files::write_atomic(&crate::files::unique_path(&ir_dir, &crate::irs::file_name(ir.slot as i64, &ir.name)), &ir.wav).is_ok())
                .count();
            lines.push(format!("✔ {written} IRs -> IR library/{stem}"));
            self.scan_tones();
        }
        Ok((n, lines))
    }

    pub(super) fn library_page(&mut self, ctx: &egui::Context) {
        let can_send = self.connected.is_some() && self.chosen.is_some();
        let actions = egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(bg()).inner_margin(egui::Margin::same(18)))
            .show(ctx, |ui| self.lib.ui(ui, can_send))
            .inner;
        for a in actions {
            match a {
                library::Action::Open(p) => {
                    self.open_file(&p);
                    self.view = View::Editor;
                }
                library::Action::SendToPedal(p) if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("hlx")) => {
                    let Some(slot) = self.chosen else { continue };
                    let Some(cat) = self.catalog.clone() else {
                        self.note(LogLevel::Bad, "Sending .hlx presets needs HX Edit's effect list (see Settings).");
                        continue;
                    };
                    let json = std::fs::read_to_string(&p).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str::<serde_json::Value>(&t).map_err(|e| e.to_string()));
                    let mut json = match json {
                        Ok(j) => j,
                        Err(e) => {
                            self.note(LogLevel::Bad, format!("Cannot read {}: {e}", p.display()));
                            continue;
                        }
                    };
                    // A preset made for another unit is converted to this pedal first.
                    if crate::snapshots::device(&json) != self.unit {
                        if let Err(why) = crate::units::convert(&mut json, self.unit, Some(&cat)) {
                            self.unit_report = Some((format!("This preset doesn't fit the {}", crate::units::unit_name(self.unit)), why, true));
                            continue;
                        }
                    }
                    let warning = dsp_warning(&cat, &json).map(|w| format!("\n\n{w}")).unwrap_or_default();
                    let name = crate::hlx::name(&json);
                    let now = self.presets.get(slot).cloned().unwrap_or_default();
                    self.confirm = Some((
                        format!("Write \"{name}\" into {} (now \"{now}\")?\n\nIt is converted from .hlx using the pedal's loaded preset as the template. The slot's current preset is backed up first.{warning}", slot_label(slot)),
                        Cmd::WriteHlx { setlist: self.setlist, index: slot as i64, name, json, cat, backup: self.backup_dir() },
                    ));
                }
                library::Action::SendToPedal(p) => {
                    let Some(slot) = self.chosen else { continue };
                    match std::fs::read(&p).map_err(|e| e.to_string()).and_then(|b| DevicePreset::parse(&b)) {
                        Ok(preset) => {
                            let doc = preset.encode();
                            let name = p.file_stem().and_then(|s| s.to_str()).unwrap_or("Preset");
                            let name = match name.split_once(' ') {
                                Some((a, b)) if a.len() == 3 && a.ends_with(['A', 'B', 'C']) => b.to_owned(),
                                _ => name.to_owned(),
                            };
                            let now = self.presets.get(slot).cloned().unwrap_or_default();
                            self.confirm = Some((
                                format!("Write \"{name}\" into {} (now \"{now}\")?", slot_label(slot)),
                                Cmd::WriteSlot { setlist: self.setlist, index: slot as i64, name, doc, backup: self.backup_dir() },
                            ));
                        }
                        Err(e) => self.note(LogLevel::Bad, format!("Cannot send {} to the pedal: {e}", p.display())),
                    }
                }
                library::Action::Delete(p) => {
                    // Without a Recycle Bin the delete is final, so it is always asked about.
                    if self.settings.confirm_delete || !crate::files::HAS_RECYCLE_BIN {
                        self.pending_delete = Some(p);
                    } else {
                        self.delete_library_file(&p);
                    }
                }
                library::Action::MoveTo(p, setlist) => {
                    let dir = if setlist.is_empty() { self.lib.root.clone() } else { self.lib.root.join(&setlist) };
                    if p.parent() != Some(dir.as_path()) {
                        match crate::folders::move_into(&p, &dir) {
                            Ok(_) => self.note(LogLevel::Good, format!("Moved to {}", if setlist.is_empty() { "the top folder" } else { &setlist })),
                            Err(e) => self.note(LogLevel::Bad, format!("Could not move: {e}")),
                        }
                    }
                    self.scan_library();
                }
                library::Action::Rescan => self.scan_library(),
                library::Action::Import => {
                    if let Some(files) = rfd::FileDialog::new()
                        .add_filter("Presets, backups and setlists", &["hlx", "hxpreset", "hxb", "hls"])
                        .pick_files()
                    {
                        let dir = self.lib.folder();
                        let _ = std::fs::create_dir_all(&dir);
                        let mut n = 0;
                        let mut report = Vec::new();
                        for f in files {
                            let ext = f.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
                            if ext == "hxb" || ext == "hls" {
                                match self.import_backup(&f, &dir) {
                                    Ok((count, lines)) => {
                                        n += count;
                                        report.extend(lines);
                                    }
                                    Err(e) => report.push(format!("✖ {}: {e}", f.file_name().and_then(|s| s.to_str()).unwrap_or("?"))),
                                }
                            } else {
                                let shown = f.file_name().and_then(|s| s.to_str()).unwrap_or("?").to_owned();
                                match crate::files::copy_into(&f, &dir) {
                                    Ok(to) => {
                                        n += 1;
                                        if to.file_name() != f.file_name() {
                                            let new = to.file_name().and_then(|s| s.to_str()).unwrap_or("?");
                                            report.push(format!("✔ {shown} was already there; kept both (new copy: {new})"));
                                        }
                                    }
                                    Err(e) => report.push(format!("✖ {shown}: {e}")),
                                }
                            }
                        }
                        let failures = report.iter().filter(|l| l.starts_with('✖')).count();
                        let (level, summary) = match (n, failures) {
                            (_, 0) if n > 0 => (LogLevel::Good, format!("Imported {n} presets")),
                            (0, 0) => (LogLevel::Info, "Nothing was imported".to_owned()),
                            (_, f) => (LogLevel::Bad, format!("Imported {n} presets; {f} could not be imported (see the report)")),
                        };
                        self.note(level, summary);
                        if !report.is_empty() {
                            self.unit_report = Some((format!("Imported {n} presets"), report, false));
                        }
                        self.scan_library();
                    }
                }
            }
        }
        if let Some(p) = self.pending_delete.clone() {
            let mut result = None;
            egui::Window::new("Delete preset")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
                    if crate::files::HAS_RECYCLE_BIN {
                        ui.label(format!("Move {name} to the Recycle Bin?"));
                    } else {
                        ui.label(format!("Delete {name} from your library? This cannot be undone."));
                    }
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new(if crate::files::HAS_RECYCLE_BIN { "Move to Recycle Bin" } else { "Delete" }).color(BAD)).clicked() {
                            result = Some(true);
                        }
                        if ui.button("Cancel").clicked() {
                            result = Some(false);
                        }
                    });
                });
            if let Some(yes) = result {
                if yes {
                    self.delete_library_file(&p);
                }
                self.pending_delete = None;
            }
        }
    }

    /// Delete a library preset: to the Recycle Bin where there is one.
    fn delete_library_file(&mut self, p: &Path) {
        match crate::files::delete_file(p) {
            Ok(_) => self.note(LogLevel::Good, if crate::files::HAS_RECYCLE_BIN { "Moved to the Recycle Bin" } else { "Deleted" }),
            Err(e) => self.note(LogLevel::Bad, format!("Could not delete: {e}")),
        }
        self.scan_library();
    }

    /// Keep a copy of an open preset in the library (in the setlist being shown there).
    pub(super) fn save_to_library(&mut self, i: usize) {
        let Some(doc) = self.docs.get(i) else { return };
        let (ext, bytes) = match &doc.kind {
            DocKind::Hlx { json } => ("hlx", serde_json::to_vec_pretty(json).unwrap_or_default()),
            DocKind::Device { preset, .. } => ("hxpreset", preset.encode()),
        };
        let base = device::sanitize(if doc.name.is_empty() { &doc.title } else { &doc.name });
        let dir = self.lib.folder();
        let _ = std::fs::create_dir_all(&dir);
        let path = crate::files::unique_path(&dir, &format!("{base}.{ext}"));
        match crate::files::write_atomic(&path, &bytes) {
            Ok(_) => self.note(LogLevel::Good, format!("Saved to your library as {}", path.file_name().and_then(|s| s.to_str()).unwrap_or(""))),
            Err(e) => self.note(LogLevel::Bad, format!("Could not save to the library: {e}")),
        }
        self.scan_library();
    }
}
