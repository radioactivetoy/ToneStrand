//! Snapshots and controller assignments inside a .hlx preset.
//!
//! - `snapshotN` keeps each block's on/off state (`blocks/dspX/blockN`) and the
//!   value of every knob under snapshot control (`controllers/dspX/blockN/Knob/@value`).
//! - `controller/dspX/blockN/Knob` says what moves a knob: snapshots, an
//!   expression pedal, a footswitch or a MIDI CC, with its low and high ends.
//! - `footswitch/dspX/blockN` puts a block's on/off on a footswitch.
//!
//! Controller numbers depend on the pedal family (see docs/PROTOCOL.md).

use serde_json::{json, Map, Value};

use crate::hlx::{self, HlxBlock};

#[derive(Clone, Copy, Debug)]
pub struct Family {
    pub name: &'static str,
    pub snapshots_ctrl: i64,
    pub midi_ctrl: i64,
    /// Controller number of footswitch 1; footswitch n is `fs_first + n - 1`.
    pub fs_first: i64,
    pub fs_count: i64,
    pub snapshot_count: usize,
    /// Whether a path can split into two parallel lanes (Helix units only).
    pub lanes: bool,
    /// How many DSP paths the unit has.
    pub paths: usize,
    /// How many expression pedals it accepts.
    pub exp_count: i64,
    /// Positions shown on a single lane, and how many blocks a path holds in all
    /// (on shared-slot units, parallel-lane slots reduce the main lane so the
    /// total number of slots across both lanes equals `max_blocks`).
    pub positions: i64,
    pub max_blocks: i64,
    pub shared_slots: bool,
    /// Command Center footswitch slots (`FS 1`..`FS N`).
    pub cc_switches: i64,
    /// Mono FX loops (send/return jack pairs): 4 on Helix Floor/Rack, 2 on the HX Effects (two
    /// jack rows on its rear panel), the Helix LT and the HX Stomp / XL (whose TRS send and two
    /// returns make 2 mono or 1 stereo).
    pub fx_loops: i64,
}

/// The units a preset can be made for: (device number, name).
pub const UNITS: [(i64, &str); 6] = [
    (2162694, "HX Stomp"),
    (2162699, "HX Stomp XL"),
    (2162693, "HX Effects"),
    (2162689, "Helix Floor"),
    (2162692, "Helix LT"),
    (2162690, "Helix Rack"),
];

/// The device number for a pedal's USB product name ("HX Stomp", "HELIX LT"...).
pub fn unit_for_product(product: &str) -> Option<i64> {
    // "HX Stomp (USB)" or "Helix LT via Stage box": only the pedal's own name counts, not the
    // connection note or the name given to a ToneStrand Box.
    let lower = product.to_ascii_lowercase();
    let name = lower.split(" via ").next().unwrap_or(&lower);
    let words: Vec<&str> = name.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let has = |w: &str| words.contains(&w);
    if has("stadium") {
        None // another product with another protocol
    } else if has("stomp") && has("xl") {
        Some(2162699)
    } else if has("stomp") {
        Some(2162694)
    } else if has("effects") {
        Some(2162693)
    } else if has("lt") {
        Some(2162692)
    } else if has("rack") {
        Some(2162690)
    } else if has("helix") {
        Some(2162689)
    } else {
        None
    }
}

