//! File dialogs are asynchronous: a call site starts a request (`platform::FileDialog`) and records
//! what it was for as a `Pick`; the answer is collected at the start of the next frame and
//! handled here.

use super::*;
use crate::platform::{self, PickResult};

/// What a file dialog in progress is for: one variant per place that opens one.
#[allow(clippy::enum_variant_names)]
pub(super) enum Pick {
    OpenPreset,
    /// The compare window: which side (0 = A, 1 = B) the file goes to.
    ComparePick(usize),
    ImportLibrary,
    /// Copy the files into this folder (the library's, as it was at the click).
    ImportTones(PathBuf),
    UploadIrs,
    NamCaptures,
    /// The captures picked first, waiting for the folder to write the .wav files in.
    NamOutFolder(Vec<PathBuf>),
    /// Save a document (its `Doc::id`) as a file; the bytes are what to write.
    SaveAs(u64, Vec<u8>),
    SavePedalPreset(Vec<u8>),
    SaveLog,
    /// Back up a setlist (number, count and names as they were at the click) into the folder.
    BackupAllFolder(i64, usize, Vec<String>),
    FullBackupFolder,
    RestoreAllFolder,
    LibraryFolder,
    ToneFolder,
    HxEditFolder,
    TestsFolder,
}

impl Pick {
    /// The start of the note when the dialog failed.
    fn failure(&self) -> &'static str {
        match self {
            Pick::SaveAs(..) | Pick::SavePedalPreset(_) => "Could not save",
            Pick::SaveLog => "Could not save the log",
            Pick::OpenPreset | Pick::ComparePick(_) | Pick::ImportLibrary | Pick::ImportTones(_) | Pick::UploadIrs | Pick::NamCaptures => {
                "Could not pick the file"
            }
            _ => "Could not pick the folder",
        }
    }
}

/// The tab with this `Doc::id`, if it is still open.
pub(super) fn doc_index(docs: &[Doc], id: u64) -> Option<usize> {
    docs.iter().position(|d| d.id == id)
}

/// Whether a path may be kept in the settings (recent files, last session's tabs): not a
/// temporary copy in the app's cache folder.
pub(super) fn rememberable(p: &Path) -> bool {
    !platform::is_cache_path(p)
}

/// A new, dated folder for a backup: `<library>/Pedal backups/<label> <YYYY-MM-DD HHMM>` (UTC).
pub(super) fn dated_backup_folder(library: &Path, label: &str, secs: u64) -> PathBuf {
    library.join("Pedal backups").join(format!("{label} {}", crate::files::stamp_text(secs)))
}

