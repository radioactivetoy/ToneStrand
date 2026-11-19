//! Restoring a whole-pedal backup (the folder "Back up the whole pedal..." makes): what it holds
//! is listed first, nothing is written until the person confirms, and every slot it replaces is
//! backed up before it is written over.

use super::backups::parse_backup_name;
use super::*;

/// One preset file of the backup: the setlist folder it sits in and the slot its name gives.
pub(super) struct FullPreset {
    pub setlist: usize,
    pub slot: usize,
    pub name: String,
    pub path: PathBuf,
    pub take: bool,
}

/// One IR file: "003 name.wav" is User IR slot 3 (index 2).
pub(super) struct FullIr {
    pub slot: i64,
    pub name: String,
    pub path: PathBuf,
    pub take: bool,
}

pub(super) struct RestoreAllState {
    folder: PathBuf,
    presets: Vec<FullPreset>,
    irs: Vec<FullIr>,
    /// What in the folder could not be used, and why.
    problems: Vec<String>,
}

/// Read a whole-pedal backup folder: a folder per setlist (named as the pedal's setlist) with
/// "<slot> <name>.hxpreset" files, and an "IRs" folder of "<number> <name>.wav" files.
pub(super) fn scan_full_backup(folder: &Path, setlists: &[String]) -> (Vec<FullPreset>, Vec<FullIr>, Vec<String>) {
    let mut presets = Vec::new();
    let mut irs = Vec::new();
    let mut problems = Vec::new();
    let Ok(rd) = std::fs::read_dir(folder) else {
        return (presets, irs, vec![format!("can't read {}", folder.display())]);
    };
    let mut dirs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    dirs.sort();
    for dir in dirs {
        let dname = dir.file_name().and_then(|s| s.to_str()).unwrap_or("").to_owned();
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir).map(|rd| rd.flatten().map(|e| e.path()).collect()).unwrap_or_default();
        files.sort();
        if dname.eq_ignore_ascii_case("IRs") {
            for f in files.into_iter().filter(|f| f.extension().is_some_and(|x| x.eq_ignore_ascii_case("wav"))) {
                let stem = f.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_owned();
                match stem.split_once(' ').and_then(|(n, name)| n.parse::<i64>().ok().map(|n| (n, name))) {
                    Some((n, name)) if (1..=crate::session::IR_SLOTS).contains(&n) => {
                        irs.push(FullIr { slot: n - 1, name: name.to_owned(), path: f, take: true });
                    }
                    _ => problems.push(format!("IRs/{stem}: the file name doesn't start with a slot number")),
                }
            }
            continue;
        }
        let Some(setlist) = setlists.iter().position(|s| crate::device::sanitize(s) == dname) else {
            problems.push(format!("folder \"{dname}\" matches no setlist on this pedal; skipped"));
            continue;
        };
        for f in files.into_iter().filter(|f| f.extension().is_some_and(|x| x.eq_ignore_ascii_case("hxpreset"))) {
            let file = f.file_name().and_then(|s| s.to_str()).unwrap_or("").to_owned();
            match parse_backup_name(&file) {
                (Some(slot), name, _, _) => presets.push(FullPreset { setlist, slot, name, path: f, take: true }),
                _ => problems.push(format!("{dname}/{file}: the file name doesn't start with a slot like 01A")),
            }
        }
    }
    (presets, irs, problems)
}

impl App {
    /// "Restore the whole pedal": pick a backup folder and show what is in it. Android's picker
    /// cannot give a folder, so there the backups folders are listed in the app instead.
    pub(super) fn open_restore_all(&mut self) {
        if cfg!(target_os = "android") {
            // (The private backups folder is added to these by the code that keeps backups there.)
            self.restore_all_folders = Some(picks::backup_folders(&[self.backup_dir()]));
            return;
        }
        let id =
            crate::platform::FileDialog::new().set_directory(&self.library_dir).set_title("Pick a whole-pedal backup folder").pick_folder();
        self.start_pick(id, picks::Pick::RestoreAllFolder);
    }