pub fn family_of_device(device: i64) -> Family {
    let helix = Family {
        name: "Helix",
        snapshots_ctrl: 19,
        midi_ctrl: 18,
        fs_first: 6,
        fs_count: 11,
        snapshot_count: 8,
        lanes: true,
        paths: 2,
        exp_count: 3,
        positions: 8,
        max_blocks: 16,
        shared_slots: false,
        cc_switches: 12,
        fx_loops: 4,
    };
    let stomp = Family {
        name: "HX Stomp",
        snapshots_ctrl: 9,
        midi_ctrl: 8,
        fs_first: 3,
        fs_count: 5,
        snapshot_count: 3,
        lanes: true,
        paths: 1,
        exp_count: 2,
        positions: 8,
        max_blocks: 8,
        shared_slots: true,
        cc_switches: 5,
        fx_loops: 2,
    };
    match device {
        2162694 => stomp,
        2162699 => Family {
            name: "HX Stomp XL",
            snapshots_ctrl: 12,
            midi_ctrl: 11,
            fs_first: 3,
            fs_count: 8,
            snapshot_count: 4,
            cc_switches: 8,
            ..stomp
        },
        2162693 => Family {
            name: "HX Effects",
            snapshots_ctrl: 10,
            midi_ctrl: 9,
            fs_first: 3,
            fs_count: 6,
            snapshot_count: 4,
            positions: 9,
            max_blocks: 9,
            cc_switches: 6,
            fx_loops: 2,
            ..stomp
        },
        2162689 => Family { name: "Helix Floor", ..helix },
        2162690 => Family { name: "Helix Rack", ..helix },
        2162692 => Family { name: "Helix LT", exp_count: 2, fx_loops: 2, ..helix },
        _ => helix,
    }
}

/// The pedal family from the preset's device number.
pub fn family(doc: &Value) -> Family {
    family_of_device(device(doc))
}

pub fn device(doc: &Value) -> i64 {
    doc.pointer("/data/device").and_then(Value::as_i64).unwrap_or(2162694)
}

/// What drives a knob, as the person sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Source {
    Snapshots,
    Exp(i64),
    Footswitch(i64),
    Midi,
}

impl Source {
    pub fn code(self, f: Family) -> i64 {
        match self {
            Source::Snapshots => f.snapshots_ctrl,
            Source::Exp(n) => n,
            Source::Footswitch(n) => f.fs_first + n - 1,
            Source::Midi => f.midi_ctrl,
        }
    }
    pub fn from_code(code: i64, f: Family) -> Option<Source> {
        if code == f.snapshots_ctrl {
            Some(Source::Snapshots)
        } else if code == f.midi_ctrl {
            Some(Source::Midi)
        } else if (1..=2).contains(&code) {
            Some(Source::Exp(code))
        } else if code >= f.fs_first && code < f.fs_first + f.fs_count {
            Some(Source::Footswitch(code - f.fs_first + 1))
        } else {
            None
        }
    }
    pub fn label(self) -> String {
        match self {
            Source::Snapshots => "Snapshots".into(),
            Source::Exp(n) => format!("EXP {n}"),
            Source::Footswitch(n) => format!("FS {n}"),
            Source::Midi => "MIDI CC".into(),
        }
    }
    pub fn short(self) -> String {
        match self {
            Source::Snapshots => "SNAP".into(),
            Source::Exp(n) => format!("EXP{n}"),
            Source::Footswitch(n) => format!("FS{n}"),
            Source::Midi => "MIDI".into(),
        }
    }
}

fn obj_at<'a>(doc: &'a mut Value, path: &[&str]) -> Option<&'a mut Map<String, Value>> {
    let mut v = doc.pointer_mut("/data/tone")?;
    for p in path {
        let o = v.as_object_mut()?;
        v = o.entry((*p).to_owned()).or_insert_with(|| json!({}));
    }
    v.as_object_mut()
}

// ------------------------------------------------------------------ snapshots ---

pub fn current(doc: &Value) -> usize {
    doc.pointer("/data/tone/global/@current_snapshot").and_then(Value::as_i64).unwrap_or(0).max(0) as usize
}

/// Snapshots in use: (index, name).
pub fn list(doc: &Value) -> Vec<(usize, String)> {
    let f = family(doc);
    (0..f.snapshot_count)
        .filter_map(|i| {
            let s = doc.pointer(&format!("/data/tone/snapshot{i}"))?;
            let valid = s.get("@valid").and_then(Value::as_bool).unwrap_or(true);
            valid.then(|| (i, s.get("@name").and_then(Value::as_str).unwrap_or("").to_owned()))
        })
        .collect()
}

