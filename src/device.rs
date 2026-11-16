//! A background thread that owns the pedal session, so the window never waits
//! on USB or Wi-Fi. The window sends commands; the thread answers with events.

use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use crate::msgpack::Value;
use crate::session::{Session, IR_NAME_MAX, PRESET_NAME_MAX, SNAPSHOT_NAME_MAX};

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
use crate::wire::{BoxWire, UsbWire, Wire};

pub enum Target {
    Usb { pid: u16 },
    Box { host: String, token: String, label: String },
}

pub enum Cmd {
    Refresh,
    Presets(i64),
    Select(i64, i64),
    ReadCurrent,
    FetchSlot { setlist: i64, index: i64, purpose: Purpose },
    WriteSlot { setlist: i64, index: i64, name: String, doc: Vec<u8> },
    Rename { setlist: i64, index: i64, name: String },
    Clear { setlist: i64, index: i64 },
    SaveCurrent { setlist: i64, index: i64, name: String },
    SetParam { block: i64, path: i64, index: i64, value: Value },
    SetEnabled { block: i64, enabled: bool },
    SetModel { block: i64, model: i64, paired: Option<i64> },
    ReplaceBlock { block: i64, slot: Value },
    ClearBlock { block: i64 },
    AddBlock { block: i64, body: Value },
    MoveBlock { from: i64, to: i64 },
    MoveSplit { block: i64 },
    MoveJoin { block: i64 },
    SetRoute { block: i64, input: bool, value: i64 },
    Snapshot(i64),
    RenameSnapshot { index: i64, name: String },
    Backup { setlist: i64, count: usize, folder: std::path::PathBuf, names: Vec<String> },
    /// Read the names of the pedal's User IRs.
    ListIrs,
    /// Write IRs into slots: (slot from 0, name, 8,192 bytes of audio).
    ImportIrs(Vec<(i64, String, Vec<u8>)>),
    /// Save IRs from slots as .wav files in a folder: (slot, name).
    ExportIrs { slots: Vec<(i64, String)>, folder: std::path::PathBuf },
    RenameIr { slot: i64, name: String },
    DeleteIrs(Vec<i64>),
    /// Look at slots' audio to tell IRs from NAM captures: (slot, name).
    IrKinds(Vec<(i64, String)>),
    /// Convert a .hlx preset to the pedal's format (using the loaded preset as the
    /// template) and write it into a slot, then read it back to check.
    WriteHlx { setlist: i64, index: i64, name: String, json: serde_json::Value, cat: std::sync::Arc<crate::catalog::Catalog> },
    /// Live controller and footswitch changes on the loaded preset.
    AssignController { block: i64, cab: bool, index: i64, controller: i64, cc: i64 },
    UnassignController { block: i64, cab: bool, index: i64 },
    ControllerBound { max: bool, block: i64, cab: bool, index: i64, value: Value },
    ControllerCc { block: i64, cab: bool, index: i64, cc: i64 },
    AssignFootswitch { block: i64, fs: i64 },
    UnassignFootswitch { block: i64, fs: i64 },
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
    Document { setlist: i64, index: i64, live: bool, purpose: Purpose, blob: Option<Vec<u8>> },
    Done(String),
    /// The pedal's User IRs that hold something: (slot from 0, name).
    Irs(Vec<(i64, String)>),
    /// Which slots hold NAM captures: (slot, name, is a NAM capture).
    IrKinds(Vec<(i64, String, bool)>),
    /// A self-test step: passed (Some(true)), failed (Some(false)) or a note (None).
    TestStep { name: String, ok: Option<bool>, detail: String },
    TestDone { failures: usize },
}

pub struct Link {
    pub tx: Sender<Cmd>,
    pub rx: Receiver<Event>,
}

