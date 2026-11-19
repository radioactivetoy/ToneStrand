//! Things that differ between the desktop and Android. File pickers: on the desktop they are
//! the system's dialogs (rfd); Android has no such dialog here yet, so they pick nothing.

use std::path::{Path, PathBuf};

#[cfg(not(target_os = "android"))]
pub struct FileDialog(rfd::FileDialog);

#[cfg(not(target_os = "android"))]
impl FileDialog {
    pub fn new() -> Self {
        Self(rfd::FileDialog::new())
    }
    pub fn add_filter(self, name: impl Into<String>, extensions: &[impl ToString]) -> Self {
        Self(self.0.add_filter(name, extensions))
    }
    pub fn set_directory<P: AsRef<Path>>(self, path: P) -> Self {
        Self(self.0.set_directory(path))
    }
    pub fn set_title(self, title: impl Into<String>) -> Self {
        Self(self.0.set_title(title))
    }
    pub fn set_file_name(self, name: impl Into<String>) -> Self {
        Self(self.0.set_file_name(name))
    }
    pub fn pick_file(self) -> Option<PathBuf> {
        self.0.pick_file()
    }
    pub fn pick_files(self) -> Option<Vec<PathBuf>> {
        self.0.pick_files()
    }
    pub fn pick_folder(self) -> Option<PathBuf> {
        self.0.pick_folder()
    }
    pub fn save_file(self) -> Option<PathBuf> {
        self.0.save_file()
    }
}

#[cfg(target_os = "android")]
pub struct FileDialog;

#[cfg(target_os = "android")]
impl FileDialog {
    pub fn new() -> Self {
        Self
    }
    pub fn add_filter(self, _name: impl Into<String>, _extensions: &[impl ToString]) -> Self {
        self
    }
    pub fn set_directory<P: AsRef<Path>>(self, _path: P) -> Self {
        self
    }
    pub fn set_title(self, _title: impl Into<String>) -> Self {
        self
    }
    pub fn set_file_name(self, _name: impl Into<String>) -> Self {
        self
    }
    pub fn pick_file(self) -> Option<PathBuf> {
        None
    }
    pub fn pick_files(self) -> Option<Vec<PathBuf>> {
        None
    }
    pub fn pick_folder(self) -> Option<PathBuf> {
        None
    }
    pub fn save_file(self) -> Option<PathBuf> {
        None
    }
}

impl Default for FileDialog {
    fn default() -> Self {
        Self::new()
    }
}
