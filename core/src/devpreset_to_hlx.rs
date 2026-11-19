//! A pedal document turned back into a `.hlx` preset: the reverse of `DevicePreset::from_hlx`. With it a pedal preset
//! can be saved as `.hlx` and, through `units::convert`, made for another unit.
//!
//! Carried: blocks with their knob values and cabs, the split and merge, input and output, names, tempo, each
//! snapshot's block on/offs, footswitch assignments, knob controllers with their snapshot values, the Command Center
//! with its per-snapshot values, and the guitar input settings. What the document holds and the `.hlx` has no place
//! for, or a model the catalog has no `.hlx` name for, is reported and left out.

use serde_json::{json, Value as J};

use super::*;
use crate::catalog::Catalog;
use crate::hlx::{self, NewModel, ParamValue};

/// A pedal value as a number (a switch is 0 or 1).
fn number(v: &Value) -> Option<f64> {
    match v {
        Value::Bool(b) => Some(*b as u8 as f64),
        Value::Int(n, _) => Some(*n as f64),
        Value::F32(f) => Some(*f as f64),
        Value::F64(f) => Some(*f),
        _ => None,
    }
}

/// A pedal value as it is written in a `.hlx`.
fn to_json(v: &Value) -> J {
    match v {
        Value::Bool(b) => J::Bool(*b),
        Value::Int(n, _) => json!(n),
        Value::F32(f) => json!(*f as f64),
        Value::F64(f) => json!(f),
        _ => J::Null,
    }
}

/// Put a value at a path of objects, making the objects on the way.
fn set_path(doc: &mut J, path: &[&str], value: J) {
    let mut cur = doc;
    for (i, k) in path.iter().enumerate() {
        let Some(map) = cur.as_object_mut() else { return };
        if i + 1 == path.len() {
            map.insert((*k).to_owned(), value);
            return;
        }
        cur = map.entry((*k).to_owned()).or_insert_with(|| json!({}));
    }
}

/// The text of a pedal string (they end with a zero byte).
fn text(v: Option<&Value>) -> String {
    v.and_then(Value::as_str).unwrap_or("").trim_end_matches('\0').to_owned()
}

/// A footswitch LED colour in the `.hlx` numbering (the reverse of `led_colour`).
fn led_to_hlx(c: i64) -> i64 {
    const PAIRS: [(i64, i64); 10] = [
        (0xd0af00, 525824),
        (0xff0003, 786434),
        (3277055, 196619),
        (458496, 67840),
        (13055, 1037),
        (0xff2d00, 1049600),
        (8716032, 527360),
        (65408, 3334),
        (7077838, 462860),
        (0xff0028, 720903),
    ];
    PAIRS.iter().find(|(on, off)| c == *on || c == *off).map(|p| p.0).unwrap_or(c)
}

/// A block put in the `.hlx`: its slot's wire number, where it is (dsp section and key), its model, and the cab riding
/// with it (model and key).
struct Placed {
    wire: i64,
    dsp: String,
    key: String,
    model: i64,
    cab: Option<(i64, String)>,
}

/// One record of the footswitch table, read.
struct FsRecord {
    /// The switch, from 1.
    index: i64,
    /// 1 a block's on/off, 2 a knob, 3 a Command Center command.
    kind: i64,
    wire: i64,
    knob: Option<i64>,
    label: String,
    led: i64,
    enabled: bool,
    momentary: bool,
    primary: bool,
    custom_label: String,
    custom_colour: i64,
}

impl FsRecord {
    /// The `@fs_*` fields of a `.hlx` entry.
    fn fields(&self) -> serde_json::Map<String, J> {
        let mut m = serde_json::Map::new();
        m.insert("@fs_label".into(), json!(self.label));
        m.insert("@fs_ledcolor".into(), json!(led_to_hlx(self.led)));
        m.insert("@fs_enabled".into(), json!(self.enabled));
        m.insert("@fs_momentary".into(), json!(self.momentary));
        m.insert("@fs_primary".into(), json!(self.primary));
        if !self.custom_label.is_empty() {
            m.insert("@fs_customlabel".into(), json!(self.custom_label));
        }
        if self.custom_colour != 0 {
            m.insert("@fs_customcolor".into(), json!(self.custom_colour));
        }
        m
    }
}