pub fn connect(target: Target, repaint: impl Fn() + Send + 'static) -> Link {
    let (cmd_tx, cmd_rx) = channel::<Cmd>();
    let (ev_tx, ev_rx) = channel::<Event>();
    std::thread::spawn(move || {
        let send = |e: Event| {
            let _ = ev_tx.send(e);
            repaint();
        };
        send(Event::Busy("Connecting...".into()));
        let wire: Result<Box<dyn Wire>, String> = match target {
            Target::Usb { pid } => UsbWire::open(pid).map(|w| Box::new(w) as Box<dyn Wire>),
            Target::Box { host, token, label } => {
                BoxWire::open(&host, &token, &label).map(|w| Box::new(w) as Box<dyn Wire>)
            }
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
    Link { tx: cmd_tx, rx: ev_rx }
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
    let mut setlist = 0i64;
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
        let result: Result<(), String> = (|| {
            match cmd {
                Cmd::Disconnect => return Err("__bye".into()),
                Cmd::Refresh => {
                    setlist = refresh(s, send)?;
                }
                Cmd::Presets(sl) => {
                    setlist = sl;
                    let p = s.presets(sl).map_err(|e| e.to_string())?;
                    send(Event::Presets(sl, p));
                }
                Cmd::Select(sl, idx) => {
                    send(Event::Busy("Loading preset...".into()));
                    s.select_preset(sl, idx).map_err(|e| e.to_string())?;
                    last_index = read_live(s, send)?;
                    send(Event::Done("Preset loaded".into()));
                }
                Cmd::ReadCurrent => {
                    send(Event::Busy("Reading the loaded preset...".into()));
                    let blob = s.read_current().map_err(|e| e.to_string())?;
                    let (sl, idx, _) = s.preset_info().map_err(|e| e.to_string())?;
                    send(Event::Document { setlist: sl.max(0), index: idx, live: true, purpose: Purpose::Edit, blob: Some(blob) });
                    send(Event::Done("Loaded preset opened for live editing".into()));
                }
                Cmd::FetchSlot { setlist: sl, index, purpose } => {
                    send(Event::Busy("Reading preset from the pedal...".into()));
                    let blob = s.fetch_slot(sl, index).map_err(|e| e.to_string())?;
                    send(Event::Document { setlist: sl, index, live: false, purpose, blob });
                    send(Event::Done("Preset read".into()));
                }
                Cmd::WriteSlot { setlist: sl, index, name, doc } => {
                    send(Event::Busy("Backing up the slot...".into()));
                    if let Some(note) = backup_slot(s, sl, index)? {
                        send(Event::Info(note));
                    }
                    send(Event::Busy("Writing preset to the pedal...".into()));
                    let sent = s.write_slot(sl, index, &name, doc).map_err(|e| e.to_string())?;
                    note_name(send, &name, PRESET_NAME_MAX);
                    let same = reads_back_same(s, sl, index, &sent)?;
                    let p = s.presets(sl).map_err(|e| e.to_string())?;
                    send(Event::Presets(sl, p));
                    if same {
                        send(Event::Done(format!("Wrote \"{name}\" to the pedal (read back the same)")));
                    } else {
                        send(Event::Error(format!("Wrote \"{name}\", but it reads back different from what was sent: check it on the pedal")));
                    }
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
                Cmd::Backup { setlist: sl, count, folder, names } => {
                    std::fs::create_dir_all(&folder).map_err(|e| format!("can't make {} ({e})", folder.display()))?;
                    let mut saved = 0;
                    // Keep going past a slot that fails; stop only when the pedal seems gone.
                    let mut failed: Vec<String> = Vec::new();
                    let mut in_a_row = 0;
                    for i in 0..count {
                        send(Event::Busy(format!("Backing up {}/{}...", i + 1, count)));
                        let label = crate::proto::slot_label(i);
                        let outcome = s.fetch_slot(sl, i as i64).map_err(|e| e.to_string()).and_then(|blob| {
                            let Some(blob) = blob else { return Ok(false) };
                            let n = names.get(i).cloned().unwrap_or_default();
                            let file = folder.join(format!("{label} {}.hxpreset", sanitize(&n)));
                            crate::files::write_atomic(&file, &blob).map_err(|e| e.to_string()).map(|_| true)
                        });
                        match outcome {
                            Ok(wrote) => {
                                saved += wrote as usize;
                                in_a_row = 0;
                            }
                            Err(e) => {
                                failed.push(format!("{label} ({e})"));
                                in_a_row += 1;
                                if in_a_row >= 3 {
                                    return Err(format!(
                                        "Backed up {saved} presets to {}, then stopped: {} slots in a row failed: {}",
                                        folder.display(),
                                        in_a_row,
                                        failed.join(", ")
                                    ));
                                }
                            }
                        }
                    }
                    if failed.is_empty() {
                        send(Event::Done(format!("Backed up {saved} presets to {}", folder.display())));
                    } else {
                        send(Event::Error(format!(
                            "Backed up {saved} presets to {}; {} failed: {}",
                            folder.display(),
                            failed.len(),
                            failed.join(", ")
                        )));
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
                Cmd::WriteHlx { setlist: sl, index, name, json, cat } => {
                    send(Event::Busy("Converting the .hlx preset...".into()));
                    let template = s.read_current().map_err(|e| e.to_string())?;
                    let template = crate::devpreset::DevicePreset::parse(&template)?;
                    let (doc, notes) = crate::devpreset::DevicePreset::from_hlx(&template, &json, &cat).map_err(|e| format!("Can't convert it: {e}"))?;
                    let bytes = doc.encode_checked()?;
                    send(Event::Busy("Backing up the slot...".into()));
                    if let Some(note) = backup_slot(s, sl, index)? {
                        send(Event::Info(note));
                    }
                    send(Event::Busy("Writing preset to the pedal...".into()));
                    let sent = s.write_slot(sl, index, &name, bytes).map_err(|e| e.to_string())?;
                    note_name(send, &name, PRESET_NAME_MAX);
                    let same = reads_back_same(s, sl, index, &sent)?;
                    for n in notes {
                        send(Event::Info(n));
                    }
                    let p = s.presets(sl).map_err(|e| e.to_string())?;
                    send(Event::Presets(sl, p));
                    if same {
                        send(Event::Done(format!("Wrote \"{name}\" to the pedal (converted from .hlx; read back the same)")));
                    } else {
                        send(Event::Error(format!("Wrote \"{name}\", but it reads back different from what was sent: check it on the pedal")));
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
                Cmd::ImportIrs(items) => {
                    let total = items.len();
                    let mut done = 0;
                    send(Event::Busy("Backing up the IR slots...".into()));
                    let in_use = s.list_irs().map_err(|e| format!("can't read the IR list to back it up ({e}); nothing was written"))?;
                    for (slot, _, _) in &items {
                        if let Some(note) = backup_ir(s, *slot, &in_use)? {
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
                Cmd::DeleteIrs(slots) => {
                    send(Event::Busy("Backing up the IR slots...".into()));
                    let in_use = s.list_irs().map_err(|e| format!("can't read the IR list to back it up ({e}); nothing was removed"))?;
                    for slot in &slots {
                        if let Some(note) = backup_ir(s, *slot, &in_use)? {
                            send(Event::Info(note));
                        }
                    }
                    for (i, slot) in slots.iter().enumerate() {
                        send(Event::Busy(format!("Removing IR {}/{}...", i + 1, slots.len())));
                        s.delete_ir(*slot).map_err(|e| e.to_string())?;
                    }
                    send(Event::Irs(s.list_irs().map_err(|e| e.to_string())?));
                    send(Event::Done(format!("Removed {} IR{}", slots.len(), if slots.len() == 1 { "" } else { "s" })));
                }
            }
            Ok(())
        })();
        let _ = setlist;
        for w in s.take_warnings() {
            send(Event::Info(w));
        }
        if let Err(e) = result {
            if e == "__bye" {
                return send(Event::Disconnected("Disconnected".into()));
            }
            send(Event::Error(e));
            // Whatever failed, re-open the loaded preset so the next live edit has a fresh link.
            if let Ok(i) = read_live(s, send) {
                last_index = i;
            }
            last_beat = std::time::Instant::now();
        }
    }
}

/// Where a slot's old preset is kept before it's overwritten (set by the window).
pub static BACKUP_DIR: std::sync::Mutex<Option<std::path::PathBuf>> = std::sync::Mutex::new(None);

/// The backup folder, or an error that stops the write: nothing is written to the pedal
/// unless what it replaces can be kept first.
fn backup_dir() -> Result<std::path::PathBuf, String> {
    let dir = BACKUP_DIR
        .lock()
        .ok()
        .and_then(|d| d.clone())
        .ok_or("there is no folder to keep a backup in yet; nothing was written")?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("can't make the backup folder ({e}); nothing was written"))?;
    Ok(dir)
}

fn stamp() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Keep a copy of what a slot holds before writing over it. Returns a note for the person.
fn backup_slot(s: &mut Session, setlist: i64, index: i64) -> Result<Option<String>, String> {
    let dir = backup_dir()?;
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
fn backup_ir(s: &mut Session, slot: i64, in_use: &[(i64, String)]) -> Result<Option<String>, String> {
    let dir = backup_dir()?.join("IRs");
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
    let s: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() || " -_.()".contains(c) { c } else { '_' })
        .collect();
    let t = s.trim().to_owned();
    if t.is_empty() {
        "preset".into()
    } else {
        t
    }
}
