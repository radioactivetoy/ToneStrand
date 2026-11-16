//! Preset files kept on the computer.
//!
//! `.hlx` is the JSON preset file Helix editors export. `.hxpreset` is the
//! pedal's own document saved byte for byte, which can go back to any slot.

use serde_json::{json, Map, Value};

#[derive(Clone, Debug)]
pub struct HlxBlock {
    pub dsp: String,
    pub key: String,
    pub model: String,
    pub enabled: Option<bool>,
    pub path: i64,
    pub position: i64,
}

pub fn name(doc: &Value) -> String {
    doc.pointer("/data/meta/name").and_then(Value::as_str).unwrap_or("Untitled").to_owned()
}

pub fn set_name(doc: &mut Value, name: &str) {
    if let Some(meta) = doc.pointer_mut("/data/meta").and_then(Value::as_object_mut) {
        meta.insert("name".into(), Value::String(name.to_owned()));
    }
}

pub fn tempo(doc: &Value) -> Option<f64> {
    doc.pointer("/data/tone/global/@tempo").and_then(Value::as_f64)
}

pub fn set_tempo(doc: &mut Value, bpm: f64) {
    if let Some(g) = doc.pointer_mut("/data/tone/global").and_then(Value::as_object_mut) {
        g.insert("@tempo".into(), json!(bpm));
    }
}

/// The DSP sections present (dsp0, dsp1).
pub fn dsps(doc: &Value) -> Vec<String> {
    let mut out: Vec<String> = doc
        .pointer("/data/tone")
        .and_then(Value::as_object)
        .map(|t| t.keys().filter(|k| k.starts_with("dsp")).cloned().collect())
        .unwrap_or_default();
    out.sort();
    out
}

/// Every effect block and cab, in signal order.
pub fn blocks(doc: &Value) -> Vec<HlxBlock> {
    let mut out = Vec::new();
    for dsp in dsps(doc) {
        let Some(section) = doc.pointer(&format!("/data/tone/{dsp}")).and_then(Value::as_object) else {
            continue;
        };
        for (k, v) in section {
            let is_block = k.starts_with("block") || k.starts_with("cab");
            if !is_block || !v.is_object() {
                continue;
            }
            out.push(HlxBlock {
                dsp: dsp.clone(),
                key: k.clone(),
                model: v.get("@model").and_then(Value::as_str).unwrap_or("?").to_owned(),
                enabled: v.get("@enabled").and_then(Value::as_bool),
                path: v.get("@path").and_then(Value::as_i64).unwrap_or(0),
                position: v.get("@position").and_then(Value::as_i64).unwrap_or(if k.starts_with("cab") { 99 } else { 0 }),
            });
        }
    }
    out.sort_by(|a, b| (&a.dsp, a.path, a.position, &a.key).cmp(&(&b.dsp, b.path, b.position, &b.key)));
    out
}

pub fn block<'a>(doc: &'a Value, b: &HlxBlock) -> Option<&'a Map<String, Value>> {
    doc.pointer(&format!("/data/tone/{}/{}", b.dsp, b.key)).and_then(Value::as_object)
}

pub fn block_mut<'a>(doc: &'a mut Value, b: &HlxBlock) -> Option<&'a mut Map<String, Value>> {
    doc.pointer_mut(&format!("/data/tone/{}/{}", b.dsp, b.key)).and_then(Value::as_object_mut)
}

pub fn snapshot_names(doc: &Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for i in 0..8 {
        let k = format!("snapshot{i}");
        if let Some(s) = doc.pointer(&format!("/data/tone/{k}")) {
            let n = s.get("@name").and_then(Value::as_str).unwrap_or("").to_owned();
            out.push((k, n));
        }
    }
    out
}

pub fn set_snapshot_name(doc: &mut Value, key: &str, name: &str) {
    if let Some(s) = doc.pointer_mut(&format!("/data/tone/{key}")).and_then(Value::as_object_mut) {
        s.insert("@name".into(), Value::String(name.to_owned()));
    }
}

/// A readable model name: `HD2_AmpBritPlexiBrt` becomes `Amp Brit Plexi Brt`.
pub fn pretty_model(model: &str) -> String {
    let core = model.split_once('_').map(|(_, r)| r).unwrap_or(model);
    let mut out = String::new();
    let mut prev_lower = false;
    for c in core.chars() {
        if c.is_uppercase() && prev_lower {
            out.push(' ');
        }
        prev_lower = c.is_lowercase() || c.is_ascii_digit();
        out.push(c);
    }
    out
}