impl DevicePreset {
    /// The unit a pedal document was made for, from its layout: its Command Center slot count and paths.
    pub fn guess_device(&self) -> i64 {
        match (self.command_count(), self.layout().len()) {
            (13, _) => 2162694,
            (16, _) => 2162699,
            (14, _) => 2162693,
            (24, _) => 2162689,
            (_, 1) => 2162694,
            _ => 2162689,
        }
    }

    fn footswitch_records(&self) -> Vec<FsRecord> {
        let Some(table) = self.tone.at(&[3, 8]).and_then(Value::as_array) else { return Vec::new() };
        let mut out = Vec::new();
        for (n, sw) in table.iter().enumerate() {
            let Some(recs) = sw.as_array() else { continue };
            for rec in recs {
                let Some(inner) = rec.get(11) else { continue };
                out.push(FsRecord {
                    index: n as i64 + 1,
                    kind: inner.get(0).and_then(Value::as_i64).unwrap_or(0),
                    wire: inner.get(8).and_then(Value::as_i64).unwrap_or(-1),
                    knob: inner.get(9).and_then(|k| k.get(29)).and_then(Value::as_i64),
                    label: text(inner.get(5)),
                    led: inner.get(6).and_then(Value::as_i64).unwrap_or(0),
                    enabled: matches!(inner.get(7), Some(Value::Bool(true))),
                    momentary: matches!(rec.get(12), Some(Value::Bool(true))),
                    primary: rec.get(10).and_then(Value::as_i64) == Some(0),
                    custom_label: text(rec.get(14)),
                    custom_colour: rec.get(16).and_then(Value::as_i64).unwrap_or(0),
                });
            }
        }
        out
    }

