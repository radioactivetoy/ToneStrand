//! The preset document the pedal stores and sends: the text `l6-helix`, a
//! table of byte offsets, then the tone as one MessagePack map.

use crate::catalog::Catalog;
use crate::msgpack::{Decoder, Value};

pub const MAGIC: &str = "l6-helix";

/// Keys inside the tone.
pub mod key {
    pub const PATH0: i64 = 0;
    pub const SLOTS: i64 = 22;
    pub const KIND: i64 = 19;
    pub const BODY: i64 = 20;
    pub const MODEL_REF: i64 = 24;
    pub const MODEL: i64 = 25;
    pub const PAIRED_MODEL: i64 = 26;
    pub const PAIRED_VALUES: i64 = 12;
    pub const INLINE_MODEL: i64 = 8;
    pub const ENABLED: i64 = 10;
    pub const VALUES: i64 = 11;
    pub const IO_VALUES: i64 = 7;
    pub const SPLIT_BODY: i64 = 15;
    pub const JOIN_BODY: i64 = 17;
    pub const ARRAY_VALUES: i64 = 4;
    pub const SETTINGS: i64 = 5;
    pub const TEMPO: i64 = 16;
    pub const SNAPSHOT_SECTION: i64 = 10;
    pub const SNAPSHOTS: i64 = 10;
    pub const SNAPSHOT_NAME: i64 = 4;
    /// Command Center: `{0: [command id per slot], 1: [[id, arg1..arg6] per slot]}`.
    pub const COMMANDS: i64 = 2;
    /// Preset settings (`tone[5]`): guitar input impedance (0 Auto, 1 10k, 2 22k, 3 32k, 4 70k, 5 90k, 6 136k,
    /// 7 230k, 8 1M) and guitar pad (0 Global, 1 Off, 2 On).
    pub const GUITAR_IN_Z: i64 = 30;
    pub const GUITAR_PAD: i64 = 134;
}

/// Which top-level tone keys the offset table points at, in table order.
const TABLE_ORDER: [i64; 9] = [0, 1, 3, 4, 2, 5, 6, 7, 10];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Input,
    Output,
    Split,
    Join,
    Block,
    Empty,
    Other(i64),
}

impl Kind {
    pub fn from_wire(n: i64) -> Kind {
        match n {
            0 => Kind::Input,
            1 => Kind::Output,
            2 => Kind::Split,
            3 => Kind::Join,
            6 | 7 => Kind::Block,
            8 => Kind::Empty,
            n => Kind::Other(n),
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Kind::Input => "Input",
            Kind::Output => "Output",
            Kind::Split => "Split",
            Kind::Join => "Merge",
            Kind::Block => "Block",
            Kind::Empty => "Empty",
            Kind::Other(_) => "Other",
        }
    }
}

/// Where one slot lives and what it holds, for drawing and editing.
#[derive(Clone, Debug)]
pub struct SlotView {
    /// Index in `DevicePreset::slots()`.
    pub global: usize,
    /// Path key in the tone (0 or 1) and position within that path's slots.
    pub path: i64,
    pub position: usize,
    pub kind: Kind,
    pub model: Option<i64>,
    pub paired: Option<i64>,
    pub enabled: Option<bool>,
    pub values: Vec<Value>,
}

impl SlotView {
    /// The slot address the pedal's wire protocol uses: `20 * path + position`.
    pub fn wire_block(&self) -> i64 {
        20 * self.path + self.position as i64
    }
}

/// One slot's Command Center as `command_state` reports it: command, Momentary flag, values per snapshot.
pub type CommandState = (Option<(i64, Vec<i64>)>, Option<bool>, Vec<Option<Vec<i64>>>);

#[derive(Clone)]
pub struct DevicePreset {
    /// The mark as it arrived (the pedal writes it with a trailing zero byte).
    magic: Value,
    table_fmt: u8,
    table: Vec<u8>,
    pub tone: Value,
    /// Anything after the tone (none seen so far). Kept and written back so a document
    /// read and saved unchanged stays byte for byte the same.
    rest: Vec<Value>,
}

impl DevicePreset {
    pub fn parse(blob: &[u8]) -> Result<DevicePreset, String> {
        let values = Decoder::decode_all(blob).map_err(|e| format!("not a preset document: {e}"))?;
        let mut it = values.into_iter();
        let magic = it.next().unwrap_or(Value::Nil);
        if magic.as_str().map(|s| s.trim_end_matches('\0')) != Some(MAGIC) {
            return Err("not a preset document (no l6-helix mark)".into());
        }
        let (table, table_fmt) = match it.next() {
            Some(Value::Bin(b, f)) => (b, f),
            Some(Value::Str(s, f)) => (s.into_bytes(), f),
            _ => return Err("the preset's offset table is missing".into()),
        };
        let tone = it.next().ok_or("the preset has no tone")?;
        let rest: Vec<Value> = it.collect();
        Ok(DevicePreset { magic, table_fmt, table, tone, rest })
    }

    /// Serialise, rebuilding the offset table so it matches the new lengths. When the table
    /// can't be rebuilt the stored one is kept (right for a document saved unchanged); use
    /// [`DevicePreset::encode_checked`] for anything that goes to the pedal.
    pub fn encode(&self) -> Vec<u8> {
        let table = self.computed_table().unwrap_or_else(|| self.table.clone());
        self.encode_with(table)
    }

    /// Serialise for the pedal: refuses rather than send an offset table that may no
    /// longer match the document.
    pub fn encode_checked(&self) -> Result<Vec<u8>, String> {
        if !self.rest.is_empty() {
            return Err(format!("the document has {} unknown part(s) after the tone; it was not sent", self.rest.len()));
        }
        let table = self.computed_table().ok_or("the document's offset table can't be rebuilt (a section is missing); it was not sent")?;
        Ok(self.encode_with(table))
    }

    /// The same document (as `encode()` would tell, without encoding anything).
    pub fn same_as(&self, other: &DevicePreset) -> bool {
        self.magic.same_as(&other.magic)
            && self.table_fmt == other.table_fmt
            && self.table == other.table
            && self.tone.same_as(&other.tone)
            && self.rest.len() == other.rest.len()
            && self.rest.iter().zip(&other.rest).all(|(a, b)| a.same_as(b))
    }

    fn encode_with(&self, table: Vec<u8>) -> Vec<u8> {
        let mut out = self.magic.encode();
        out.extend(Value::Bin(table, self.table_fmt).encode());
        out.extend(self.tone.encode());
        for v in &self.rest {
            out.extend(v.encode());
        }
        out
    }

    fn computed_table(&self) -> Option<Vec<u8>> {
        // Where trailing parts would sit in the table is unknown: don't guess.
        if !self.rest.is_empty() {
            return None;
        }
        let magic_len = self.magic.encode().len();
        let table_len = (1 + TABLE_ORDER.len() + 2) * 4;
        let table_hdr = Value::Bin(vec![0; table_len], self.table_fmt).encode().len() - table_len;
        let tone_at = magic_len + table_hdr + table_len;
        let tone = self.tone.encode();
        let total = tone_at + tone.len();

        let Value::Map(pairs, _) = &self.tone else { return None };
        // Where each top-level key starts: walk the encoded map entry by entry.
        let header = tone.len() - {
            let mut body = Vec::new();
            for (k, v) in pairs {
                body.extend(k.encode());
                body.extend(v.encode());
            }
            body.len()
        };
        let mut at = header;
        let mut key_at = std::collections::HashMap::new();
        for (k, v) in pairs {
            if let Some(n) = k.as_i64() {
                key_at.insert(n, tone_at + at);
            }
            at += k.encode().len() + v.encode().len();
        }
        let mut out = Vec::with_capacity(table_len);
        out.extend_from_slice(&(tone_at as u32).to_le_bytes());
        for k in TABLE_ORDER {
            out.extend_from_slice(&(*key_at.get(&k)? as u32).to_le_bytes());
        }
        out.extend_from_slice(&(total as u32).to_le_bytes());
        out.extend_from_slice(&(total as u32).to_le_bytes());
        Some(out)
    }

    pub fn tempo(&self) -> Option<f64> {
        self.tone.at(&[key::SETTINGS, key::TEMPO])?.as_f64()
    }
    pub fn set_tempo(&mut self, bpm: f64) {
        let bpm = bpm.clamp(20.0, 250.0);
        if let Some(v) = self.tone.at_mut(&[key::SETTINGS, key::TEMPO]) {
            *v = match v {
                Value::Int(_, f) => Value::Int(bpm.round() as i64, *f),
                _ => Value::F32(bpm as f32),
            };
        }
        let cur = self.tone.at(&[key::SNAPSHOT_SECTION, 6]).and_then(Value::as_i64).unwrap_or(0).max(0) as usize;
        if let Some(Value::Array(snaps, _)) = self.tone.at_mut(&[key::SNAPSHOT_SECTION, key::SNAPSHOTS]) {
            if let Some(v) = snaps.get_mut(cur).and_then(|s| s.get_mut(5)) {
                *v = Value::F32(bpm as f32);
            }
        }
    }

    /// The snapshot the preset is on (from 0).
    pub fn current_snapshot(&self) -> usize {
        self.tone.at(&[key::SNAPSHOT_SECTION, 6]).and_then(Value::as_i64).unwrap_or(0).max(0) as usize
    }

    /// Snapshot names, in order.
    pub fn snapshot_names(&self) -> Vec<String> {
        self.tone
            .at(&[key::SNAPSHOT_SECTION, key::SNAPSHOTS])
            .and_then(Value::as_array)
            .map(|a| a.iter().map(|s| crate::session::clean(s.get(key::SNAPSHOT_NAME).and_then(Value::as_str).unwrap_or(""))).collect())
            .unwrap_or_default()
    }
    pub fn set_snapshot_name(&mut self, index: usize, name: &str) {
        let clean: String = name.chars().filter(|c| (' '..='~').contains(c)).take(10).collect();
        if let Some(Value::Array(a, _)) = self.tone.at_mut(&[key::SNAPSHOT_SECTION, key::SNAPSHOTS]) {
            if let Some(snap) = a.get_mut(index) {
                if let Some(Value::Str(s, _)) = snap.get_mut(key::SNAPSHOT_NAME) {
                    *s = format!("{clean}\0");
                }
                if let Some(Value::Bool(custom)) = snap.get_mut(14) {
                    *custom = true;
                }
            }
        }
    }

    /// Every slot of every path.
    pub fn slots(&self) -> Vec<SlotView> {
        let mut out = Vec::new();
        let mut path = key::PATH0;
        while let Some(Value::Array(items, _)) = self.tone.at(&[path, key::SLOTS]) {
            for (position, raw) in items.iter().enumerate() {
                let mut s = read_slot(raw, path, position);
                s.global = out.len();
                out.push(s);
            }
            path += 1;
            if path > 3 {
                break;
            }
        }
        out
    }

    fn body_mut(&mut self, path: i64, position: usize) -> Option<&mut Value> {
        let Value::Array(items, _) = self.tone.at_mut(&[path, key::SLOTS])? else { return None };
        let raw = items.get_mut(position)?;
        let kind = Kind::from_wire(raw.get(key::KIND).and_then(Value::as_i64).unwrap_or(-1));
        let body = raw.get_mut(key::BODY)?;
        let nested = match kind {
            Kind::Split => Some(key::SPLIT_BODY),
            Kind::Join => Some(key::JOIN_BODY),
            _ => None,
        };
        match nested {
            Some(k) if body.get(k).is_some() => body.get_mut(k),
            _ => Some(body),
        }
    }

    fn slot_mut(&mut self, path: i64, position: usize) -> Option<&mut Value> {
        let Value::Array(items, _) = self.tone.at_mut(&[path, key::SLOTS])? else { return None };
        items.get_mut(position)
    }

    /// Put a model (and optionally its cab) into a slot, with default knob values.
    pub fn place_model(&mut self, cat: &Catalog, path: i64, position: usize, model: i64, cab: Option<i64>) {
        let slot_val = block_slot(cat, model, cab, true, true);
        if let Some(slot) = self.slot_mut(path, position) {
            *slot = slot_val;
        }
        self.set_snapshot_bypass_all(path, position, true);
        self.sync_split_active(path);
    }

    /// Change the second model of a paired block (an amp's cab, or a dual cab's second cab),
    /// keeping the first model and its knobs; the new cab starts at its default knobs.
    pub fn set_paired_model(&mut self, cat: &Catalog, path: i64, position: usize, cab: i64) {
        let Some(body) = self.body_mut(path, position) else { return };
        let first = body.get(key::MODEL_REF).and_then(|r| r.get(key::MODEL)).and_then(Value::as_i64);
        let Some(first) = first else { return };
        if let Some(r) = body.get_mut(key::MODEL_REF) {
            if let Some(v) = r.get_mut(23) {
                *v = Value::Bool(true);
            }
            if let Some(v) = r.get_mut(key::PAIRED_MODEL) {
                *v = Value::int(cab);
            }
        }
        if let Some(v) = body.get_mut(9) {
            *v = Value::int(cat.paired_kind(first, cab));
        }
        if let Some(v) = body.get_mut(key::PAIRED_VALUES) {
            *v = values_container(cat, cab);
        }
    }

