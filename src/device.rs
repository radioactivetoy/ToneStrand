//! A background thread that owns the pedal session, so the window never waits
//! on USB or Wi-Fi. The window sends commands; the thread answers with events.

use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use crate::msgpack::Value;
use std::path::{Path, PathBuf};

use crate::session::{Session, IR_NAME_MAX, PRESET_NAME_MAX, SNAPSHOT_NAME_MAX};
use crate::wire::{BoxWire, UsbWire, Wire};

#[derive(Clone)]
pub enum Target {
    Usb { pid: u16 },
    Box { host: String, token: String, label: String },
}

pub enum Cmd {
    Refresh,
    Presets(i64),
    Select(i64, i64),
    FetchSlot {
        setlist: i64,
        index: i64,
        purpose: Purpose,
    },
    /// `backup`: the folder the slot is kept in before it is written over.
    WriteSlot {
        setlist: i64,
        index: i64,
        name: String,
        doc: Vec<u8>,
        backup: PathBuf,
    },
    /// Copy, move or swap two preset slots (each slot that changes is backed up first).
    SlotOp {
        setlist: i64,
        from: i64,
        to: i64,
        op: SlotOp,
        backup: PathBuf,
    },
    Rename {
        setlist: i64,
        index: i64,
        name: String,
    },
    Clear {
        setlist: i64,
        index: i64,
    },
    SaveCurrent {
        setlist: i64,
        index: i64,
        name: String,
    },
    SetParam {
        block: i64,
        path: i64,
        index: i64,
        value: Value,
    },
    SetEnabled {
        block: i64,
        enabled: bool,
    },
    SetModel {
        block: i64,
        model: i64,
        paired: Option<i64>,
    },
    ReplaceBlock {
        block: i64,
        slot: Value,
    },
    ClearBlock {
        block: i64,
    },
    AddBlock {
        block: i64,
        body: Value,
    },
    MoveBlock {
        from: i64,
        to: i64,
    },
    MoveSplit {
        block: i64,
    },
    MoveJoin {
        block: i64,
    },
    SetRoute {
        block: i64,
        input: bool,
        value: i64,
    },
    Snapshot(i64),
    RenameSnapshot {
        index: i64,
        name: String,
    },
    /// Change a whole-number preset setting of the loaded preset (guitar impedance 30, guitar pad 134): the
    /// pedal's current document with that setting changed is written back to its edit buffer (not saved to a slot).
    SetSetting {
        field: i64,
        value: i64,
    },
    /// Set a Command Center slot of the loaded preset: the pedal's current document with that slot
    /// changed is written back to its edit buffer (not saved to a slot).
    SetCommand {
        slot: usize,
        id: i64,
        args: Vec<i64>,
    },
    /// Change a command's values in one snapshot of the loaded preset (the other snapshots keep theirs).
    SetCommandValues {
        snapshot: usize,
        slot: usize,
        args: Vec<i64>,
    },
    /// Make a Command Center footswitch momentary or latching on the loaded preset.
    SetMomentary {
        slot: usize,
        momentary: bool,
    },
    /// Make a snapshot of the loaded preset the same as another (the pedal's own copy and paste).
    CopySnapshot {
        from: i64,
        to: i64,
        /// The destination's name, kept.
        keep_name: String,
    },
    Backup {
        setlist: i64,
        count: usize,
        folder: std::path::PathBuf,
        names: Vec<String>,
    },
    /// Save every setlist's presets and all User IRs into a new folder inside `folder`.
    FullBackup {
        folder: std::path::PathBuf,
    },
    /// Write a whole-pedal backup back: presets as (setlist, slot, name, document) and IRs as
    /// (slot, name, audio). Every slot replaced is backed up into `backup` first.
    RestoreAll {
        presets: Vec<(i64, i64, String, Vec<u8>)>,
        irs: Vec<(i64, String, Vec<u8>)>,
        backup: PathBuf,
    },
    /// Read the names of the pedal's User IRs.
    ListIrs,
    /// Write IRs into slots: (slot from 0, name, 8,192 bytes of audio).
    /// `backup`: the folder replaced IRs are kept in first.
    ImportIrs {
        items: Vec<(i64, String, Vec<u8>)>,
        backup: PathBuf,
    },
    /// Save IRs from slots as .wav files in a folder: (slot, name).
    ExportIrs {
        slots: Vec<(i64, String)>,
        folder: std::path::PathBuf,
    },
    RenameIr {
        slot: i64,
        name: String,
    },
    DeleteIrs {
        slots: Vec<i64>,
        backup: PathBuf,
    },
    /// Look at slots' audio to tell IRs from NAM captures: (slot, name).
    IrKinds(Vec<(i64, String)>),
    /// Convert a .hlx preset to the pedal's format (using the loaded preset as the
    /// template) and write it into a slot, then read it back to check.
    WriteHlx {
        setlist: i64,
        index: i64,
        name: String,
        json: serde_json::Value,
        cat: std::sync::Arc<crate::catalog::Catalog>,
        backup: PathBuf,
    },
    /// Live controller and footswitch changes on the loaded preset.
    AssignController {
        block: i64,
        cab: bool,
        index: i64,
        controller: i64,
        cc: i64,
    },
    UnassignController {
        block: i64,
        cab: bool,
        index: i64,
    },
    ControllerBound {
        max: bool,
        block: i64,
        cab: bool,
        index: i64,
        value: Value,
    },
    ControllerCc {
        block: i64,
        cab: bool,
        index: i64,
        cc: i64,
    },
    AssignFootswitch {
        block: i64,
        fs: i64,
    },
    UnassignFootswitch {
        block: i64,
        fs: i64,
    },
    /// The hidden Tests page's careful run of every operation.
    SelfTest(crate::selftest::Plan),
    Disconnect,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Purpose {
    Edit,
    SaveToFile,
}

pub enum Event {
    Connected(String),
    Failed(String),
    Disconnected(String),
    Busy(String),
    Info(String),
    Error(String),
    Setlists(Vec<String>),
    Presets(i64, Vec<String>),
    Current(i64, i64, String),
    Document {
        setlist: i64,
        index: i64,
        live: bool,
        purpose: Purpose,
        blob: Option<Vec<u8>>,
    },
    Done(String),
    /// The pedal's User IRs that hold something: (slot from 0, name).
    Irs(Vec<(i64, String)>),
    /// Which slots hold NAM captures: (slot, name, is a NAM capture).
    IrKinds(Vec<(i64, String, bool)>),
    /// A self-test step: passed (Some(true)), failed (Some(false)) or a note (None).
    TestStep {
        name: String,
        ok: Option<bool>,
        detail: String,
    },
    TestDone {
        failures: usize,
    },
}

impl Cmd {
    /// Whether the pedal thread answers this command with a fresh copy of the loaded preset. The window
    /// counts these so that an older copy, still on its way when the next edit is made, doesn't put the
    /// previous state back on screen.
    pub fn refreshes_live(&self) -> bool {
        matches!(
            self,
            Cmd::SetModel { .. }
                | Cmd::ReplaceBlock { .. }
                | Cmd::AddBlock { .. }
                | Cmd::MoveBlock { .. }
                | Cmd::MoveSplit { .. }
                | Cmd::MoveJoin { .. }
                | Cmd::ClearBlock { .. }
                | Cmd::Snapshot(_)
                | Cmd::SetSetting { .. }
                | Cmd::CopySnapshot { .. }
                | Cmd::AssignController { .. }
                | Cmd::UnassignController { .. }
                | Cmd::AssignFootswitch { .. }
                | Cmd::UnassignFootswitch { .. }
        )
    }
}

pub struct Link {
    pub tx: Sender<Cmd>,
    pub rx: Receiver<Event>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Link {
    /// Ask the pedal thread to stop and wait up to `limit` for it, so a write in progress
    /// finishes and the USB / box link is closed cleanly. True when it has stopped.
    pub fn shut_down(&mut self, limit: Duration) -> bool {
        let _ = self.tx.send(Cmd::Disconnect);
        let Some(t) = self.thread.take() else { return true };
        let until = std::time::Instant::now() + limit;
        while !t.is_finished() && std::time::Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
        }
        if t.is_finished() {
            let _ = t.join();
            true
        } else {
            false
        }
    }
}

pub fn connect(target: Target, repaint: impl Fn() + Send + 'static) -> Link {
    let (cmd_tx, cmd_rx) = channel::<Cmd>();
    let (ev_tx, ev_rx) = channel::<Event>();
    let thread = std::thread::spawn(move || {
        let send = |e: Event| {
            let _ = ev_tx.send(e);
            repaint();
        };
        send(Event::Busy("Connecting...".into()));
        let wire: Result<Box<dyn Wire>, String> = match target {
            Target::Usb { pid } => UsbWire::open(pid).map(|w| Box::new(w) as Box<dyn Wire>),
            Target::Box { host, token, label } => BoxWire::open(&host, &token, &label).map(|w| Box::new(w) as Box<dyn Wire>),
        };
        let wire = match wire {
            Ok(w) => w,
            Err(e) => return send(Event::Failed(e)),
        };
        let mut s = match Session::open(wire) {
            Ok(s) => s,
            Err(e) => return send(Event::Failed(e.to_string())),
        };
        send(Event::Connected(s.describe()));
        run(&mut s, &cmd_rx, &send);
    });
    Link { tx: cmd_tx, rx: ev_rx, thread: Some(thread) }
}

fn refresh(s: &mut Session, send: &dyn Fn(Event)) -> Result<i64, String> {
    let setlists = s.setlists().map_err(|e| e.to_string())?;
    let (setlist, index, name) = s.preset_info().map_err(|e| e.to_string())?;
    let setlist = setlist.max(0);
    send(Event::Setlists(setlists));
    let presets = s.presets(setlist).map_err(|e| e.to_string())?;
    send(Event::Presets(setlist, presets));
    send(Event::Current(setlist, index, name));
    Ok(setlist)
}

/// Read the playing preset and hand it to the window as the live document.
fn read_live(s: &mut Session, send: &dyn Fn(Event)) -> Result<i64, String> {
    let blob = s.read_current().map_err(|e| e.to_string())?;
    let (sl, idx, name) = s.preset_info().map_err(|e| e.to_string())?;
    send(Event::Current(sl.max(0), idx, name));
    send(Event::Document { setlist: sl.max(0), index: idx, live: true, purpose: Purpose::Edit, blob: Some(blob) });
    Ok(idx)
}

/// Write a changed Command Center to the pedal's edit buffer. The window already shows the change, so it is
/// only handed the pedal's document back when the pedal kept something different (a refresh after every edit
/// raced with the next one and put an older state back on screen).
fn write_command_center(s: &mut Session, send: &dyn Fn(Event), doc: &crate::devpreset::DevicePreset) -> Result<(), String> {
    s.write_edit_buffer(doc.encode_checked()?).map_err(|e| e.to_string())?;
    let blob = s.read_current().map_err(|e| e.to_string())?;
    let same = crate::devpreset::DevicePreset::parse(&blob).is_ok_and(|back| back.command_state() == doc.command_state());
    if !same {
        read_live(s, send)?;
    }
    Ok(())
}

fn run(s: &mut Session, rx: &Receiver<Cmd>, send: &dyn Fn(Event)) {
    if let Err(e) = refresh(s, send) {
        send(Event::Error(format!("Could not read the preset list: {e}")));
    }
    // Open the preset that is playing, ready to edit.
    let mut last_index = match read_live(s, send) {
        Ok(i) => i,
        Err(e) => {
            send(Event::Error(format!("Could not read the loaded preset: {e}")));
            -2
        }
    };
    // A small question every so often while nothing happens keeps the pedal's editing link
    // in use (left alone for long, live edits stopped landing) and finds out early when it
    // stopped answering, to pick the loaded preset up again.
    let mut last_beat = std::time::Instant::now();
    loop {
        let cmd = match rx.recv_timeout(Duration::from_millis(40)) {
            Ok(c) => Some(c),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        if cmd.is_some() {
            last_beat = std::time::Instant::now();
        }
        if cmd.is_none() && last_beat.elapsed() > Duration::from_secs(15) {
            last_beat = std::time::Instant::now();
            match s.preset_info() {
                Ok((sl, idx, name)) => {
                    if idx != last_index && idx >= 0 {
                        send(Event::Current(sl.max(0), idx, name));
                        if let Ok(i) = read_live(s, send) {
                            last_index = i;
                        }
                    }
                }
                Err(_) => {
                    // No answer: open the loaded preset again so live edits land; if even that
                    // fails, say the link is gone so the app offers to reconnect.
                    match read_live(s, send) {
                        Ok(i) => {
                            last_index = i;
                            send(Event::Info("The pedal stopped answering for a while; picked the loaded preset up again".into()));
                        }
                        Err(e) => return send(Event::Disconnected(format!("The pedal stopped answering ({e}). Connect again."))),
                    }
                }
            }
            continue;
        }
        let Some(cmd) = cmd else {
            match s.idle(Duration::from_millis(30)) {
                Ok(notes) => {
                    // A preset change on the pedal itself: find out which.
                    use crate::proto::event::{PRESET_CHANGED, PRESET_CHANGED_ALT};
                    if notes.iter().any(|(e, _)| *e == PRESET_CHANGED || *e == PRESET_CHANGED_ALT) {
                        if let Ok((sl, idx, name)) = s.preset_info() {
                            send(Event::Current(sl.max(0), idx, name));
                            // Another preset was picked on the pedal: follow it.
                            if idx != last_index {
                                if let Ok(i) = read_live(s, send) {
                                    last_index = i;
                                }
                            }
                        }
                    }
                }
                Err(e) => return send(Event::Disconnected(e.to_string())),
            }
            continue;
        };
        if matches!(cmd, Cmd::Disconnect) {
            return send(Event::Disconnected("Disconnected".into()));
        }
        let result: Result<(), String> = (|| {
            match cmd {
                Cmd::Disconnect => {} // handled above
                Cmd::Refresh => {
                    refresh(s, send)?;
                }
                Cmd::Presets(sl) => {
                    let p = s.presets(sl).map_err(|e| e.to_string())?;
                    send(Event::Presets(sl, p));
                }
                Cmd::Select(sl, idx) => {
                    send(Event::Busy("Loading preset...".into()));
                    s.select_preset(sl, idx).map_err(|e| e.to_string())?;
                    last_index = read_live(s, send)?;
                    send(Event::Done("Preset loaded".into()));
                }
                Cmd::FetchSlot { setlist: sl, index, purpose } => {
                    send(Event::Busy("Reading preset from the pedal...".into()));
                    let blob = s.fetch_slot(sl, index).map_err(|e| e.to_string())?;
                    send(Event::Document { setlist: sl, index, live: false, purpose, blob });
                    send(Event::Done("Preset read".into()));
                }
                Cmd::WriteSlot { setlist: sl, index, name, doc, backup } => {
                    // A document for another unit has other paths and slots: the pedal's own loaded
                    // preset shows the shape this one has to have.
                    if let (Ok(now), Ok(new)) = (
                        s.read_current().map_err(|e| e.to_string()).and_then(|b| crate::devpreset::DevicePreset::parse(&b)),
                        crate::devpreset::DevicePreset::parse(&doc),
                    ) {
                        if now.layout() != new.layout() {
                            return Err(format!(
                                "\"{name}\" has {:?} slots per path but this pedal's presets have {:?}: it was made for another unit, so nothing was written",
                                new.layout(),
                                now.layout()
                            ));
                        }
                    }
                    send(Event::Busy("Backing up the slot...".into()));
                    if let Some(note) = backup_slot(s, &backup, sl, index)? {
                        send(Event::Info(note));
                    }
                    send(Event::Busy("Writing preset to the pedal...".into()));
                    let sent = s.write_slot(sl, index, &name, doc).map_err(|e| e.to_string())?;
                    note_name(send, &name, PRESET_NAME_MAX);
                    let same = reads_back_same(s, sl, index, &sent)?;
                    follow_write(s, send, sl, index, &mut last_index);
                    let p = s.presets(sl).map_err(|e| e.to_string())?;
                    send(Event::Presets(sl, p));
                    if same {
                        send(Event::Done(format!("Wrote \"{name}\" to the pedal (read back the same)")));
                    } else {
                        send(Event::Error(format!(
                            "Wrote \"{name}\", but it reads back different from what was sent: check it on the pedal"
                        )));
                    }
                }
                Cmd::SlotOp { setlist: sl, from, to, op, backup } => {
                    slot_op(s, send, sl, from, to, op, &backup)?;
                    let p = s.presets(sl).map_err(|e| e.to_string())?;
                    send(Event::Presets(sl, p));
                }
                Cmd::Rename { setlist: sl, index, name } => {
                    s.rename_slot(sl, index, &name).map_err(|e| e.to_string())?;
                    note_name(send, &name, PRESET_NAME_MAX);
                    let p = s.presets(sl).map_err(|e| e.to_string())?;
                    send(Event::Presets(sl, p));
                    send(Event::Done(format!("Renamed to \"{name}\"")));
                }
                Cmd::Clear { setlist: sl, index } => {
                    s.clear_slot(sl, index).map_err(|e| e.to_string())?;
                    let p = s.presets(sl).map_err(|e| e.to_string())?;
                    send(Event::Presets(sl, p));
                    send(Event::Done("Slot cleared".into()));
                }
                Cmd::SaveCurrent { setlist: sl, index, name } => {
                    send(Event::Busy("Saving on the pedal...".into()));
                    s.save_current(sl, index, &name).map_err(|e| e.to_string())?;
                    note_name(send, &name, PRESET_NAME_MAX);
                    let p = s.presets(sl).map_err(|e| e.to_string())?;
                    send(Event::Presets(sl, p));
                    send(Event::Done(format!("Saved \"{name}\" on the pedal")));
                }
                Cmd::SetParam { block, path, index, value } => {
                    s.set_param(block, path, index, value).map_err(|e| e.to_string())?;
                }
                Cmd::SetEnabled { block, enabled } => {
                    s.set_enabled(block, enabled).map_err(|e| e.to_string())?;
                }
                Cmd::SetModel { block, model, paired } => {
                    send(Event::Busy("Changing the effect...".into()));
                    s.set_model(block, model, paired).map_err(|e| e.to_string())?;
                    read_live(s, send)?;
                    send(Event::Done("Effect changed".into()));
                }
                Cmd::ReplaceBlock { block, slot } => {
                    send(Event::Busy("Replacing the effect...".into()));
                    s.clear_block(block).map_err(|e| e.to_string())?;
                    s.add_block(block, slot).map_err(|e| e.to_string())?;
                    read_live(s, send)?;
                    send(Event::Done("Effect replaced".into()));
                }
                Cmd::AddBlock { block, body } => {
                    send(Event::Busy("Adding the effect...".into()));
                    s.add_block(block, body).map_err(|e| e.to_string())?;
                    read_live(s, send)?;
                    send(Event::Done("Effect added".into()));
                }
                Cmd::SetRoute { block, input, value } => {
                    s.set_route(block, input, value).map_err(|e| e.to_string())?;
                }
                Cmd::MoveBlock { from, to } => {
                    s.move_block(from, to).map_err(|e| e.to_string())?;
                    read_live(s, send)?;
                }
                Cmd::MoveSplit { block } => {
                    s.move_split(block).map_err(|e| e.to_string())?;
                    read_live(s, send)?;
                }
                Cmd::MoveJoin { block } => {
                    s.move_join(block).map_err(|e| e.to_string())?;
                    read_live(s, send)?;
                }
                Cmd::ClearBlock { block } => {
                    s.clear_block(block).map_err(|e| e.to_string())?;
                    read_live(s, send)?;
                    send(Event::Done("Block removed".into()));
                }
                Cmd::Snapshot(i) => {
                    s.select_snapshot(i).map_err(|e| e.to_string())?;
                    read_live(s, send)?;
                }
                Cmd::RenameSnapshot { index, name } => {
                    s.rename_snapshot(index, &name).map_err(|e| e.to_string())?;
                    note_name(send, &name, SNAPSHOT_NAME_MAX);
                }
                Cmd::SetSetting { field, value } => {
                    let mut doc = crate::devpreset::DevicePreset::parse(&s.read_current().map_err(|e| e.to_string())?)?;
                    if !doc.set_setting_int(field, value) {
                        return Err("this preset has no such setting".into());
                    }
                    s.write_edit_buffer(doc.encode_checked()?).map_err(|e| e.to_string())?;
                    read_live(s, send)?;
                    send(Event::Done("Changed on the pedal (Save on pedal keeps it)".into()));
                }
                Cmd::SetCommandValues { snapshot, slot, args } => {
                    let mut doc = crate::devpreset::DevicePreset::parse(&s.read_current().map_err(|e| e.to_string())?)?;
                    if !doc.set_command_values(snapshot, slot, &args) {
                        return Err("this preset has no such Command Center slot".into());
                    }
                    write_command_center(s, send, &doc)?;
                    send(Event::Done("Command Center changed on the pedal (Save on pedal keeps it)".into()));
                }
                Cmd::SetMomentary { slot, momentary } => {
                    let mut doc = crate::devpreset::DevicePreset::parse(&s.read_current().map_err(|e| e.to_string())?)?;
                    if !doc.set_command_momentary(slot, momentary) {
                        return Err("this preset has no such footswitch".into());
                    }
                    write_command_center(s, send, &doc)?;
                    send(Event::Done("Footswitch changed on the pedal (Save on pedal keeps it)".into()));
                }
                Cmd::SetCommand { slot, id, args } => {
                    // Start from the pedal's own document right now, so a knob turned on the pedal isn't overwritten.
                    let mut doc = crate::devpreset::DevicePreset::parse(&s.read_current().map_err(|e| e.to_string())?)?;
                    if !doc.set_command(slot, id, &args) {
                        return Err("this preset has no such Command Center slot".into());
                    }
                    write_command_center(s, send, &doc)?;
                    send(Event::Done("Command Center changed on the pedal (Save on pedal keeps it)".into()));
                }
                Cmd::CopySnapshot { from, to, keep_name } => {
                    send(Event::Busy("Copying the snapshot...".into()));
                    s.copy_snapshot(from, to, &keep_name).map_err(|e| e.to_string())?;
                    read_live(s, send)?;
                    send(Event::Done(format!("Snapshot {} is now the same as snapshot {}", to + 1, from + 1)));
                }
                Cmd::Backup { setlist: sl, count, folder, names } => {
                    std::fs::create_dir_all(&folder).map_err(|e| format!("can't make {} ({e})", folder.display()))?;
                    let mut run = BackupRun::default();
                    if backup_setlist(s, send, sl, count, &names, &folder, "", &mut run) {
                        return Err(format!(
                            "Backed up {} presets to {}, then stopped: {} slots in a row failed: {}",
                            run.saved,
                            folder.display(),
                            run.in_a_row,
                            run.failed.join(", ")
                        ));
                    }
                    if run.failed.is_empty() {
                        send(Event::Done(format!("Backed up {} presets to {}", run.saved, folder.display())));
                    } else {
                        send(Event::Error(format!(
                            "Backed up {} presets to {}; {} failed: {}",
                            run.saved,
                            folder.display(),
                            run.failed.len(),
                            run.failed.join(", ")
                        )));
                    }
                }
                Cmd::FullBackup { folder } => {
                    // One new folder, named by the time, with a folder per setlist and the IRs.
                    let root = crate::files::unique_path(&folder, &format!("Full backup {} UTC", crate::files::stamp_text(stamp())));
                    std::fs::create_dir_all(&root).map_err(|e| format!("can't make {} ({e})", root.display()))?;
                    let setlists = s.setlists().map_err(|e| e.to_string())?;
                    let mut run = BackupRun::default();
                    let mut stopped = false;
                    for (sl, set_name) in setlists.iter().enumerate() {
                        let names = s.presets(sl as i64).map_err(|e| format!("can't read setlist {set_name} ({e})"))?;
                        let dir = root.join(sanitize(set_name));
                        std::fs::create_dir_all(&dir).map_err(|e| format!("can't make {} ({e})", dir.display()))?;
                        if backup_setlist(s, send, sl as i64, names.len(), &names, &dir, &format!("{set_name}: "), &mut run) {
                            stopped = true;
                            break;
                        }
                    }
                    let presets_saved = run.saved;
                    let mut irs_saved = 0;
                    if !stopped {
                        let in_use = s.list_irs().map_err(|e| {
                            format!("Backed up {presets_saved} presets to {}, but can't read the IR list ({e})", root.display())
                        })?;
                        let dir = root.join("IRs");
                        for (i, (slot, name)) in in_use.iter().enumerate() {
                            send(Event::Busy(format!("Backing up IR {}/{}...", i + 1, in_use.len())));
                            let outcome = s.ir_audio(*slot).map_err(|e| e.to_string()).and_then(|audio| {
                                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                                let file = crate::files::unique_path(&dir, &crate::irs::file_name(*slot, name));
                                crate::files::write_atomic(&file, &crate::irs::audio_to_wav(&audio)).map_err(|e| e.to_string())?;
                                Ok(true)
                            });
                            let before = run.saved;
                            if run.record(&format!("IR {}", slot + 1), outcome) {
                                stopped = true;
                                break;
                            }
                            irs_saved += run.saved - before;
                        }
                    }
                    let summary = format!("{presets_saved} presets and {irs_saved} IRs to {}", root.display());
                    if stopped {
                        return Err(format!(
                            "Backed up {summary}, then stopped: {} in a row failed: {}",
                            run.in_a_row,
                            run.failed.join(", ")
                        ));
                    }
                    if run.failed.is_empty() {
                        send(Event::Done(format!("Backed up {summary}")));
                    } else {
                        send(Event::Error(format!("Backed up {summary}; {} failed: {}", run.failed.len(), run.failed.join(", "))));
                    }
                }
                Cmd::RestoreAll { presets, irs, backup } => {
                    // Every document must have the shape of this pedal's presets before anything is written.
                    let now = s
                        .read_current()
                        .map_err(|e| e.to_string())
                        .and_then(|b| crate::devpreset::DevicePreset::parse(&b))
                        .map_err(|e| format!("can't read the loaded preset to check the backup against ({e}); nothing was written"))?;
                    for (_, index, name, doc) in &presets {
                        let new = crate::devpreset::DevicePreset::parse(doc)
                            .map_err(|e| format!("\"{name}\" isn't readable ({e}); nothing was written"))?;
                        if new.layout() != now.layout() {
                            return Err(format!(
                                "\"{name}\" ({}) was made for another unit (its paths have {:?} slots, this pedal's {:?}); nothing was written",
                                crate::proto::slot_label(*index as usize),
                                new.layout(),
                                now.layout()
                            ));
                        }
                    }
                    let (np, ni) = (presets.len(), irs.len());
                    let mut run = BackupRun::default();
                    let mut stopped = false;
                    let mut written = 0;
                    let mut sets = std::collections::BTreeSet::new();
                    for (i, (sl, index, name, doc)) in presets.into_iter().enumerate() {
                        send(Event::Busy(format!("Restoring preset {}/{np}: {name}", i + 1)));
                        let label = crate::proto::slot_label(index as usize);
                        let outcome: Result<bool, String> = (|| {
                            backup_slot(s, &backup, sl, index)?;
                            let sent = s.write_slot(sl, index, &name, doc).map_err(|e| e.to_string())?;
                            if reads_back_same(s, sl, index, &sent)? {
                                Ok(true)
                            } else {
                                Err("it reads back different from what was sent".into())
                            }
                        })();
                        sets.insert(sl);
                        let before = run.saved;
                        let stop = run.record(&label, outcome);
                        written += run.saved - before;
                        if stop {
                            stopped = true;
                            break;
                        }
                    }
                    let mut irs_written = 0;
                    if !stopped && ni > 0 {
                        match s.list_irs() {
                            Ok(in_use) => {
                                for (i, (slot, name, audio)) in irs.into_iter().enumerate() {
                                    send(Event::Busy(format!("Restoring IR {}/{ni}: {name}", i + 1)));
                                    let outcome: Result<bool, String> = (|| {
                                        backup_ir(s, &backup, slot, &in_use)?;
                                        s.import_ir(slot, &name, &audio).map_err(|e| e.to_string())?;
                                        Ok(true)
                                    })();
                                    let before = run.saved;
                                    let stop = run.record(&format!("IR {}", slot + 1), outcome);
                                    irs_written += run.saved - before;
                                    if stop {
                                        stopped = true;
                                        break;
                                    }
                                }
                            }
                            Err(e) => run.failed.push(format!("IRs (can't read the IR list to back it up: {e}; none were written)")),
                        }
                    }
                    for sl in sets {
                        if let Ok(p) = s.presets(sl) {
                            send(Event::Presets(sl, p));
                        }
                    }
                    if irs_written > 0 {
                        if let Ok(l) = s.list_irs() {
                            send(Event::Irs(l));
                        }
                    }
                    // The loaded preset may be one of those written.
                    if let Ok(i) = read_live(s, send) {
                        last_index = i;
                    }
                    let summary = format!("{written} of {np} presets and {irs_written} of {ni} IRs restored");
                    if stopped {
                        return Err(format!("{summary}, then stopped: {} in a row failed: {}", run.in_a_row, run.failed.join(", ")));
                    }
                    if run.failed.is_empty() {
                        send(Event::Done(format!("{summary}. The replaced presets were kept in {}", backup.display())));
                    } else {
                        send(Event::Error(format!("{summary}; {} failed: {}", run.failed.len(), run.failed.join(", "))));
                    }
                }
                Cmd::AssignController { block, cab, index, controller, cc } => {
                    s.assign_controller(block, cab, index, controller, cc).map_err(|e| e.to_string())?;
                    read_live(s, send)?;
                    send(Event::Done("Controller assigned".into()));
                }
                Cmd::UnassignController { block, cab, index } => {
                    s.unassign_controller(block, cab, index).map_err(|e| e.to_string())?;
                    read_live(s, send)?;
                    send(Event::Done("Controller removed".into()));
                }
                Cmd::ControllerBound { max, block, cab, index, value } => {
                    s.controller_bound(max, block, cab, index, value).map_err(|e| e.to_string())?;
                }
                Cmd::ControllerCc { block, cab, index, cc } => {
                    s.controller_cc(block, cab, index, cc).map_err(|e| e.to_string())?;
                }
                Cmd::AssignFootswitch { block, fs } => {
                    s.assign_footswitch(block, fs).map_err(|e| e.to_string())?;
                    read_live(s, send)?;
                    send(Event::Done(format!("Block on footswitch {fs}")));
                }
                Cmd::UnassignFootswitch { block, fs } => {
                    s.unassign_footswitch(block, fs).map_err(|e| e.to_string())?;
                    read_live(s, send)?;
                    send(Event::Done("Footswitch removed".into()));
                }
                Cmd::WriteHlx { setlist: sl, index, name, json, cat, backup } => {
                    send(Event::Busy("Converting the .hlx preset...".into()));
                    let template = s.read_current().map_err(|e| e.to_string())?;
                    let template = crate::devpreset::DevicePreset::parse(&template)?;
                    let (doc, notes) =
                        crate::devpreset::DevicePreset::from_hlx(&template, &json, &cat).map_err(|e| format!("Can't convert it: {e}"))?;
                    let bytes = doc.encode_checked()?;
                    send(Event::Busy("Backing up the slot...".into()));
                    if let Some(note) = backup_slot(s, &backup, sl, index)? {
                        send(Event::Info(note));
                    }
                    send(Event::Busy("Writing preset to the pedal...".into()));
                    let sent = s.write_slot(sl, index, &name, bytes).map_err(|e| e.to_string())?;
                    note_name(send, &name, PRESET_NAME_MAX);
                    let same = reads_back_same(s, sl, index, &sent)?;
                    follow_write(s, send, sl, index, &mut last_index);
                    for n in notes {
                        send(Event::Info(n));
                    }
                    let p = s.presets(sl).map_err(|e| e.to_string())?;
                    send(Event::Presets(sl, p));
                    if same {
                        send(Event::Done(format!("Wrote \"{name}\" to the pedal (converted from .hlx; read back the same)")));
                    } else {
                        send(Event::Error(format!(
                            "Wrote \"{name}\", but it reads back different from what was sent: check it on the pedal"
                        )));
                    }
                }
                Cmd::SelfTest(plan) => {
                    let sl = plan.setlist;
                    crate::selftest::run(s, plan, send);
                    if let Ok(p) = s.presets(sl) {
                        send(Event::Presets(sl, p));
                    }
                    if let Ok(l) = s.list_irs() {
                        send(Event::Irs(l));
                    }
                    send(Event::Done("Self-test finished".into()));
                }
                Cmd::ListIrs => {
                    send(Event::Busy("Reading the pedal's IRs...".into()));
                    let l = s.list_irs().map_err(|e| e.to_string())?;
                    let n = l.len();
                    send(Event::Irs(l));
                    send(Event::Done(format!("{n} IR slots in use")));
                }
                Cmd::ImportIrs { items, backup } => {
                    let total = items.len();
                    let mut done = 0;
                    send(Event::Busy("Backing up the IR slots...".into()));
                    let in_use = s.list_irs().map_err(|e| format!("can't read the IR list to back it up ({e}); nothing was written"))?;
                    for (slot, _, _) in &items {
                        if let Some(note) = backup_ir(s, &backup, *slot, &in_use)? {
                            send(Event::Info(note));
                        }
                    }
                    for (i, (slot, name, audio)) in items.into_iter().enumerate() {
                        send(Event::Busy(format!("Writing IR {}/{total} to slot {}...", i + 1, slot + 1)));
                        if let Err(e) = s.import_ir(slot, &name, &audio) {
                            if let Ok(l) = s.list_irs() {
                                send(Event::Irs(l));
                            }
                            return Err(format!("Wrote {done} of {total} IRs; slot {} failed: {e}", slot + 1));
                        }
                        note_name(send, &name, IR_NAME_MAX);
                        done += 1;
                    }
                    send(Event::Irs(s.list_irs().map_err(|e| e.to_string())?));
                    send(Event::Done(format!("Wrote {done} IR{} to the pedal", if done == 1 { "" } else { "s" })));
                }
                Cmd::ExportIrs { slots, folder } => {
                    let _ = std::fs::create_dir_all(&folder);
                    let total = slots.len();
                    for (i, (slot, name)) in slots.iter().enumerate() {
                        send(Event::Busy(format!("Downloading IR {}/{total}...", i + 1)));
                        let audio = s.ir_audio(*slot).map_err(|e| e.to_string())?;
                        let file = folder.join(crate::irs::file_name(*slot, name));
                        crate::files::write_atomic(&file, &crate::irs::audio_to_wav(&audio)).map_err(|e| e.to_string())?;
                    }
                    send(Event::Done(format!("Downloaded {total} IR{} to {}", if total == 1 { "" } else { "s" }, folder.display())));
                }
                Cmd::RenameIr { slot, name } => {
                    s.rename_ir(slot, &name).map_err(|e| e.to_string())?;
                    note_name(send, &name, IR_NAME_MAX);
                    send(Event::Irs(s.list_irs().map_err(|e| e.to_string())?));
                    send(Event::Done(format!("IR slot {} renamed", slot + 1)));
                }
                Cmd::IrKinds(slots) => {
                    let total = slots.len();
                    let mut out = Vec::new();
                    for (i, (slot, name)) in slots.into_iter().enumerate() {
                        if i % 8 == 0 {
                            send(Event::Busy(format!("Sorting IRs from NAM captures {}/{total}...", i + 1)));
                        }
                        if let Ok(audio) = s.ir_audio(slot) {
                            out.push((slot, name, crate::irs::is_nam_capture(&audio)));
                        }
                    }
                    send(Event::IrKinds(out));
                }
                Cmd::DeleteIrs { slots, backup } => {
                    send(Event::Busy("Backing up the IR slots...".into()));
                    let in_use = s.list_irs().map_err(|e| format!("can't read the IR list to back it up ({e}); nothing was removed"))?;
                    for slot in &slots {
                        if let Some(note) = backup_ir(s, &backup, *slot, &in_use)? {
                            send(Event::Info(note));
                        }
                    }
                    for (i, slot) in slots.iter().enumerate() {
                        send(Event::Busy(format!("Removing IR {}/{}...", i + 1, slots.len())));
                        if let Err(e) = s.delete_ir(*slot) {
                            // Some were already removed: show the list as it is now.
                            if let Ok(l) = s.list_irs() {
                                send(Event::Irs(l));
                            }
                            return Err(format!("Removed {i} of {} IRs; slot {} failed: {e}", slots.len(), slot + 1));
                        }
                    }
                    send(Event::Irs(s.list_irs().map_err(|e| e.to_string())?));
                    send(Event::Done(format!("Removed {} IR{}", slots.len(), if slots.len() == 1 { "" } else { "s" })));
                }
            }
            Ok(())
        })();
        for w in s.take_warnings() {
            send(Event::Info(w));
        }
        if let Err(e) = result {
            send(Event::Error(e));
            // Whatever failed, re-open the loaded preset so the next live edit has a fresh link.
            if let Ok(i) = read_live(s, send) {
                last_index = i;
            }
            last_beat = std::time::Instant::now();
        }
    }
}

/// After a write into a slot: when that slot is the one loaded on the pedal, the live tab
/// has to show what is there now, not what was loaded before.
fn follow_write(s: &mut Session, send: &dyn Fn(Event), setlist: i64, index: i64, last_index: &mut i64) {
    if let Ok((sl, idx, _)) = s.preset_info() {
        if sl.max(0) == setlist && idx == index {
            if let Ok(i) = read_live(s, send) {
                *last_index = i;
            }
        }
    }
}

/// A backup gives up when this many slots in a row fail: the pedal is probably gone.
const BACKUP_STOP_AFTER: usize = 3;

/// How a backup is going: what was saved, what failed, and the failures in a row.
#[derive(Default)]
struct BackupRun {
    saved: usize,
    failed: Vec<String>,
    in_a_row: usize,
}

impl BackupRun {
    /// Note how one item went (`Ok(false)`: nothing there to save). True when it is time to
    /// stop.
    fn record(&mut self, label: &str, outcome: Result<bool, String>) -> bool {
        match outcome {
            Ok(wrote) => {
                self.saved += wrote as usize;
                self.in_a_row = 0;
            }
            Err(e) => {
                self.failed.push(format!("{label} ({e})"));
                self.in_a_row += 1;
            }
        }
        self.in_a_row >= BACKUP_STOP_AFTER
    }
}

/// Save the presets of one setlist as `.hxpreset` files in `folder`. Keeps going past a slot
/// that fails; true when it stopped because the pedal seems gone. `what` starts the
/// progress text ("" or "Setlist 2: ").
#[allow(clippy::too_many_arguments)]
fn backup_setlist(
    s: &mut Session,
    send: &dyn Fn(Event),
    sl: i64,
    count: usize,
    names: &[String],
    folder: &Path,
    what: &str,
    run: &mut BackupRun,
) -> bool {
    for i in 0..count {
        send(Event::Busy(format!("Backing up {what}{}/{count}...", i + 1)));
        let label = crate::proto::slot_label(i);
        let outcome = s.fetch_slot(sl, i as i64).map_err(|e| e.to_string()).and_then(|blob| {
            let Some(blob) = blob else { return Ok(false) };
            let n = names.get(i).cloned().unwrap_or_default();
            let file = folder.join(format!("{label} {}.hxpreset", sanitize(&n)));
            crate::files::write_atomic(&file, &blob).map_err(|e| e.to_string()).map(|_| true)
        });
        if run.record(&label, outcome) {
            return true;
        }
    }
    false
}

/// Tell the person when a name reads differently on the pedal.
fn note_name(send: &dyn Fn(Event), name: &str, max_len: usize) {
    if let Some(n) = crate::session::name_note(name, max_len) {
        send(Event::Info(n));
    }
}

/// Read a slot just written and compare it with what was sent.
fn reads_back_same(s: &mut Session, setlist: i64, index: i64, sent: &[u8]) -> Result<bool, String> {
    let back = s
        .fetch_slot(setlist, index)
        .map_err(|e| format!("it was written, but couldn't be read back to check ({e}): check it on the pedal"))?;
    Ok(back.as_deref().and_then(|b| crate::devpreset::DevicePreset::parse(b).ok()).map(|d| d.encode()).as_deref() == Some(sent))
}

/// Make the backup folder, or stop: nothing is written to the pedal unless what it
/// replaces can be kept first.
fn ready_backup_dir(dir: &Path) -> Result<PathBuf, String> {
    if dir.as_os_str().is_empty() {
        return Err("there is no folder to keep a backup in; nothing was written".into());
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("can't make the backup folder ({e}); nothing was written"))?;
    Ok(dir.to_owned())
}

fn stamp() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Keep a copy of what a slot holds before writing over it. Returns a note for the person.
fn backup_slot(s: &mut Session, backup: &Path, setlist: i64, index: i64) -> Result<Option<String>, String> {
    let dir = ready_backup_dir(backup)?;
    let Some(old) = s.fetch_slot(setlist, index).map_err(|e| e.to_string())? else { return Ok(None) };
    let name = s.presets(setlist).ok().and_then(|p| p.get(index.max(0) as usize).cloned()).unwrap_or_default();
    let file = crate::files::unique_path(
        &dir,
        &format!("{} {} - before overwrite {}.hxpreset", crate::proto::slot_label(index.max(0) as usize), sanitize(&name), stamp()),
    );
    crate::files::write_atomic(&file, &old).map_err(|e| format!("can't keep a backup of the slot ({e}); nothing was written"))?;
    Ok(Some(format!("The old \"{name}\" was kept in {}", file.display())))
}

/// Keep a copy of a User IR slot before it is replaced or removed (`IRs` in the backup
/// folder). Empty slots need nothing. Returns a note for the person.
fn backup_ir(s: &mut Session, backup: &Path, slot: i64, in_use: &[(i64, String)]) -> Result<Option<String>, String> {
    let dir = ready_backup_dir(backup)?.join("IRs");
    let Some((_, name)) = in_use.iter().find(|(n, _)| *n == slot) else { return Ok(None) };
    let audio = s.ir_audio(slot).map_err(|e| format!("can't read IR slot {} to back it up ({e}); nothing was changed", slot + 1))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("can't make the IR backup folder ({e}); nothing was changed"))?;
    let base = crate::irs::file_name(slot, name);
    let stem = base.strip_suffix(".wav").unwrap_or(&base);
    let file = crate::files::unique_path(&dir, &format!("{stem} - before overwrite {}.wav", stamp()));
    crate::files::write_atomic(&file, &crate::irs::audio_to_wav(&audio))
        .map_err(|e| format!("can't keep a backup of IR slot {} ({e}); nothing was changed", slot + 1))?;
    Ok(Some(format!("The old IR \"{name}\" was kept in {}", file.display())))
}

pub fn sanitize(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_alphanumeric() || " -_.()".contains(c) { c } else { '_' }).collect();
    let t = s.trim().to_owned();
    if t.is_empty() {
        "preset".into()
    } else {
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_backup_stops_after_three_failures_in_a_row() {
        let mut run = BackupRun::default();
        assert!(!run.record("01A", Err("no answer".into())));
        assert!(!run.record("01B", Err("no answer".into())));
        // A good slot (or an empty one) starts the count again.
        assert!(!run.record("01C", Ok(false)));
        assert!(!run.record("02A", Err("no answer".into())));
        assert!(!run.record("02B", Ok(true)));
        assert!(!run.record("02C", Err("no answer".into())));
        assert!(!run.record("03A", Err("no answer".into())));
        assert!(run.record("03B", Err("no answer".into())));
        assert_eq!(run.saved, 1);
        assert_eq!(run.failed.len(), 6);
    }
}

/// What dropping one preset slot onto another does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotOp {
    /// The target gets a copy; the source stays.
    Copy,
    /// The target gets the source; the source is emptied.
    Move,
    /// The two exchange places (with an empty target this is a move).
    Swap,
}