    /// Turn this pedal document into a `.hlx` preset for `device` (the unit it was made for: see `guess_device`),
    /// with a report of what was left out.
    pub fn to_hlx(&self, cat: &Catalog, device: i64, name: &str) -> Result<(J, Vec<String>), String> {
        let fam = crate::snapshots::family_of_device(device);
        let mut doc = hlx::blank_for(name, device);
        let mut notes: Vec<String> = Vec::new();
        let layout = self.layout();
        if layout.is_empty() {
            return Err("this document has no paths".into());
        }
        let slots = self.slots();
        let snaps = self.snapshot_names();
        // (wire, dsp, block key, model, cab model) of every block placed.
        let mut placed: Vec<Placed> = Vec::new();
        // Where each block goes (dsp, key, lane, position): kept so the split's fitting can not move them.
        let mut spots: Vec<(String, String, i64, i64)> = Vec::new();
        let defaults = |m: &str| cat.number_for_hlx(m, false).map(|n| cat.default_values(n)).unwrap_or_default();

        for (d, len) in layout.iter().copied().enumerate() {
            if len < 6 || (len - 4) % 2 != 0 {
                return Err(format!("path {} has {len} slots, a layout this app doesn't know", d + 1));
            }
            let columns = (len - 4) / 2;
            let (row_a, split_at, row_b, join_at, out_at) = (1, columns + 2, columns + 3, 2 * columns + 3, columns + 1);
            let dsp = format!("dsp{d}");
            let d = d as i64;
            if doc.pointer(&format!("/data/tone/{dsp}")).is_none() {
                notes.push(format!("Path {} has no place in a {} preset; its blocks were left out.", d + 1, fam.name));
                continue;
            }
            // Blocks.
            let mut lower_used = false;
            for s in slots.iter().filter(|s| s.path == d && s.kind == Kind::Block) {
                let Some(model) = s.model else { continue };
                let Some((symbol, stereo)) = cat.hlx_model(model) else {
                    notes.push(format!("{} has no .hlx name; left out.", cat.model_name(model)));
                    continue;
                };
                let params: Vec<ParamValue> = cat
                    .default_values(model)
                    .into_iter()
                    .enumerate()
                    .map(|(i, (id, vt, def))| (id, vt, s.values.get(i).and_then(number).unwrap_or(def)))
                    .collect();
                let cab = s.paired.and_then(|c| {
                    let (cs, _) = cat.hlx_model(c)?;
                    let paired = self.paired_values(s.path, s.position);
                    let ps: Vec<ParamValue> = cat
                        .default_values(c)
                        .into_iter()
                        .enumerate()
                        .map(|(i, (id, vt, def))| (id, vt, paired.get(i).and_then(number).unwrap_or(def)))
                        .collect();
                    Some((cs, ps))
                });
                let (lane, position) = if s.position >= row_b { (1, s.position - row_b) } else { (0, s.position - row_a) };
                lower_used |= lane == 1;
                let new = NewModel { model: &symbol, stereo, params: &params, cab: cab.as_ref().map(|(m, p)| (m.as_str(), p.as_slice())) };
                let Some(key) = hlx::place_model(&mut doc, &dsp, lane, position as i64, None, &new) else { continue };
                set_path(&mut doc, &["data", "tone", &dsp, &key, "@enabled"], json!(s.enabled.unwrap_or(true)));
                for i in 0..snaps.len() {
                    if let Some(on) = self.snapshot_enabled(i, s.wire_block() as usize) {
                        set_path(&mut doc, &["data", "tone", &format!("snapshot{i}"), "blocks", &dsp, &key], json!(on));
                    }
                }
                spots.push((dsp.clone(), key.clone(), lane, position as i64));
                let cab_key = doc.pointer(&format!("/data/tone/{dsp}/{key}/@cab")).and_then(J::as_str).map(str::to_owned);
                placed.push(Placed { wire: s.wire_block(), dsp: dsp.clone(), key, model, cab: s.paired.zip(cab_key) });
            }
            // The split and merge.
            if lower_used || self.split_active(d) {
                let at = self.attach(d, split_at).map(|a| a - 1).unwrap_or(0).max(0);
                hlx::add_split_at(&mut doc, &dsp, Some(at), &defaults);
                if let Some(j) = self.attach(d, join_at) {
                    hlx::set_split_position(&mut doc, &dsp, true, j - 1);
                }
                hlx::set_split_position(&mut doc, &dsp, false, at);
                if let Some(sp) = slots.iter().find(|s| s.path == d && s.kind == Kind::Split) {
                    if let Some((symbol, _)) = sp.model.and_then(|m| cat.hlx_model(m)) {
                        let params: Vec<ParamValue> = sp
                            .model
                            .map(|m| cat.default_values(m))
                            .unwrap_or_default()
                            .into_iter()
                            .enumerate()
                            .map(|(i, (id, vt, def))| (id, vt, sp.values.get(i).and_then(number).unwrap_or(def)))
                            .collect();
                        hlx::set_split_model(&mut doc, &dsp, &symbol, &params);
                    }
                }
            }
            // Input and output.
            for (input, at) in [(true, 0usize), (false, out_at)] {
                let Some(s) = slots.iter().find(|s| s.path == d && s.position == at) else { continue };
                if let Some(route) = self.io_route(d, at) {
                    set_path(
                        &mut doc,
                        &["data", "tone", &dsp, if input { "inputA" } else { "outputA" }, if input { "@input" } else { "@output" }],
                        json!(route),
                    );
                }
                let names: &[&str] = if input { &["noiseGate", "threshold", "decay"] } else { &["gain", "pan"] };
                for (i, n) in names.iter().enumerate() {
                    if let Some(v) = s.values.get(i) {
                        set_path(&mut doc, &["data", "tone", &dsp, if input { "inputA" } else { "outputA" }, n], to_json(v));
                    }
                }
            }
        }

        // The split's fitting of the lanes may have moved blocks: put each one back where the document had it.
        for (dsp, key, lane, position) in &spots {
            set_path(&mut doc, &["data", "tone", dsp, key, "@path"], json!(lane));
            set_path(&mut doc, &["data", "tone", dsp, key, "@position"], json!(position));
        }

        // Names, tempo, guitar input.
        for (i, n) in snaps.iter().enumerate() {
            set_path(&mut doc, &["data", "tone", &format!("snapshot{i}"), "@name"], json!(n));
            set_path(&mut doc, &["data", "tone", &format!("snapshot{i}"), "@valid"], json!(true));
        }
        set_path(&mut doc, &["data", "tone", "global", "@current_snapshot"], json!(self.current_snapshot()));
        if let Some(t) = self.tempo() {
            hlx::set_tempo(&mut doc, t);
        }
        for (name, field) in [("@guitarinputZ", key::GUITAR_IN_Z), ("@guitarpad", key::GUITAR_PAD)] {
            if let Some(v) = self.setting_int(field) {
                set_path(&mut doc, &["data", "tone", "global", name], json!(v));
            }
        }

        // Footswitches: a block's on/off, and the footswitch fields of a knob on a switch.
        let records = self.footswitch_records();
        for r in records.iter().filter(|r| r.kind == 1) {
            let Some(Placed { dsp, key, .. }) = placed.iter().find(|p| p.wire == r.wire) else { continue };
            let mut f = r.fields();
            f.insert("@fs_index".into(), json!(r.index));
            set_path(&mut doc, &["data", "tone", "footswitch", dsp, key], J::Object(f));
        }

        // Controllers: knobs on an expression pedal, a footswitch, the snapshots or MIDI.
        for c in self.controllers() {
            if c.kind != 0 {
                notes.push("A controller on a block's on/off or a global setting isn't carried over; left out.".into());
                continue;
            }
            // A knob of a block, of the cab riding with it, or of the input or output.
            let (dsp, key, param) = if let Some(Placed { dsp, key, model, cab, .. }) = placed.iter().find(|p| p.wire == c.wire) {
                let trails = c.raw.at(&[1, 6, 41]).is_some_and(|v| matches!(v, Value::Bool(true)));
                let (source, key) = match (c.cab, cab) {
                    (true, Some((cab_model, cab_key))) => (*cab_model, cab_key.clone()),
                    _ => (*model, key.clone()),
                };
                let param = if trails {
                    "@trails".to_owned()
                } else {
                    match cat.default_values(source).get(c.index) {
                        Some((id, _, _)) => id.clone(),
                        None => continue,
                    }
                };
                (dsp.clone(), key, param)
            } else {
                let (d, at) = (c.wire / 20, c.wire % 20);
                let is_out = layout.get(d as usize).is_some_and(|len| at as usize == (len - 4) / 2 + 1);
                let names: &[&str] = if at == 0 {
                    &["noiseGate", "threshold", "decay"]
                } else if is_out {
                    &["gain", "pan"]
                } else {
                    &[]
                };
                match names.get(c.index) {
                    Some(p) => (format!("dsp{d}"), (if at == 0 { "inputA" } else { "outputA" }).to_owned(), (*p).to_owned()),
                    None => {
                        notes.push("A controller on a split or merge isn't carried over; left out.".into());
                        continue;
                    }
                }
            };
            let (dsp, key) = (&dsp, &key);
            let mut cfg = serde_json::Map::new();
            cfg.insert("@controller".into(), json!(c.controller));
            cfg.insert("@min".into(), to_json(&c.min));
            cfg.insert("@max".into(), to_json(&c.max));
            cfg.insert("@cc".into(), json!(c.cc));
            cfg.insert("@snapshot_disable".into(), json!(c.snapshot_disable));
            if let Some(r) = records.iter().find(|r| r.kind == 2 && r.wire == c.wire && r.knob == Some(c.index as i64)) {
                cfg.extend(r.fields());
            }
            set_path(&mut doc, &["data", "tone", "controller", dsp, key, &param], J::Object(cfg));
            for (i, s) in c.snaps.iter().enumerate().take(snaps.len()) {
                let Some((on, v)) = s else { continue };
                if matches!(v, Value::Nil) {
                    continue;
                }
                set_path(
                    &mut doc,
                    &["data", "tone", &format!("snapshot{i}"), "controllers", dsp, key, &param],
                    json!({ "@fs_enabled": on, "@value": to_json(v) }),
                );
            }
        }

        // Command Center.
        let cc_slots = crate::cmdslots::device_slots(fam);
        if self.command_count() == cc_slots.len() {
            for (i, slot) in cc_slots.iter().enumerate() {
                let Some((id, args)) = self.command(i) else { continue };
                let keys = crate::cmdslots::hlx_arg_keys(slot.kind, id);
                let mut obj = serde_json::Map::new();
                obj.insert("@command".into(), json!(id));
                for (k, a) in keys.iter().zip(&args) {
                    obj.insert((*k).to_owned(), json!(a));
                }
                if i >= 6 && i < 6 + self.command_fs_count() {
                    if let Some(r) = records.iter().find(|r| r.kind == 3 && r.index == i as i64 - 5) {
                        obj.extend(r.fields());
                    }
                }
                set_path(&mut doc, &["data", "tone", &slot.key], J::Object(obj));
                if id != 14 {
                    for n in 0..snaps.len() {
                        let Some(own) = self.snapshot_command_args(n, i) else { continue };
                        let mut o = serde_json::Map::new();
                        for (k, a) in keys.iter().zip(&own) {
                            o.insert((*k).to_owned(), json!(a));
                        }
                        o.insert("@fs_enabled".into(), json!(false));
                        set_path(&mut doc, &["data", "tone", &format!("snapshot{n}"), "commands", &slot.key], J::Object(o));
                    }
                }
            }
        } else if self.command_count() > 0 {
            notes.push("The Command Center isn't carried over: its slots don't match this unit's.".into());
        }
        notes.dedup();
        Ok((doc, notes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real presets, made for another unit through the `.hlx`: the larger Stomp XL takes all of them, and a Helix
    /// takes them with the usual adjustments (a report, never a failure for a preset that fits).
    #[test]
    fn real_presets_can_be_made_for_other_units() {
        let (Ok(dir), Some(cat_dir)) = (std::env::var("HX_PRESETS_DIR"), crate::catalog::find_dir(None)) else { return };
        let cat = Catalog::load(&cat_dir).unwrap();
        let mut refused = [0usize; 2];
        let mut total = 0;
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) != Some("hxpreset") {
                continue;
            }
            let original = DevicePreset::parse(&std::fs::read(&path).unwrap()).unwrap();
            let (json, _) = original.to_hlx(&cat, 2162694, "t").unwrap();
            total += 1;
            for (i, unit) in [2162699, 2162689].into_iter().enumerate() {
                let mut copy = json.clone();
                if crate::units::convert(&mut copy, unit, Some(&cat)).is_err() {
                    refused[i] += 1;
                }
            }
        }
        println!("{total} presets: {} refused by the Stomp XL, {} by the Helix Floor", refused[0], refused[1]);
        assert_eq!(refused[0], 0, "the Stomp XL has room for everything a Stomp holds");
    }

    /// Every real preset, turned into a `.hlx` and back, keeps what the editor shows: blocks, values, cabs, on/off
    /// states, names, snapshots' on/offs, footswitches, controllers, the Command Center. `HX_PRESETS_DIR` is a
    /// folder of pedal documents from an HX Stomp.
    #[test]
    fn real_presets_survive_a_trip_through_hlx() {
        let (Ok(dir), Some(cat_dir)) = (std::env::var("HX_PRESETS_DIR"), crate::catalog::find_dir(None)) else { return };
        let cat = Catalog::load(&cat_dir).unwrap();
        let mut checked = 0;
        let mut problems: Vec<String> = Vec::new();
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) != Some("hxpreset") {
                continue;
            }
            let original = DevicePreset::parse(&std::fs::read(&path).unwrap()).unwrap();
            let file = path.file_name().unwrap().to_string_lossy().into_owned();
            let (json, notes) = match original.to_hlx(&cat, 2162694, "t") {
                Ok(r) => r,
                Err(e) => {
                    problems.push(format!("{file}: to_hlx: {e}"));
                    continue;
                }
            };
            let (back, _) = match DevicePreset::from_hlx(&original, &json, &cat) {
                Ok(r) => r,
                Err(e) => {
                    problems.push(format!("{file}: from_hlx: {e}"));
                    continue;
                }
            };
            checked += 1;
            let close = |a: &Value, b: &Value| match (number(a), number(b)) {
                (Some(x), Some(y)) => (x - y).abs() < 1e-4,
                _ => a == b || (matches!(a, Value::Nil) && matches!(b, Value::Nil)),
            };
            let (sa, sb) = (original.slots(), back.slots());
            for (a, b) in sa.iter().zip(&sb) {
                let tag = format!("{file}: slot {}", a.global);
                if (a.kind, a.model, a.paired, a.enabled) != (b.kind, b.model, b.paired, b.enabled) {
                    problems.push(format!(
                        "{tag}: {:?} {:?} {:?} {:?} became {:?} {:?} {:?} {:?}",
                        a.kind, a.model, a.paired, a.enabled, b.kind, b.model, b.paired, b.enabled
                    ));
                    continue;
                }
                if a.kind == Kind::Block && !a.values.iter().zip(&b.values).all(|(x, y)| close(x, y)) {
                    problems.push(format!("{tag}: values changed"));
                }
                if a.kind == Kind::Block {
                    for n in 0..original.snapshot_names().len() {
                        if original.snapshot_enabled(n, a.wire_block() as usize) != back.snapshot_enabled(n, a.wire_block() as usize) {
                            problems.push(format!("{tag}: snapshot {n} on/off changed"));
                        }
                    }
                    if original.block_footswitch(a.wire_block()) != back.block_footswitch(a.wire_block()) {
                        problems.push(format!("{tag}: footswitch changed"));
                    }
                }
            }
            if original.snapshot_names() != back.snapshot_names() {
                problems.push(format!("{file}: snapshot names changed"));
            }
            // Only knobs of a block count: the other direction leaves controllers on a cab, the input and output and global
            // settings out (reported), so a trip back has nothing to compare them with.
            let block_wires: Vec<i64> = sa.iter().filter(|s| s.kind == Kind::Block).map(|s| s.wire_block()).collect();
            let (ca, cb) = (
                original.controllers().into_iter().filter(|c| c.kind == 0 && !c.cab && block_wires.contains(&c.wire)).collect::<Vec<_>>(),
                back.controllers(),
            );
            if ca.len() != cb.len() {
                problems.push(format!(
                    "{file}: {} controllers became {}: {:?} notes {:?}",
                    ca.len(),
                    cb.len(),
                    ca.iter().map(|c| (c.kind, c.wire, c.index, c.cab, c.controller)).collect::<Vec<_>>(),
                    notes
                ));
            } else {
                for (a, b) in ca.iter().zip(&cb) {
                    if (a.controller, a.wire, a.index, a.cc) != (b.controller, b.wire, b.index, b.cc)
                        || !close(&a.min, &b.min)
                        || !close(&a.max, &b.max)
                        || a.snaps.len() != b.snaps.len()
                        || !a.snaps.iter().zip(&b.snaps).all(|(x, y)| match (x, y) {
                            (Some((o1, v1)), Some((o2, v2))) => o1 == o2 && close(v1, v2),
                            (None, None) => true,
                            (Some((_, Value::Nil)), None) | (None, Some((_, Value::Nil))) => true,
                            _ => false,
                        })
                    {
                        problems.push(format!("{file}: controller on wire {} changed", a.wire));
                    }
                }
            }
            for i in 0..original.command_count() {
                if original.command(i).map(|(id, a)| (id, a.into_iter().take(5).collect::<Vec<_>>()))
                    != back.command(i).map(|(id, a)| (id, a.into_iter().take(5).collect::<Vec<_>>()))
                {
                    problems.push(format!("{file}: command slot {i} changed"));
                }
            }
        }
        assert!(checked > 0, "no presets read");
        assert!(
            problems.is_empty(),
            "{} problems in {checked} presets, first ones:\n{}",
            problems.len(),
            problems.iter().take(15).cloned().collect::<Vec<_>>().join("\n")
        );
    }
}