/// The family a model belongs to, for colouring its tile.
pub fn family(model: &str) -> &'static str {
    let m = model.to_ascii_lowercase();
    let core = m.split_once('_').map(|(_, r)| r.to_owned()).unwrap_or(m.clone());
    for (prefix, fam) in [
        ("ampcab", "amp"),
        ("amp", "amp"),
        ("preamp", "amp"),
        ("cab", "cab"),
        ("ir", "cab"),
        ("dist", "drive"),
        ("drive", "drive"),
        ("fuzz", "drive"),
        ("comp", "dynamics"),
        ("gate", "dynamics"),
        ("dl", "delay"),
        ("delay", "delay"),
        ("rev", "reverb"),
        ("verb", "reverb"),
        ("mod", "modulation"),
        ("chorus", "modulation"),
        ("flanger", "modulation"),
        ("phaser", "modulation"),
        ("trem", "modulation"),
        ("wah", "filter"),
        ("filter", "filter"),
        ("eq", "eq"),
        ("pitch", "pitch"),
        ("vol", "volume"),
        ("looper", "volume"),
    ] {
        if core.starts_with(prefix) {
            return fam;
        }
    }
    "other"
}

// -------------------------------------------------------- editing the chain ---

/// How many block positions a path has.
/// The most positions any lane has (the unit decides how many are shown).
pub const POSITIONS: i64 = 16;

/// A parameter's default as JSON, by its type (0 whole number, 1 decimal, 2 on/off).
fn default_json(vt: i64, d: f64) -> Value {
    match vt {
        2 => Value::Bool(d == 1.0),
        0 => json!(d.round() as i64),
        _ => json!(d),
    }
}

fn free_key(section: &Map<String, Value>, prefix: &str) -> String {
    (0..1000).map(|i| format!("{prefix}{i}")).find(|k| !section.contains_key(k)).unwrap_or_else(|| format!("{prefix}999"))
}

/// Put a model at a path position (adding a block), or into an existing block
/// (`key`), with every knob at its default. `cab` adds the amp's matching cab.
#[allow(clippy::too_many_arguments)]
pub fn place_model(
    doc: &mut Value,
    dsp: &str,
    path: i64,
    position: i64,
    key: Option<&str>,
    model: &str,
    stereo: bool,
    params: &[(String, i64, f64)],
    cab: Option<(&str, &[(String, i64, f64)])>,
) -> Option<String> {
    let tone = doc.pointer_mut("/data/tone")?.as_object_mut()?;
    let section = tone.entry(dsp.to_owned()).or_insert_with(|| json!({})).as_object_mut()?;
    let k = key.map(str::to_owned).unwrap_or_else(|| free_key(section, "block"));
    let old = section.get(&k).and_then(Value::as_object).cloned();
    let enabled = old.as_ref().and_then(|o| o.get("@enabled")).cloned().unwrap_or(Value::Bool(true));
    // An old cab that rode with this block goes with it.
    if let Some(old_cab) = old.as_ref().and_then(|o| o.get("@cab")).and_then(Value::as_str) {
        section.remove(old_cab);
    }
    let mut block = Map::new();
    block.insert("@model".into(), json!(model));
    block.insert("@enabled".into(), enabled);
    block.insert("@path".into(), json!(path));
    block.insert("@position".into(), json!(position));
    block.insert("@stereo".into(), json!(stereo));
    block.insert("@type".into(), json!(0));
    block.insert("@no_snapshot_bypass".into(), json!(false));
    block.insert("@bypassvolume".into(), json!(1));
    for (id, vt, d) in params {
        block.insert(id.clone(), default_json(*vt, *d));
    }
    if let Some((cab_model, cab_params)) = cab {
        let ck = free_key(section, "cab");
        let mut c = Map::new();
        c.insert("@model".into(), json!(cab_model));
        c.insert("@enabled".into(), json!(true));
        c.insert("@mic".into(), json!(0));
        for (id, vt, d) in cab_params {
            c.insert(id.clone(), default_json(*vt, *d));
        }
        section.insert(ck.clone(), Value::Object(c));
        block.insert("@cab".into(), json!(ck));
    }
    section.insert(k.clone(), Value::Object(block));
    Some(k)
}

/// Take a block (and the cab riding with it) out of the preset.
pub fn remove_block(doc: &mut Value, b: &HlxBlock) {
    if let Some(section) = doc.pointer_mut(&format!("/data/tone/{}", b.dsp)).and_then(Value::as_object_mut) {
        if let Some(cab) = section.get(&b.key).and_then(|v| v.get("@cab")).and_then(Value::as_str).map(str::to_owned) {
            section.remove(&cab);
        }
        section.remove(&b.key);
    }
    if b.path == 1 {
        if let Some((_, sp, _)) = split_of(doc, &b.dsp) {
            let mut lane_b: Vec<HlxBlock> = blocks(doc)
                .into_iter()
                .filter(|x| x.dsp == b.dsp && x.path == 1 && x.key.starts_with("block"))
                .collect();
            lane_b.sort_by_key(|x| x.position);
            for (i, lb) in lane_b.iter().enumerate() {
                if let Some(obj) = block_mut(doc, lb) {
                    obj.insert("@position".into(), json!(sp + i as i64));
                }
            }
            set_lower_slots(doc, &b.dsp, (lane_b.len() as i64).max(1));
        }
    }
}