    /// Where an input takes its sound from (key 5) or an output sends it (key 6).
    pub fn io_route(&self, path: i64, position: usize) -> Option<i64> {
        let Value::Array(items, _) = self.tone.at(&[path, key::SLOTS])? else { return None };
        let raw = items.get(position)?;
        let body = raw.get(key::BODY)?;
        body.get(5).or_else(|| body.get(6)).and_then(Value::as_i64)
    }

    pub fn set_io_route(&mut self, path: i64, position: usize, value: i64) {
        if let Some(body) = self.body_mut(path, position) {
            for k in [5, 6] {
                if let Some(Value::Int(v, _)) = body.get_mut(k) {
                    *v = value;
                    return;
                }
            }
        }
    }

    /// Where a split or merge slot attaches: the main-lane slot it sits before (key 13).
    pub fn attach(&self, path: i64, position: usize) -> Option<i64> {
        let Value::Array(items, _) = self.tone.at(&[path, key::SLOTS])? else { return None };
        let raw = items.get(position)?;
        let kind = Kind::from_wire(raw.get(key::KIND).and_then(Value::as_i64).unwrap_or(-1));
        let body = raw.get(key::BODY)?;
        let inner = match kind {
            Kind::Split => body.get(key::SPLIT_BODY).unwrap_or(body),
            Kind::Join => body.get(key::JOIN_BODY).unwrap_or(body),
            _ => return None,
        };
        inner.get(13).and_then(Value::as_i64)
    }

    pub fn set_attach(&mut self, path: i64, position: usize, value: i64) {
        // The same place attach() reads: a split's or merge's own record inside the body.
        let kind = {
            let Some(Value::Array(items, _)) = self.tone.at(&[path, key::SLOTS]) else { return };
            items.get(position).map(|raw| Kind::from_wire(raw.get(key::KIND).and_then(Value::as_i64).unwrap_or(-1)))
        };
        let inner_key = match kind {
            Some(Kind::Split) => Some(key::SPLIT_BODY),
            Some(Kind::Join) => Some(key::JOIN_BODY),
            _ => None,
        };
        if let Some(body) = self.body_mut(path, position) {
            let has_inner = inner_key.is_some_and(|k| body.get(k).is_some());
            let target = if has_inner { inner_key.and_then(|k| body.get_mut(k)) } else { Some(body) };
            if let Some(Value::Int(v, _)) = target.and_then(|b| b.get_mut(13)) {
                *v = value;
            }
        }
        if let Some(Value::Bool(active)) = self.slot_mut(path, position).and_then(|s| s.get_mut(key::BODY)).and_then(|b| b.get_mut(18)) {
            *active = true;
        }
    }

    /// Empty a slot.
    pub fn clear_slot(&mut self, path: i64, position: usize) {
        if let Some(slot) = self.slot_mut(path, position) {
            *slot = Value::map(vec![(key::KIND, Value::int(8)), (key::BODY, Value::Nil)]);
        }
        self.set_snapshot_bypass_all(path, position, true);
        self.sync_split_active(path);
    }

    /// Exchange two slots of a path (moving a block along the chain).
    pub fn swap_slots(&mut self, path: i64, a: usize, b: usize) {
        if let Some(Value::Array(items, _)) = self.tone.at_mut(&[path, key::SLOTS]) {
            if a < items.len() && b < items.len() {
                items.swap(a, b);
            }
        }
        let wa = (20 * path + a as i64).max(0) as usize;
        let wb = (20 * path + b as i64).max(0) as usize;
        if let Some(Value::Array(snaps, _)) = self.tone.at_mut(&[key::SNAPSHOT_SECTION, key::SNAPSHOTS]) {
            for snap in snaps {
                if let Some(Value::Array(table, _)) = snap.get_mut(3) {
                    if wa < table.len() && wb < table.len() {
                        table.swap(wa, wb);
                    }
                }
            }
        }
        self.sync_split_active(path);
    }

    pub fn set_enabled(&mut self, path: i64, position: usize, on: bool) {
        if let Some(Value::Bool(b)) = self.body_mut(path, position).and_then(|b| b.get_mut(key::ENABLED)) {
            *b = on;
        }
        let cur = self.tone.at(&[key::SNAPSHOT_SECTION, 6]).and_then(Value::as_i64).unwrap_or(0).max(0) as usize;
        let wire = (20 * path + position as i64).max(0) as usize;
        if let Some(Value::Array(snaps, _)) = self.tone.at_mut(&[key::SNAPSHOT_SECTION, key::SNAPSHOTS]) {
            if let Some(Value::Array(entry, _)) =
                snaps.get_mut(cur).and_then(|s| s.get_mut(3)).and_then(Value::as_array_mut).and_then(|t| t.get_mut(wire))
            {
                if let Some(Value::Bool(b)) = entry.get_mut(1) {
                    *b = on;
                }
            }
        }
    }

    /// A block's on/off in one snapshot (snapshot entry 3, indexed by the slot's wire number).
    pub fn set_snapshot_enabled(&mut self, snapshot: usize, wire: usize, on: bool) {
        if let Some(Value::Array(snaps, _)) = self.tone.at_mut(&[key::SNAPSHOT_SECTION, key::SNAPSHOTS]) {
            if let Some(Value::Array(entry, _)) =
                snaps.get_mut(snapshot).and_then(|s| s.get_mut(3)).and_then(Value::as_array_mut).and_then(|t| t.get_mut(wire))
            {
                if let Some(Value::Bool(b)) = entry.get_mut(1) {
                    *b = on;
                }
            }
        }
    }

    /// Make snapshot `to` hold the same knob values and on/offs as snapshot `from` (its name stays).
    pub fn copy_snapshot_states(&mut self, from: usize, to: usize) -> bool {
        let Some(Value::Array(snaps, _)) = self.tone.at_mut(&[key::SNAPSHOT_SECTION, key::SNAPSHOTS]) else { return false };
        if from == to || from >= snaps.len() || to >= snaps.len() {
            return false;
        }
        let Value::Array(src_items, _) = &snaps[from] else { return false };
        let src: Vec<(usize, Value)> = [2usize, 3].into_iter().filter_map(|f| src_items.get(f).cloned().map(|v| (f, v))).collect();
        let Value::Array(dst, _) = &mut snaps[to] else { return false };
        for (f, v) in src {
            if f < dst.len() {
                dst[f] = v;
            }
        }
        true
    }

