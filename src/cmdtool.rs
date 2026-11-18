//! Tools for working out the pedal document's Command Center: print a preset's commands as the
//! pedal stores them, and line them up with an HX Edit `.hlx` export of the same preset to learn
//! which argument holds which field. Read-only; used through `tonestrand --commands` and
//! `tonestrand --cc-compare`.

use serde_json::Value as Json;

use crate::controllers::{self, CmdSlot};
use crate::devpreset::DevicePreset;
use crate::msgpack::Value;

/// One command of a pedal document: its slot (0-based, in the document's order), id and the six arguments.
#[derive(Clone, Debug, PartialEq)]
pub struct DevCmd {
    pub slot: usize,
    pub id: i64,
    pub args: Vec<i64>,
}

/// One command of a `.hlx` file: the slot key (`commandFS4`), its id and its fields, as whole numbers.
#[derive(Clone, Debug, PartialEq)]
pub struct HlxCmd {
    pub key: String,
    pub id: i64,
    pub fields: Vec<(String, i64)>,
}

/// A snapshot's own command values: snapshot number (0-based), slot key or index, and what it holds.
pub type Override = (usize, String, String);

/// The commands set in a pedal document, in slot order.
pub fn device_commands(p: &DevicePreset) -> Vec<DevCmd> {
    (0..p.command_count()).filter_map(|slot| p.command(slot).map(|(id, args)| DevCmd { slot, id, args })).collect()
}

/// The per-snapshot command entries that are not at their defaults (`[13, false, [0 x 7]]`).
pub fn device_overrides(p: &DevicePreset) -> Vec<Override> {
    let mut out = Vec::new();
    let Some(snaps) = p.tone.at(&[10, 10]).and_then(Value::as_array) else { return out };
    for (n, snap) in snaps.iter().enumerate() {
        let Some(rows) = snap.get(1).and_then(Value::as_array) else { continue };
        for (slot, row) in rows.iter().enumerate() {
            let Some(r) = row.as_array() else { continue };
            let head = r.first().and_then(Value::as_i64).unwrap_or(13);
            let on = matches!(r.get(1), Some(Value::Bool(true)));
            let vals: Vec<i64> =
                r.get(2).and_then(Value::as_array).map(|a| a.iter().map(|v| v.as_i64().unwrap_or(0)).collect()).unwrap_or_default();
            // An unused entry is [13, false, zeros] in a stored preset and [0, false, zeros] in the pedal's own copy.
            if on || vals.iter().any(|v| *v != 0) {
                // The first number is the slot the entry is for (13 means an unused entry).
                let target = if head < 13 { format!("slot {head}") } else { format!("list position {slot}") };
                out.push((n, target, format!("flag {on}, values {vals:?}")));
            }
        }
    }
    out
}

fn number(v: &Json) -> Option<i64> {
    match v {
        Json::Bool(b) => Some(*b as i64),
        Json::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f.round() as i64)),
        _ => None,
    }
}

fn fields_of(obj: &serde_json::Map<String, Json>) -> Vec<(String, i64)> {
    obj.iter()
        .filter(|(k, _)| k.starts_with('@') && *k != "@command" && !k.starts_with("@fs_"))
        .filter_map(|(k, v)| number(v).map(|n| (k.clone(), n)))
        .collect()
}

/// The commands set in a `.hlx` file (`data.tone.command...` with a `@command` above 0).
pub fn hlx_commands(doc: &Json) -> Vec<HlxCmd> {
    let Some(tone) = doc.pointer("/data/tone").and_then(Json::as_object) else { return Vec::new() };
    let mut out: Vec<HlxCmd> = tone
        .iter()
        .filter(|(k, _)| k.starts_with("command"))
        .filter_map(|(k, v)| {
            let obj = v.as_object()?;
            let id = obj.get("@command").and_then(number).filter(|i| *i > 0)?;
            Some(HlxCmd { key: k.clone(), id, fields: fields_of(obj) })
        })
        .collect();
    out.sort_by(|a, b| a.key.cmp(&b.key));
    out
}

/// Each snapshot's own command values in a `.hlx` file (`snapshotN/commands/<key>`).
pub fn hlx_overrides(doc: &Json) -> Vec<Override> {
    let mut out = Vec::new();
    let Some(tone) = doc.pointer("/data/tone").and_then(Json::as_object) else { return out };
    for (k, v) in tone {
        let Some(n) = k.strip_prefix("snapshot").and_then(|s| s.parse::<usize>().ok()) else { continue };
        let Some(cmds) = v.get("commands").and_then(Json::as_object) else { continue };
        for (key, c) in cmds {
            if let Some(o) = c.as_object() {
                out.push((n, key.clone(), format!("{:?}", fields_of(o))));
            }
        }
    }
    out.sort();
    out
}

/// What a command id is called (from the Command Center definitions).
pub fn command_name(id: i64) -> String {
    for kind in [controllers::SlotKind::Switch, controllers::SlotKind::Instant, controllers::SlotKind::Pedal] {
        if let Some(n) = controllers::command_name(kind, id) {
            return n;
        }
    }
    format!("command {id}")
}

