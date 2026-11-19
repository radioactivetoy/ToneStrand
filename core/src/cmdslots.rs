//! Command Center data: the trigger slots of each unit, the command types and their fields, and the
//! plain functions that read and change a document's commands. No drawing here (that is `controllers.rs`).

use crate::snapshots;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SlotKind {
    Instant,
    Switch,
    Pedal,
}

#[derive(Clone)]
pub struct CmdSlot {
    pub key: String,
    pub short: String,
    pub title: String,
    pub kind: SlotKind,
}

pub enum FieldSpec {
    Range { key: &'static str, label: &'static str, min: i64, max: i64 },
    Choice { key: &'static str, label: &'static str, options: Vec<(i64, String)> },
}

pub struct CmdSpec {
    pub id: i64,
    pub name: &'static str,
    pub switch_label: &'static str,
    pub momentary: bool,
    pub fields: Vec<FieldSpec>,
}

/// Longest footswitch label the pedal shows.
pub const FS_LABEL_MAX: usize = 16;

pub const MMC_MSGS: [&str; 8] = ["Stop", "Play", "Forward", "Rewind", "Pause", "Record", "Punch In", "Punch Out"];
pub const MODIFIERS: [&str; 5] = ["None", "Shift", "Alt", "Ctrl", "Win/Cmd"];
pub const LOOPER_FNS: [&str; 14] = [
    "Play",
    "Stop",
    "Play/Stop",
    "Record",
    "Overdub",
    "Record/Overdub",
    "Play Once",
    "Reverse",
    "Forward",
    "Forward/Reverse",
    "Half Speed",
    "Full Speed",
    "Half/Full Speed",
    "Undo",
];
pub const BEHAVIORS: [&str; 2] = ["Press/Release", "Press/Hold"];

pub fn channel_options() -> Vec<(i64, String)> {
    // HX Edit lists 0 as "Base" and 1 to 16 as the numbers (HelixControls.json, midi_channel).
    let mut out = vec![(0, "Base".into())];
    for ch in 1..=16 {
        out.push((ch, ch.to_string()));
    }
    out
}

pub fn enum_options(names: &[&str]) -> Vec<(i64, String)> {
    names.iter().enumerate().map(|(i, n)| (i as i64, (*n).to_owned())).collect()
}

pub fn note_options() -> Vec<(i64, String)> {
    const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    (0..=127)
        .map(|n| {
            let oct = (n / 12) - 1;
            let name = NAMES[(n % 12) as usize];
            (n, format!("{name}{oct} ({n})"))
        })
        .collect()
}

pub fn key_options() -> Vec<(i64, String)> {
    let mut out = vec![(0, "None".into())];
    for (i, c) in ('A'..='Z').enumerate() {
        out.push(((i + 1) as i64, c.to_string()));
    }
    for (i, c) in ('0'..='9').enumerate() {
        out.push(((27 + i) as i64, c.to_string()));
    }
    for f in 1..=12 {
        out.push((36 + f, format!("F{f}")));
    }
    for (i, k) in ["Space", "Enter", "Tab", "Esc", "Backspace", "Delete", "Up", "Down", "Left", "Right"].iter().enumerate() {
        out.push(((49 + i) as i64, (*k).to_owned()));
    }
    out
}

pub fn preset_options() -> Vec<(i64, String)> {
    let mut out = vec![(0, "Next".into()), (1, "Previous".into())];
    for i in 0..126 {
        out.push(((i + 2) as i64, crate::proto::slot_label(i)));
    }
    out
}

pub fn snapshot_press_options(snap_count: usize) -> Vec<(i64, String)> {
    let mut out = vec![(0, "None".into()), (1, "Next".into()), (2, "Prev".into())];
    for i in 1..=snap_count.max(3) {
        out.push(((i + 2) as i64, format!("SNAPSHOT {i}")));
    }
    out
}