/// Make another snapshot available, copying the current one's states.
pub fn add(doc: &mut Value) -> Option<usize> {
    let f = family(doc);
    let used: Vec<usize> = list(doc).into_iter().map(|(i, _)| i).collect();
    let next = (0..f.snapshot_count).find(|i| !used.contains(i))?;
    let from = current(doc);
    let mut copy = doc.pointer(&format!("/data/tone/snapshot{from}")).cloned().unwrap_or_else(|| json!({}));
    if let Some(o) = copy.as_object_mut() {
        o.insert("@name".into(), json!(format!("SNAPSHOT {}", next + 1)));
        o.insert("@valid".into(), json!(true));
        o.insert("@custom_name".into(), json!(false));
    }
    obj_at(doc, &[])?.insert(format!("snapshot{next}"), copy);
    Some(next)
}

pub fn rename(doc: &mut Value, index: usize, name: &str) {
    if let Some(s) = obj_at(doc, &[&format!("snapshot{index}")]) {
        s.insert("@name".into(), json!(name));
        s.insert("@custom_name".into(), json!(true));
        s.insert("@valid".into(), json!(true));
    }
}

/// Every block's on/off and every snapshot-controlled knob as they are now,
/// written into snapshot `index`.
fn store_into(doc: &mut Value, index: usize) {
    let f = family(doc);
    let blocks = hlx::blocks(doc);
    let states: Vec<(String, String, bool)> =
        blocks.iter().filter(|b| b.key.starts_with("block")).map(|b| (b.dsp.clone(), b.key.clone(), b.enabled.unwrap_or(true))).collect();
    let knobs = snapshot_knobs(doc, f);
    let values: Vec<(String, String, String, Value)> = knobs
        .into_iter()
        .filter_map(|(dsp, key, knob)| {
            let v = doc.pointer(&format!("/data/tone/{dsp}/{key}/{knob}"))?.clone();
            Some((dsp, key, knob, v))
        })
        .collect();
    let name = format!("snapshot{index}");
    for (dsp, key, on) in states {
        if let Some(m) = obj_at(doc, &[&name, "blocks", &dsp]) {
            m.insert(key, json!(on));
        }
    }
    for (dsp, key, knob, v) in values {
        if let Some(m) = obj_at(doc, &[&name, "controllers", &dsp, &key, &knob]) {
            m.insert("@fs_enabled".into(), json!(false));
            m.insert("@value".into(), v);
        }
    }
}

/// Knobs under snapshot control: (dsp, block key, knob).
fn snapshot_knobs(doc: &Value, f: Family) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    let Some(ctrl) = doc.pointer("/data/tone/controller").and_then(Value::as_object) else { return out };
    for (dsp, blocks) in ctrl {
        let Some(blocks) = blocks.as_object() else { continue };
        for (key, knobs) in blocks {
            let Some(knobs) = knobs.as_object() else { continue };
            for (knob, c) in knobs {
                if c.get("@controller").and_then(Value::as_i64) == Some(f.snapshots_ctrl) {
                    out.push((dsp.clone(), key.clone(), knob.clone()));
                }
            }
        }
    }
    out
}

/// Switch to another snapshot: remember the current one, then set every
/// block's on/off and snapshot-controlled knob from the chosen one.
pub fn select(doc: &mut Value, index: usize) {
    let cur = current(doc);
    store_into(doc, cur);
    apply(doc, index);
}

/// Keep what is on screen in the current snapshot.
pub fn store_current(doc: &mut Value) {
    let cur = current(doc);
    store_into(doc, cur);
}