fn trigger_names(slots: &[CmdSlot], i: usize) -> String {
    slots.get(i).map(|s| s.title.clone()).unwrap_or_else(|| format!("slot {i}"))
}

/// The pedal's commands, one line each, with the trigger each slot is taken to be.
pub fn describe(p: &DevicePreset, slots: &[CmdSlot]) -> Vec<String> {
    let mut lines = vec![format!("{} Command Center slots in the document", p.command_count())];
    for c in device_commands(p) {
        lines.push(format!(
            "  slot {:>2} ({}): {} (id {}) args {:?}",
            c.slot,
            trigger_names(slots, c.slot),
            command_name(c.id),
            c.id,
            c.args
        ));
    }
    if device_commands(p).is_empty() {
        lines.push("  no commands set".into());
    }
    for (n, what, v) in device_overrides(p) {
        let trigger = what
            .strip_prefix("slot ")
            .and_then(|s| s.parse::<usize>().ok())
            .map(|i| format!(" ({})", trigger_names(slots, i)))
            .unwrap_or_default();
        lines.push(format!("  snapshot {} has its own values for {what}{trigger}: {v}", n + 1));
    }
    lines
}

/// Line up a `.hlx` export with the pedal's document: is each command in the slot its trigger is taken
/// to be, and which argument holds each field. A field that equals several arguments (every zero, for
/// one) is reported as ambiguous: choose distinct values in HX Edit to settle it.
pub fn compare(dev: &[DevCmd], hlx: &[HlxCmd], slots: &[CmdSlot]) -> Vec<String> {
    let mut lines = Vec::new();
    let mut used = vec![false; dev.len()];
    for h in hlx {
        let expected = slots.iter().position(|s| s.key == h.key);
        let fields = h.fields.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(" ");
        lines.push(format!(
            "{} ({}): {} (id {}) {fields}",
            h.key,
            expected.map(|i| format!("expected slot {i}")).unwrap_or_else(|| "unknown trigger".into()),
            command_name(h.id),
            h.id
        ));
        // The row in the expected slot, else any row with the same id and the same values.
        let at_expected = expected.and_then(|e| dev.iter().position(|d| d.slot == e));
        let same_id: Vec<usize> = (0..dev.len()).filter(|i| dev[*i].id == h.id).collect();
        let pick = match at_expected {
            Some(i) if dev[i].id == h.id => {
                lines.push(format!("  slot matches: the pedal has id {} in slot {}", h.id, dev[i].slot));
                Some(i)
            }
            Some(i) => {
                lines.push(format!("  DIFFERENT: slot {} holds id {} ({})", dev[i].slot, dev[i].id, command_name(dev[i].id)));
                None
            }
            None => None,
        }
        .or_else(|| {
            let free: Vec<usize> = same_id.iter().copied().filter(|i| !used[*i]).collect();
            // Prefer the row whose arguments contain the most of the file's values.
            let best = free.iter().copied().max_by_key(|i| h.fields.iter().filter(|(_, v)| dev[*i].args.contains(v)).count());
            if let Some(i) = best {
                lines.push(format!(
                    "  not in its expected slot: the best match with id {} is slot {} ({})",
                    h.id,
                    dev[i].slot,
                    trigger_names(slots, dev[i].slot)
                ));
            } else {
                lines.push("  NOT FOUND on the pedal".into());
            }
            best
        });
        let Some(i) = pick else { continue };
        used[i] = true;
        let args = &dev[i].args;
        lines.push(format!("  pedal arguments {args:?}"));
        let mut taken = vec![false; args.len()];
        for (name, v) in &h.fields {
            let at: Vec<usize> = (0..args.len()).filter(|a| args[*a] == *v).collect();
            match at.as_slice() {
                [one] => {
                    taken[*one] = true;
                    lines.push(format!("    {name} = {v}  ->  arg{}", one + 1));
                }
                [] => lines.push(format!("    {name} = {v}  ->  no argument holds this")),
                many => lines.push(format!(
                    "    {name} = {v}  ->  ambiguous: {}",
                    many.iter().map(|a| format!("arg{}", a + 1)).collect::<Vec<_>>().join(" or ")
                )),
            }
        }
        let rest: Vec<String> =
            (0..args.len()).filter(|a| !taken[*a] && args[*a] != 0).map(|a| format!("arg{}={}", a + 1, args[a])).collect();
        if !rest.is_empty() {
            lines.push(format!("    pedal values no field explains: {}", rest.join(" ")));
        }
    }
    for (i, d) in dev.iter().enumerate().filter(|(i, _)| !used[*i]) {
        let _ = i;
        lines.push(format!(
            "slot {} ({}): {} (id {}) args {:?}: no command in the .hlx",
            d.slot,
            trigger_names(slots, d.slot),
            command_name(d.id),
            d.id,
            d.args
        ));
    }
    lines
}

