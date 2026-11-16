//! The pedal's slot backups (`Pedal backups/`): listing them with their slot and date, and
//! writing one back (through the same confirm-and-back-up path as any other slot write).

use super::*;

/// One `.hxpreset` in the backup folder.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct BackupFile {
    pub path: PathBuf,
    /// The slot from the file name (zero-based), when it can be read.
    pub slot: Option<usize>,
    pub name: String,
    /// Unix seconds: from the file name when it has one, else the file's date.
    pub when: Option<u64>,
    /// Made by a write ("before overwrite") rather than by "Back up this setlist".
    pub before_write: bool,
}

/// The window's state: the files found and the slots picked for files whose own slot
/// can't be read.
pub(super) struct RestoreState {
    files: Vec<BackupFile>,
    asked: std::collections::BTreeMap<PathBuf, usize>,
}

/// A slot label such as "01A" back to its zero-based index.
pub(super) fn parse_slot_label(label: &str) -> Option<usize> {
    let b = label.as_bytes();
    if b.len() != 3 || !b[0].is_ascii_digit() || !b[1].is_ascii_digit() {
        return None;
    }
    let n: usize = label[..2].parse().ok()?;
    let letter = match b[2] {
        b'A' => 0,
        b'B' => 1,
        b'C' => 2,
        _ => return None,
    };
    (n >= 1).then(|| (n - 1) * 3 + letter)
}

/// Read a backup's file name: "<slot> <name> - before overwrite <secs>.hxpreset" (made by
/// a write) or "<slot> <name>.hxpreset" (made by "Back up this setlist"). The slot is None when the
/// name doesn't start with one.
pub(super) fn parse_backup_name(file_name: &str) -> (Option<usize>, String, Option<u64>, bool) {
    let stem = file_name.rsplit_once('.').map_or(file_name, |(s, _)| s);
    let (slot, rest) = match stem.split_once(' ') {
        Some((a, b)) if parse_slot_label(a).is_some() => (parse_slot_label(a), b),
        _ => (None, stem),
    };
    if let Some((name, tail)) = rest.rsplit_once(" - before overwrite ") {
        // A second backup in the same second gets " (2)" added by `unique_path`.
        let secs = tail.split(' ').next().and_then(|s| s.parse::<u64>().ok());
        return (slot, name.to_owned(), secs, true);
    }
    (slot, rest.to_owned(), None, false)
}

/// "16 Nov 2026 14:03 UTC" for Unix seconds.
pub(super) fn when_text(secs: u64) -> String {
    let (year, month, day, hour, minute) = crate::files::civil(secs);
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!("{day} {} {year} {hour:02}:{minute:02} UTC", MONTHS[(month - 1) as usize])
}

/// The backups in a folder, newest first.
pub(super) fn scan_backups(dir: &Path) -> Vec<BackupFile> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return out };
    for e in rd.flatten() {
        let path = e.path();
        if !path.is_file() || !path.extension().is_some_and(|x| x.eq_ignore_ascii_case("hxpreset")) {
            continue;
        }
        let file = path.file_name().and_then(|s| s.to_str()).unwrap_or("").to_owned();
        let (slot, name, stamp, before_write) = parse_backup_name(&file);
        let modified = e
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs());
        out.push(BackupFile { path, slot, name, when: stamp.or(modified), before_write });
    }
    out.sort_by(|a, b| b.when.cmp(&a.when).then_with(|| a.path.cmp(&b.path)));
    out
}

impl App {
    pub(super) fn open_restore(&mut self) {
        self.restore = Some(RestoreState { files: scan_backups(&self.backup_dir()), asked: Default::default() });
    }

    /// Ask to write a backup back into a slot of the shown setlist. The write itself keeps
    /// the slot's current preset first and goes through the usual confirmation.
    fn restore_to(&mut self, file: &BackupFile, slot: usize) {
        let bytes = match std::fs::read(&file.path) {
            Ok(b) => b,
            Err(e) => return self.note(LogLevel::Bad, format!("Cannot read {}: {e}", file.path.display())),
        };
        let doc = match DevicePreset::parse(&bytes).and_then(|p| p.encode_checked()) {
            Ok(d) => d,
            Err(e) => return self.note(LogLevel::Bad, format!("{} is not a usable preset: {e}", file.path.display())),
        };
        let now = self.presets.get(slot).cloned().unwrap_or_default();
        let setlist_name = self.setlists.get(self.setlist as usize).cloned().filter(|_| self.setlists.len() > 1);
        let into = match setlist_name {
            Some(s) => format!("{} in {s}", slot_label(slot)),
            None => slot_label(slot),
        };
        let from = file.when.map(|w| format!(" saved {}", when_text(w))).unwrap_or_default();
        self.confirm = Some((
            format!("Write back \"{}\"{from} into {into} (now \"{now}\")?\n\nThe slot's current preset is backed up first.", file.name),
            Cmd::WriteSlot { setlist: self.setlist, index: slot as i64, name: file.name.clone(), doc, backup: self.backup_dir() },
        ));
    }