/// Take a snapshot out. The others move up to close the gap; at least one stays.
pub fn delete(doc: &mut Value, index: usize) -> bool {
    let f = family(doc);
    let l = list(doc);
    if l.len() <= 1 || !l.iter().any(|(i, _)| *i == index) {
        return false;
    }
    let cur = current(doc);
    store_into(doc, cur);
    let kept: Vec<(usize, Value)> = l
        .iter()
        .filter(|(i, _)| *i != index)
        .filter_map(|(i, _)| Some((*i, doc.pointer(&format!("/data/tone/snapshot{i}"))?.clone())))
        .collect();
    // The same snapshot stays picked; when it was the one deleted, the next one (or the last).
    let new_cur = kept.iter().position(|(i, _)| *i == cur).or_else(|| kept.iter().position(|(i, _)| *i > index)).unwrap_or(kept.len() - 1);
    let n = kept.len();
    for (k, (_, mut v)) in kept.into_iter().enumerate() {
        if let Some(o) = v.as_object_mut() {
            if !o.get("@custom_name").and_then(Value::as_bool).unwrap_or(false) {
                o.insert("@name".into(), json!(format!("SNAPSHOT {}", k + 1)));
            }
        }
        if let Some(t) = obj_at(doc, &[]) {
            t.insert(format!("snapshot{k}"), v);
        }
    }
    for k in n..f.snapshot_count {
        if let Some(s) = doc.pointer_mut(&format!("/data/tone/snapshot{k}")).and_then(Value::as_object_mut) {
            s.insert("@valid".into(), json!(false));
            s.insert("@name".into(), json!(format!("SNAPSHOT {}", k + 1)));
            s.insert("@custom_name".into(), json!(false));
        }
    }
    if let Some(g) = obj_at(doc, &["global"]) {
        g.insert("@current_snapshot".into(), json!(new_cur));
    }
    apply(doc, new_cur);
    tidy_all(doc);
    true
}

/// A snapshot's settings, to paste into another one later.
pub fn copy(doc: &mut Value, index: usize) -> Option<Value> {
    store_current(doc);
    doc.pointer(&format!("/data/tone/snapshot{index}")).cloned()
}

/// Put copied settings into snapshot `index`, keeping its own name.
pub fn paste(doc: &mut Value, index: usize, copied: &Value) {
    store_current(doc);
    let mut v = copied.clone();
    let old = doc.pointer(&format!("/data/tone/snapshot{index}")).cloned();
    if let Some(o) = v.as_object_mut() {
        for k in ["@name", "@custom_name"] {
            match old.as_ref().and_then(|x| x.get(k)) {
                Some(x) => o.insert(k.into(), x.clone()),
                None => o.remove(k),
            };
        }
        o.insert("@valid".into(), json!(true));
    }
    if let Some(t) = obj_at(doc, &[]) {
        t.insert(format!("snapshot{index}"), v);
    }
    if index == current(doc) {
        apply(doc, index);
    }
    tidy_all(doc);
}

/// Whether two stored values are the same setting.
fn same(a: &Value, b: &Value) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => (x - y).abs() < 1e-6,
        _ => a == b,
    }
}

/// A knob only stays on snapshot control while its value differs between snapshots.
fn tidy(doc: &mut Value, b: &HlxBlock, knob: &str) {
    if controller(doc, b, knob).map(|c| c.0) != Some(Source::Snapshots) {
        return;
    }
    let cur = current(doc);
    let live = doc.pointer(&format!("/data/tone/{}/{}/{knob}", b.dsp, b.key)).cloned();
    let values: Vec<Value> = list(doc)
        .into_iter()
        .filter_map(|(i, _)| {
            if i == cur {
                live.clone()
            } else {
                doc.pointer(&format!("/data/tone/snapshot{i}/controllers/{}/{}/{knob}/@value", b.dsp, b.key)).cloned()
            }
        })
        .collect();
    if values.windows(2).all(|w| same(&w[0], &w[1])) {
        release(doc, b, knob);
    }
}