/// Move a block to another position on its path, swapping with whatever is there.
pub fn move_block(doc: &mut Value, b: &HlxBlock, to: i64) {
    move_to(doc, b, b.path, to);
}

/// Move a block to a lane and position of the same DSP (a drag and drop),
/// swapping with the block already there.
pub fn move_to(doc: &mut Value, b: &HlxBlock, path: i64, to: i64) {
    if !(0..POSITIONS).contains(&to) || (path == b.path && to == b.position) {
        return;
    }
    let other = blocks(doc).into_iter().find(|x| x.dsp == b.dsp && x.path == path && x.position == to && x.key.starts_with("block"));
    let moved_from_lower_to_empty_main = b.path == 1 && path == 0 && other.is_none();
    if let Some(o) = other {
        if let Some(obj) = block_mut(doc, &o) {
            obj.insert("@position".into(), json!(b.position));
            obj.insert("@path".into(), json!(b.path));
        }
    }
    if let Some(obj) = block_mut(doc, b) {
        obj.insert("@position".into(), json!(to));
        obj.insert("@path".into(), json!(path));
    }
    if moved_from_lower_to_empty_main {
        if let Some((_, sp, _)) = split_of(doc, &b.dsp) {
            let mut lane_b: Vec<HlxBlock> = blocks(doc)
                .into_iter()
                .filter(|x| x.dsp == b.dsp && x.path == 1 && x.key.starts_with("block"))
                .collect();
            lane_b.sort_by_key(|x| x.position);
            for (i, lb) in lane_b.iter().enumerate() {
                if let Some(obj) = block_mut(doc, lb) {
                    obj.insert("@position".into(), json!(sp + i as i64));
                }
            }
            set_lower_slots(doc, &b.dsp, (lane_b.len() as i64).max(1));
        }
    }
}

// ------------------------------------------------------- splits and merges ---

/// The split types a path can use: (model id, name).
pub const SPLITS: [(&str, &str); 4] = [
    ("HD2_AppDSPFlowSplitY", "Split Y"),
    ("HD2_AppDSPFlowSplitAB", "Split A/B"),
    ("HD2_AppDSPFlowSplitXOver", "Split Crossover"),
    ("HD2_AppDSPFlowSplitDyn", "Split Dynamic"),
];
pub const JOIN_MODEL: &str = "HD2_AppDSPFlowJoin";

fn dsp_index(dsp: &str) -> String {
    dsp.trim_start_matches("dsp").to_owned()
}

/// Where a path splits and merges: (split model, split position, merge position).
pub fn split_of(doc: &Value, dsp: &str) -> Option<(String, i64, i64)> {
    let section = doc.pointer(&format!("/data/tone/{dsp}"))?;
    let split = section.get("split")?;
    let join = section.get("join");
    Some((
        split.get("@model").and_then(Value::as_str).unwrap_or(SPLITS[0].0).to_owned(),
        split.get("@position").and_then(Value::as_i64).unwrap_or(0),
        join.and_then(|j| j.get("@position")).and_then(Value::as_i64).unwrap_or(POSITIONS),
    ))
}

/// How many slot positions the parallel lane (`@path == 1`) holds on this DSP path.
pub fn lower_slots_of(doc: &Value, dsp: &str) -> i64 {
    let Some((_, sp, jp)) = split_of(doc, dsp) else { return 0 };
    let lane_b: Vec<i64> = blocks(doc)
        .into_iter()
        .filter(|b| b.dsp == dsp && b.path == 1 && b.key.starts_with("block"))
        .map(|b| b.position)
        .collect();
    let occupied = lane_b.iter().max().map(|&m| (m - sp + 1).max(1)).unwrap_or(0);
    let stored = doc
        .pointer(&format!("/data/tone/{dsp}/split/@lower_slots"))
        .and_then(Value::as_i64);
    match stored {
        Some(n) => n.max(occupied).max(1),
        None if occupied > 0 => occupied,
        None => (jp - sp).max(1),
    }
}

pub fn set_lower_slots(doc: &mut Value, dsp: &str, count: i64) {
    if let Some(s) = doc.pointer_mut(&format!("/data/tone/{dsp}/split")).and_then(Value::as_object_mut) {
        s.insert("@lower_slots".into(), json!(count.max(1)));
    }
}