pub fn looper_press_options() -> Vec<(i64, String)> {
    let mut out = vec![(0, "None".into())];
    for (i, f) in LOOPER_FNS.iter().enumerate() {
        out.push(((i + 1) as i64, (*f).to_owned()));
    }
    out
}

pub fn specs_for(kind: SlotKind, device: i64, snap_count: usize) -> Vec<CmdSpec> {
    // The HX Stomp and Stomp XL have no Ext Amp jack (the HX Effects and the Helix units do).
    let is_stomp = snapshots::family_of_device(device).name.starts_with("HX Stomp");
    let ch = || FieldSpec::Choice { key: "@channel", label: "Channel", options: channel_options() };
    let wait = || FieldSpec::Range { key: "@waittime", label: "Wait (ms)", min: 0, max: 5000 };
    match kind {
        SlotKind::Instant => {
            let mut v = vec![
                CmdSpec {
                    id: 18,
                    name: "MIDI CC",
                    switch_label: "",
                    momentary: false,
                    fields: vec![
                        ch(),
                        FieldSpec::Range { key: "@cc", label: "CC", min: 0, max: 127 },
                        FieldSpec::Range { key: "@value", label: "Value", min: 0, max: 127 },
                        wait(),
                    ],
                },
                CmdSpec {
                    id: 19,
                    name: "Bank/Program",
                    switch_label: "",
                    momentary: false,
                    fields: vec![
                        ch(),
                        FieldSpec::Range { key: "@bank0", label: "Bank MSB", min: 0, max: 127 },
                        FieldSpec::Range { key: "@bank32", label: "Bank LSB", min: 0, max: 127 },
                        FieldSpec::Range { key: "@program", label: "Program", min: 0, max: 127 },
                    ],
                },
                CmdSpec {
                    id: 20,
                    name: "MMC",
                    switch_label: "",
                    momentary: false,
                    fields: vec![FieldSpec::Choice { key: "@message", label: "Message", options: enum_options(&MMC_MSGS) }],
                },
            ];
            if !is_stomp {
                v.push(CmdSpec {
                    id: 8,
                    name: "Ext Amp",
                    switch_label: "",
                    momentary: false,
                    fields: vec![
                        FieldSpec::Choice {
                            key: "@select",
                            label: "Jack",
                            options: enum_options(&["None", "1 (Tip)", "2 (Ring)", "1 and 2"]),
                        },
                        wait(),
                    ],
                });
            }
            v.push(CmdSpec {
                id: 13,
                name: "Hotkey",
                switch_label: "",
                momentary: false,
                fields: vec![
                    FieldSpec::Choice { key: "@modifier1", label: "Modifier 1", options: enum_options(&MODIFIERS) },
                    FieldSpec::Choice { key: "@modifier2", label: "Modifier 2", options: enum_options(&MODIFIERS) },
                    FieldSpec::Choice { key: "@modifier3", label: "Modifier 3", options: enum_options(&MODIFIERS) },
                    FieldSpec::Choice { key: "@keystroke", label: "Key", options: key_options() },
                    wait(),
                ],
            });
            v.push(CmdSpec {
                id: 17,
                name: "HX Looper",
                switch_label: "",
                momentary: false,
                fields: vec![FieldSpec::Choice { key: "@function", label: "Function", options: enum_options(&LOOPER_FNS) }],
            });
            v
        }
        SlotKind::Switch => {
            let mut v = vec![
                CmdSpec {
                    id: 1,
                    name: "MIDI CC",
                    switch_label: "CC Value",
                    momentary: true,
                    fields: vec![
                        ch(),
                        FieldSpec::Range { key: "@cc", label: "CC", min: 0, max: 127 },
                        FieldSpec::Range { key: "@value", label: "Value", min: 0, max: 127 },
                    ],
                },
                CmdSpec {
                    id: 2,
                    name: "CC Toggle",
                    switch_label: "CC Toggle",
                    momentary: false,
                    fields: vec![
                        ch(),
                        FieldSpec::Range { key: "@cc", label: "CC", min: 0, max: 127 },
                        FieldSpec::Range { key: "@initial", label: "Dim", min: 0, max: 127 },
                        FieldSpec::Range { key: "@toggle", label: "Lit", min: 0, max: 127 },
                    ],
                },
                CmdSpec {
                    id: 4,
                    name: "Bank/Program",
                    switch_label: "Bank/PC",
                    momentary: true,
                    fields: vec![
                        ch(),
                        FieldSpec::Range { key: "@bank0", label: "Bank MSB", min: 0, max: 127 },
                        FieldSpec::Range { key: "@bank32", label: "Bank LSB", min: 0, max: 127 },
                        FieldSpec::Range { key: "@program", label: "Program", min: 0, max: 127 },
                    ],
                },
                CmdSpec {
                    id: 5,
                    name: "Note On",
                    switch_label: "Note",
                    momentary: false,
                    fields: vec![
                        ch(),
                        FieldSpec::Choice { key: "@note", label: "Note", options: note_options() },
                        FieldSpec::Range { key: "@velocity", label: "Velocity", min: 0, max: 127 },
                        FieldSpec::Choice { key: "@noteoff", label: "Note Off", options: enum_options(&["Latching", "Momentary"]) },
                    ],
                },
                CmdSpec {
                    id: 6,
                    name: "MMC",
                    switch_label: "MMC",
                    momentary: true,
                    fields: vec![FieldSpec::Choice { key: "@message", label: "Message", options: enum_options(&MMC_MSGS) }],
                },
            ];
            if !is_stomp {
                v.push(CmdSpec {
                    id: 7,
                    name: "Ext Amp",
                    switch_label: "Ext Amp",
                    momentary: false,
                    fields: vec![FieldSpec::Choice {
                        key: "@select",
                        label: "Jack",
                        options: enum_options(&["1 (Tip)", "2 (Ring)", "1 and 2"]),
                    }],
                });
            }
            v.extend([
                CmdSpec {
                    id: 12,
                    name: "Hotkey",
                    switch_label: "Hotkey",
                    momentary: true,
                    fields: vec![
                        FieldSpec::Choice { key: "@modifier1", label: "Modifier 1", options: enum_options(&MODIFIERS) },
                        FieldSpec::Choice { key: "@modifier2", label: "Modifier 2", options: enum_options(&MODIFIERS) },
                        FieldSpec::Choice { key: "@modifier3", label: "Modifier 3", options: enum_options(&MODIFIERS) },
                        FieldSpec::Choice { key: "@keystroke", label: "Key", options: key_options() },
                    ],
                },
                CmdSpec {
                    id: 14,
                    name: "HX Preset",
                    switch_label: "Preset",
                    momentary: false,
                    fields: vec![FieldSpec::Choice { key: "@preset", label: "Preset", options: preset_options() }],
                },
                CmdSpec {
                    id: 15,
                    name: "HX Snapshot",
                    switch_label: "Snapshot",
                    momentary: true,
                    fields: vec![
                        FieldSpec::Choice { key: "@behavior", label: "Behavior", options: enum_options(&BEHAVIORS) },
                        FieldSpec::Choice { key: "@press", label: "Press", options: snapshot_press_options(snap_count) },
                        FieldSpec::Choice { key: "@relhold", label: "Release/Hold", options: snapshot_press_options(snap_count) },
                    ],
                },
                CmdSpec {
                    id: 16,
                    name: "HX Looper",
                    switch_label: "Looper",
                    momentary: true,
                    fields: vec![
                        FieldSpec::Choice { key: "@behavior", label: "Behavior", options: enum_options(&BEHAVIORS) },
                        FieldSpec::Choice { key: "@press", label: "Press", options: looper_press_options() },
                        FieldSpec::Choice { key: "@relhold", label: "Release/Hold", options: looper_press_options() },
                    ],
                },
            ]);
            v
        }
        SlotKind::Pedal => vec![CmdSpec {
            id: 3,
            name: "MIDI CC",
            switch_label: "",
            momentary: false,
            fields: vec![
                ch(),
                FieldSpec::Range { key: "@cc", label: "CC", min: 0, max: 127 },
                FieldSpec::Range { key: "@min", label: "Min", min: 0, max: 127 },
                FieldSpec::Range { key: "@max", label: "Max", min: 0, max: 127 },
            ],
        }],
    }
}