    /// A block's on/off in one snapshot, if stored.
    pub fn snapshot_enabled(&self, snapshot: usize, wire: usize) -> Option<bool> {
        let snaps = self.tone.at(&[key::SNAPSHOT_SECTION, key::SNAPSHOTS])?.as_array()?;
        let entry = snaps.get(snapshot)?.get(3)?.as_array()?.get(wire)?.as_array()?;
        match entry.get(1)? {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    fn set_snapshot_bypass_all(&mut self, path: i64, position: usize, on: bool) {
        let wire = (20 * path + position as i64).max(0) as usize;
        if let Some(Value::Array(snaps, _)) = self.tone.at_mut(&[key::SNAPSHOT_SECTION, key::SNAPSHOTS]) {
            for snap in snaps {
                if let Some(Value::Array(entry, _)) = snap.get_mut(3).and_then(Value::as_array_mut).and_then(|t| t.get_mut(wire)) {
                    if let Some(Value::Bool(b)) = entry.get_mut(1) {
                        *b = on;
                    }
                }
            }
        }
    }

    /// How many slots each path has: the shape that tells which unit a document is for.
    pub fn layout(&self) -> Vec<usize> {
        (0..8).map_while(|d| self.tone.at(&[key::PATH0 + d, key::SLOTS]).and_then(Value::as_array).map(|a| a.len())).collect()
    }

    /// A whole-number setting of the preset (`tone[5]`), if the document has it.
    pub fn setting_int(&self, field: i64) -> Option<i64> {
        self.tone.at(&[key::SETTINGS, field])?.as_i64()
    }

    /// Change a whole-number setting the document already has. False when it doesn't have it.
    pub fn set_setting_int(&mut self, field: i64, value: i64) -> bool {
        match self.tone.at_mut(&[key::SETTINGS, field]) {
            Some(v @ Value::Int(..)) => {
                *v = Value::int(value);
                true
            }
            _ => false,
        }
    }

    // ------------------------------------------------------------ Command Center ---

    /// How many Command Center slots the document has (13 on an HX Stomp: Instant 1-6, FS 1-5,
    /// EXP 1, EXP 2), or 0 when it has none.
    pub fn command_count(&self) -> usize {
        self.tone.at(&[key::COMMANDS, 0]).and_then(Value::as_array).map_or(0, |a| a.len())
    }

    /// The command in a slot as (command id, its six arguments), or None when the slot is empty.
    /// The ids are the ones `.hlx` files use (14 HX Preset, 15 HX Snapshot, 18 MIDI CC ...).
    pub fn command(&self, slot: usize) -> Option<(i64, Vec<i64>)> {
        let row = self.tone.at(&[key::COMMANDS, 1])?.as_array()?.get(slot)?.as_array()?;
        // The type is in the list of ids; a row only repeats it for HX Preset (the pedal's own output
        // starts every other row with 0).
        let listed = self.tone.at(&[key::COMMANDS, 0]).and_then(Value::as_array).and_then(|a| a.get(slot)).and_then(Value::as_i64);
        let id = match listed {
            Some(id) if id > 0 => id,
            _ => row.first()?.as_i64()?,
        };
        if id <= 0 {
            return None;
        }
        Some((id, row.iter().skip(1).map(|v| v.as_i64().unwrap_or(0)).collect()))
    }

    /// The per-snapshot command records of one snapshot as (slot or padding number, flag, seven values).
    fn snapshot_command_rows(snap: &Value) -> Vec<(i64, bool, Vec<i64>)> {
        let Some(rows) = snap.get(1).and_then(Value::as_array) else { return Vec::new() };
        rows.iter()
            .map(|r| {
                let a = r.as_array().map(Vec::as_slice).unwrap_or(&[]);
                let head = a.first().and_then(Value::as_i64).unwrap_or(0);
                let on = matches!(a.get(1), Some(Value::Bool(true)));
                let vals =
                    a.get(2).and_then(Value::as_array).map(|v| v.iter().map(|x| x.as_i64().unwrap_or(0)).collect()).unwrap_or_default();
                (head, on, vals)
            })
            .collect()
    }

    /// Keep every snapshot's command records in step with the commands, as the pedal does: one record
    /// `[slot, false, [0, arg1 .. arg6]]` per slot in use, in slot order, except HX Preset (14), which has
    /// none, then padding `[slot count, false, zeros]` up to the number of records the document has.
    /// A slot whose command is `changed` starts from its base arguments; the others keep the values the
    /// snapshot has for them. Nothing is touched when the records are already right.
    pub fn sync_snapshot_commands(&mut self, changed: Option<usize>) {
        let count = self.command_count();
        if count == 0 {
            return;
        }
        let used: Vec<(usize, Vec<i64>)> =
            (0..count).filter_map(|s| self.command(s).filter(|(id, _)| *id != 14).map(|(_, args)| (s, args))).collect();
        let snaps: Vec<Value> =
            self.tone.at(&[key::SNAPSHOT_SECTION, key::SNAPSHOTS]).and_then(Value::as_array).cloned().unwrap_or_default();
        for (n, snap) in snaps.iter().enumerate() {
            let have = Self::snapshot_command_rows(snap);
            if have.is_empty() {
                continue;
            }
            let existing = |slot: usize| have.iter().find(|(h, _, _)| *h == slot as i64);
            let mut want: Vec<(i64, bool, Vec<i64>)> = Vec::new();
            for (slot, args) in &used {
                let kept = if changed == Some(*slot) { None } else { existing(*slot) };
                want.push(match kept {
                    Some(r) => r.clone(),
                    None => {
                        let mut vals = vec![0];
                        vals.extend((0..6).map(|i| args.get(i).copied().unwrap_or(0)));
                        (*slot as i64, false, vals)
                    }
                });
            }
            while want.len() < have.len() {
                want.push((count as i64, false, vec![0; 7]));
            }
            want.truncate(have.len().max(used.len()));
            if want == have {
                continue;
            }
            let ints = |v: &[i64]| Value::Array(v.iter().map(|x| Value::int(*x)).collect(), crate::msgpack::AUTO);
            let rows: Vec<Value> = want
                .iter()
                .map(|(h, on, vals)| Value::Array(vec![Value::int(*h), Value::Bool(*on), ints(vals)], crate::msgpack::AUTO))
                .collect();
            // A snapshot is a map; its field 1 holds the command records.
            let target = self
                .tone
                .at_mut(&[key::SNAPSHOT_SECTION, key::SNAPSHOTS])
                .and_then(|v| v.as_array_mut())
                .and_then(|a| a.get_mut(n))
                .and_then(|s| s.get_mut(1));
            if let Some(Value::Array(old, _)) = target {
                *old = rows;
            }
        }
    }

    /// Put a command in a slot (id 0 empties it). The arguments follow the command's fields in the
    /// order `.hlx` files list them; missing ones are 0. False when the slot doesn't exist.
    pub fn set_command(&mut self, slot: usize, id: i64, args: &[i64]) -> bool {
        let id = id.max(0);
        let old_id = self.command(slot).map_or(0, |c| c.0);
        // The pedal's own rows start with 0, except HX Preset's, which start with its id.
        let mut row = vec![Value::int(if id == 14 { 14 } else { 0 })];
        row.extend((0..6).map(|i| Value::int(if id == 0 { 0 } else { args.get(i).copied().unwrap_or(0) })));
        if id == 2 || id == 12 {
            // CC Toggle and Hotkey keep their Momentary flag in the fifth argument (the flag itself sits in the
            // footswitch table) and a latching CC Toggle its current state, the Dim value at first, in the
            // sixth. A new CC Toggle starts latching, a new Hotkey momentary.
            if old_id != id {
                self.set_command_momentary(slot, id == 12);
            }
            let momentary = self.command_momentary(slot).unwrap_or(id == 12);
            row[5] = Value::int(momentary as i64);
            row[6] = Value::int(if momentary || id == 12 { 0 } else { args.get(2).copied().unwrap_or(0) });
        }
        let Some(Value::Array(ids, _)) = self.tone.at_mut(&[key::COMMANDS, 0]) else { return false };
        let Some(cell) = ids.get_mut(slot) else { return false };
        *cell = Value::int(id);
        let Some(Value::Array(rows, _)) = self.tone.at_mut(&[key::COMMANDS, 1]) else { return false };
        let set = match rows.get_mut(slot) {
            Some(Value::Array(items, _)) => {
                *items = row;
                true
            }
            _ => false,
        };
        if set {
            if id == 0 {
                self.remove_cc_record(slot);
            } else {
                self.set_command_label(slot, id, id != old_id);
            }
            // The snapshots keep their own copy of what each command does: bring them in step.
            self.sync_snapshot_commands(Some(slot));
        }
        set
    }

    /// The footswitch table's Command Center record of a footswitch (slots 6 to 10 on an HX Stomp): the one
    /// whose kind (field 11, entry 0) is 3. A switch can also carry block on/off records. With `create`, a
    /// missing record is made with that name.
    fn fs_record_mut(&mut self, slot: usize, create: Option<&str>) -> Option<&mut Value> {
        let i = slot.checked_sub(6).filter(|i| *i < 5)?;
        let Some(Value::Array(table, _)) = self.tone.at_mut(&[3, 8]) else { return None };
        let sw = table.get_mut(i)?;
        let is_cc = |r: &Value| r.get(11).and_then(|k| k.get(0)).and_then(Value::as_i64) == Some(3);
        if !matches!(sw, Value::Array(recs, _) if recs.iter().any(is_cc)) {
            let label = create?;
            let rec = Value::map(vec![
                (10, Value::int(0)),
                (
                    11,
                    Value::map(vec![
                        (0, Value::int(3)),
                        (5, Value::str(&format!("{label}\0"))),
                        (6, Value::int(462860)),
                        (7, Value::Bool(false)),
                    ]),
                ),
                (12, Value::Bool(false)),
                (13, Value::Bool(false)),
                (14, Value::str("\0")),
                (15, Value::Bool(false)),
                (16, Value::int(0)),
            ]);
            match sw {
                Value::Array(recs, _) => recs.push(rec),
                _ => *sw = Value::Array(vec![rec], crate::msgpack::AUTO),
            }
        }
        match sw {
            Value::Array(recs, _) => recs.iter_mut().find(|r| is_cc(r)),
            _ => None,
        }
    }

    /// Take a footswitch's Command Center record away (its command was removed).
    fn remove_cc_record(&mut self, slot: usize) {
        let Some(i) = slot.checked_sub(6).filter(|i| *i < 5) else { return };
        let Some(Value::Array(table, _)) = self.tone.at_mut(&[3, 8]) else { return };
        let Some(sw) = table.get_mut(i) else { return };
        if let Value::Array(recs, _) = sw {
            recs.retain(|r| r.get(11).and_then(|k| k.get(0)).and_then(Value::as_i64) != Some(3));
            if recs.is_empty() {
                *sw = Value::Nil;
            }
        }
    }

    /// Whether a Command Center footswitch is momentary (the footswitch table's flag; the other editor's
    /// "Momentary" switch, off = latching). None for a slot that isn't a footswitch or has no record.
    pub fn command_momentary(&self, slot: usize) -> Option<bool> {
        let i = slot.checked_sub(6).filter(|i| *i < 5)?;
        let sw = self.tone.at(&[3, 8])?.as_array()?.get(i)?.as_array()?;
        let rec = sw.iter().find(|r| r.get(11).and_then(|k| k.get(0)).and_then(Value::as_i64) == Some(3))?;
        match rec.get(12)? {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Set the Momentary flag of a Command Center footswitch. A CC Toggle or Hotkey also keeps it in its
    /// fifth argument. False when the slot isn't a footswitch.
    pub fn set_command_momentary(&mut self, slot: usize, momentary: bool) -> bool {
        let Some(rec) = self.fs_record_mut(slot, Some("Command")) else { return false };
        let Some(flag) = rec.get_mut(12) else { return false };
        *flag = Value::Bool(momentary);
        if self.command(slot).is_some_and(|c| c.0 == 2 || c.0 == 12) {
            let (id, dim) = self.command(slot).map(|c| (c.0, c.1.get(2).copied().unwrap_or(0))).unwrap_or((0, 0));
            if let Some(Value::Array(rows, _)) = self.tone.at_mut(&[key::COMMANDS, 1]) {
                if let Some(Value::Array(row, _)) = rows.get_mut(slot) {
                    if row.len() > 6 {
                        row[5] = Value::int(momentary as i64);
                        row[6] = Value::int(if momentary || id == 12 { 0 } else { dim });
                    }
                }
            }
            self.sync_snapshot_commands(Some(slot));
        }
        true
    }

    /// A footswitch with a command has a record in the footswitch table, named after the command type. When
    /// the type is new the pedal also picks whether the switch is momentary. Seen on an HX Stomp: CC Value,
    /// CC Toggle, Note, MMC, Hotkey, Preset, Snapshot; the names and defaults of Bank/Program and HX
    /// Looper are guesses.
    fn set_command_label(&mut self, slot: usize, id: i64, new_type: bool) {
        let (label, momentary) = match id {
            1 => ("CC Value", true),
            2 => ("CC Toggle", false),
            4 => ("Bank/Program", true),
            5 => ("Note", false),
            6 => ("MMC", true),
            12 => ("Hotkey", true),
            14 => ("Preset", false),
            15 => ("Snapshot", true),
            16 => ("Looper", true),
            _ => return,
        };
        let Some(rec) = self.fs_record_mut(slot, Some(label)) else { return };
        if new_type {
            if let Some(Value::Str(s, _)) = rec.get_mut(11).and_then(|i| i.get_mut(5)) {
                *s = format!("{label}\0");
            }
            if let Some(flag) = rec.get_mut(12) {
                *flag = Value::Bool(momentary);
            }
        }
    }

    /// Whether a path is split (the split's own "active" flag, set when it's placed).
    pub fn split_active(&self, path: i64) -> bool {
        let Some(Value::Array(items, _)) = self.tone.at(&[path, key::SLOTS]) else { return false };
        items
            .iter()
            .find(|r| Kind::from_wire(r.get(key::KIND).and_then(Value::as_i64).unwrap_or(-1)) == Kind::Split)
            .and_then(|r| r.get(key::BODY))
            .and_then(|b| b.get(18))
            .is_some_and(|v| matches!(v, Value::Bool(true)))
    }

    fn sync_split_active(&mut self, path: i64) {
        let Some(Value::Array(items, _)) = self.tone.at_mut(&[path, key::SLOTS]) else { return };
        let split_pos = items.iter().position(|r| Kind::from_wire(r.get(key::KIND).and_then(Value::as_i64).unwrap_or(-1)) == Kind::Split);
        let join_pos = items.iter().position(|r| Kind::from_wire(r.get(key::KIND).and_then(Value::as_i64).unwrap_or(-1)) == Kind::Join);
        let (Some(sp), Some(jp)) = (split_pos, join_pos) else { return };
        if sp >= jp {
            return;
        }
        let lower_has_block =
            items[sp + 1..jp].iter().any(|r| Kind::from_wire(r.get(key::KIND).and_then(Value::as_i64).unwrap_or(-1)) != Kind::Empty);
        for idx in [sp, jp] {
            if let Some(Value::Bool(active)) = items.get_mut(idx).and_then(|s| s.get_mut(key::BODY)).and_then(|b| b.get_mut(18)) {
                *active = lower_has_block;
            }
        }
    }

    /// Replace one parameter value, keeping its type.
    pub fn set_value(&mut self, path: i64, position: usize, index: usize, value: Value) {
        let Some(body) = self.body_mut(path, position) else { return };
        let has_values = body.get(key::VALUES).is_some();
        let arr = if has_values { body.get_mut(key::VALUES) } else { body.get_mut(key::IO_VALUES) };
        if let Some(Value::Array(items, _)) = arr.and_then(|a| a.get_mut(key::ARRAY_VALUES)) {
            if let Some(slot) = items.get_mut(index) {
                *slot = value;
            }
        }
    }

    /// The knobs of the cab paired with a block (amp + cab).
    pub fn paired_values(&self, path: i64, position: usize) -> Vec<Value> {
        let Some(Value::Array(items, _)) = self.tone.at(&[path, key::SLOTS]) else { return Vec::new() };
        items.get(position).and_then(|raw| raw.get(key::BODY)).and_then(|b| b.get(key::PAIRED_VALUES)).map(read_values).unwrap_or_default()
    }

    pub fn set_paired_value(&mut self, path: i64, position: usize, index: usize, value: Value) {
        let Some(body) = self.body_mut(path, position) else { return };
        if let Some(Value::Array(items, _)) = body.get_mut(key::PAIRED_VALUES).and_then(|a| a.get_mut(key::ARRAY_VALUES)) {
            if let Some(slot) = items.get_mut(index) {
                *slot = value;
            }
        }
    }
}

/// One controller assignment of a pedal preset (tone key 4), with the values each
/// snapshot keeps for it (each snapshot's field 2).
#[derive(Clone, Debug)]
pub struct Ctl {
    /// Controller number (EXP 1 = 1, footswitches from `fs_first`, snapshots, MIDI CC...).
    pub controller: i64,
    /// 0 a knob, 1 a global setting, 2 a block's on/off (only knobs are edited here).
    pub kind: i64,
    /// The slot's wire number (`20 * path + slot`).
    pub wire: i64,
    /// Which knob, in the pedal's order; and whether it's the paired cab's.
    pub index: usize,
    pub cab: bool,
    pub min: Value,
    pub max: Value,
    pub cc: i64,
    pub snapshot_disable: bool,
    /// The record as read, so fields this app doesn't edit stay as they were.
    raw: Value,
    /// Per snapshot: (footswitch on, value), when stored.
    pub snaps: Vec<Option<(bool, Value)>>,
}

impl Ctl {
    /// A new knob assignment with no range, CC or snapshot values yet; set those next.
    pub fn knob(controller: i64, wire: i64, index: usize) -> Ctl {
        Ctl {
            controller,
            kind: 0,
            wire,
            index,
            cab: false,
            min: Value::Nil,
            max: Value::Nil,
            cc: 0,
            snapshot_disable: false,
            raw: Value::Nil,
            snaps: Vec::new(),
        }
    }
}

/// A footswitch record while sorting: (primary flag, kind order, ordinal) and the record.
type FootswitchEntry = ((i64, i64, i64), Vec<(i64, Value)>);

impl DevicePreset {
    /// Every controller assignment, in the pedal's own order.
    pub fn controllers(&self) -> Vec<Ctl> {
        let mut out: Vec<(i64, Ctl)> = Vec::new();
        let Some(table) = self.tone.get(4).and_then(Value::as_array) else { return Vec::new() };
        let snap_records: Vec<Vec<Value>> = self
            .tone
            .at(&[key::SNAPSHOT_SECTION, key::SNAPSHOTS])
            .and_then(Value::as_array)
            .map(|s| s.iter().map(|x| x.get(2).and_then(Value::as_array).cloned().unwrap_or_default()).collect())
            .unwrap_or_default();
        for row in table.iter().filter_map(Value::as_array) {
            for e in row {
                let (Some(t), Some(inner)) = (e.get(0).and_then(Value::as_i64), e.get(1)) else { continue };
                let target = inner.get(6);
                let snaps = snap_records
                    .iter()
                    .map(|recs| {
                        recs.iter()
                            .filter_map(Value::as_array)
                            .find(|r| r.get(1).and_then(Value::as_i64) == Some(t))
                            .map(|r| (matches!(r.first(), Some(Value::Bool(true))), r.get(2).cloned().unwrap_or(Value::Nil)))
                    })
                    .collect();
                out.push((
                    t,
                    Ctl {
                        controller: inner.get(0).and_then(Value::as_i64).unwrap_or(0),
                        kind: inner.get(4).and_then(Value::as_i64).unwrap_or(0),
                        wire: inner.get(5).and_then(Value::as_i64).unwrap_or(0),
                        index: target.and_then(|x| x.get(29)).and_then(Value::as_i64).unwrap_or(0).max(0) as usize,
                        cab: target.and_then(|x| x.get(28)).and_then(Value::as_i64).unwrap_or(0) != 0,
                        min: inner.get(2).cloned().unwrap_or(Value::Nil),
                        max: inner.get(3).cloned().unwrap_or(Value::Nil),
                        cc: inner.get(1).and_then(Value::as_i64).unwrap_or(4),
                        snapshot_disable: matches!(inner.get(13), Some(Value::Bool(true))),
                        raw: inner.clone(),
                        snaps,
                    },
                ));
            }
        }
        out.sort_by_key(|(t, _)| *t);
        out.into_iter().map(|(_, c)| c).collect()
    }

    /// Write the controller list back: the table (tone key 4) and each snapshot's values,
    /// renumbering the assignments in list order.
    pub fn set_controllers(&mut self, list: &[Ctl]) {
        let Some(len) = self.tone.get(4).and_then(Value::as_array).map(|a| a.len()) else { return };
        let fmt = match self.tone.get(4) {
            Some(Value::Array(_, f)) => *f,
            _ => crate::msgpack::AUTO,
        };
        let mut table: Vec<Value> = vec![Value::Nil; len];
        for (t, c) in list.iter().enumerate() {
            let Some(row) = table.get_mut(c.controller.max(0) as usize) else { continue };
            let mut inner = match &c.raw {
                Value::Map(..) => c.raw.clone(),
                _ => Value::map(vec![
                    (0, Value::int(c.controller)),
                    (1, Value::int(c.cc)),
                    (2, c.min.clone()),
                    (3, c.max.clone()),
                    (4, Value::int(0)),
                    (5, Value::int(c.wire)),
                    (6, Value::map(vec![(28, Value::int(c.cab as i64)), (29, Value::int(c.index as i64)), (41, Value::Bool(false))])),
                    (7, Value::int(0)),
                    (13, Value::Bool(c.snapshot_disable)),
                ]),
            };
            for (k, v) in [
                (0, Value::int(c.controller)),
                (1, Value::int(c.cc)),
                (2, c.min.clone()),
                (3, c.max.clone()),
                (13, Value::Bool(c.snapshot_disable)),
            ] {
                if let Some(slot) = inner.get_mut(k) {
                    *slot = v;
                }
            }
            let rec = Value::map(vec![(0, Value::int(t as i64)), (1, inner)]);
            match row {
                Value::Array(items, _) => items.push(rec),
                _ => *row = Value::Array(vec![rec], crate::msgpack::AUTO),
            }
        }
        if let Some(slot) = self.tone.get_mut(4) {
            *slot = Value::Array(table, fmt);
        }
        if let Some(Value::Array(snaps, _)) = self.tone.at_mut(&[key::SNAPSHOT_SECTION, key::SNAPSHOTS]) {
            for (s, snap) in snaps.iter_mut().enumerate() {
                let Some(Value::Array(old, f2)) = snap.get(2).cloned() else { continue };
                let blank = || Value::Array(vec![Value::Bool(false), Value::int(64), Value::Nil], crate::msgpack::AUTO);
                let mut recs: Vec<Value> = (0..old.len().max(64)).map(|_| blank()).collect();
                let mut p = 0;
                for (t, c) in list.iter().enumerate() {
                    if c.kind == 2 || p >= recs.len() {
                        continue;
                    }
                    let (on, v) = c.snaps.get(s).cloned().flatten().unwrap_or((false, Value::Nil));
                    recs[p] = Value::Array(vec![Value::Bool(on), Value::int(t as i64), v], crate::msgpack::AUTO);
                    p += 1;
                }
                if let Some(slot) = snap.get_mut(2) {
                    *slot = Value::Array(recs, f2);
                }
            }
        }
    }

    /// The footswitch (1-based) that switches a slot on and off, if any (tone key 3).
    pub fn block_footswitch(&self, wire: i64) -> Option<usize> {
        let table = self.tone.at(&[3, 8])?.as_array()?;
        table
            .iter()
            .position(|sw| {
                sw.as_array().is_some_and(|recs| {
                    recs.iter().any(|r| {
                        r.get(11)
                            .is_some_and(|i| i.get(0).and_then(Value::as_i64) == Some(1) && i.get(8).and_then(Value::as_i64) == Some(wire))
                    })
                })
            })
            .map(|i| i + 1)
    }

    /// Put a slot's on/off on a footswitch (1-based), or take it off (None).
    pub fn set_block_footswitch(&mut self, wire: i64, fs: Option<usize>, label: &str, enabled: bool) {
        let Some(Value::Array(table, _)) = self.tone.at_mut(&[3, 8]) else { return };
        let is_this = |r: &Value| {
            r.get(11).is_some_and(|i| i.get(0).and_then(Value::as_i64) == Some(1) && i.get(8).and_then(Value::as_i64) == Some(wire))
        };
        for sw in table.iter_mut() {
            if let Value::Array(recs, _) = sw {
                recs.retain(|r| !is_this(r));
                if recs.is_empty() {
                    *sw = Value::Nil;
                }
            }
        }
        if let Some(n) = fs.filter(|n| *n >= 1 && *n <= table.len()) {
            let label: String = label.chars().filter(|c| (' '..='~').contains(c)).take(16).collect();
            let rec = Value::map(vec![
                (10, Value::int(0)),
                (
                    11,
                    Value::map(vec![
                        (0, Value::int(1)),
                        (5, Value::str(&format!("{label}\0"))),
                        (6, Value::int(0)),
                        (7, Value::Bool(enabled)),
                        (8, Value::int(wire)),
                        (2, Value::int(0)),
                    ]),
                ),
                (12, Value::Bool(false)),
                (13, Value::Bool(false)),
                (14, Value::str("\0")),
                (15, Value::Bool(false)),
                (16, Value::int(0)),
            ]);
            let sw = &mut table[n - 1];
            match sw {
                Value::Array(recs, _) => recs.push(rec),
                _ => *sw = Value::Array(vec![rec], crate::msgpack::AUTO),
            }
        }
        // Keep each switch's records numbered in order (field 10).
        for sw in table.iter_mut() {
            if let Value::Array(recs, _) = sw {
                for (i, r) in recs.iter_mut().enumerate() {
                    if let Some(v) = r.get_mut(10) {
                        *v = Value::int(i as i64);
                    }
                }
            }
        }
    }

    /// The live value of a knob of a slot (for a new snapshot assignment).
    pub fn knob_value(&self, wire: i64, index: usize, cab: bool) -> Option<Value> {
        let s = self.slots().into_iter().find(|s| s.wire_block() == wire)?;
        if cab {
            self.paired_values(s.path, s.position).get(index).cloned()
        } else {
            s.values.get(index).cloned()
        }
    }
}

/// A footswitch LED colour as the pedal stores it: lit or dim variant of the colour.
fn led_colour(c: i64, lit: bool) -> i64 {
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
    for (on, off) in PAIRS {
        if c == on || c == off {
            return if lit { on } else { off };
        }
    }
    let (r, g, b) = ((c >> 16) & 255, (c >> 8) & 255, c & 255);
    if lit || r.max(g).max(b) < 64 {
        return c;
    }
    let d = |x: i64| (x as f64 / 21.0).round() as i64;
    (d(r) << 16) | (d(g) << 8) | d(b)
}

/// A controller's min/max as the pedal stores it for the knob's type.
fn typed_bound(v: Option<&serde_json::Value>, vt: i64, default: f64) -> Value {
    match v {
        Some(serde_json::Value::Bool(b)) => Value::Bool(*b),
        Some(serde_json::Value::Number(n)) => {
            let x = n.as_f64().unwrap_or(default);
            if vt == 0 {
                Value::int(x.round() as i64)
            } else {
                Value::F32(x as f32)
            }
        }
        _ => {
            if vt == 0 {
                Value::int(default.round() as i64)
            } else {
                Value::F32(default as f32)
            }
        }
    }
}

/// One knob assigned to a controller, in the pedal's order.
struct Assign {
    controller: i64,
    dsp: i64,
    slot: usize,
    key: String,
    param: String,
    index: usize,
    vt: i64,
    trails: bool,
    cfg: serde_json::Map<String, serde_json::Value>,
}

/// A .hlx knob value as the pedal stores it for that knob's type.
fn knob_value(v: Option<&serde_json::Value>, vt: i64, default: f64) -> Value {
    let n = match v {
        Some(serde_json::Value::Bool(b)) => *b as u8 as f64,
        Some(serde_json::Value::Number(x)) => x.as_f64().unwrap_or(default),
        _ => default,
    };
    match vt {
        2 => Value::Bool(n != 0.0),
        0 => Value::int(n.round() as i64),
        _ => Value::F32(n as f32),
    }
}

impl DevicePreset {
    /// Turn a .hlx preset into the pedal's own document, starting from `template`
    /// (a preset read from the same pedal, so the document has its firmware's shape).
    ///
    /// The pedal's document layout: a path
    /// of `c` columns has `2c + 4` slots: input 0, lane A at 1..=c, output c+1,
    /// split c+2, lane B from c+3, merge 2c+3; a split or merge attaches before
    /// main-lane slot `@position + 1`. Blocks, their cabs, knob values and on/off,
    /// split and merge points, tempo and snapshot names are carried over.
    /// Snapshot knob values, controllers and footswitches are not (listed in the notes).
    pub fn from_hlx(template: &DevicePreset, doc: &serde_json::Value, cat: &Catalog) -> Result<(DevicePreset, Vec<String>), String> {
        let mut out = template.clone();
        let mut notes = Vec::new();
        // (dsp, block key, slot, model) of every block placed, for the controllers.
        let mut placed: Vec<(i64, String, usize, i64)> = Vec::new();
        let blocks = crate::hlx::blocks(doc);
        for dsp in crate::hlx::dsps(doc) {
            let d: i64 = dsp.trim_start_matches("dsp").parse().unwrap_or(0);
            let pk = key::PATH0 + d;
            let on_path: Vec<&crate::hlx::HlxBlock> = blocks.iter().filter(|b| b.dsp == dsp && b.key.starts_with("block")).collect();
            let Some(len) = out.tone.at(&[pk, key::SLOTS]).and_then(Value::as_array).map(|a| a.len()) else {
                if !on_path.is_empty() {
                    return Err(format!("it uses Path {} ({} blocks), and this pedal has no such path", d + 1, on_path.len()));
                }
                continue;
            };
            if len < 6 || (len - 4) % 2 != 0 {
                return Err(format!("this pedal's path has {len} slots, a layout this app doesn't know"));
            }
            let columns = (len - 4) / 2;
            let (row_a, split_at, row_b, join_at) = (1, columns + 2, columns + 3, 2 * columns + 3);
            // Start from empty lanes.
            for i in (row_a..row_a + columns).chain(row_b..row_b + columns) {
                out.clear_slot(pk, i);
            }
            for b in &on_path {
                let pos = b.position.max(0) as usize;
                if pos >= columns {
                    notes.push(format!("{} sits at position {} beyond this pedal's {columns}; left out.", b.model, pos + 1));
                    continue;
                }
                let slot = if b.path == 1 { row_b } else { row_a } + pos;
                let obj = crate::hlx::block(doc, b).cloned().unwrap_or_default();
                let stereo = obj.get("@stereo").and_then(serde_json::Value::as_bool).unwrap_or(false);
                let Some(model) = cat.number_for_hlx(&b.model, stereo) else {
                    notes.push(format!("{} isn't in this pedal's effect list; left out.", b.model));
                    continue;
                };
                if !cat.allowed_on(&b.model, crate::snapshots::device(doc)) {
                    notes.push(format!("{} may not be available on this unit.", cat.model_name(model)));
                }
                let cab_obj = obj
                    .get("@cab")
                    .and_then(serde_json::Value::as_str)
                    .and_then(|k| doc.pointer(&format!("/data/tone/{dsp}/{k}")))
                    .cloned();
                let cab = cab_obj
                    .as_ref()
                    .and_then(|c| c.get("@model"))
                    .and_then(serde_json::Value::as_str)
                    .and_then(|m| cat.number_for_hlx(m, false));
                out.place_model(cat, pk, slot, model, cab);
                placed.push((d, b.key.clone(), slot, model));
                for (i, (id, vt, def)) in cat.default_values(model).into_iter().enumerate() {
                    out.set_value(pk, slot, i, knob_value(obj.get(&id), vt, def));
                }
                if let (Some(c), Some(cobj)) = (cab, cab_obj.as_ref()) {
                    for (i, (id, vt, def)) in cat.default_values(c).into_iter().enumerate() {
                        out.set_paired_value(pk, slot, i, knob_value(cobj.get(&id), vt, def));
                    }
                }
                let on = obj.get("@enabled").and_then(serde_json::Value::as_bool).unwrap_or(true);
                out.set_enabled(pk, slot, on);
                // Each snapshot's own on/off for this block (snapshotN/blocks/dspX/blockY in the .hlx).
                let wire = (20 * (pk - key::PATH0) + slot as i64).max(0) as usize;
                for i in 0..out.snapshot_names().len() {
                    let state = doc.pointer(&format!("/data/tone/snapshot{i}/blocks/{dsp}/{}", b.key)).and_then(serde_json::Value::as_bool);
                    if let Some(state) = state {
                        out.set_snapshot_enabled(i, wire, state);
                    }
                }
            }
            // Split and merge points.
            let tone = format!("/data/tone/{dsp}");
            let pos_of = |k: &str| doc.pointer(&format!("{tone}/{k}/@position")).and_then(serde_json::Value::as_i64);
            // The split sits before a main-lane slot (at most the last one), the merge before
            // one or at the end; a file from a longer path is held to this pedal's columns.
            let cols = columns as i64;
            let split_attach = (pos_of("split").unwrap_or(0) + 1).clamp(1, cols);
            let join_attach = (pos_of("join").unwrap_or(cols) + 1).clamp(split_attach, cols + 1);
            if (pos_of("split").unwrap_or(0) + 1 > cols) || (pos_of("join").unwrap_or(cols) + 1 > cols + 1) {
                notes.push(format!("Path {}'s split or merge sat beyond this pedal's {columns} places; moved to the end.", d + 1));
            }
            for (at, attach) in [(split_at, split_attach), (join_at, join_attach)] {
                if out.attach(pk, at).is_some() {
                    out.set_attach(pk, at, attach);
                }
            }
            // The path's input and output: where the sound comes from and goes, and their knobs.
            let (hlx_fam, pedal_fam) = (crate::snapshots::family(doc), crate::snapshots::family_of_device(crate::nam::unit()));
            for (sec, input, slot) in [("inputA", true, 0usize), ("outputA", false, columns + 1)] {
                let Some(obj) = doc.pointer(&format!("{tone}/{sec}")).and_then(serde_json::Value::as_object) else { continue };
                let route = obj.get(if input { "@input" } else { "@output" }).and_then(serde_json::Value::as_i64);
                if let (Some(v), Some(_)) = (route, out.io_route(pk, slot)) {
                    let (nv, exact) = crate::routes::map_route(hlx_fam, pedal_fam, input, v);
                    out.set_io_route(pk, slot, nv);
                    if !exact {
                        let was = crate::routes::route_name(hlx_fam, input, v).map(str::to_owned).unwrap_or(format!("route {v}"));
                        let now = crate::routes::route_name(pedal_fam, input, nv).unwrap_or("the main one");
                        let what = if input { "input" } else { "output" };
                        notes.push(format!("Path {} {what}: \"{was}\" isn't on this pedal; set to \"{now}\".", d + 1));
                    }
                }
                let knobs: &[(usize, &str, i64)] =
                    if input { &[(0, "noiseGate", 2), (1, "threshold", 1), (2, "decay", 1)] } else { &[(0, "gain", 1), (1, "pan", 1)] };
                for (i, id, vt) in knobs {
                    if let Some(v) = obj.get(*id) {
                        out.set_value(pk, slot, *i, knob_value(Some(v), *vt, 0.0));
                    }
                }
            }
            out.sync_split_active(pk);
        }
        if let Some(t) = doc.pointer("/data/tone/global/@tempo").and_then(serde_json::Value::as_f64) {
            out.set_tempo(t);
        }
        // The guitar input impedance and pad: preset settings on the pedal, preset globals in the file
        // (the same numbering: 0 Auto .. 8 1M, and 0 Global / 1 Off / 2 On).
        for (name, field) in [("@guitarinputZ", key::GUITAR_IN_Z), ("@guitarpad", key::GUITAR_PAD)] {
            if let Some(v) = doc.pointer(&format!("/data/tone/global/{name}")).and_then(serde_json::Value::as_i64) {
                if out.setting_int(field) != Some(v) && !out.set_setting_int(field, v) {
                    notes.push(format!("{name} has no place in this pedal's preset; left out."));
                }
            }
        }
        let n = out.snapshot_names().len();
        for (i, name) in crate::snapshots::list(doc) {
            if i < n && !name.is_empty() {
                out.set_snapshot_name(i, &name);
            }
        }
        out.apply_hlx_controllers(doc, cat, &placed, &mut notes);
        out.apply_hlx_commands(doc, &mut notes);
        Ok((out, notes))
    }

    /// Everything about the Command Center that an edit can change, for comparing two documents: each slot's
    /// command, its Momentary flag and its values in every snapshot.
    pub fn command_state(&self) -> Vec<CommandState> {
        let snaps = self.snapshot_names().len();
        (0..self.command_count())
            .map(|slot| {
                (self.command(slot), self.command_momentary(slot), (0..snaps).map(|n| self.snapshot_command_args(n, slot)).collect())
            })
            .collect()
    }

    /// A command's values in one snapshot (its six arguments), or None when that snapshot has no record for
    /// the slot (no command, or HX Preset, which is the same everywhere).
    pub fn snapshot_command_args(&self, snapshot: usize, slot: usize) -> Option<Vec<i64>> {
        let snap = self.tone.at(&[key::SNAPSHOT_SECTION, key::SNAPSHOTS])?.as_array()?.get(snapshot)?;
        Self::snapshot_command_rows(snap)
            .into_iter()
            .find(|(h, _, _)| *h == slot as i64)
            .map(|(_, _, v)| v.into_iter().skip(1).take(6).collect())
    }

    /// Change a command's values in one snapshot, as the pedal does when a value is set while that snapshot is
    /// selected: the snapshot's record changes, and the command's own row too when it is the snapshot the
    /// pedal has selected. HX Preset has one set of values for all, kept in its row.
    pub fn set_command_values(&mut self, snapshot: usize, slot: usize, args: &[i64]) -> bool {
        let Some((id, _)) = self.command(slot) else { return false };
        if id == 14 {
            return self.set_command(slot, id, args);
        }
        if !self.set_snapshot_command(snapshot, slot, args) {
            return false;
        }
        if snapshot == self.current_snapshot() {
            if let Some(Value::Array(rows, _)) = self.tone.at_mut(&[key::COMMANDS, 1]) {
                if let Some(Value::Array(row, _)) = rows.get_mut(slot) {
                    for (i, a) in args.iter().take(6).enumerate() {
                        if let Some(v) = row.get_mut(i + 1) {
                            *v = Value::int(*a);
                        }
                    }
                }
            }
        }
        true
    }

    /// Change a snapshot's own values for a command: the first arguments of its record (the rest, such as a
    /// CC Toggle's state, stay). False when the slot has no record (no command, or HX Preset).
    pub fn set_snapshot_command(&mut self, snapshot: usize, slot: usize, args: &[i64]) -> bool {
        self.sync_snapshot_commands(None);
        let Some(Value::Array(rows, _)) = self
            .tone
            .at_mut(&[key::SNAPSHOT_SECTION, key::SNAPSHOTS])
            .and_then(|v| v.as_array_mut())
            .and_then(|a| a.get_mut(snapshot))
            .and_then(|s| s.get_mut(1))
        else {
            return false;
        };
        for row in rows.iter_mut() {
            let Value::Array(parts, _) = row else { continue };
            if parts.first().and_then(Value::as_i64) != Some(slot as i64) {
                continue;
            }
            let Some(Value::Array(vals, _)) = parts.get_mut(2) else { return false };
            for (i, a) in args.iter().take(6).enumerate() {
                if let Some(v) = vals.get_mut(i + 1) {
                    *v = Value::int(*a);
                }
            }
            return true;
        }
        false
    }

    /// The Command Center of a `.hlx` (`commandInst1`, `commandFS2`, ... and each snapshot's own copy) into
    /// this document: every slot is cleared first, then the file's commands are set in the pedal's argument
    /// order, with the wait held to the pedal's 20, the Momentary flags and the snapshots' own values.
    fn apply_hlx_commands(&mut self, doc: &serde_json::Value, notes: &mut Vec<String>) {
        use crate::controllers::{hlx_arg_keys, stomp_device_slots, stomp_slot_of_hlx_key};
        let Some(tone) = doc.pointer("/data/tone").and_then(serde_json::Value::as_object) else { return };
        let in_file = tone.keys().any(|k| k.starts_with("command"));
        if self.command_count() != 13 {
            if in_file {
                notes.push("This pedal's Command Center isn't carried over from a .hlx yet (only the HX Stomp's is).".into());
            }
            return;
        }
        let slots = stomp_device_slots();
        for slot in 0..slots.len() {
            self.set_command(slot, 0, &[]);
        }
        let number = |v: Option<&serde_json::Value>| v.and_then(serde_json::Value::as_i64);
        let clamp = |key: &str, v: i64| if key == "@waittime" { v.clamp(0, 20) } else { v };
        for (k, cmd) in tone.iter().filter(|(k, _)| k.starts_with("command")) {
            let Some(slot) = stomp_slot_of_hlx_key(k) else {
                notes.push(format!("{k} has no place on this pedal; left out."));
                continue;
            };
            let id = number(cmd.get("@command")).unwrap_or(0);
            if id == 0 {
                continue;
            }
            let keys = hlx_arg_keys(slots[slot].kind, id);
            if keys.is_empty() {
                notes.push(format!("{k}: command {id} isn't one this pedal's {} takes; left out.", slots[slot].title));
                continue;
            }
            let args: Vec<i64> = keys.iter().map(|key| clamp(key, number(cmd.get(*key)).unwrap_or(0))).collect();
            if keys.contains(&"@waittime")
                && keys.iter().zip(&args).any(|(key, a)| *key == "@waittime" && Some(*a) != number(cmd.get(*key)))
            {
                notes.push(format!("{}'s wait was held to the pedal's limit of 20.", slots[slot].title));
            }
            self.set_command(slot, id, &args);
            if let Some(m) = cmd.get("@fs_momentary").and_then(serde_json::Value::as_bool) {
                if matches!(id, 2 | 12) {
                    self.set_command_momentary(slot, m);
                } else if let Some(rec) = self.fs_record_mut(slot, None) {
                    if let Some(flag) = rec.get_mut(12) {
                        *flag = Value::Bool(m);
                    }
                }
            }
        }
        // Each snapshot's own values.
        for n in 0..self.snapshot_names().len() {
            let Some(own) = doc.pointer(&format!("/data/tone/snapshot{n}/commands")).and_then(serde_json::Value::as_object) else {
                continue;
            };
            for (k, cmd) in own {
                let Some(slot) = stomp_slot_of_hlx_key(k) else { continue };
                let Some((id, base)) = self.command(slot) else { continue };
                if id == 14 {
                    continue;
                }
                let keys = hlx_arg_keys(slots[slot].kind, id);
                let args: Vec<i64> = keys
                    .iter()
                    .enumerate()
                    .map(|(i, key)| clamp(key, number(cmd.get(*key)).unwrap_or(base.get(i).copied().unwrap_or(0))))
                    .collect();
                self.set_snapshot_command(n, slot, &args);
            }
        }
    }

    /// Controllers, per-snapshot knob values and footswitches from a .hlx, laid out
    /// as the pedal keeps them (tone key 4 = controllers, key 3 =
    /// footswitches, each snapshot's field 2 = values of the assigned knobs).
    fn apply_hlx_controllers(
        &mut self,
        doc: &serde_json::Value,
        cat: &Catalog,
        placed: &[(i64, String, usize, i64)],
        notes: &mut Vec<String>,
    ) {
        let fam = crate::snapshots::family(doc);
        // Every knob on a controller.
        let mut list: Vec<Assign> = Vec::new();
        if let Some(ctl) = doc.pointer("/data/tone/controller").and_then(serde_json::Value::as_object) {
            for (dsp_key, blocks) in ctl {
                let Some(d) = dsp_key.strip_prefix("dsp").and_then(|n| n.parse::<i64>().ok()) else {
                    notes.push(format!("Controllers on {dsp_key} aren't carried over yet."));
                    continue;
                };
                let Some(blocks) = blocks.as_object() else { continue };
                for (key, knobs) in blocks {
                    let Some((_, _, slot, model)) = placed.iter().find(|p| p.0 == d && &p.1 == key) else {
                        notes.push(format!("Controllers on {key} (not an effect block) aren't carried over yet."));
                        continue;
                    };
                    let params = cat.default_values(*model);
                    let Some(knobs) = knobs.as_object() else { continue };
                    for (param, cfg) in knobs {
                        let Some(cfg) = cfg.as_object() else { continue };
                        let Some(controller) = cfg.get("@controller").and_then(serde_json::Value::as_i64) else { continue };
                        let trails = param == "@trails";
                        let (index, vt) = if trails {
                            (0, 2)
                        } else if let Some(i) = params.iter().position(|p| &p.0 == param) {
                            (i, params[i].1)
                        } else {
                            notes.push(format!(
                                "{}'s {param} has no place on the pedal; its controller was left out.",
                                cat.model_name(*model)
                            ));
                            continue;
                        };
                        list.push(Assign {
                            controller,
                            dsp: d,
                            slot: *slot,
                            key: key.clone(),
                            param: param.clone(),
                            index,
                            vt,
                            trails,
                            cfg: cfg.clone(),
                        });
                    }
                }
            }
        }
        list.sort_by(|a, b| {
            a.controller
                .cmp(&b.controller)
                .then((1000 * a.dsp + a.slot as i64).cmp(&(1000 * b.dsp + b.slot as i64)))
                .then(a.param.cmp(&b.param))
        });

        // Tone key 4: per controller number, its knobs.
        if let Some(Value::Array(table, fmt)) = self.tone.get(4).cloned() {
            let mut table: Vec<Value> = vec![Value::Nil; table.len()];
            for (t, a) in list.iter().enumerate() {
                let Some(row) = table.get_mut(a.controller as usize) else {
                    notes.push(format!("Controller {} has no place on this pedal; left out.", a.controller));
                    continue;
                };
                let cc = a.cfg.get("@cc").and_then(serde_json::Value::as_i64).unwrap_or(4);
                let rec = Value::map(vec![
                    (0, Value::int(t as i64)),
                    (
                        1,
                        Value::map(vec![
                            (0, Value::int(a.controller)),
                            (1, Value::int(cc)),
                            (2, typed_bound(a.cfg.get("@min"), a.vt, 0.0)),
                            (3, typed_bound(a.cfg.get("@max"), a.vt, 1.0)),
                            (4, Value::int(0)),
                            (5, Value::int(20 * a.dsp + a.slot as i64)),
                            (6, Value::map(vec![(28, Value::int(0)), (29, Value::int(a.index as i64)), (41, Value::Bool(a.trails))])),
                            (7, Value::int(0)),
                            (13, Value::Bool(a.cfg.get("@snapshot_disable").and_then(serde_json::Value::as_bool).unwrap_or(false))),
                        ]),
                    ),
                ]);
                match row {
                    Value::Array(items, _) => items.push(rec),
                    _ => *row = Value::Array(vec![rec], crate::msgpack::AUTO),
                }
            }
            if let Some(slot) = self.tone.get_mut(4) {
                *slot = Value::Array(table, fmt);
            }
        } else if !list.is_empty() {
            notes.push("This pedal's preset has no controller table; controllers were left out.".into());
        }

        // Each snapshot's values for the assigned knobs (field 2: [footswitch on, which, value] x 64).
        let snaps = self.snapshot_names().len();
        if let Some(Value::Array(snap_list, _)) = self.tone.at_mut(&[key::SNAPSHOT_SECTION, key::SNAPSHOTS]) {
            for (i, snap) in snap_list.iter_mut().enumerate().take(snaps) {
                let mut f: Vec<Value> =
                    (0..64).map(|_| Value::Array(vec![Value::Bool(false), Value::int(64), Value::Nil], crate::msgpack::AUTO)).collect();
                for (t, a) in list.iter().enumerate() {
                    let p = t;
                    if p >= 64 {
                        break;
                    }
                    let s = doc.pointer(&format!("/data/tone/snapshot{i}/controllers/dsp{}/{}/{}", a.dsp, a.key, a.param));
                    let on = s.and_then(|v| v.get("@fs_enabled")).and_then(serde_json::Value::as_bool).unwrap_or(false);
                    let value = match s.and_then(|v| v.get("@value")) {
                        Some(serde_json::Value::Bool(b)) => Value::Bool(*b),
                        Some(serde_json::Value::Number(n)) => {
                            let x = n.as_f64().unwrap_or(0.0);
                            if a.vt == 0 {
                                Value::int(x.round() as i64)
                            } else {
                                Value::F32(x as f32)
                            }
                        }
                        _ => Value::Nil,
                    };
                    f[p] = Value::Array(vec![Value::Bool(on), Value::int(t as i64), value], crate::msgpack::AUTO);
                }
                if let Some(slot) = snap.get_mut(2) {
                    *slot = Value::Array(f, crate::msgpack::AUTO);
                }
            }
        }

        // Tone key 3: footswitches (block bypass, and knobs on a footswitch).
        let Some(switches) = self.tone.at(&[3, 8]).and_then(Value::as_array).map(|a| a.len()) else { return };
        // (sort keys: primary flag, kind order, ordinal), record
        let mut per: Vec<Vec<FootswitchEntry>> = vec![Vec::new(); switches];
        let wrap = |n: &serde_json::Map<String, serde_json::Value>, inner: Value| -> Vec<(i64, Value)> {
            let custom = n.get("@fs_customlabel").and_then(serde_json::Value::as_str).unwrap_or("").to_owned();
            let colour = n.get("@fs_customcolor").and_then(serde_json::Value::as_i64).unwrap_or(0);
            vec![
                (10, Value::int(if n.get("@fs_primary").and_then(serde_json::Value::as_bool) == Some(true) { 0 } else { 1 })),
                (11, inner),
                (12, Value::Bool(n.get("@fs_momentary").and_then(serde_json::Value::as_bool).unwrap_or(false))),
                (13, Value::Bool(!custom.is_empty())),
                (14, Value::str(&format!("{custom}\0"))),
                (15, Value::Bool(colour != 0)),
                (16, Value::int(colour)),
            ]
        };
        if let Some(fs) = doc.pointer("/data/tone/footswitch").and_then(serde_json::Value::as_object) {
            for (dsp_key, blocks) in fs {
                let d: i64 = dsp_key.trim_start_matches("dsp").parse().unwrap_or(0);
                let Some(blocks) = blocks.as_object() else { continue };
                for (key, n) in blocks {
                    let Some(n) = n.as_object() else { continue };
                    if n.get("@fs_label").and_then(serde_json::Value::as_str).is_none() {
                        continue; // bypass on an expression pedal: not carried yet
                    }
                    let Some((_, _, slot, _)) = placed.iter().find(|p| p.0 == d && &p.1 == key) else { continue };
                    let Some(index) = n.get("@fs_index").and_then(serde_json::Value::as_i64) else { continue };
                    if index < 1 || index as usize > switches {
                        notes.push(format!("{key} is on footswitch {index}, which this pedal doesn't have; left out."));
                        continue;
                    }
                    let enabled = n.get("@fs_enabled").and_then(serde_json::Value::as_bool).unwrap_or(false);
                    let led = n.get("@fs_ledcolor").and_then(serde_json::Value::as_i64).unwrap_or(0);
                    let led = if n.get("@fs_customcolor").and_then(serde_json::Value::as_i64).unwrap_or(0) != 0 {
                        led
                    } else {
                        led_colour(led, enabled)
                    };
                    let ordinal = 20 * d + *slot as i64;
                    let inner = Value::map(vec![
                        (0, Value::int(1)),
                        (5, Value::str(&format!("{}\0", n.get("@fs_label").and_then(serde_json::Value::as_str).unwrap_or("")))),
                        (6, Value::int(led)),
                        (7, Value::Bool(enabled)),
                        (8, Value::int(ordinal)),
                        (2, Value::int(0)),
                    ]);
                    let primary = if n.get("@fs_primary").and_then(serde_json::Value::as_bool) == Some(true) { 0 } else { 1 };
                    per[index as usize - 1].push(((primary, 1, ordinal), wrap(n, inner)));
                }
            }
        }
        // Knobs on a footswitch (controller numbers fs_first.. are the switches).
        for a in &list {
            let n = a.controller - fam.fs_first + 1;
            if a.controller < fam.fs_first
                || n < 1
                || n as usize > switches
                || a.cfg.get("@fs_label").and_then(serde_json::Value::as_str).is_none()
            {
                continue;
            }
            let ordinal = 20 * a.dsp + a.slot as i64;
            let inner = Value::map(vec![
                (0, Value::int(2)),
                (5, Value::str(&format!("{}\0", a.cfg.get("@fs_label").and_then(serde_json::Value::as_str).unwrap_or("")))),
                (6, Value::int(a.cfg.get("@fs_ledcolor").and_then(serde_json::Value::as_i64).unwrap_or(0))),
                (7, Value::Bool(a.cfg.get("@fs_enabled").and_then(serde_json::Value::as_bool).unwrap_or(false))),
                (8, Value::int(ordinal)),
                (2, Value::int(0)),
                (9, Value::map(vec![(28, Value::int(0)), (29, Value::int(a.index as i64)), (41, Value::Bool(a.trails))])),
            ]);
            let primary = if a.cfg.get("@fs_primary").and_then(serde_json::Value::as_bool) == Some(true) { 0 } else { 1 };
            per[n as usize - 1].push(((primary, 2, ordinal), wrap(&a.cfg, inner)));
        }
        let table: Vec<Value> = per
            .into_iter()
            .map(|mut recs| {
                if recs.is_empty() {
                    return Value::Nil;
                }
                recs.sort_by_key(|r| r.0);
                let items = recs
                    .into_iter()
                    .enumerate()
                    .map(|(i, (_, mut fields))| {
                        fields[0] = (10, Value::int(i as i64));
                        Value::map(fields)
                    })
                    .collect();
                Value::Array(items, crate::msgpack::AUTO)
            })
            .collect();
        if let Some(t) = self.tone.at_mut(&[3, 8]) {
            *t = Value::Array(table, crate::msgpack::AUTO);
        }
    }
}

/// A model's knob values as the pedal stores them: `{2: count, 3: count, 4: [values]}`.
/// Decimals are 32-bit floats, whole numbers are integers, switches are true/false.
pub fn values_container(cat: &Catalog, model: i64) -> Value {
    let values: Vec<Value> = cat
        .default_values(model)
        .into_iter()
        .map(|(_, vt, d)| match vt {
            2 => Value::Bool(d == 1.0),
            0 => Value::int(d.round() as i64),
            _ => Value::F32(d as f32),
        })
        .collect();
    let n = values.len() as i64;
    Value::map(vec![(2, Value::int(n)), (3, Value::int(n)), (4, Value::Array(values, crate::msgpack::AUTO))])
}

/// A full slot map (`{19: kind, 20: body}`) for placing or live-adding a model.
/// Looper blocks (`kind_code == 22`) use outer kind `7` (`{8: model, 9: 22, 10: enabled, 7: values}`);
/// all other blocks use outer kind `6`.
pub fn block_slot(cat: &Catalog, model: i64, cab: Option<i64>, enabled: bool, with_values: bool) -> Value {
    if cab.is_none() && cat.kind_code(model) == 22 {
        let mut fields = vec![(key::INLINE_MODEL, Value::int(model)), (9, Value::int(22)), (key::ENABLED, Value::Bool(enabled))];
        if with_values {
            fields.push((key::IO_VALUES, values_container(cat, model)));
        }
        return Value::map(vec![(key::KIND, Value::int(7)), (key::BODY, Value::map(fields))]);
    }
    Value::map(vec![(key::KIND, Value::int(6)), (key::BODY, block_body(cat, model, cab, enabled, with_values))])
}

/// The body of an effect slot, built the way the pedal expects it:
/// model reference (key 24), kind (9), on/off (10), knobs (11) and the cab's knobs (12).
/// `with_values` false leaves the knobs out, which is how a live "add block" is sent.
pub fn block_body(cat: &Catalog, model: i64, cab: Option<i64>, enabled: bool, with_values: bool) -> Value {
    let kind = match cab {
        Some(c) => cat.paired_kind(model, c),
        None => cat.kind_code(model),
    };
    let reference = Value::map(vec![
        (23, Value::Bool(cab.is_some())),
        (key::MODEL, Value::int(model)),
        (key::PAIRED_MODEL, Value::int(cab.unwrap_or(-1))),
    ]);
    let mut fields = vec![(key::MODEL_REF, reference), (9, Value::int(kind)), (key::ENABLED, Value::Bool(enabled))];
    if with_values {
        fields.push((key::VALUES, values_container(cat, model)));
        let cab_values = match cab {
            Some(c) => values_container(cat, c),
            None => Value::map(vec![(2, Value::int(0)), (3, Value::int(0)), (4, Value::Array(Vec::new(), crate::msgpack::AUTO))]),
        };
        fields.push((key::PAIRED_VALUES, cab_values));
    }
    Value::map(fields)
}

fn read_values(array: &Value) -> Vec<Value> {
    array.get(key::ARRAY_VALUES).and_then(Value::as_array).cloned().unwrap_or_default()
}

fn read_slot(raw: &Value, path: i64, position: usize) -> SlotView {
    let kind = Kind::from_wire(raw.get(key::KIND).and_then(Value::as_i64).unwrap_or(-1));
    let Some(body) = raw.get(key::BODY) else {
        return SlotView { global: 0, path, position, kind, model: None, paired: None, enabled: None, values: Vec::new() };
    };
    let body = match kind {
        Kind::Split => body.get(key::SPLIT_BODY).unwrap_or(body),
        Kind::Join => body.get(key::JOIN_BODY).unwrap_or(body),
        _ => body,
    };
    let reference = body.get(key::MODEL_REF);
    let model = reference.and_then(|r| r.get(key::MODEL)).or_else(|| body.get(key::INLINE_MODEL)).and_then(Value::as_i64);
    let paired = reference.and_then(|r| r.get(key::PAIRED_MODEL)).and_then(Value::as_i64).filter(|n| *n >= 0);
    let values = body.get(key::VALUES).or_else(|| body.get(key::IO_VALUES)).map(read_values).unwrap_or_default();
    let enabled = match body.get(key::ENABLED) {
        Some(Value::Bool(b)) => Some(*b),
        _ => None,
    };
    SlotView { global: 0, path, position, kind, model, paired, enabled, values }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny made-up document: an unchanged round trip must be byte-exact and
    /// the offset table must point at each key.
    #[test]
    fn rebuilds_its_offset_table() {
        let mut pairs = Vec::new();
        for k in [0i64, 1, 2, 3, 4, 5, 6, 7, 10] {
            pairs.push((Value::int(k), Value::map(vec![(1, Value::int(k))])));
        }
        let doc = DevicePreset {
            magic: Value::str("l6-helix\0"),
            table_fmt: 0xc4,
            table: Vec::new(),
            tone: Value::Map(pairs, 0x80),
            rest: Vec::new(),
        };
        let blob = doc.encode();
        let back = DevicePreset::parse(&blob).unwrap();
        assert_eq!(back.encode(), blob);
        assert_eq!(back.encode_checked().unwrap(), blob);
        let t = &back.table;
        let tone_at = u32::from_le_bytes([t[0], t[1], t[2], t[3]]) as usize;
        assert_eq!(blob[tone_at], 0x89);
        let k0 = u32::from_le_bytes([t[4], t[5], t[6], t[7]]) as usize;
        assert_eq!(blob[k0], 0x00);
    }

    fn tiny(keys: &[i64]) -> DevicePreset {
        let pairs = keys.iter().map(|k| (Value::int(*k), Value::map(vec![(1, Value::int(*k))]))).collect();
        DevicePreset {
            magic: Value::str("l6-helix\0"),
            table_fmt: 0xc4,
            table: vec![0; 48],
            tone: Value::Map(pairs, 0x80),
            rest: Vec::new(),
        }
    }

    /// A section the offset table points at is missing: the stale table must never go to the pedal.
    #[test]
    fn refuses_to_send_a_stale_offset_table() {
        let doc = tiny(&[0, 1, 2]);
        assert!(doc.computed_table().is_none());
        assert!(doc.encode_checked().is_err());
    }

    /// Values after the tone are kept on a round trip, and such a document isn't sent.
    #[test]
    fn keeps_trailing_values() {
        let doc = tiny(&[0, 1, 2, 3, 4, 5, 6, 7, 10]);
        let mut blob = doc.encode();
        blob.extend(Value::int(42).encode());
        let back = DevicePreset::parse(&blob).unwrap();
        assert_eq!(back.encode(), blob);
        assert!(back.encode_checked().is_err());
    }

    #[test]
    fn wire_block_addresses_second_path_from_twenty() {
        let s0 = SlotView {
            global: 3,
            path: 0,
            position: 3,
            kind: Kind::Block,
            model: None,
            paired: None,
            enabled: Some(true),
            values: Vec::new(),
        };
        let s1 = SlotView {
            global: 15,
            path: 1,
            position: 1,
            kind: Kind::Block,
            model: None,
            paired: None,
            enabled: Some(true),
            values: Vec::new(),
        };
        assert_eq!(s0.wire_block(), 3);
        assert_eq!(s1.wire_block(), 21);
        assert_eq!(Kind::from_wire(7), Kind::Block);
    }
}

#[cfg(test)]
mod fixture_tests {
    use super::*;

    /// With HX_FIXTURE pointing at a preset document captured from a pedal,
    /// reading and writing it back unchanged must give the same bytes.
    #[test]
    fn real_document_round_trips() {
        let Ok(path) = std::env::var("HX_FIXTURE") else { return };
        let blob = std::fs::read(path).unwrap();
        let doc = DevicePreset::parse(&blob).unwrap_or_else(|e| panic!("parse: {e}"));
        assert!(!doc.slots().is_empty());
        assert_eq!(doc.encode(), blob, "round trip changed the bytes");
    }
}

#[cfg(test)]
mod editing_tests {
    use super::*;

    /// Offline chain editing on a real pedal document: add, move, remove.
    #[test]
    fn add_move_remove_offline() {
        let (Ok(path), Some(dir)) = (std::env::var("HX_FIXTURE"), crate::catalog::find_dir(None)) else { return };
        let cat = crate::catalog::Catalog::load(&dir).unwrap();
        let mut doc = DevicePreset::parse(&std::fs::read(path).unwrap()).unwrap();
        let empty = doc.slots().into_iter().find(|s| s.kind == Kind::Empty && s.path == 0).unwrap();
        let scream = cat.number_of_symbol("HD2_DistScream808Mono").unwrap();
        doc.place_model(&cat, 0, empty.position, scream, None);
        let back = DevicePreset::parse(&doc.encode()).unwrap();
        let s = back.slots().into_iter().find(|s| s.position == empty.position && s.path == 0).unwrap();
        assert_eq!(s.kind, Kind::Block);
        assert_eq!(s.model, Some(scream));
        assert_eq!(s.values.len(), 3, "Scream 808 has three knobs");
        let mut back = back;
        back.swap_slots(0, empty.position, empty.position + 1);
        assert_eq!(back.slots()[empty.global + 1].model, Some(scream));
        back.clear_slot(0, empty.position + 1);
        let again = DevicePreset::parse(&back.encode()).unwrap();
        assert_eq!(again.slots()[empty.global + 1].kind, Kind::Empty);
    }

    #[test]
    fn hlx_add_move_remove() {
        let Some(dir) = crate::catalog::find_dir(None) else { return };
        let cat = crate::catalog::Catalog::load(&dir).unwrap();
        let mut doc = crate::hlx::blank("t");
        let n = cat.number_of_symbol("HD2_DistScream808Mono").unwrap();
        let (m, st) = cat.hlx_model(n).unwrap();
        crate::hlx::place_model(
            &mut doc,
            "dsp0",
            0,
            2,
            None,
            &crate::hlx::NewModel { model: &m, stereo: st, params: &cat.default_values(n), cab: None },
        )
        .unwrap();
        let b = crate::hlx::blocks(&doc).into_iter().next().unwrap();
        assert_eq!(b.model, "HD2_DistScream808");
        assert_eq!(b.position, 2);
        crate::hlx::move_block(&mut doc, &b, 3);
        let b = crate::hlx::blocks(&doc).into_iter().next().unwrap();
        assert_eq!(b.position, 3);
        crate::hlx::remove_block(&mut doc, &b);
        assert!(crate::hlx::blocks(&doc).is_empty());
    }
}

#[cfg(test)]
impl DevicePreset {
    /// A tiny made-up document for tests in other modules.
    pub(crate) fn tiny_for_tests() -> DevicePreset {
        let pairs = [0i64, 1, 2].iter().map(|k| (Value::int(*k), Value::map(vec![(1, Value::int(*k))]))).collect();
        DevicePreset {
            magic: Value::str("l6-helix\0"),
            table_fmt: 0xc4,
            table: Vec::new(),
            tone: Value::Map(pairs, 0x80),
            rest: Vec::new(),
        }
    }
}

#[cfg(test)]
mod command_tests {
    use super::*;

    /// A tiny document with a Command Center of 13 empty slots, as the HX Stomp keeps it.
    fn with_commands() -> DevicePreset {
        let zeros = || Value::Array((0..7).map(|_| Value::int(0)).collect(), crate::msgpack::AUTO);
        let cc = Value::map(vec![
            (0, Value::Array((0..13).map(|_| Value::int(0)).collect(), crate::msgpack::AUTO)),
            (1, Value::Array((0..13).map(|_| zeros()).collect(), crate::msgpack::AUTO)),
        ]);
        let mut pairs: Vec<(Value, Value)> =
            [0i64, 1, 3, 4, 5, 6, 7, 10].iter().map(|k| (Value::int(*k), Value::map(vec![(1, Value::int(*k))]))).collect();
        pairs.insert(4, (Value::int(2), cc));
        DevicePreset {
            magic: Value::str("l6-helix\0"),
            table_fmt: 0xc4,
            table: vec![0; 48],
            tone: Value::Map(pairs, 0x80),
            rest: Vec::new(),
        }
    }

    #[test]
    fn a_command_is_set_read_and_survives_a_round_trip() {
        let mut d = with_commands();
        assert_eq!(d.command_count(), 13);
        assert_eq!(d.command(6), None);
        // FS 1 (slot 6): HX Preset, preset value 39.
        assert!(d.set_command(6, 14, &[39]));
        // FS 2 (slot 7): HX Snapshot, Press/Release, press = snapshot 3 (value 5).
        assert!(d.set_command(7, 15, &[0, 5, 0]));
        assert_eq!(d.command(6), Some((14, vec![39, 0, 0, 0, 0, 0])));
        assert_eq!(d.command(7), Some((15, vec![0, 5, 0, 0, 0, 0])));
        let back = DevicePreset::parse(&d.encode()).unwrap();
        assert_eq!(back.command(6), Some((14, vec![39, 0, 0, 0, 0, 0])));
        assert_eq!(back.command(7), Some((15, vec![0, 5, 0, 0, 0, 0])));
        // Emptying a slot clears its arguments too.
        assert!(d.set_command(6, 0, &[39]));
        assert_eq!(d.command(6), None);
        assert!(!d.set_command(13, 14, &[]), "there is no slot 13");
    }

    /// A document with the footswitch table's records for five Command Center switches.
    fn with_switches() -> DevicePreset {
        let mut d = with_commands();
        let rec = || {
            Value::Array(
                vec![Value::map(vec![
                    (10, Value::int(7)),
                    (
                        11,
                        Value::map(vec![(0, Value::int(3)), (5, Value::str("CC Value")), (6, Value::int(462860)), (7, Value::Bool(false))]),
                    ),
                    (12, Value::Bool(true)),
                ])],
                crate::msgpack::AUTO,
            )
        };
        let table = Value::map(vec![(7, Value::int(2)), (8, Value::Array((0..5).map(|_| rec()).collect(), crate::msgpack::AUTO))]);
        if let Value::Map(pairs, _) = &mut d.tone {
            for (k, v) in pairs.iter_mut() {
                if k.as_i64() == Some(3) {
                    *v = table.clone();
                }
            }
        }
        d
    }

    /// `HX_FIXTURE` is a pedal document, `HLX_FIXTURE` the Stomp preset "Wish acoustic" exported by another
    /// editor: its Command Center goes to the pedal document, with the wait held to 20 and the snapshots'
    /// own values.
    #[test]
    fn a_hlx_command_center_goes_to_the_pedal_document() {
        let (Ok(tpl), Ok(file), Some(dir)) = (std::env::var("HX_FIXTURE"), std::env::var("HLX_FIXTURE"), crate::catalog::find_dir(None))
        else {
            return;
        };
        let cat = crate::catalog::Catalog::load(&dir).unwrap();
        let template = DevicePreset::parse(&std::fs::read(tpl).unwrap()).unwrap();
        let json = crate::hlx::parse_text(&std::fs::read_to_string(file).unwrap()).unwrap();
        if json.pointer("/data/tone/commandInst1/@command").and_then(serde_json::Value::as_i64) != Some(18) {
            return;
        }
        let (out, notes) = DevicePreset::from_hlx(&template, &json, &cat).unwrap();
        assert_eq!(out.command(0), Some((18, vec![5, 55, 66, 20, 0, 0])), "wait held to 20");
        assert!(notes.iter().any(|n| n.contains("limit of 20")), "{notes:?}");
        assert_eq!(out.command(1), Some((19, vec![5, 7, 9, 11, 0, 0])));
        assert_eq!(out.command(2), Some((20, vec![2, 0, 0, 0, 0, 0])));
        assert_eq!(out.command(3), Some((13, vec![1, 3, 1, 3, 20, 0])));
        assert_eq!(out.command(4), Some((17, vec![4, 0, 0, 0, 0, 0])));
        assert_eq!(out.command(6), Some((2, vec![1, 0, 0, 0, 1, 0])), "FS 1: CC Toggle, momentary");
        assert_eq!(out.command_momentary(6), Some(true));
        assert_eq!(out.command(7), Some((2, vec![3, 22, 33, 11, 0, 33])), "FS 2: CC Toggle, latching");
        assert_eq!(out.command_momentary(7), Some(false));
        assert_eq!(out.command(8), Some((5, vec![4, 4, 22, 0, 0, 0])));
        assert_eq!(out.command(9), Some((15, vec![0, 4, 0, 0, 0, 0])));
        assert_eq!(out.command(10), Some((15, vec![1, 4, 5, 0, 0, 0])));
        assert_eq!(out.command(11), Some((3, vec![5, 23, 11, 99, 0, 0])));
        assert_eq!(out.command(12), None, "the file has no EXP 2");
        let back = DevicePreset::parse(&out.encode()).unwrap();
        assert_eq!(back.command(7), out.command(7));
        // Snapshot 1 keeps the file's own values for Instant 1.
        let rows = back.tone.at(&[10, 10]).and_then(Value::as_array).unwrap();
        let snap0 = DevicePreset::snapshot_command_rows(&rows[0]);
        assert_eq!(snap0[0], (0, false, vec![0, 5, 55, 66, 20, 0, 0]));
    }
    /// The pedal's own rows (captured from an HX Stomp): the id is in the list, rows start with 0 except
    /// HX Preset's, and a CC Toggle's fifth argument is its Momentary flag.
    #[test]
    fn rows_follow_the_pedals_own_layout_and_momentary_is_kept_in_two_places() {
        let mut d = with_switches();
        assert!(d.set_command(0, 18, &[5, 55, 66, 20]));
        assert!(d.set_command(6, 14, &[1]));
        let rows = d.tone.at(&[2, 1]).and_then(Value::as_array).unwrap();
        let first = |i: usize| rows[i].as_array().unwrap()[0].as_i64().unwrap();
        assert_eq!((first(0), first(6)), (0, 14), "only HX Preset repeats its id");
        assert_eq!(d.command(0), Some((18, vec![5, 55, 66, 20, 0, 0])));
        // A row the pedal wrote (id only in the list) is read too.
        assert_eq!(d.command(6), Some((14, vec![1, 0, 0, 0, 0, 0])));

        // A CC Toggle on FS 2 (slot 7): the switch is named and starts latching.
        assert!(d.set_command(7, 2, &[3, 22, 33, 11]));
        assert_eq!(d.command_momentary(7), Some(false));
        assert_eq!(d.command(7), Some((2, vec![3, 22, 33, 11, 0, 33])));
        let name = d.tone.at(&[3, 8]).and_then(Value::as_array).unwrap()[1].as_array().unwrap()[0].get(11).unwrap().get(5).unwrap();
        assert_eq!(name.as_str().map(|s| s.trim_end_matches('\0')), Some("CC Toggle"));
        // Momentary on: the flag and the fifth argument change, the current state goes back to 0.
        assert!(d.set_command_momentary(7, true));
        assert_eq!(d.command_momentary(7), Some(true));
        assert_eq!(d.command(7), Some((2, vec![3, 22, 33, 11, 1, 0])));
        // Editing a field keeps the flag.
        assert!(d.set_command(7, 2, &[4, 22, 33, 11]));
        assert_eq!(d.command(7), Some((2, vec![4, 22, 33, 11, 1, 0])));
        let back = DevicePreset::parse(&d.encode()).unwrap();
        assert_eq!(back.command_momentary(7), Some(true));
        assert_eq!(back.command(7), d.command(7));
        // A Hotkey on FS 3 (slot 8) starts momentary, with the flag in its fifth argument and its own name.
        assert!(d.set_command(8, 12, &[3, 1, 4, 5]));
        assert_eq!(d.command_momentary(8), Some(true));
        assert_eq!(d.command(8), Some((12, vec![3, 1, 4, 5, 1, 0])));
        let name = d.tone.at(&[3, 8]).and_then(Value::as_array).unwrap()[2].as_array().unwrap()[0].get(11).unwrap().get(5).unwrap();
        assert_eq!(name.as_str().map(|s| s.trim_end_matches('\0')), Some("Hotkey"));
        // A slot that isn't a footswitch has no flag.
        assert_eq!(d.command_momentary(0), None);
        assert!(!d.set_command_momentary(0, true));
    }

    /// The presets read from a real HX Stomp: "Shine on 2" has HX Preset on FS 1 and FS 2.
    /// `HX_COMMANDS_FIXTURE` is such a pedal document.
    #[test]
    fn a_real_presets_commands_are_read() {
        let Ok(path) = std::env::var("HX_COMMANDS_FIXTURE") else { return };
        let d = DevicePreset::parse(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(d.command_count(), 13);
        assert_eq!(d.command(6), Some((14, vec![1, 0, 0, 0, 0, 0])), "FS 1: previous preset");
        assert_eq!(d.command(7), Some((14, vec![0, 0, 0, 0, 0, 0])), "FS 2: next preset");
        assert_eq!(d.command(0), None);
        // Editing one slot and re-encoding keeps the rest: the other command, the blocks, the names.
        let mut e = d.clone();
        assert!(e.set_command(6, 15, &[0, 4, 0]));
        let back = DevicePreset::parse(&e.encode()).unwrap();
        assert_eq!(back.command(6), Some((15, vec![0, 4, 0, 0, 0, 0])));
        assert_eq!(back.command(7), d.command(7));
        assert_eq!(back.snapshot_names(), d.snapshot_names());
        assert_eq!(
            back.slots().iter().map(|s| (s.kind, s.model)).collect::<Vec<_>>(),
            d.slots().iter().map(|s| (s.kind, s.model)).collect::<Vec<_>>()
        );
    }
}

#[cfg(test)]
mod setting_tests {
    use super::*;

    /// `HX_FIXTURE` is a pedal document, `HLX_FIXTURE` a .hlx: the file's guitar impedance and pad go to the
    /// pedal document's settings (fields 30 and 134), and a file without them leaves the template's.
    #[test]
    fn guitar_impedance_and_pad_are_carried_from_a_hlx() {
        let (Ok(tpl), Ok(file), Some(dir)) = (std::env::var("HX_FIXTURE"), std::env::var("HLX_FIXTURE"), crate::catalog::find_dir(None))
        else {
            return;
        };
        let cat = crate::catalog::Catalog::load(&dir).unwrap();
        let template = DevicePreset::parse(&std::fs::read(tpl).unwrap()).unwrap();
        let mut json = crate::hlx::parse_text(&std::fs::read_to_string(file).unwrap()).unwrap();
        json["data"]["tone"]["global"]["@guitarinputZ"] = serde_json::json!(8);
        json["data"]["tone"]["global"]["@guitarpad"] = serde_json::json!(2);
        let (out, notes) = DevicePreset::from_hlx(&template, &json, &cat).unwrap();
        assert_eq!((out.setting_int(key::GUITAR_IN_Z), out.setting_int(key::GUITAR_PAD)), (Some(8), Some(2)));
        assert!(!notes.iter().any(|n| n.contains("guitar")), "{notes:?}");
        let back = DevicePreset::parse(&out.encode()).unwrap();
        assert_eq!((back.setting_int(key::GUITAR_IN_Z), back.setting_int(key::GUITAR_PAD)), (Some(8), Some(2)));
    }
}

#[cfg(test)]
mod snapshot_command_tests {
    use super::*;

    fn row(head: i64, vals: &[i64]) -> Value {
        Value::Array(
            vec![Value::int(head), Value::Bool(false), Value::Array(vals.iter().map(|v| Value::int(*v)).collect(), crate::msgpack::AUTO)],
            crate::msgpack::AUTO,
        )
    }

    /// A document with 13 empty Command Center slots and three snapshots, each with 13 unused records.
    fn doc() -> DevicePreset {
        let zeros = || Value::Array((0..7).map(|_| Value::int(0)).collect(), crate::msgpack::AUTO);
        let cc = Value::map(vec![
            (0, Value::Array((0..13).map(|_| Value::int(0)).collect(), crate::msgpack::AUTO)),
            (1, Value::Array((0..13).map(|_| zeros()).collect(), crate::msgpack::AUTO)),
        ]);
        // A snapshot is a map: 0 valid, 1 the command records.
        let snap = || {
            Value::map(vec![(0, Value::Bool(true)), (1, Value::Array((0..13).map(|_| row(13, &[0; 7])).collect(), crate::msgpack::AUTO))])
        };
        let snaps = Value::map(vec![(10, Value::Array((0..3).map(|_| snap()).collect(), crate::msgpack::AUTO))]);
        let mut pairs: Vec<(Value, Value)> =
            [0i64, 1, 3, 4, 5, 6, 7].iter().map(|k| (Value::int(*k), Value::map(vec![(1, Value::int(*k))]))).collect();
        pairs.insert(4, (Value::int(2), cc));
        pairs.push((Value::int(10), snaps));
        DevicePreset {
            magic: Value::str("l6-helix\0"),
            table_fmt: 0xc4,
            table: vec![0; 48],
            tone: Value::Map(pairs, 0x80),
            rest: Vec::new(),
        }
    }

    fn rows(d: &DevicePreset, snap: usize) -> Vec<(i64, Vec<i64>)> {
        let s = d.tone.at(&[10, 10]).and_then(Value::as_array).unwrap();
        DevicePreset::snapshot_command_rows(&s[snap]).into_iter().map(|(h, _, v)| (h, v)).collect()
    }

    #[test]
    fn snapshot_commands_get_one_record_per_snapshot_in_slot_order_with_padding() {
        let mut d = doc();
        assert!(d.set_command(7, 15, &[0, 4, 0]));
        for n in 0..3 {
            let r = rows(&d, n);
            assert_eq!(r.len(), 13);
            assert_eq!(r[0], (7, vec![0, 0, 4, 0, 0, 0, 0]));
            assert!(r[1..].iter().all(|(h, v)| *h == 13 && v.iter().all(|x| *x == 0)), "padding after the record");
        }
        // A second command sorts before it; the first keeps its values.
        assert!(d.set_command(6, 15, &[0, 3, 0]));
        let r = rows(&d, 0);
        assert_eq!((r[0].0, r[1].0), (6, 7));
        assert_eq!(r[0].1, vec![0, 0, 3, 0, 0, 0, 0]);
    }

    #[test]
    fn a_snapshots_own_value_survives_changes_to_other_slots_and_resets_with_its_own() {
        let mut d = doc();
        d.set_command(6, 15, &[0, 3, 0]);
        d.set_command(7, 15, &[0, 4, 0]);
        // Snapshot 2 presses snapshot 3 on slot 7 instead of the base 4.
        if let Some(Value::Array(rows, _)) =
            d.tone.at_mut(&[10, 10]).and_then(Value::as_array_mut).and_then(|a| a.get_mut(1)).and_then(|s| s.get_mut(1))
        {
            rows[1] = row(7, &[0, 0, 5, 0, 0, 0, 0]);
        }
        // Changing slot 6 leaves slot 7's own value alone.
        d.set_command(6, 15, &[0, 2, 0]);
        assert_eq!(rows(&d, 1)[0], (6, vec![0, 0, 2, 0, 0, 0, 0]));
        assert_eq!(rows(&d, 1)[1], (7, vec![0, 0, 5, 0, 0, 0, 0]));
        // Changing slot 7 itself starts it again from its base.
        d.set_command(7, 15, &[0, 4, 0]);
        assert_eq!(rows(&d, 1)[1], (7, vec![0, 0, 4, 0, 0, 0, 0]));
    }

    #[test]
    fn a_value_set_in_one_snapshot_leaves_the_others_and_the_type_alone() {
        let mut d = doc();
        d.set_command(7, 15, &[0, 4, 0]);
        let current = d.current_snapshot();
        let other = if current == 1 { 2 } else { 1 };
        assert!(d.set_command_values(other, 7, &[0, 5, 0]));
        assert_eq!(d.snapshot_command_args(other, 7), Some(vec![0, 5, 0, 0, 0, 0]));
        assert_eq!(d.snapshot_command_args(current, 7), Some(vec![0, 4, 0, 0, 0, 0]));
        assert_eq!(d.command(7), Some((15, vec![0, 4, 0, 0, 0, 0])), "the row follows the selected snapshot only");
        // In the selected snapshot the row changes too.
        assert!(d.set_command_values(current, 7, &[0, 3, 0]));
        assert_eq!(d.command(7), Some((15, vec![0, 3, 0, 0, 0, 0])));
        assert_eq!(d.snapshot_command_args(other, 7), Some(vec![0, 5, 0, 0, 0, 0]));
        // HX Preset has one value for all.
        d.set_command(6, 14, &[2]);
        assert!(d.set_command_values(other, 6, &[7]));
        assert_eq!(d.command(6), Some((14, vec![7, 0, 0, 0, 0, 0])));
        assert_eq!(d.snapshot_command_args(other, 6), None);
        assert!(!d.set_command_values(other, 0, &[1]), "no command in slot 0");
    }
    #[test]
    fn hx_preset_has_no_record_and_clearing_a_command_removes_its_record() {
        let mut d = doc();
        d.set_command(6, 14, &[5]);
        assert!(rows(&d, 0).iter().all(|(h, _)| *h == 13), "an HX Preset command has no per-snapshot record");
        d.set_command(7, 15, &[0, 4, 0]);
        d.set_command(6, 15, &[0, 3, 0]);
        assert_eq!(rows(&d, 2).iter().filter(|(h, _)| *h != 13).count(), 2);
        d.set_command(6, 0, &[]);
        assert_eq!(rows(&d, 2).iter().filter(|(h, _)| *h != 13).count(), 1);
        // From a snapshot command to HX Preset, the record goes.
        d.set_command(7, 14, &[0]);
        assert_eq!(rows(&d, 2).iter().filter(|(h, _)| *h != 13).count(), 0);
    }

    /// Every real preset's records are already what the rules produce: nothing is rewritten.
    /// `HX_PRESETS_DIR` is a folder of pedal documents (`.hxpreset`).
    #[test]
    fn the_rules_reproduce_the_records_of_every_real_preset() {
        let Ok(dir) = std::env::var("HX_PRESETS_DIR") else { return };
        let mut seen = 0;
        let mut with_records = 0;
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) != Some("hxpreset") {
                continue;
            }
            let mut d = DevicePreset::parse(&std::fs::read(&path).unwrap()).unwrap();
            let before = d.encode();
            d.sync_snapshot_commands(None);
            assert_eq!(d.encode(), before, "{} would be rewritten", path.display());
            seen += 1;
            with_records += (0..3).any(|n| rows(&d, n).iter().any(|(h, _)| *h != 13)) as usize;
        }
        assert!(seen > 0);
        println!("{seen} presets unchanged; {with_records} of them have per-snapshot records");
    }
}