fn tidy_all(doc: &mut Value) {
    let f = family(doc);
    for (dsp, key, knob) in snapshot_knobs(doc, f) {
        let b = HlxBlock { dsp, key, model: String::new(), path: 0, position: 0, enabled: None };
        tidy(doc, &b, &knob);
    }
}

/// Set every block's on/off and snapshot-controlled knob from snapshot `index`.
fn apply(doc: &mut Value, index: usize) {
    let f = family(doc);
    let snap = doc.pointer(&format!("/data/tone/snapshot{index}")).cloned().unwrap_or_else(|| json!({}));
    for b in hlx::blocks(doc).into_iter().filter(|b| b.key.starts_with("block")) {
        if let Some(on) = snap.pointer(&format!("/blocks/{}/{}", b.dsp, b.key)).and_then(Value::as_bool) {
            if let Some(o) = hlx::block_mut(doc, &b) {
                o.insert("@enabled".into(), json!(on));
            }
        }
    }
    for (dsp, key, knob) in snapshot_knobs(doc, f) {
        if let Some(v) = snap.pointer(&format!("/controllers/{dsp}/{key}/{knob}/@value")).cloned() {
            if let Some(o) = doc.pointer_mut(&format!("/data/tone/{dsp}/{key}")).and_then(Value::as_object_mut) {
                o.insert(knob, v);
            }
        }
    }
    if let Some(cmds) = snap.get("commands").and_then(Value::as_object) {
        for (ck, cv) in cmds {
            if let (Some(src), Some(dst)) = (cv.as_object(), doc.pointer_mut(&format!("/data/tone/{ck}")).and_then(Value::as_object_mut)) {
                for (fk, fv) in src {
                    if fk != "@fs_enabled" {
                        dst.insert(fk.clone(), fv.clone());
                    }
                }
            }
        }
    }
    if let Some(g) = obj_at(doc, &["global"]) {
        g.insert("@current_snapshot".into(), json!(index));
    }
    store_into(doc, index);
}

/// After a knob or switch changed: keep it in the current snapshot.
/// A knob moved while the preset has more than one snapshot comes under
/// snapshot control, as on the pedal itself.
/// The person turned a knob from `old`: it comes under snapshot control when that makes it
/// differ between snapshots, and leaves it again once every snapshot has the same value.
pub fn knob_moved(doc: &mut Value, b: &HlxBlock, knob: &str, old: &Value) {
    if list(doc).len() > 1 && controller(doc, b, knob).is_none() {
        let now = doc.pointer(&format!("/data/tone/{}/{}/{knob}", b.dsp, b.key)).cloned();
        if now.as_ref().is_some_and(|n| same(n, old)) {
            return;
        }
        assign(doc, b, knob, Some(Source::Snapshots), 0.0, 1.0, None);
        // The other snapshots keep the value it had before.
        let cur = current(doc);
        for (i, _) in list(doc) {
            if i != cur {
                if let Some(m) = obj_at(doc, &[&format!("snapshot{i}"), "controllers", &b.dsp, &b.key, knob]) {
                    m.insert("@fs_enabled".into(), json!(false));
                    m.insert("@value".into(), old.clone());
                }
            }
        }
    }
    let cur = current(doc);
    store_into(doc, cur);
    tidy(doc, b, knob);
}

pub fn knob_changed(doc: &mut Value, b: &HlxBlock, knob: &str) {
    let f = family(doc);
    if list(doc).len() > 1 && controller(doc, b, knob).is_none() {
        assign(doc, b, knob, Some(Source::Snapshots), 0.0, 1.0, None);
        // Every snapshot starts from the value the knob has now.
        let cur = current(doc);
        let value = doc.pointer(&format!("/data/tone/{}/{}/{knob}", b.dsp, b.key)).cloned();
        if let Some(v) = value {
            for (i, _) in list(doc) {
                if i == cur {
                    continue;
                }
                let name = format!("snapshot{i}");
                let has = doc.pointer(&format!("/data/tone/{name}/controllers/{}/{}/{knob}/@value", b.dsp, b.key)).is_some();
                if !has {
                    if let Some(m) = obj_at(doc, &[&name, "controllers", &b.dsp, &b.key, knob]) {
                        m.insert("@fs_enabled".into(), json!(false));
                        m.insert("@value".into(), v.clone());
                    }
                }
            }
        }
    }
    let _ = f;
    let cur = current(doc);
    store_into(doc, cur);
}