/// Make sure a path that uses its lower lane has a split and a merge around it
/// (and the matching topology), with every split/merge knob at its default.
pub fn ensure_split(doc: &mut Value, dsp: &str, defaults: &dyn Fn(&str) -> Vec<(String, i64, f64)>) {
    let lane_b: Vec<i64> = blocks(doc).iter().filter(|b| b.dsp == dsp && b.path == 1).map(|b| b.position).collect();
    if lane_b.is_empty() {
        return;
    }
    let first = *lane_b.iter().min().unwrap_or(&0);
    let last = *lane_b.iter().max().unwrap_or(&0);
    make_split(doc, dsp, first, last + 1, defaults);
}

/// Keep the split before the first lower-lane block and make sure the lower lane
/// has enough slots to cover all its blocks, without overriding a merge position
/// placed earlier on the primary path.
pub fn fit_split(doc: &mut Value, dsp: &str) {
    let lane_b: Vec<i64> = blocks(doc).iter().filter(|b| b.dsp == dsp && b.path == 1 && b.key.starts_with("block")).map(|b| b.position).collect();
    let (Some(min), Some(max)) = (lane_b.iter().min().copied(), lane_b.iter().max().copied()) else { return };
    let Some((_, mut sp, jp)) = split_of(doc, dsp) else { return };
    if min < sp {
        sp = min;
        set_split_position(doc, dsp, false, sp);
    }
    let needed = (max - sp + 1).max(1);
    if lower_slots_of(doc, dsp) < needed {
        set_lower_slots(doc, dsp, needed);
    }
    if jp < sp {
        set_split_position(doc, dsp, true, sp);
    }
}

/// How many blocks a DSP path holds, on both lanes.
pub fn block_count(doc: &Value, dsp: &str) -> i64 {
    blocks(doc).iter().filter(|b| b.dsp == dsp && b.key.starts_with("block")).count() as i64
}

/// How many positions the main lane shows on this DSP path.
/// On shared-slot units (HX Stomp, HX Stomp XL, HX Effects), each slot on the
/// parallel lane reduces the main lane by one so `main_slots + parallel_slots == max_blocks`.
pub fn main_cols_for(doc: &Value, dsp: &str, positions: i64, max_blocks: i64, shared_slots: bool) -> i64 {
    if shared_slots && split_of(doc, dsp).is_some() {
        let lower_slots = lower_slots_of(doc, dsp).clamp(1, (max_blocks - 1).max(1));
        return (max_blocks - lower_slots).clamp(1, positions);
    }
    positions
}

/// Keep main and parallel lanes within the unit's slot limits.
/// On shared-slot units, parallel slots reduce the main lane and any main-lane
/// blocks beyond the reduced column count are compacted left in order.
/// The parallel lane can be longer than the primary lane, and the merge position
/// can sit at any column on the primary lane from `split` up to `main_cols`.
pub fn fit_lanes(doc: &mut Value, dsp: &str, positions: i64, max_blocks: i64, shared_slots: bool) {
    fit_split(doc, dsp);
    let Some((_, mut sp, jp)) = split_of(doc, dsp) else { return };
    let max_lower = if shared_slots { (max_blocks - 1).max(1) } else { positions };
    let lower_slots = lower_slots_of(doc, dsp).clamp(1, max_lower);
    set_lower_slots(doc, dsp, lower_slots);
    let main_cols = if shared_slots {
        (max_blocks - lower_slots).clamp(1, positions)
    } else {
        positions
    };
    let max_sp = main_cols.min(positions - lower_slots).max(0);
    if sp > max_sp {
        let shift = sp - max_sp;
        sp = max_sp;
        set_split_position(doc, dsp, false, sp);
        let lower_blocks: Vec<HlxBlock> = blocks(doc).into_iter().filter(|b| b.dsp == dsp && b.path == 1 && b.key.starts_with("block")).collect();
        for b in lower_blocks {
            if let Some(obj) = block_mut(doc, &b) {
                obj.insert("@position".into(), json!((b.position - shift).clamp(sp, sp + lower_slots - 1)));
            }
        }
    }
    let clamped_jp = jp.clamp(sp, main_cols);
    if clamped_jp != jp {
        set_split_position(doc, dsp, true, clamped_jp);
    }
    if shared_slots {
        let main_blocks: Vec<HlxBlock> = blocks(doc).into_iter().filter(|b| b.dsp == dsp && b.path == 0 && b.key.starts_with("block")).collect();
        let mut limit = main_cols - 1;
        for b in main_blocks.into_iter().rev() {
            if b.position > limit && limit >= 0 {
                if let Some(obj) = block_mut(doc, &b) {
                    obj.insert("@position".into(), json!(limit));
                }
                limit -= 1;
            } else {
                limit = (b.position - 1).min(limit - 1);
            }
        }
    }
}