/// The HX Stomp's Command Center triggers in the order of the pedal document's slots (the only layout read so far).
pub fn unit_slots() -> Vec<CmdSlot> {
    controllers::unit_slots()
}

/// "42B" (a slot label) to the slot's index.
pub fn slot_index(label: &str) -> Option<usize> {
    let b = label.trim().as_bytes();
    if b.len() != 3 || !b[0].is_ascii_digit() || !b[1].is_ascii_digit() {
        return None;
    }
    let n: usize = label.trim()[..2].parse().ok()?;
    let letter = match b[2].to_ascii_uppercase() {
        b'A' => 0,
        b'B' => 1,
        b'C' => 2,
        _ => return None,
    };
    (n >= 1).then(|| (n - 1) * 3 + letter)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn slots() -> Vec<CmdSlot> {
        unit_slots()
    }

    #[test]
    fn slot_labels_become_indexes() {
        assert_eq!(slot_index("42B"), Some(124));
        assert_eq!(slot_index("01a"), Some(0));
        assert_eq!(slot_index("00A"), None);
        assert_eq!(slot_index("42"), None);
    }

    #[test]
    fn hlx_commands_are_read_with_their_numeric_fields() {
        let doc = json!({"data": {"tone": {
            "commandFS4": {"@command": 14, "@preset": 5, "@fs_label": "x"},
            "commandInst1": {"@command": 18, "@channel": 3, "@cc": 20, "@value": 99, "@waittime": 0},
            "commandFS1": {"@command": 0},
            "snapshot1": {"commands": {"commandFS4": {"@preset": 7}}}
        }}});
        let cmds = hlx_commands(&doc);
        assert_eq!(cmds.iter().map(|c| c.key.as_str()).collect::<Vec<_>>(), vec!["commandFS4", "commandInst1"]);
        assert_eq!(cmds[0].fields, vec![("@preset".to_owned(), 5)]);
        assert_eq!(cmds[1].fields.len(), 4);
        assert_eq!(hlx_overrides(&doc), vec![(1, "commandFS4".into(), "[(\"@preset\", 7)]".into())]);
    }

    #[test]
    fn distinct_values_tell_which_argument_holds_which_field() {
        let s = slots();
        // A MIDI CC on Instant 1 with distinct values, as the pedal might store it (order unknown to the code).
        let hlx = vec![HlxCmd {
            key: "commandInst1".into(),
            id: 18,
            fields: vec![("@channel".into(), 3), ("@cc".into(), 20), ("@value".into(), 99), ("@waittime".into(), 0)],
        }];
        let dev = vec![DevCmd { slot: 0, id: 18, args: vec![3, 20, 99, 0, 0, 0] }];
        let out = compare(&dev, &hlx, &s).join("\n");
        assert!(out.contains("slot matches"), "{out}");
        assert!(out.contains("@channel = 3  ->  arg1"), "{out}");
        assert!(out.contains("@cc = 20  ->  arg2"), "{out}");
        assert!(out.contains("@value = 99  ->  arg3"), "{out}");
        assert!(out.contains("@waittime = 0  ->  ambiguous"), "zeros can't be told apart: {out}");
    }

    #[test]
    fn a_command_in_another_slot_is_found_and_reported() {
        let s = slots();
        let hlx = vec![HlxCmd { key: "commandFS1".into(), id: 15, fields: vec![("@press".into(), 5)] }];
        // FS 1 is expected at slot 8, but the pedal has it at slot 7.
        let dev = vec![DevCmd { slot: 7, id: 15, args: vec![0, 5, 0, 0, 0, 0] }];
        let out = compare(&dev, &hlx, &s).join("\n");
        assert!(out.contains("not in its expected slot") && out.contains("slot 7"), "{out}");
        assert!(out.contains("@press = 5  ->  arg2"), "{out}");
        // And a command the file lacks is listed too.
        let out2 = compare(&[DevCmd { slot: 3, id: 18, args: vec![1; 6] }], &[], &s).join("\n");
        assert!(out2.contains("no command in the .hlx"), "{out2}");
    }

    /// With a real pedal document that has HX Preset on FS 1 and FS 2 (`HX_COMMANDS_FIXTURE`).
    #[test]
    fn a_real_preset_is_described() {
        let Ok(path) = std::env::var("HX_COMMANDS_FIXTURE") else { return };
        let p = DevicePreset::parse(&std::fs::read(path).unwrap()).unwrap();
        let text = describe(&p, &slots()).join("\n");
        assert!(text.contains("slot  6 (FS 1): HX Preset (id 14) args [1, 0, 0, 0, 0, 0]"), "{text}");
        assert!(text.contains("slot  7 (FS 2): HX Preset (id 14) args [0, 0, 0, 0, 0, 0]"), "{text}");
        let hlx = vec![HlxCmd { key: "commandFS4".into(), id: 14, fields: vec![("@preset".into(), 1)] }];
        let out = compare(&device_commands(&p), &hlx, &slots()).join("\n");
        assert!(out.contains("@preset = 1  ->  arg1"), "{out}");
    }
}