pub fn block_switched(doc: &mut Value) {
    let cur = current(doc);
    store_into(doc, cur);
}

/// Take a knob off snapshot control again (it keeps its current value everywhere).
pub fn release(doc: &mut Value, b: &HlxBlock, knob: &str) {
    assign(doc, b, knob, None, 0.0, 1.0, None);
    for (i, _) in list(doc) {
        if let Some(m) = doc.pointer_mut(&format!("/data/tone/snapshot{i}/controllers/{}/{}", b.dsp, b.key)).and_then(Value::as_object_mut)
        {
            m.remove(knob);
        }
    }
}

// -------------------------------------------------------------- assignments ---

/// What drives a knob: (source, low end, high end, MIDI CC).
pub fn controller(doc: &Value, b: &HlxBlock, knob: &str) -> Option<(Source, f64, f64, Option<i64>)> {
    let f = family(doc);
    let c = doc.pointer(&format!("/data/tone/controller/{}/{}/{knob}", b.dsp, b.key))?;
    let src = Source::from_code(c.get("@controller")?.as_i64()?, f)?;
    Some((
        src,
        c.get("@min").and_then(Value::as_f64).unwrap_or(0.0),
        c.get("@max").and_then(Value::as_f64).unwrap_or(1.0),
        c.get("@cc").and_then(Value::as_i64),
    ))
}

/// Put a knob on a controller (or take it off with `None`).
pub fn assign(doc: &mut Value, b: &HlxBlock, knob: &str, src: Option<Source>, min: f64, max: f64, cc: Option<i64>) {
    let f = family(doc);
    match src {
        Some(s) => {
            if let Some(m) = obj_at(doc, &["controller", &b.dsp, &b.key, knob]) {
                m.clear();
                m.insert("@min".into(), json!(min));
                m.insert("@max".into(), json!(max));
                m.insert("@controller".into(), json!(s.code(f)));
                m.insert("@snapshot_disable".into(), json!(false));
                if s == Source::Midi {
                    m.insert("@cc".into(), json!(cc.unwrap_or(1)));
                }
            }
        }
        None => {
            if let Some(m) = doc.pointer_mut(&format!("/data/tone/controller/{}/{}", b.dsp, b.key)).and_then(Value::as_object_mut) {
                m.remove(knob);
            }
        }
    }
}

/// A block's on/off footswitch: (switch number, momentary).
pub fn footswitch(doc: &Value, b: &HlxBlock) -> Option<(i64, bool)> {
    let fs = doc.pointer(&format!("/data/tone/footswitch/{}/{}", b.dsp, b.key))?;
    let index = fs.get("@fs_index")?.as_i64()?;
    Some((index, fs.get("@fs_momentary").and_then(Value::as_bool).unwrap_or(false)))
}

/// Put a block's on/off on a footswitch (or take it off with `None`).
pub fn set_footswitch(doc: &mut Value, b: &HlxBlock, index: Option<i64>, momentary: bool, label: &str) {
    match index {
        Some(i) => {
            if let Some(m) = obj_at(doc, &["footswitch", &b.dsp, &b.key]) {
                m.insert("@fs_index".into(), json!(i));
                m.insert("@fs_enabled".into(), json!(true));
                m.insert("@fs_momentary".into(), json!(momentary));
                m.insert("@fs_primary".into(), json!(true));
                m.insert("@fs_label".into(), json!(label));
                m.entry("@fs_ledcolor").or_insert(json!(0));
            }
        }
        None => {
            if let Some(m) = doc.pointer_mut(&format!("/data/tone/footswitch/{}", b.dsp)).and_then(Value::as_object_mut) {
                m.remove(&b.key);
            }
        }
    }
}