/// Move the split or merge point along the primary lane.
/// Moving the split shifts parallel-lane blocks with it; moving the merge
/// changes where the parallel lane rejoins the primary lane without altering
/// the parallel lane's length.
pub fn move_split_or_join(
    doc: &mut Value,
    dsp: &str,
    merge: bool,
    position: i64,
    positions: i64,
    max_blocks: i64,
    shared_slots: bool,
) {
    let Some((_, sp, jp)) = split_of(doc, dsp) else { return };
    let lower_slots = lower_slots_of(doc, dsp);
    set_lower_slots(doc, dsp, lower_slots);
    let main_cols = main_cols_for(doc, dsp, positions, max_blocks, shared_slots);
    if merge {
        let new_jp = position.clamp(sp, main_cols);
        set_split_position(doc, dsp, true, new_jp);
    } else {
        let max_sp = main_cols.min(positions - lower_slots).max(0);
        let new_sp = position.clamp(0, max_sp);
        let delta = new_sp - sp;
        if delta != 0 {
            let lower_blocks: Vec<HlxBlock> = blocks(doc)
                .into_iter()
                .filter(|b| b.dsp == dsp && b.path == 1 && b.key.starts_with("block"))
                .collect();
            for b in lower_blocks {
                if let Some(obj) = block_mut(doc, &b) {
                    obj.insert("@position".into(), json!((b.position + delta).clamp(new_sp, new_sp + lower_slots - 1)));
                }
            }
        }
        set_split_position(doc, dsp, false, new_sp);
        if jp < new_sp {
            set_split_position(doc, dsp, true, new_sp);
        }
    }
    fit_lanes(doc, dsp, positions, max_blocks, shared_slots);
}

/// Widen the parallel lane by one slot so it can hold another effect, returning
/// the column index of the newly opened parallel slot.
pub fn extend_lower(doc: &mut Value, dsp: &str, positions: i64, max_blocks: i64, shared_slots: bool) -> Option<usize> {
    let (_, sp, jp) = split_of(doc, dsp)?;
    let cur_lower = lower_slots_of(doc, dsp);
    let max_lower = if shared_slots { (max_blocks - 1).max(1) } else { positions };
    if cur_lower >= max_lower {
        return None;
    }
    let old_main_cols = main_cols_for(doc, dsp, positions, max_blocks, shared_slots);
    let new_lower = cur_lower + 1;
    set_lower_slots(doc, dsp, new_lower);
    if jp >= (sp + cur_lower).min(old_main_cols) {
        let new_main_cols = if shared_slots { (max_blocks - new_lower).clamp(1, positions) } else { positions };
        set_split_position(doc, dsp, true, (sp + new_lower).min(new_main_cols));
    }
    fit_lanes(doc, dsp, positions, max_blocks, shared_slots);
    let (_, new_sp, _) = split_of(doc, dsp)?;
    Some((new_sp + new_lower - 1).max(0) as usize)
}

/// Move an existing block onto a newly opened slot at the end of the parallel lane.
pub fn move_to_extend_lower(doc: &mut Value, b: &HlxBlock, positions: i64, max_blocks: i64, shared_slots: bool) {
    let Some((_, sp, jp)) = split_of(doc, &b.dsp) else { return };
    let cur_lower = lower_slots_of(doc, &b.dsp);
    let max_lower = if shared_slots { (max_blocks - 1).max(1) } else { positions };
    if cur_lower >= max_lower {
        return;
    }
    let old_main_cols = main_cols_for(doc, &b.dsp, positions, max_blocks, shared_slots);
    let new_lower = cur_lower + 1;
    if let Some(obj) = block_mut(doc, b) {
        obj.insert("@path".into(), json!(1));
        obj.insert("@position".into(), json!(sp + cur_lower));
    }
    set_lower_slots(doc, &b.dsp, new_lower);
    if jp >= (sp + cur_lower).min(old_main_cols) {
        let new_main_cols = if shared_slots { (max_blocks - new_lower).clamp(1, positions) } else { positions };
        set_split_position(doc, &b.dsp, true, (sp + new_lower).min(new_main_cols));
    }
    fit_lanes(doc, &b.dsp, positions, max_blocks, shared_slots);
}

/// Split a path on purpose (before any block is on the lower lane): the
/// split goes before the first free position, starting with one parallel slot.
/// Whether a path is really split: a split and merge that HX Edit keeps in the file while the
/// path is a single lane (topology "A", nothing on the lower lane) don't count.
pub fn split_active(doc: &Value, dsp: &str) -> bool {
    if split_of(doc, dsp).is_none() {
        return false;
    }
    if blocks(doc).iter().any(|b| b.dsp == dsp && b.path == 1 && b.key.starts_with("block")) {
        return true;
    }
    doc.pointer(&format!("/data/tone/global/@topology{}", dsp_index(dsp))).and_then(Value::as_str).unwrap_or("A") != "A"
}