    /// "Restore from a backup": the files in `Pedal backups`, each with a "Write back".
    pub(super) fn restore_window(&mut self, ctx: &egui::Context) {
        let Some(mut st) = self.restore.take() else { return };
        let mut open = true;
        let mut pick: Option<(BackupFile, usize)> = None;
        let mut rescan = false;
        let slots = self.presets.len();
        let connected = self.connected.is_some();
        let dir = self.backup_dir();
        egui::Window::new("Restore from a backup")
            .open(&mut open)
            .collapsible(false)
            .default_width(560.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Presets kept before a write, and from \"Back up this setlist\"").color(MUTED));
                    theme::help(
                        ui,
                        &format!(
                            "These files are in {}.\nWriting one back replaces whatever is in the slot now; the slot's current preset is backed up first.\nIt is written into the setlist shown in the pedal list.",
                            dir.display()
                        ),
                    );
                });
                ui.horizontal(|ui| {
                    if ui.button("Look again").clicked() {
                        rescan = true;
                    }
                    if ui.button("Show folder").clicked() {
                        crate::folders::open_in_explorer(&dir);
                    }
                });
                ui.add_space(4.0);
                if st.files.is_empty() {
                    ui.label(RichText::new("No backups yet. One is made each time a slot is written over.").color(MUTED));
                }
                egui::ScrollArea::vertical().max_height(360.0).auto_shrink([true, true]).show(ui, |ui| {
                    for f in &st.files {
                        let own = f.slot.filter(|s| *s < slots);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(f.slot.map(slot_label).unwrap_or_else(|| "???".into())).monospace().color(accent()));
                            ui.label(RichText::new(&f.name).strong());
                            if f.before_write {
                                theme::pill(ui, "before a write", MUTED);
                            }
                            if let Some(w) = f.when {
                                ui.label(RichText::new(when_text(w)).small().color(MUTED));
                            }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let target = own.or_else(|| st.asked.get(&f.path).copied());
                                let label = target.map(|s| format!("Write back to {}", slot_label(s))).unwrap_or_else(|| "Write back".into());
                                let go = ui.add_enabled(connected && target.is_some(), egui::Button::new(label));
                                let go = go.on_disabled_hover_text(if connected { "Pick a slot first" } else { "Connect to a pedal first" });
                                if go.clicked() {
                                    if let Some(t) = target {
                                        pick = Some((f.clone(), t));
                                    }
                                }
                                if own.is_none() {
                                    // The file name doesn't say which slot it came from: ask.
                                    let cur = st.asked.get(&f.path).copied();
                                    egui::ComboBox::from_id_salt(("restore-slot", &f.path))
                                        .width(70.0)
                                        .selected_text(cur.map(slot_label).unwrap_or_else(|| "Slot".into()))
                                        .show_ui(ui, |ui| {
                                            for s in 0..slots {
                                                if ui.selectable_label(cur == Some(s), slot_label(s)).clicked() {
                                                    st.asked.insert(f.path.clone(), s);
                                                }
                                            }
                                        });
                                }
                            });
                        });
                    }
                });
            });
        if rescan {
            st.files = scan_backups(&dir);
        }
        if let Some((f, slot)) = pick {
            self.restore_to(&f, slot);
            open = false;
        }
        if open {
            self.restore = Some(st);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_labels_parse_back() {
        for i in 0..126 {
            assert_eq!(parse_slot_label(&slot_label(i)), Some(i));
        }
        assert_eq!(parse_slot_label("00A"), None);
        assert_eq!(parse_slot_label("01D"), None);
        assert_eq!(parse_slot_label("1A"), None);
    }

    #[test]
    fn backup_names_are_read() {
        let (slot, name, when, before) = parse_backup_name("03B Clean - Dry - before overwrite 1763300000.hxpreset");
        assert_eq!((slot, name.as_str(), when, before), (Some(7), "Clean - Dry", Some(1763300000), true));
        let (slot, name, when, before) = parse_backup_name("01A Lead (2).hxpreset");
        assert_eq!((slot, name.as_str(), when, before), (Some(0), "Lead (2)", None, false));
        let (slot, name, _, _) = parse_backup_name("My preset.hxpreset");
        assert_eq!((slot, name.as_str()), (None, "My preset"));
    }

    #[test]
    fn dates_read_in_utc() {
        assert_eq!(when_text(0), "1 Jan 1970 00:00 UTC");
        assert_eq!(crate::files::stamp_text(1_763_300_000), "2025-11-16 1333");
        assert_eq!(when_text(1_763_300_000), "16 Nov 2025 13:33 UTC");
        assert_eq!(when_text(951_782_400 + 3600 + 120), "29 Feb 2000 01:02 UTC");
    }
}