    /// Show what a whole-pedal backup folder holds.
    pub(super) fn restore_all_from(&mut self, folder: PathBuf) {
        let (presets, irs, problems) = scan_full_backup(&folder, &self.setlists);
        if presets.is_empty() && irs.is_empty() {
            let why = problems.first().cloned().unwrap_or_else(|| "no presets or IRs in it".into());
            return self.note(LogLevel::Bad, format!("{} is not a whole-pedal backup: {why}", folder.display()));
        }
        self.restore_all = Some(RestoreAllState { folder, presets, irs, problems });
    }

    /// Android: the list of backup folders to restore from.
    pub(super) fn restore_all_list_window(&mut self, ctx: &egui::Context) {
        let Some(folders) = self.restore_all_folders.take() else { return };
        let mut open = true;
        let mut chosen = None;
        egui::Window::new("Restore the whole pedal")
            .open(&mut open)
            .collapsible(false)
            .default_width(480.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                if folders.is_empty() {
                    ui.label(RichText::new("No whole-pedal backups yet. Use Backups > Back up the whole pedal first.").color(muted()));
                    return;
                }
                ui.label(RichText::new("Pick a backup. Nothing is written until you confirm.").color(muted()));
                egui::ScrollArea::vertical().max_height(360.0).auto_shrink([true, true]).show(ui, |ui| {
                    for f in &folders {
                        ui.horizontal(|ui| {
                            ui.label(f.file_name().and_then(|n| n.to_str()).unwrap_or("?"));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.add(egui::Button::new("Restore from this").min_size(egui::vec2(0.0, 32.0))).clicked() {
                                    chosen = Some(f.clone());
                                }
                            });
                        });
                    }
                });
            });
        match chosen {
            Some(f) => self.restore_all_from(f),
            None if open => self.restore_all_folders = Some(folders),
            None => {}
        }
    }

    /// Read the picked files and ask for the go-ahead. Nothing is written yet.
    fn ask_restore_all(&mut self, st: &RestoreAllState) {
        let mut presets = Vec::new();
        for p in st.presets.iter().filter(|p| p.take) {
            let doc =
                std::fs::read(&p.path).map_err(|e| e.to_string()).and_then(|b| DevicePreset::parse(&b)).and_then(|d| d.encode_checked());
            match doc {
                Ok(d) => presets.push((p.setlist as i64, p.slot as i64, p.name.clone(), d)),
                Err(e) => return self.note(LogLevel::Bad, format!("{} can't be restored ({e}); nothing was written", p.path.display())),
            }
        }
        let mut irs = Vec::new();
        for i in st.irs.iter().filter(|i| i.take) {
            let audio = std::fs::read(&i.path).map_err(|e| e.to_string()).and_then(|b| crate::irs::wav_to_audio(&b).map(|a| a.0));
            match audio {
                Ok(a) => irs.push((i.slot, i.name.clone(), a)),
                Err(e) => return self.note(LogLevel::Bad, format!("{} can't be restored ({e}); nothing was written", i.path.display())),
            }
        }
        let text = format!(
            "Write {} preset{} and {} IR{} from\n{}\nback to the pedal?\n\nEvery slot this replaces is backed up first (in {}). This can take several minutes.",
            presets.len(),
            if presets.len() == 1 { "" } else { "s" },
            irs.len(),
            if irs.len() == 1 { "" } else { "s" },
            st.folder.display(),
            self.backup_dir().display()
        );
        self.confirm = Some((text, Cmd::RestoreAll { presets, irs, backup: self.backup_dir() }));
    }

    pub(super) fn restore_all_window(&mut self, ctx: &egui::Context) {
        let Some(mut st) = self.restore_all.take() else { return };
        let mut open = true;
        let mut go = false;
        let connected = self.connected.is_some();
        let known: Vec<String> = self.presets.clone();
        let shown = self.setlist as usize;
        egui::Window::new("Restore the whole pedal")
            .open(&mut open)
            .collapsible(false)
            .default_width(560.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(RichText::new(st.folder.display().to_string()).small().color(muted()));
                ui.add_space(4.0);
                let (np, ni) = (st.presets.iter().filter(|p| p.take).count(), st.irs.iter().filter(|i| i.take).count());
                ui.label(format!(
                    "{np} preset{} and {ni} IR{} will be written. Nothing is written until you confirm.",
                    if np == 1 { "" } else { "s" },
                    if ni == 1 { "" } else { "s" }
                ));
                egui::ScrollArea::vertical().max_height(360.0).auto_shrink([true, true]).show(ui, |ui| {
                    let mut last: Option<usize> = None;
                    for p in st.presets.iter_mut() {
                        if last != Some(p.setlist) {
                            last = Some(p.setlist);
                            let name = self.setlists.get(p.setlist).cloned().unwrap_or_default();
                            ui.add_space(4.0);
                            ui.label(RichText::new(name).strong());
                        }
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut p.take, "");
                            ui.label(RichText::new(slot_label(p.slot)).monospace().color(accent()));
                            ui.label(&p.name);
                            // Only the shown setlist's current names are known.
                            if p.setlist == shown {
                                let now = known.get(p.slot).map(String::as_str).unwrap_or("");
                                let text = if now.is_empty() { "empty now".to_owned() } else { format!("replaces \"{now}\"") };
                                ui.label(RichText::new(text).small().color(muted()));
                            }
                        });
                    }
                    if !st.irs.is_empty() {
                        ui.add_space(4.0);
                        ui.label(RichText::new("User IRs").strong());
                    }
                    for i in st.irs.iter_mut() {
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut i.take, "");
                            ui.label(RichText::new(format!("{:03}", i.slot + 1)).monospace().color(accent()));
                            ui.label(&i.name);
                        });
                    }
                    if !st.problems.is_empty() {
                        ui.add_space(6.0);
                        ui.label(RichText::new("Left out").strong().color(bad()));
                        for p in &st.problems {
                            ui.label(RichText::new(p).small().color(muted()));
                        }
                    }
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let label =
                        format!("Restore {np} preset{} and {ni} IR{}...", if np == 1 { "" } else { "s" }, if ni == 1 { "" } else { "s" });
                    let b = ui.add_enabled(connected && np + ni > 0, egui::Button::new(label));
                    if b.on_disabled_hover_text(if connected { "Pick something to restore" } else { "Connect to a pedal first" }).clicked()
                    {
                        go = true;
                    }
                });
            });
        if go {
            self.ask_restore_all(&st);
            open = false;
        }
        if open {
            self.restore_all = Some(st);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_whole_pedal_backup_is_read_by_folder_and_file_name() {
        let d = std::env::temp_dir().join(format!("tonestrand-restore-all-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for sub in ["PRESETS", "IRs", "Other"] {
            std::fs::create_dir_all(d.join(sub)).unwrap();
        }
        for f in [
            "PRESETS/01A Clean.hxpreset",
            "PRESETS/42B Wish acoustic.hxpreset",
            "PRESETS/oops.hxpreset",
            "IRs/003 Room.wav",
            "IRs/room.wav",
            "Other/01A X.hxpreset",
        ] {
            std::fs::write(d.join(f), b"x").unwrap();
        }
        let (presets, irs, problems) = scan_full_backup(&d, &["PRESETS".to_owned()]);
        let slots: Vec<(usize, usize, &str)> = presets.iter().map(|p| (p.setlist, p.slot, p.name.as_str())).collect();
        assert_eq!(slots, vec![(0, 0, "Clean"), (0, 124, "Wish acoustic")]);
        assert_eq!(irs.len(), 1);
        assert_eq!((irs[0].slot, irs[0].name.as_str()), (2, "Room"));
        // The unnamed preset, the unnumbered IR and the unknown setlist folder are reported.
        assert_eq!(problems.len(), 3, "{problems:?}");
        let _ = std::fs::remove_dir_all(&d);
    }
}