pub fn add_split(doc: &mut Value, dsp: &str, defaults: &dyn Fn(&str) -> Vec<(String, i64, f64)>) {
    add_split_at(doc, dsp, None, defaults);
}

/// Split the path before position `at` (the first free place when None), merging one place later.
pub fn add_split_at(doc: &mut Value, dsp: &str, at: Option<i64>, defaults: &dyn Fn(&str) -> Vec<(String, i64, f64)>) {
    // A split and merge left in the file from a single-lane path start over where they're needed.
    if split_of(doc, dsp).is_some() && !split_active(doc, dsp) {
        if let Some(section) = doc.pointer_mut(&format!("/data/tone/{dsp}")).and_then(Value::as_object_mut) {
            section.remove("split");
            section.remove("join");
        }
    }
    let fam = crate::snapshots::family(doc);
    let max_pos = if fam.shared_slots { (fam.max_blocks - 1).max(1) } else { fam.positions };
    let used: Vec<i64> = blocks(doc).iter().filter(|b| b.dsp == dsp && b.path == 0 && b.key.starts_with("block")).map(|b| b.position).collect();
    let at = at.map(|a| a.clamp(0, (max_pos - 1).max(0))).unwrap_or_else(|| (0..max_pos).find(|p| !used.contains(p)).unwrap_or(0));
    make_split(doc, dsp, at, (at + 1).min(POSITIONS), defaults);
    set_lower_slots(doc, dsp, 1);
    fit_lanes(doc, dsp, fam.positions, fam.max_blocks, fam.shared_slots);
}

/// Take a split away again: blocks on the lower lane move to free places on the main lane.
pub fn remove_split(doc: &mut Value, dsp: &str) {
    for b in blocks(doc).into_iter().filter(|b| b.dsp == dsp && b.path == 1 && b.key.starts_with("block")) {
        let taken: Vec<i64> = blocks(doc).iter().filter(|x| x.dsp == dsp && x.path == 0).map(|x| x.position).collect();
        match (0..POSITIONS).find(|p| !taken.contains(p)) {
            Some(free) => move_to(doc, &b, 0, free),
            None => remove_block(doc, &b),
        }
    }
    if let Some(section) = doc.pointer_mut(&format!("/data/tone/{dsp}")).and_then(Value::as_object_mut) {
        section.remove("split");
        section.remove("join");
    }
    let key = format!("@topology{}", dsp_index(dsp));
    if let Some(g) = doc.pointer_mut("/data/tone/global").and_then(Value::as_object_mut) {
        g.insert(key, json!("A"));
    }
}

fn make_split(doc: &mut Value, dsp: &str, first: i64, merge_at: i64, defaults: &dyn Fn(&str) -> Vec<(String, i64, f64)>) {
    let last = merge_at - 1;
    let Some(section) = doc.pointer_mut(&format!("/data/tone/{dsp}")).and_then(Value::as_object_mut) else { return };
    if !section.contains_key("split") {
        let mut s = Map::new();
        s.insert("@model".into(), json!(SPLITS[0].0));
        s.insert("@position".into(), json!(first));
        s.insert("@enabled".into(), json!(true));
        s.insert("@no_snapshot_bypass".into(), json!(false));
        for (id, vt, d) in defaults(SPLITS[0].0) {
            s.insert(id, default_json(vt, d));
        }
        section.insert("split".into(), Value::Object(s));
    }
    if !section.contains_key("join") {
        let mut j = Map::new();
        j.insert("@model".into(), json!(JOIN_MODEL));
        j.insert("@position".into(), json!((last + 1).min(POSITIONS)));
        j.insert("@enabled".into(), json!(true));
        j.insert("@no_snapshot_bypass".into(), json!(false));
        for (id, vt, d) in defaults(JOIN_MODEL) {
            j.insert(id, default_json(vt, d));
        }
        section.insert("join".into(), Value::Object(j));
    }
    let key = format!("@topology{}", dsp_index(dsp));
    if let Some(g) = doc.pointer_mut("/data/tone/global").and_then(Value::as_object_mut) {
        g.insert(key, json!("SABJ"));
    }
}

/// Change the split type, keeping its position.
pub fn set_split_model(doc: &mut Value, dsp: &str, model: &str, params: &[(String, i64, f64)]) {
    if let Some(s) = doc.pointer_mut(&format!("/data/tone/{dsp}/split")).and_then(Value::as_object_mut) {
        let position = s.get("@position").cloned().unwrap_or(json!(0));
        s.retain(|k, _| k.starts_with('@'));
        s.insert("@model".into(), json!(model));
        s.insert("@position".into(), position);
        for (id, vt, d) in params {
            s.insert(id.clone(), default_json(*vt, *d));
        }
    }
}