/// Change which unit a preset is for. Units without parallel lanes get their
/// lower-lane blocks moved onto the main lane where there is room.
#[cfg(test)]
pub fn set_device(doc: &mut Value, device: i64) {
    if let Some(d) = doc.pointer_mut("/data").and_then(Value::as_object_mut) {
        d.insert("device".into(), json!(device));
    }
    let f = family_of_device(device);
    if !f.lanes {
        for b in hlx::blocks(doc).into_iter().filter(|b| b.path == 1 && b.key.starts_with("block")) {
            let taken: Vec<i64> = hlx::blocks(doc).iter().filter(|x| x.dsp == b.dsp && x.path == 0).map(|x| x.position).collect();
            if let Some(free) = (0..hlx::POSITIONS).find(|p| !taken.contains(p)) {
                hlx::move_to(doc, &b, 0, free);
            }
        }
        crate::hlx::clear_split(doc);
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_keep_their_own_values() {
        let mut doc = hlx::sample();
        let b = hlx::blocks(&doc).into_iter().find(|b| b.key == "block1").unwrap();
        let turn = |doc: &mut Value, to: f64| {
            let old = doc.pointer("/data/tone/dsp0/block1/Gain").cloned().unwrap();
            hlx::block_mut(doc, &b).unwrap().insert("Gain".into(), json!(to));
            knob_moved(doc, &b, "Gain", &old);
        };
        // Snapshot 1: Gain to 9.
        select(&mut doc, 0);
        turn(&mut doc, 9.0);
        // Snapshot 2: Gain to 2, and the block off.
        select(&mut doc, 1);
        turn(&mut doc, 2.0);
        hlx::block_mut(&mut doc, &b).unwrap().insert("@enabled".into(), json!(false));
        block_switched(&mut doc);
        // Back to snapshot 1: Gain 9, block on.
        select(&mut doc, 0);
        assert_eq!(doc.pointer("/data/tone/dsp0/block1/Gain").unwrap(), &json!(9.0));
        assert_eq!(doc.pointer("/data/tone/dsp0/block1/@enabled").unwrap(), &json!(true));
        select(&mut doc, 1);
        assert_eq!(doc.pointer("/data/tone/dsp0/block1/Gain").unwrap(), &json!(2.0));
        assert_eq!(doc.pointer("/data/tone/dsp0/block1/@enabled").unwrap(), &json!(false));
        assert_eq!(controller(&doc, &b, "Gain").map(|c| c.0), Some(Source::Snapshots));
        // Once every snapshot has the same Gain again, it is no longer on snapshot control.
        for (i, _) in list(&doc).into_iter().skip(1) {
            select(&mut doc, i);
            turn(&mut doc, 9.0);
        }
        assert_eq!(controller(&doc, &b, "Gain"), None);
        assert_eq!(doc.pointer("/data/tone/dsp0/block1/Gain").unwrap(), &json!(9.0));
    }

    #[test]
    fn assignments_round_trip() {
        let mut doc = hlx::sample();
        let b = hlx::blocks(&doc).into_iter().find(|b| b.key == "block1").unwrap();
        assign(&mut doc, &b, "Tone", Some(Source::Exp(1)), 0.2, 0.8, None);
        assert_eq!(controller(&doc, &b, "Tone"), Some((Source::Exp(1), 0.2, 0.8, None)));
        set_footswitch(&mut doc, &b, Some(2), false, "Drive");
        assert_eq!(footswitch(&doc, &b), Some((2, false)));
        // On an HX Stomp, footswitch 2 for a knob is controller 4.
        assign(&mut doc, &b, "Level", Some(Source::Footswitch(2)), 0.0, 1.0, None);
        assert_eq!(doc.pointer("/data/tone/controller/dsp0/block1/Level/@controller").unwrap(), &json!(4));
    }
}

/// One knob on a controller.
pub struct Assignment {
    pub block: HlxBlock,
    pub knob: String,
    pub src: Source,
    pub lo: f64,
    pub hi: f64,
    pub cc: Option<i64>,
}

/// Every knob assignment in the preset.
pub fn all_assignments(doc: &Value) -> Vec<Assignment> {
    let mut out = Vec::new();
    for b in hlx::blocks(doc) {
        let Some(knobs) = doc.pointer(&format!("/data/tone/controller/{}/{}", b.dsp, b.key)).and_then(Value::as_object) else { continue };
        for knob in knobs.keys() {
            if let Some((src, lo, hi, cc)) = controller(doc, &b, knob) {
                out.push(Assignment { block: b.clone(), knob: knob.clone(), src, lo, hi, cc });
            }
        }
    }
    out
}

/// Every block whose on/off sits on a footswitch: (block, switch, momentary).
pub fn all_footswitches(doc: &Value) -> Vec<(HlxBlock, i64, bool)> {
    hlx::blocks(doc).into_iter().filter_map(|b| footswitch(doc, &b).map(|(i, m)| (b, i, m))).collect()
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn units_shape_new_presets_and_lanes() {
        let helix = hlx::blank_for("h", 2162689);
        assert!(helix.pointer("/data/tone/dsp1").is_some(), "Helix has two paths");
        let stomp = hlx::blank_for("s", 2162694);
        assert!(stomp.pointer("/data/tone/dsp1").is_none(), "HX Stomp has one path");
        assert_eq!(list(&stomp).len(), 1, "a new preset starts with one snapshot");
        let mut s = stomp.clone();
        assert_eq!(add(&mut s), Some(1));
        assert_eq!(add(&mut s), Some(2));
        rename(&mut s, 2, "LEAD");
        select(&mut s, 2);
        assert!(delete(&mut s, 1));
        assert_eq!(list(&s), vec![(0, "SNAPSHOT 1".to_owned()), (1, "LEAD".to_owned())]);
        assert_eq!(current(&s), 1);
        assert!(delete(&mut s, 0));
        assert!(!delete(&mut s, 0), "the last snapshot stays");
        assert_eq!(unit_for_product("HX Stomp XL"), Some(2162699));
        assert_eq!(unit_for_product("HX Stomp XL (USB)"), Some(2162699));
        assert_eq!(unit_for_product("Helix Floor via Alt box"), Some(2162689), "the box name is not the pedal");
        assert_eq!(unit_for_product("HX Stomp via Rack room"), Some(2162694));
        assert_eq!(unit_for_product("Helix LT (USB)"), Some(2162692));
        assert_eq!(unit_for_product("Helix Stadium"), None);
        assert_eq!((family_of_device(2162693).fx_loops, family_of_device(2162689).fx_loops), (2, 4), "HX Effects has two loop rows");

        // A Helix preset with a block on the lower lane gets a split and a merge.
        let mut doc = helix;
        hlx::place_model(
            &mut doc,
            "dsp0",
            1,
            2,
            None,
            &hlx::NewModel { model: "HD2_Chorus70sChorus", stereo: false, params: &[], cab: None },
        );
        hlx::ensure_split(&mut doc, "dsp0", &|_| Vec::new());
        let (_, sp, jp) = hlx::split_of(&doc, "dsp0").unwrap();
        assert_eq!((sp, jp), (2, 3));
        // An HX Stomp splits its single path too; taking the split away folds the lane back.
        set_device(&mut doc, 2162694);
        assert!(hlx::split_of(&doc, "dsp0").is_some());
        assert!(hlx::remove_split(&mut doc, "dsp0", 8));
        assert!(hlx::split_of(&doc, "dsp0").is_none());
        assert!(hlx::blocks(&doc).iter().all(|b| b.path == 0));
    }
}
