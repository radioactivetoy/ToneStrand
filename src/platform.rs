//! Things that differ between the desktop and Android. File pickers are asynchronous: a request
//! is started with `FileDialog::open_file` and friends, which return a request id, and the answer
//! is collected with `poll()` each frame. On the desktop the system's dialogs (rfd) answer at
//! once; on Android a backend installed by the android crate (`set_backend`) answers later with
//! `deliver`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

pub enum PickKind {
    Open { multiple: bool },
    Save { suggested: String },
    Folder,
}

pub struct PickRequest {
    pub id: u64,
    pub kind: PickKind,
    pub title: Option<String>,
    pub filters: Vec<(String, Vec<String>)>,
    pub dir: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PickResult {
    Files(Vec<PathBuf>),
    SaveTo(PathBuf),
    Folder(PathBuf),
    Saved,
    Cancelled,
    Failed(String),
}

impl PickResult {
    /// Whether this ends the request (a `SaveTo` still waits for the write and `commit_save`).
    fn is_final(&self) -> bool {
        !matches!(self, PickResult::SaveTo(_))
    }
}

type Backend = Arc<dyn Fn(PickRequest) + Send + Sync>;
type CommitBackend = Arc<dyn Fn(u64, PathBuf) + Send + Sync>;

struct State {
    pending: Option<u64>,
    /// The pending request already has its final result queued (it stays pending until polled).
    answered: bool,
    next_id: u64,
    queue: Vec<(u64, PickResult)>,
    backend: Option<Backend>,
    commit_backend: Option<CommitBackend>,
}

impl State {
    const fn new() -> Self {
        Self { pending: None, answered: false, next_id: 1, queue: Vec::new(), backend: None, commit_backend: None }
    }
}

static STATE: Mutex<State> = Mutex::new(State::new());

fn state() -> MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Installs the picker of the platform (Android). Replaces the desktop dialogs.
pub fn set_backend(backend: Box<dyn Fn(PickRequest) + Send + Sync>) {
    state().backend = Some(Arc::from(backend));
}

/// Installs what `commit_save` hands the finished file to (Android).
pub fn set_commit_backend(f: Box<dyn Fn(u64, PathBuf) + Send + Sync>) {
    state().commit_backend = Some(Arc::from(f));
}

/// Answers a request; called by the backend, from any thread.
/// Results for a request that is not the pending one (stale answers) are dropped.
pub fn deliver(id: u64, result: PickResult) {
    let mut s = state();
    if s.pending == Some(id) && !s.answered {
        s.answered = result.is_final();
        s.queue.push((id, result));
    }
}

/// Escape hatch for a stuck request: clears the pending one and queues `Cancelled` for it.
pub fn cancel_pending() {
    let mut s = state();
    if let (Some(id), false) = (s.pending, s.answered) {
        s.answered = true;
        s.queue.push((id, PickResult::Cancelled));
    }
}

/// The app's write after `SaveTo` failed: ends the save with `Failed`.
pub fn abort_save(id: u64, error: String) {
    deliver(id, PickResult::Failed(error));
}

/// Finished requests, oldest first. A request stops being pending when its final result is taken.
pub fn poll() -> Vec<(u64, PickResult)> {
    let mut s = state();
    let out = std::mem::take(&mut s.queue);
    for (id, r) in &out {
        if r.is_final() && s.pending == Some(*id) {
            s.pending = None;
            s.answered = false;
        }
    }
    out
}

/// After the whole file has been written to the `SaveTo` path.
pub fn commit_save(id: u64, written: &Path) {
    let commit = {
        let s = state();
        if s.pending != Some(id) || s.answered {
            return;
        }
        s.commit_backend.clone()
    };
    match commit {
        Some(f) => f(id, written.to_path_buf()),
        #[cfg(not(target_os = "android"))]
        None => deliver(id, PickResult::Saved),
        #[cfg(target_os = "android")]
        None => deliver(id, PickResult::Failed("No file picker".into())),
    }
}

static CACHE_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Where the picker keeps its temporary copies (Android); set at start-up by the android crate.
pub fn set_cache_dir(dir: PathBuf) {
    *CACHE_DIR.lock().unwrap_or_else(|e| e.into_inner()) = Some(dir);
}

/// Whether a path is inside the app's cache folder: such files are temporary copies, so they are
/// never remembered (recent files, last session's tabs). Always false when no cache folder is set
/// (the desktop).
pub fn is_cache_path(p: &Path) -> bool {
    CACHE_DIR.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|c| p.starts_with(c))
}

#[cfg(test)]
fn reset_for_tests() {
    *state() = State::new();
}