/// Move the split or the merge to another position.
pub fn set_split_position(doc: &mut Value, dsp: &str, merge: bool, position: i64) {
    let name = if merge { "join" } else { "split" };
    if let Some(s) = doc.pointer_mut(&format!("/data/tone/{dsp}/{name}")).and_then(Value::as_object_mut) {
        s.insert("@position".into(), json!(position.clamp(0, POSITIONS)));
    }
}

/// The split or merge section, for its knobs.
pub fn split_mut<'a>(doc: &'a mut Value, dsp: &str, merge: bool) -> Option<&'a mut Map<String, Value>> {
    let name = if merge { "join" } else { "split" };
    doc.pointer_mut(&format!("/data/tone/{dsp}/{name}")).and_then(Value::as_object_mut)
}

/// Remove every split and merge (for units without parallel lanes).
pub fn clear_split(doc: &mut Value) {
    for dsp in dsps(doc) {
        if let Some(section) = doc.pointer_mut(&format!("/data/tone/{dsp}")).and_then(Value::as_object_mut) {
            section.remove("split");
            section.remove("join");
        }
        let key = format!("@topology{}", dsp_index(&dsp));
        if let Some(g) = doc.pointer_mut("/data/tone/global").and_then(Value::as_object_mut) {
            g.insert(key, json!("A"));
        }
    }
}

/// The input or output section of a DSP (`inputA`, `outputA`).
pub fn io_mut<'a>(doc: &'a mut Value, dsp: &str, input: bool) -> Option<&'a mut Map<String, Value>> {
    let name = if input { "inputA" } else { "outputA" };
    doc.pointer_mut(&format!("/data/tone/{dsp}/{name}")).and_then(Value::as_object_mut)
}

/// Give a preset as many paths as its unit has: an empty Path 2 (input and
/// output only) is added for Helix units when the file has only one.
/// Returns true when something was added.
pub fn ensure_paths(doc: &mut Value, paths: usize) -> bool {
    if paths < 2 || doc.pointer("/data/tone/dsp1").is_some() {
        return false;
    }
    let Some(tone) = doc.pointer_mut("/data/tone").and_then(Value::as_object_mut) else { return false };
    tone.insert(
        "dsp1".into(),
        json!({
            "inputA": { "@input": 0, "@model": "HD2_AppDSPFlow2Input", "noiseGate": false, "threshold": -48.0, "decay": 0.1 },
            "outputA": { "@model": "HD2_AppDSPFlowOutput", "@output": 1, "gain": 0.0, "pan": 0.5 }
        }),
    );
    if let Some(g) = tone.get_mut("global").and_then(Value::as_object_mut) {
        g.entry("@topology1").or_insert(json!("A"));
    }
    true
}

/// A new, empty preset for a unit: input and output on each of its paths,
/// and as many snapshots as the unit has.
pub fn blank_for(name: &str, device: i64) -> Value {
    let fam = crate::snapshots::family_of_device(device);
    let mut doc = blank(name);
    if let Some(d) = doc.pointer_mut("/data").and_then(Value::as_object_mut) {
        d.insert("device".into(), json!(device));
    }
    let tone = doc.pointer_mut("/data/tone").and_then(Value::as_object_mut).expect("blank has a tone");
    if fam.paths > 1 {
        tone.insert(
            "dsp1".into(),
            json!({
                "inputA": { "@input": 0, "@model": "HD2_AppDSPFlow2Input", "noiseGate": false, "threshold": -48.0, "decay": 0.1 },
                "outputA": { "@model": "HD2_AppDSPFlowOutput", "@output": 1, "gain": 0.0, "pan": 0.5 }
            }),
        );
        if let Some(g) = tone.get_mut("global").and_then(Value::as_object_mut) {
            g.insert("@topology1".into(), json!("A"));
        }
    }
    // A new preset starts with one snapshot; "+ Snapshot" adds more.
    for i in 0..fam.snapshot_count.max(3) {
        let valid = i == 0;
        tone.insert(format!("snapshot{i}"), json!({ "@name": format!("SNAPSHOT {}", i + 1), "@valid": valid }));
    }
    doc
}

/// A new, empty preset for the HX Stomp: input and output only.
pub fn blank(name: &str) -> Value {
    json!({
        "data": {
            "device": 2162694,
            "tone": {
                "dsp0": {
                    "inputA": { "@input": 1, "@model": "HD2_AppDSPFlow1Input", "noiseGate": false, "threshold": -48.0, "decay": 0.1 },
                    "outputA": { "@model": "HD2_AppDSPFlowOutput", "@output": 1, "gain": 0.0, "pan": 0.5 }
                },
                "global": { "@tempo": 120.0, "@topology0": "A", "@current_snapshot": 0 },
                "snapshot0": { "@name": "SNAPSHOT 1", "@valid": true },
                "snapshot1": { "@name": "SNAPSHOT 2", "@valid": true },
                "snapshot2": { "@name": "SNAPSHOT 3", "@valid": true }
            },
            "meta": { "name": name, "application": "ToneStrand" }
        },
        "meta": { "original": 0, "pbn": 0, "premium": 0 },
        "schema": "L6Preset",
        "version": 6
    })
}