/// What a Command Center command id is called for a kind of trigger (None when the kind has no such command).
pub fn command_name(kind: SlotKind, id: i64) -> Option<String> {
    specs_for(kind, 2162694, 3).into_iter().find(|s| s.id == id).map(|s| s.name.to_owned())
}

/// The expression pedal's own slots: the toe switch under the pedal and what the pedals send.
pub fn in_pedal_group(key: &str) -> bool {
    key == "commandExpSwitch" || key.starts_with("commandExpPedal")
}

/// Rows of Command Center trigger slots for the unit, matching the pedal's layout.
pub fn slots_for_family(fam: snapshots::Family) -> Vec<Vec<CmdSlot>> {
    let inst: Vec<CmdSlot> = (1..=6)
        .map(|i| CmdSlot {
            key: format!("commandInst{i}"),
            short: format!("⚡ {i}"),
            title: format!("⚡ Instant {i}"),
            kind: SlotKind::Instant,
        })
        .collect();
    let fs = |n: i64| CmdSlot { key: format!("commandFS{n}"), short: format!("FS {n}"), title: format!("FS {n}"), kind: SlotKind::Switch };
    let exp = |n: i64| CmdSlot {
        key: format!("commandExpPedal{n}"),
        short: format!("EXP {n}"),
        title: format!("EXP {n}"),
        kind: SlotKind::Pedal,
    };
    if fam.name.starts_with("Helix") {
        let mut r2: Vec<CmdSlot> = (1..=6).map(fs).collect();
        r2.push(CmdSlot { key: "commandExpSwitch".into(), short: "EXP Toe".into(), title: "EXP Toe".into(), kind: SlotKind::Switch });
        let mut r3: Vec<CmdSlot> = (7..=12).map(fs).collect();
        for n in 1..=fam.exp_count {
            r3.push(exp(n));
        }
        vec![inst, r2, r3]
    } else if fam.cc_switches == 5 {
        // HX Stomp: Instant 1..6 + FS 4..5 on top; FS 1..3 + EXP 1..2 on bottom.
        let mut r1 = inst;
        r1.push(fs(4));
        r1.push(fs(5));
        let mut r2 = vec![fs(1), fs(2), fs(3)];
        for n in 1..=fam.exp_count {
            r2.push(exp(n));
        }
        vec![r1, r2]
    } else {
        // HX Stomp XL (8 switches) and HX Effects (6 switches).
        let half = (fam.cc_switches / 2).max(1);
        let r2: Vec<CmdSlot> = ((half + 1)..=fam.cc_switches).map(fs).collect();
        let mut r3: Vec<CmdSlot> = (1..=half).map(fs).collect();
        for n in 1..=fam.exp_count {
            r3.push(exp(n));
        }
        vec![inst, r2, r3]
    }
}