#[derive(Default)]
pub struct FileDialog {
    title: Option<String>,
    filters: Vec<(String, Vec<String>)>,
    dir: Option<PathBuf>,
    file_name: Option<String>,
}

impl FileDialog {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn add_filter(mut self, name: impl Into<String>, extensions: &[impl ToString]) -> Self {
        self.filters.push((name.into(), extensions.iter().map(|e| e.to_string()).collect()));
        self
    }
    pub fn set_directory<P: AsRef<Path>>(mut self, path: P) -> Self {
        self.dir = Some(path.as_ref().to_path_buf());
        self
    }
    pub fn set_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }
    pub fn set_file_name(mut self, name: impl Into<String>) -> Self {
        self.file_name = Some(name.into());
        self
    }

    pub fn open_file(self) -> Option<u64> {
        self.request(PickKind::Open { multiple: false })
    }
    pub fn open_files(self) -> Option<u64> {
        self.request(PickKind::Open { multiple: true })
    }
    pub fn save_file(self) -> Option<u64> {
        let suggested = self.file_name.clone().filter(|n| !n.is_empty()).unwrap_or_else(|| "Untitled".into());
        self.request(PickKind::Save { suggested })
    }
    pub fn pick_folder(self) -> Option<u64> {
        self.request(PickKind::Folder)
    }

    /// Starts a request; `None` when one is already pending.
    fn request(self, kind: PickKind) -> Option<u64> {
        let (id, backend) = {
            let mut s = state();
            if s.pending.is_some() {
                return None;
            }
            let id = s.next_id;
            s.next_id += 1;
            s.pending = Some(id);
            s.answered = false;
            (id, s.backend.clone())
        };
        match backend {
            Some(b) => b(PickRequest { id, kind, title: self.title, filters: self.filters, dir: self.dir }),
            None => {
                let result = self.run_without_backend(&kind);
                deliver(id, result)
            }
        }
        Some(id)
    }

    #[cfg(not(target_os = "android"))]
    fn run_without_backend(self, kind: &PickKind) -> PickResult {
        let d = self.to_rfd();
        match kind {
            PickKind::Open { multiple: false } => d.pick_file().map_or(PickResult::Cancelled, |p| PickResult::Files(vec![p])),
            PickKind::Open { multiple: true } => d.pick_files().map_or(PickResult::Cancelled, PickResult::Files),
            PickKind::Save { .. } => d.save_file().map_or(PickResult::Cancelled, PickResult::SaveTo),
            PickKind::Folder => d.pick_folder().map_or(PickResult::Cancelled, PickResult::Folder),
        }
    }

    #[cfg(target_os = "android")]
    fn run_without_backend(self, _kind: &PickKind) -> PickResult {
        PickResult::Failed("No file picker".into())
    }

    #[cfg(not(target_os = "android"))]
    fn to_rfd(&self) -> rfd::FileDialog {
        let mut d = rfd::FileDialog::new();
        if let Some(t) = &self.title {
            d = d.set_title(t);
        }
        for (name, exts) in &self.filters {
            d = d.add_filter(name, exts);
        }
        if let Some(p) = &self.dir {
            d = d.set_directory(p);
        }
        if let Some(n) = &self.file_name {
            d = d.set_file_name(n);
        }
        d
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static TEST_LOCK: Mutex<()> = Mutex::new(());
    static LAST: Mutex<Option<PickRequest>> = Mutex::new(None);
    static COMMITS: Mutex<Vec<(u64, PathBuf)>> = Mutex::new(Vec::new());

    fn setup() -> MutexGuard<'static, ()> {
        let g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_for_tests();
        *LAST.lock().unwrap() = None;
        COMMITS.lock().unwrap().clear();
        set_backend(Box::new(|r| *LAST.lock().unwrap() = Some(r)));
        g
    }

    #[test]
    fn cancelled_request_yields_cancelled() {
        let _g = setup();
        let id = FileDialog::new().open_file().unwrap();
        deliver(id, PickResult::Cancelled);
        assert_eq!(poll(), vec![(id, PickResult::Cancelled)]);
        assert!(FileDialog::new().open_file().is_some());
    }

    #[test]
    fn second_request_while_pending_is_refused() {
        let _g = setup();
        let id = FileDialog::new().open_file().unwrap();
        assert!(FileDialog::new().open_file().is_none());
        deliver(id, PickResult::Files(vec![PathBuf::from("a")]));
        assert!(FileDialog::new().open_file().is_none(), "still pending until polled");
        assert_eq!(poll().len(), 1);
        assert!(FileDialog::new().open_file().is_some());
    }

    #[test]
    fn save_flow() {
        let _g = setup();
        set_commit_backend(Box::new(|id, p| COMMITS.lock().unwrap().push((id, p))));
        let id = FileDialog::new().set_file_name("x.hlx").save_file().unwrap();
        assert!(matches!(&LAST.lock().unwrap().as_ref().unwrap().kind, PickKind::Save { suggested } if suggested == "x.hlx"));
        deliver(id, PickResult::SaveTo(PathBuf::from("/tmp/x.hlx")));
        assert_eq!(poll(), vec![(id, PickResult::SaveTo(PathBuf::from("/tmp/x.hlx")))]);
        assert!(FileDialog::new().open_file().is_none(), "SaveTo is not final");
        commit_save(id, Path::new("/tmp/x.hlx"));
        assert_eq!(*COMMITS.lock().unwrap(), vec![(id, PathBuf::from("/tmp/x.hlx"))]);
        deliver(id, PickResult::Saved);
        assert_eq!(poll(), vec![(id, PickResult::Saved)]);
        assert!(FileDialog::new().open_file().is_some());
    }

    #[test]
    fn cancel_pending_unblocks() {
        let _g = setup();
        cancel_pending();
        assert!(poll().is_empty());
        let id = FileDialog::new().open_file().unwrap();
        cancel_pending();
        assert_eq!(poll(), vec![(id, PickResult::Cancelled)]);
        assert!(FileDialog::new().open_file().is_some());
    }

    #[test]
    fn abort_save_ends_a_save() {
        let _g = setup();
        let id = FileDialog::new().save_file().unwrap();
        deliver(id, PickResult::SaveTo(PathBuf::from("p")));
        poll();
        abort_save(id, "disk full".into());
        assert_eq!(poll(), vec![(id, PickResult::Failed("disk full".into()))]);
        assert!(FileDialog::new().open_file().is_some());
    }

    #[test]
    fn stale_results_are_dropped() {
        let _g = setup();
        let id = FileDialog::new().open_file().unwrap();
        deliver(id + 7, PickResult::Cancelled);
        commit_save(id + 7, Path::new("p"));
        assert!(poll().is_empty());
        deliver(id, PickResult::Cancelled);
        assert_eq!(poll().len(), 1);
    }

    #[test]
    fn double_final_answer_queues_only_one() {
        let _g = setup();
        let id = FileDialog::new().open_file().unwrap();
        deliver(id, PickResult::Cancelled);
        cancel_pending();
        abort_save(id, "x".into());
        deliver(id, PickResult::Files(vec![]));
        assert_eq!(poll(), vec![(id, PickResult::Cancelled)]);
        assert!(FileDialog::new().open_file().is_some());
    }

    #[test]
    fn abort_save_with_stale_id_is_ignored() {
        let _g = setup();
        let id = FileDialog::new().save_file().unwrap();
        abort_save(id + 1, "x".into());
        assert!(poll().is_empty());
        assert!(FileDialog::new().open_file().is_none());
    }

    #[test]
    fn save_suggests_untitled() {
        let _g = setup();
        FileDialog::new().set_file_name("").save_file().unwrap();
        assert!(matches!(&LAST.lock().unwrap().as_ref().unwrap().kind, PickKind::Save { suggested } if suggested == "Untitled"));
        poll();
        cancel_pending();
        poll();
        FileDialog::new().save_file().unwrap();
        assert!(matches!(&LAST.lock().unwrap().as_ref().unwrap().kind, PickKind::Save { suggested } if suggested == "Untitled"));
    }

    #[test]
    fn commit_without_backend_queues_saved() {
        let _g = setup();
        let id = FileDialog::new().save_file().unwrap();
        deliver(id, PickResult::SaveTo(PathBuf::from("p")));
        poll();
        commit_save(id, Path::new("p"));
        assert_eq!(poll(), vec![(id, PickResult::Saved)]);
    }

    #[test]
    fn filters_and_title_reach_the_backend() {
        let _g = setup();
        FileDialog::new().set_title("T").add_filter("Presets", &["hlx", "hxpreset"]).set_directory("/d").open_files().unwrap();
        let r = LAST.lock().unwrap().take().unwrap();
        assert_eq!(r.title.as_deref(), Some("T"));
        assert_eq!(r.filters, vec![("Presets".to_string(), vec!["hlx".to_string(), "hxpreset".to_string()])]);
        assert_eq!(r.dir, Some(PathBuf::from("/d")));
        assert!(matches!(r.kind, PickKind::Open { multiple: true }));
    }
}