/// A small made-up preset to start from or to show when nothing is open.
pub fn sample() -> Value {
    json!({
        "data": {
            "device": 2162694,
            "tone": {
                "dsp0": {
                    "inputA": { "@input": 1, "@model": "HD2_AppDSPFlow1Input", "noiseGate": false, "threshold": -48.0, "decay": 0.5 },
                    "outputA": { "@model": "HD2_AppDSPFlowOutput", "@output": 1, "gain": 0.0, "pan": 0.5 },
                    "split": { "@model": "HD2_AppDSPFlowSplitY", "@position": 3, "@enabled": true, "@no_snapshot_bypass": false, "BalanceA": 0.0, "BalanceB": 0.0, "bypass": false },
                    "join": { "@model": "HD2_AppDSPFlowJoin", "@position": 4, "@enabled": true, "@no_snapshot_bypass": false, "A Level": 0.0, "A Pan": 0.5, "B Level": 0.0, "B Pan": 0.5, "B Polarity": false, "Level": 0.0 },
                    "block0": { "@model": "HD2_CompressorDeluxeComp", "@enabled": true, "@path": 0, "@position": 0, "@stereo": false, "@type": 0, "@no_snapshot_bypass": false, "@bypassvolume": 1,
                                "Threshold": -22.0, "Ratio": 2, "Attack": 0.038, "Release": 0.2, "Mix": 1.0, "Level": 1.5, "Knee": 6.0 },
                    "block1": { "@model": "HD2_DistScream808", "@enabled": true, "@path": 0, "@position": 1, "@stereo": false, "@type": 0, "@no_snapshot_bypass": false, "@bypassvolume": 1,
                                "Gain": 0.52, "Tone": 0.65, "Level": 0.67 },
                    "block2": { "@model": "HD2_AmpUSDeluxeNrm", "@enabled": true, "@cab": "cab0", "@path": 0, "@position": 2, "@stereo": false, "@type": 1, "@no_snapshot_bypass": false, "@bypassvolume": 1,
                                "Drive": 0.45, "Bass": 0.5, "Mid": 0.6, "Treble": 0.55, "Presence": 0.0, "ChVol": 0.7, "Master": 1.0, "Sag": 0.5, "Hum": 0.5, "Ripple": 0.5, "Bias": 0.6, "BiasX": 0.5 },
                    "cab0": { "@model": "HD2_Cab1x12USDeluxe", "@enabled": true, "@mic": 6, "Distance": 2.0, "LowCut": 80.0, "HighCut": 8000.0, "EarlyReflections": 0.0, "Level": 0.0 },
                    "block3": { "@model": "HD2_ChorusPlastiChorus", "@enabled": true, "@path": 1, "@position": 3, "@stereo": false, "@type": 0, "@no_snapshot_bypass": false, "@bypassvolume": 1,
                                "Rate": 0.35, "Depth": 0.47, "Mode": false, "Tone": 0.75, "Mix": 0.35, "Level": 0.0, "Headroom": 0.0, "Spread": 1.0, "SyncSelect1": 6, "TempoSync1": false },
                    "block4": { "@model": "HD2_DelaySimpleDelay", "@enabled": true, "@path": 0, "@position": 4, "@stereo": false, "@type": 0, "@trails": true, "@no_snapshot_bypass": false, "@bypassvolume": 1,
                                "Time": 0.38, "Feedback": 0.3, "Mix": 0.25, "Level": 0.0, "SyncSelect1": 6, "TempoSync1": false },
                    "block5": { "@model": "HD2_ReverbPlate", "@enabled": true, "@path": 0, "@position": 5, "@stereo": false, "@type": 0, "@trails": false, "@no_snapshot_bypass": false, "@bypassvolume": 1,
                                "Decay": 0.5, "Predelay": 0.015, "LowCut": 90.0, "HighCut": 5600.0, "Mix": 0.3, "Level": 0.0 }
                },
                "global": { "@tempo": 120.0, "@topology0": "SABJ", "@current_snapshot": 0 },
                "snapshot0": { "@name": "CLEAN", "@valid": true },
                "snapshot1": { "@name": "CRUNCH", "@valid": true },
                "snapshot2": { "@name": "LEAD", "@valid": true }
            },
            "meta": { "name": "Sample Tone", "application": "ToneStrand" }
        },
        "meta": { "original": 0, "pbn": 0, "premium": 0 },
        "schema": "L6Preset",
        "version": 6
    })
}