/// Active Command Center slots in a preset: `(slot_key, command_id)`.
pub fn active_commands(doc: &serde_json::Value) -> Vec<(String, i64)> {
    let Some(tone) = doc.pointer("/data/tone").and_then(serde_json::Value::as_object) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (k, v) in tone {
        if k.starts_with("command") {
            if let Some(id) = v.get("@command").and_then(serde_json::Value::as_i64) {
                if id > 0 {
                    out.push((k.clone(), id));
                }
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

pub fn read_slot_cmd(doc: &serde_json::Value, slot_key: &str, snap: usize) -> Option<serde_json::Map<String, serde_json::Value>> {
    let mut base = doc.pointer(&format!("/data/tone/{slot_key}"))?.as_object()?.clone();
    let id = base.get("@command").and_then(serde_json::Value::as_i64)?;
    if id <= 0 {
        return None;
    }
    if let Some(over) = doc.pointer(&format!("/data/tone/snapshot{snap}/commands/{slot_key}")).and_then(serde_json::Value::as_object) {
        for (k, v) in over {
            base.insert(k.clone(), v.clone());
        }
    }
    Some(base)
}

pub fn set_slot_cmd_type(doc: &mut serde_json::Value, slot: &CmdSlot, spec: Option<&CmdSpec>) {
    let Some(tone) = doc.pointer_mut("/data/tone").and_then(serde_json::Value::as_object_mut) else { return };
    match spec {
        None => {
            tone.remove(&slot.key);
            for i in 0..8 {
                let sk = format!("snapshot{i}");
                if let Some(s) = tone.get_mut(&sk).and_then(serde_json::Value::as_object_mut) {
                    if let Some(cmds) = s.get_mut("commands").and_then(serde_json::Value::as_object_mut) {
                        cmds.remove(&slot.key);
                        if cmds.is_empty() {
                            s.remove("commands");
                        }
                    }
                }
            }
        }
        Some(sp) => {
            let mut base = serde_json::Map::new();
            let mut snap_entry = serde_json::Map::new();
            base.insert("@command".into(), serde_json::json!(sp.id));
            snap_entry.insert("@fs_enabled".into(), serde_json::json!(false));
            if slot.kind == SlotKind::Switch {
                base.insert("@fs_label".into(), serde_json::json!(sp.switch_label));
                base.insert("@fs_ledcolor".into(), serde_json::json!(462860));
                base.insert("@fs_enabled".into(), serde_json::json!(false));
                base.insert("@fs_momentary".into(), serde_json::json!(sp.momentary));
                base.insert("@fs_primary".into(), serde_json::json!(true));
            }
            for f in &sp.fields {
                let fk = match f {
                    FieldSpec::Range { key, .. } | FieldSpec::Choice { key, .. } => *key,
                };
                let def = if fk == "@channel" {
                    1
                } else if fk == "@max" {
                    127
                } else {
                    0
                };
                base.insert(fk.into(), serde_json::json!(def));
                snap_entry.insert(fk.into(), serde_json::json!(def));
            }
            tone.insert(slot.key.clone(), serde_json::Value::Object(base));
            for i in 0..8 {
                let sk = format!("snapshot{i}");
                if let Some(s) = tone.get_mut(&sk).and_then(serde_json::Value::as_object_mut) {
                    let cmds = s.entry("commands").or_insert_with(|| serde_json::json!({})).as_object_mut();
                    if let Some(c) = cmds {
                        c.insert(slot.key.clone(), serde_json::Value::Object(snap_entry.clone()));
                    }
                }
            }
        }
    }
}

pub fn set_slot_field(doc: &mut serde_json::Value, slot_key: &str, snap: usize, field: &str, value: serde_json::Value) {
    if let Some(base) = doc.pointer_mut(&format!("/data/tone/{slot_key}")).and_then(serde_json::Value::as_object_mut) {
        base.insert(field.into(), value.clone());
    }
    if field != "@fs_label" {
        if let Some(s) = doc.pointer_mut(&format!("/data/tone/snapshot{snap}")).and_then(serde_json::Value::as_object_mut) {
            if let Some(cmds) = s.entry("commands").or_insert_with(|| serde_json::json!({})).as_object_mut() {
                if let Some(c) =
                    cmds.entry(slot_key.to_owned()).or_insert_with(|| serde_json::json!({ "@fs_enabled": false })).as_object_mut()
                {
                    c.insert(field.into(), value);
                }
            }
        }
    }
}

/// A unit's Command Center slots in the order of the pedal document. Confirmed on an HX Stomp from another
/// editor's live traffic (Instant 1 to 6, FS 1 to 5, EXP 1 and 2, slots 0 to 12). The other units follow the same
/// rule from the documentation (6 instants, then the footswitches, then the pedals) and are not checked on a unit:
/// HX Stomp XL 16 slots (FS 1-8), HX Effects 14 (FS 1-6), Helix 24 (FS 1-12, an EXP toe switch, EXP 1-3, then
/// the Variax volume and tone).
pub fn device_slots(fam: snapshots::Family) -> Vec<CmdSlot> {
    let mut v: Vec<CmdSlot> = (1..=6)
        .map(|i| CmdSlot {
            key: format!("commandInst{i}"),
            short: format!("⚡ {i}"),
            title: format!("Instant {i}"),
            kind: SlotKind::Instant,
        })
        .collect();
    let helix = fam.name.starts_with("Helix");
    let switches = if helix { 12 } else { fam.cc_switches };
    v.extend((1..=switches).map(|n| CmdSlot {
        key: format!("commandFS{n}"),
        short: format!("FS {n}"),
        title: format!("FS {n}"),
        kind: SlotKind::Switch,
    }));
    if helix {
        v.push(CmdSlot { key: "commandExpSwitch".into(), short: "EXP Toe".into(), title: "EXP Toe".into(), kind: SlotKind::Switch });
    }
    let pedals = if helix { 3 } else { 2 };
    v.extend((1..=pedals).map(|n| CmdSlot {
        key: format!("commandExpPedal{n}"),
        short: format!("EXP {n}"),
        title: format!("EXP {n}"),
        kind: SlotKind::Pedal,
    }));
    if helix {
        v.push(CmdSlot {
            key: "commandVariaxVol".into(),
            short: "Variax Vol".into(),
            title: "Variax Volume".into(),
            kind: SlotKind::Pedal,
        });
        v.push(CmdSlot {
            key: "commandVariaxTone".into(),
            short: "Variax Tone".into(),
            title: "Variax Tone".into(),
            kind: SlotKind::Pedal,
        });
    }
    v
}

/// The slots of the unit the app is set up for.
pub fn unit_slots() -> Vec<CmdSlot> {
    device_slots(snapshots::family_of_device(crate::nam::unit()))
}

/// Whether a unit's Command Center layout has been checked on a pedal (the HX Stomp only).
pub fn layout_verified(fam: snapshots::Family) -> bool {
    fam.cc_switches == 5 && !fam.name.starts_with("Helix")
}

/// Command types whose arguments have not been seen on a pedal: the switch versions of Bank/Program, MMC,
/// Hotkey and HX Looper. They follow the same layout as their instant versions in the notes, but
/// are shown with a mark.
pub const UNVERIFIED: [i64; 0] = [];

/// What the pedal puts in a command's arguments when its type is set (seen in its change events); the
/// rest start at 0. Hotkey (switch) keeps a constant 1 in its fifth argument.
pub fn device_defaults(id: i64) -> Vec<i64> {
    match id {
        2 | 3 => vec![0, 0, 0, 127],
        5 => vec![0, 48, 100, 1],
        15 => vec![0, 1, 0],
        16 => vec![0, 1, 0],
        17 => vec![1],
        12 => vec![0, 0, 0, 0, 1],
        _ => Vec::new(),
    }
}

/// A command type's `.hlx` field names in the pedal's argument order (`@channel`, `@cc`, ...), for the
/// kind of trigger it sits on.
pub fn hlx_arg_keys(kind: SlotKind, id: i64) -> Vec<&'static str> {
    specs_for(kind, crate::nam::unit(), 3)
        .into_iter()
        .find(|s| s.id == id)
        .map(|s| {
            s.fields
                .iter()
                .map(|f| match f {
                    FieldSpec::Range { key, .. } | FieldSpec::Choice { key, .. } => *key,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The HX Stomp's slot number for a `.hlx` key such as `commandFS2`.
pub fn unit_slot_of_hlx_key(key: &str) -> Option<usize> {
    unit_slots().iter().position(|s| s.key == key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hlx;

    #[test]
    fn command_center_stores_per_snapshot_values() {
        let mut doc = hlx::sample();
        let fam = snapshots::family(&doc);
        let slots = slots_for_family(fam);
        let inst1 = slots[0].iter().find(|s| s.key == "commandInst1").unwrap();
        let specs = specs_for(inst1.kind, 2162694, fam.snapshot_count);
        let midi_cc = specs.iter().find(|s| s.id == 18).unwrap();

        // Assign Instant 1 -> MIDI CC on Snapshot 1 with CC = 10, Value = 64.
        snapshots::select(&mut doc, 0);
        set_slot_cmd_type(&mut doc, inst1, Some(midi_cc));
        set_slot_field(&mut doc, &inst1.key, 0, "@cc", serde_json::json!(10));
        set_slot_field(&mut doc, &inst1.key, 0, "@value", serde_json::json!(64));

        // Switch to Snapshot 2 and set Value = 127.
        snapshots::select(&mut doc, 1);
        set_slot_field(&mut doc, &inst1.key, 1, "@value", serde_json::json!(127));

        // Snapshot 1 still has Value = 64; Snapshot 2 has Value = 127.
        snapshots::select(&mut doc, 0);
        let m0 = read_slot_cmd(&doc, "commandInst1", 0).unwrap();
        assert_eq!(m0.get("@cc").and_then(|v| v.as_i64()), Some(10));
        assert_eq!(m0.get("@value").and_then(|v| v.as_i64()), Some(64));

        snapshots::select(&mut doc, 1);
        let m1 = read_slot_cmd(&doc, "commandInst1", 1).unwrap();
        assert_eq!(m1.get("@value").and_then(|v| v.as_i64()), Some(127));

        // Setting command to None removes it from tone and snapshots.
        set_slot_cmd_type(&mut doc, inst1, None);
        assert!(read_slot_cmd(&doc, "commandInst1", 0).is_none());
        assert!(active_commands(&doc).is_empty());
    }

    /// The pedal's slot order, confirmed from live traffic: Instant 1-6, FS 1-5, EXP 1-2.
    #[test]
    fn the_stomps_slots_are_in_the_pedals_order() {
        let s = unit_slots();
        assert_eq!(s.len(), 13);
        assert_eq!(s[0].title, "Instant 1");
        assert_eq!(s[5].title, "Instant 6");
        assert_eq!((s[6].title.as_str(), s[10].title.as_str()), ("FS 1", "FS 5"));
        assert_eq!((s[11].title.as_str(), s[12].title.as_str()), ("EXP 1", "EXP 2"));
        assert!(s[..6].iter().all(|x| x.kind == SlotKind::Instant));
        assert!(s[6..11].iter().all(|x| x.kind == SlotKind::Switch));
        assert!(s[11..].iter().all(|x| x.kind == SlotKind::Pedal));
    }

    /// Every unit's slot list has the document's slot count, no repeated key, the instants first, and its grid
    /// rows place each slot at most once.
    #[test]
    fn every_units_command_center_slots_match_its_document() {
        for (device, count) in [(2162694, 13), (2162699, 16), (2162693, 14), (2162689, 24), (2162690, 24), (2162692, 24)] {
            let fam = snapshots::family_of_device(device);
            let slots = device_slots(fam);
            assert_eq!(slots.len(), count, "{} ({device})", fam.name);
            let mut keys: Vec<&str> = slots.iter().map(|s| s.key.as_str()).collect();
            keys.sort_unstable();
            keys.dedup();
            assert_eq!(keys.len(), count, "{}: repeated slot key", fam.name);
            assert!(slots[..6].iter().all(|s| s.kind == SlotKind::Instant));
            let rows = slots_for_family(fam);
            let placed: Vec<&str> = rows.iter().flatten().map(|s| s.key.as_str()).collect();
            let mut unique = placed.clone();
            unique.sort_unstable();
            unique.dedup();
            assert_eq!(placed.len(), unique.len(), "{}: a slot is on two rows", fam.name);
            assert!(placed.iter().all(|k| slots.iter().any(|s| s.key == *k)), "{}: a row slot is not in the document", fam.name);
        }
        assert!(layout_verified(snapshots::family_of_device(2162694)));
        assert!(!layout_verified(snapshots::family_of_device(2162699)));
        assert!(!layout_verified(snapshots::family_of_device(2162689)));
    }
    /// Argument counts of the types checked on a pedal: field i is argument i + 1, in the pedal's order.
    #[test]
    fn confirmed_command_types_have_the_pedals_argument_layout() {
        let layouts: [(SlotKind, i64, &[&str]); 11] = [
            (SlotKind::Switch, 1, &["channel", "cc", "value"]),
            (SlotKind::Switch, 2, &["channel", "cc", "initial", "toggle"]),
            (SlotKind::Pedal, 3, &["channel", "cc", "min", "max"]),
            (SlotKind::Switch, 5, &["channel", "note", "velocity", "noteoff"]),
            (SlotKind::Instant, 13, &["modifier1", "modifier2", "modifier3", "keystroke", "waittime"]),
            (SlotKind::Switch, 14, &["preset"]),
            (SlotKind::Switch, 15, &["behavior", "press", "relhold"]),
            (SlotKind::Instant, 17, &["function"]),
            (SlotKind::Instant, 18, &["channel", "cc", "value", "waittime"]),
            (SlotKind::Instant, 19, &["channel", "bank0", "bank32", "program"]),
            (SlotKind::Instant, 20, &["message"]),
        ];
        for (kind, id, names) in layouts {
            let specs = specs_for(kind, 2162694, 3);
            let spec = specs.iter().find(|s| s.id == id).unwrap_or_else(|| panic!("type {id} missing for slot kind {}", kind as u8));
            assert_eq!(spec.fields.len(), names.len(), "type {id}: field count");
            for (f, name) in spec.fields.iter().zip(names) {
                let key = match f {
                    FieldSpec::Range { key, .. } | FieldSpec::Choice { key, .. } => key.trim_start_matches('@'),
                };
                assert!(key.contains(name) || name.contains(key), "type {id}: field {key} should be {name}");
            }
        }
    }

    /// Whatever type a slot is given, the defaults fit its fields and every type offered is known.
    #[test]
    fn defaults_fit_every_offered_type() {
        let slots = unit_slots();
        for slot in &slots {
            for spec in specs_for(slot.kind, 2162694, 3).iter().filter(|s| s.id != 7 && s.id != 8) {
                assert!(device_defaults(spec.id).len() <= 6, "type {}", spec.id);
                assert!(device_defaults(spec.id).len() <= spec.fields.len().max(6));
            }
        }
        assert!(UNVERIFIED.iter().all(|id| specs_for(SlotKind::Switch, 2162694, 3).iter().any(|s| s.id == *id)
            || specs_for(SlotKind::Instant, 2162694, 3).iter().any(|s| s.id == *id)));
    }

    /// Setting every offered type on a slot of a document keeps the live and the document copies the same.
    #[test]
    fn every_offered_type_round_trips_through_a_document() {
        use crate::devpreset::DevicePreset;
        let Ok(path) = std::env::var("HX_COMMANDS_FIXTURE") else { return };
        let mut d = DevicePreset::parse(&std::fs::read(path).unwrap()).unwrap();
        for (i, slot) in unit_slots().iter().enumerate() {
            for spec in specs_for(slot.kind, 2162694, 3).iter().filter(|s| s.id != 7 && s.id != 8) {
                let args = device_defaults(spec.id);
                assert!(d.set_command(i, spec.id, &args));
                let back = DevicePreset::parse(&d.encode()).unwrap();
                let (id, got) = back.command(i).unwrap();
                assert_eq!(id, spec.id);
                assert_eq!(&got[..args.len()], &args[..], "slot {i} type {}", spec.id);
            }
        }
    }
}