/// Copy, move or swap two preset slots of a setlist. Every slot that is about to change is kept
/// in `backup` first, the first write is read back and compared before anything else is
/// touched, and a failure stops there (the backups say where the old presets are).
pub fn slot_op(s: &mut Session, send: &dyn Fn(Event), sl: i64, from: i64, to: i64, op: SlotOp, backup: &Path) -> Result<(), String> {
    if from == to {
        return Err("pick two different slots".into());
    }
    let label = |i: i64| crate::proto::slot_label(i.max(0) as usize);
    send(Event::Busy("Reading the two presets...".into()));
    let names = s.presets(sl).map_err(|e| e.to_string())?;
    let name_of = |i: i64| names.get(i.max(0) as usize).cloned().unwrap_or_default();
    let a = s.fetch_slot(sl, from).map_err(|e| e.to_string())?.ok_or_else(|| format!("{} is empty: nothing to copy", label(from)))?;
    let b = s.fetch_slot(sl, to).map_err(|e| e.to_string())?;

    send(Event::Busy("Backing up the slots that change...".into()));
    let changing = if op == SlotOp::Copy { vec![to] } else { vec![to, from] };
    for i in changing {
        if let Some(note) = backup_slot(s, backup, sl, i)? {
            send(Event::Info(note));
        }
    }

    send(Event::Busy(format!("Writing {}...", label(to))));
    let sent = s.write_slot(sl, to, &name_of(from), a).map_err(|e| e.to_string())?;
    note_name(send, &name_of(from), PRESET_NAME_MAX);
    if !reads_back_same(s, sl, to, &sent)? {
        return Err(format!(
            "{} reads back different from what was written, so nothing else was changed. The old presets are kept in {}",
            label(to),
            backup.display()
        ));
    }
    match (op, b) {
        (SlotOp::Copy, _) => {}
        (SlotOp::Move, _) | (SlotOp::Swap, None) => {
            send(Event::Busy(format!("Emptying {}...", label(from))));
            s.clear_slot(sl, from).map_err(|e| format!("{} was copied but {} could not be emptied ({e})", label(to), label(from)))?;
        }
        (SlotOp::Swap, Some(b)) => {
            send(Event::Busy(format!("Writing {}...", label(from))));
            let sent = s.write_slot(sl, from, &name_of(to), b).map_err(|e| {
                format!(
                    "{} now holds the other preset but {} could not be written ({e}); the old presets are kept in {}",
                    label(to),
                    label(from),
                    backup.display()
                )
            })?;
            note_name(send, &name_of(to), PRESET_NAME_MAX);
            if !reads_back_same(s, sl, from, &sent)? {
                return Err(format!(
                    "{} reads back different from what was written; the old presets are kept in {}",
                    label(from),
                    backup.display()
                ));
            }
        }
    }
    send(Event::Done(match op {
        SlotOp::Copy => format!("Copied {} to {}", label(from), label(to)),
        SlotOp::Move => format!("Moved {} to {}", label(from), label(to)),
        SlotOp::Swap => format!("Swapped {} and {}", label(from), label(to)),
    }));
    Ok(())
}