/// The folders to choose from when restoring a whole-pedal backup without a folder picker: the
/// folders inside each base folder, newest name first.
pub(super) fn backup_folders(bases: &[PathBuf]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for base in bases {
        if let Ok(rd) = std::fs::read_dir(base) {
            out.extend(rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()));
        }
    }
    out.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
    out.dedup_by(|a, b| a.file_name() == b.file_name());
    out
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl App {
    /// Remember what a started dialog is for (`None`: another dialog is still open).
    pub(super) fn start_pick(&mut self, id: Option<u64>, pick: Pick) {
        match id {
            Some(id) => {
                self.pending.insert(id, pick);
            }
            None => self.note(LogLevel::Info, "A file picker is already open"),
        }
    }

    /// The answers that came in since the last frame.
    pub(super) fn poll_picks(&mut self) {
        for (id, result) in platform::poll() {
            let Some(pick) = self.pending.remove(&id) else { continue };
            self.handle_pick(id, pick, result);
        }
    }

    fn handle_pick(&mut self, id: u64, pick: Pick, result: PickResult) {
        // A save waits for its file to be written (SaveTo), then for the platform to deliver it (Saved).
        if let PickResult::SaveTo(path) = &result {
            let bytes = match &pick {
                Pick::SaveAs(_, b) | Pick::SavePedalPreset(b) => Some(b.clone()),
                Pick::SaveLog => Some(self.log_text().into_bytes()),
                _ => None,
            };
            if let Some(bytes) = bytes {
                return self.write_save(id, pick, path, &bytes);
            }
        }
        match result {
            PickResult::Cancelled => {
                self.saved_to.remove(&id);
                if matches!(pick, Pick::SaveAs(..)) {
                    self.after_save = None;
                }
            }
            PickResult::Failed(e) => {
                self.saved_to.remove(&id);
                if matches!(pick, Pick::SaveAs(..)) {
                    self.after_save = None;
                }
                self.note(LogLevel::Bad, format!("{}: {e}", pick.failure()));
            }
            PickResult::Saved => {
                let path = self.saved_to.remove(&id);
                self.saved(pick, path);
            }
            PickResult::Files(files) => self.picked_files(pick, files),
            PickResult::Folder(folder) => self.picked_folder(pick, folder),
            // A save path for a pick that does not save: nothing to do with it.
            PickResult::SaveTo(_) => {}
        }
    }

    /// Write the whole file to where the person said, then hand it to the platform.
    fn write_save(&mut self, id: u64, pick: Pick, path: &Path, bytes: &[u8]) {
        // The pick stays pending until the save is final (`Saved` or `Failed`).
        match crate::files::write_atomic(path, bytes) {
            Ok(_) => {
                self.pending.insert(id, pick);
                self.saved_to.insert(id, path.to_owned());
                platform::commit_save(id, path);
            }
            Err(e) => {
                self.pending.insert(id, pick);
                platform::abort_save(id, e.to_string());
            }
        }
    }

    fn saved(&mut self, pick: Pick, path: Option<PathBuf>) {
        let Some(path) = path else { return };
        let shown = if rememberable(&path) {
            path.display().to_string()
        } else {
            path.file_name().and_then(|s| s.to_str()).unwrap_or("the file").to_owned()
        };
        match pick {
            Pick::SaveAs(id, _) => {
                let found = doc_index(&self.docs, id);
                if let Some(doc) = found.and_then(|i| self.docs.get_mut(i)) {
                    if rememberable(&path) {
                        doc.path = Some(path.clone());
                    }
                    doc.dirty = false;
                    doc.example = false;
                }
                self.note(LogLevel::Good, format!("Saved {shown}"));
                self.scan_library();
                match self.after_save.take() {
                    Some(CloseAsk::Tab(t)) if t == id => {
                        if let Some(i) = doc_index(&self.docs, t).filter(|&i| !self.docs[i].needs_save()) {
                            self.close_tab(i);
                        }
                    }
                    Some(CloseAsk::Quit) => self.save_unsaved_then_quit(),
                    _ => {}
                }
            }
            Pick::SavePedalPreset(_) => {
                self.note(LogLevel::Good, format!("Saved {shown}"));
                self.scan_library();
            }
            Pick::SaveLog => self.note(LogLevel::Good, format!("Saved the log to {shown}")),
            _ => {}
        }
    }

    fn picked_files(&mut self, pick: Pick, files: Vec<PathBuf>) {
        if files.is_empty() {
            return;
        }
        match pick {
            Pick::OpenPreset => {
                for p in &files {
                    self.open_file(p);
                }
            }
            Pick::ComparePick(side) => {
                if let Some(cat) = self.catalog.clone() {
                    self.set_compare_side(side, &files[0], &cat);
                }
            }
            Pick::ImportLibrary => self.import_files(files),
            Pick::ImportTones(dir) => {
                let _ = std::fs::create_dir_all(&dir);
                let n = files.iter().filter(|f| crate::files::copy_into(f, &dir).is_ok()).count();
                self.note(LogLevel::Good, format!("Imported {n} file{}", if n == 1 { "" } else { "s" }));
                self.scan_tones();
            }
            Pick::UploadIrs => self.upload_ir_files(files),
            Pick::NamCaptures => {
                if cfg!(target_os = "android") {
                    // No folder picker here: the converted files go to a fixed folder.
                    let out = self.tone_dir().join("From NAM");
                    let _ = std::fs::create_dir_all(&out);
                    self.convert_nam(files, out);
                } else {
                    let start = self.settings.nam_out_dir.clone().map(PathBuf::from).unwrap_or_else(|| self.library_dir.clone());
                    let id =
                        crate::platform::FileDialog::new().set_title("Where to save the .wav files").set_directory(&start).pick_folder();
                    self.start_pick(id, Pick::NamOutFolder(files));
                }
            }
            _ => {}
        }
    }

    fn picked_folder(&mut self, pick: Pick, folder: PathBuf) {
        match pick {
            Pick::NamOutFolder(files) => {
                self.settings.nam_out_dir = Some(folder.to_string_lossy().into_owned());
                self.settings.save();
                self.convert_nam(files, folder);
            }
            Pick::BackupAllFolder(setlist, count, names) => self.send(Cmd::Backup { setlist, count, folder, names }),
            Pick::FullBackupFolder => self.send(Cmd::FullBackup { folder }),
            Pick::RestoreAllFolder => self.restore_all_from(folder),
            Pick::LibraryFolder => self.set_library_dir(Some(folder)),
            Pick::ToneFolder => {
                self.settings.tone_library = Some(folder.to_string_lossy().into_owned());
                self.settings.save();
                self.scan_tones();
                self.scan_library();
            }
            Pick::HxEditFolder => {
                self.settings.hx_edit = Some(folder.to_string_lossy().into_owned());
                self.settings.save();
                self.reload_catalog();
            }
            Pick::TestsFolder => self.tests.folder = Some(folder),
            _ => {}
        }
    }

    /// Back up the shown setlist's presets into `folder`.
    pub(super) fn back_up_setlist(&mut self, folder: PathBuf) {
        let (names, setlist) = (self.presets.clone(), self.setlist);
        self.send(Cmd::Backup { setlist, count: names.len(), folder, names });
    }

    /// Android has no folder picker: the setlist goes into a new dated folder of the backups.
    pub(super) fn back_up_setlist_here(&mut self) {
        let label = match self.setlists.get(self.setlist as usize) {
            Some(n) if !n.is_empty() => format!("Setlist {}", device::sanitize(n)),
            _ => "Setlist".to_owned(),
        };
        let wanted = dated_backup_folder(&self.library_dir, &label, now_secs());
        let (Some(parent), Some(name)) = (wanted.parent(), wanted.file_name().and_then(|n| n.to_str())) else { return };
        let folder = crate::files::unique_path(parent, name);
        self.note(LogLevel::Info, format!("Backing up to {}", folder.display()));
        self.back_up_setlist(folder);
    }

    /// Android has no folder picker: the whole pedal goes into a new folder (named by the
    /// time, as on the desktop) inside the backups folder.
    pub(super) fn back_up_pedal_here(&mut self) {
        let folder = self.backup_dir();
        self.note(LogLevel::Info, format!("Backing up the whole pedal into {}", folder.display()));
        self.send(Cmd::FullBackup { folder });
    }

    /// "Save" in the close dialog for every unsaved tab: those with a file first, then one at a
    /// time through the save dialog; quits when nothing is left unsaved.
    pub(super) fn save_unsaved_then_quit(&mut self) {
        let with_file: Vec<usize> = (0..self.docs.len()).filter(|&i| self.docs[i].needs_save() && self.docs[i].path.is_some()).collect();
        for i in with_file {
            self.save_doc(i, false);
        }
        if let Some(i) = (0..self.docs.len()).find(|&i| self.docs[i].needs_save() && self.docs[i].path.is_none()) {
            self.save_doc(i, false);
            if self.pending.values().any(|p| matches!(p, Pick::SaveAs(..))) {
                self.after_save = Some(CloseAsk::Quit);
            }
            return;
        }
        if !self.docs.iter().any(Doc::needs_save) {
            self.quit_ok = true;
            self.close_requested = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_paths_are_not_remembered() {
        let cache = std::env::temp_dir().join("tonestrand-test-picker-cache");
        platform::set_cache_dir(cache.clone());
        assert!(!rememberable(&cache.join("1").join("x.hlx")));
        assert!(rememberable(&std::env::temp_dir().join("tonestrand-test-other").join("x.hlx")));
    }

    #[test]
    fn android_backup_folder_name() {
        let p = dated_backup_folder(Path::new("/lib"), "Setlist PRESETS", 0);
        assert_eq!(p, Path::new("/lib").join("Pedal backups").join("Setlist PRESETS 1970-01-01 0000"));
        // One minute and an hour and a day later.
        let p = dated_backup_folder(Path::new("/lib"), "Full backup", 86_400 + 3_600 * 14 + 180);
        assert_eq!(p, Path::new("/lib").join("Pedal backups").join("Full backup 1970-01-02 1403"));
    }

    #[test]
    fn a_tab_is_found_by_its_id_after_others_close() {
        let doc = || Doc {
            id: next_doc_id(),
            title: String::new(),
            name: String::new(),
            path: None,
            kind: DocKind::Hlx { json: crate::hlx::blank_for("x", 0) },
            dirty: false,
            selected: None,
            example: false,
            undo: Vec::new(),
            redo: Vec::new(),
            last_edit: None,
        };
        let mut docs = vec![doc(), doc(), doc()];
        let (first, last) = (docs[0].id, docs[2].id);
        assert_ne!(first, last);
        assert_eq!(doc_index(&docs, last), Some(2));
        docs.remove(0);
        assert_eq!(doc_index(&docs, last), Some(1));
        assert_eq!(doc_index(&docs, first), None);
    }

    #[test]
    fn backup_folders_are_listed_newest_first_without_files() {
        let d = std::env::temp_dir().join(format!("tonestrand-backup-folders-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let (a, b) = (d.join("one"), d.join("two"));
        for f in ["Full backup 2026-11-01 1000", "Full backup 2026-11-02 1000"] {
            std::fs::create_dir_all(a.join(f)).unwrap();
        }
        std::fs::create_dir_all(b.join("Full backup 2026-11-01 1000")).unwrap();
        std::fs::write(a.join("01A x - before overwrite 1.hxpreset"), b"x").unwrap();
        let list = backup_folders(&[a.clone(), b]);
        let names: Vec<&str> = list.iter().filter_map(|p| p.file_name().and_then(|n| n.to_str())).collect();
        assert_eq!(names, vec!["Full backup 2026-11-02 1000", "Full backup 2026-11-01 1000"]);
        let _ = std::fs::remove_dir_all(&d);
    }
}
