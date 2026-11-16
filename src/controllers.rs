//! The Controllers panel: every footswitch, expression pedal, snapshot and MIDI
//! assignment of a .hlx preset in one place, with ways to add, change and remove them.

use eframe::egui::{self, RichText, Vec2};

use crate::catalog::Catalog;
use crate::hlx::{self, HlxBlock};
use crate::snapshots::{self, Source};
use crate::theme::{self, *};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    #[default]
    Footswitches,
    Expression,
    Snapshots,
    Midi,
    Command,
}

/// A block's display name.
fn block_name(cat: Option<&Catalog>, b: &HlxBlock) -> String {
    cat.and_then(|c| c.info_for_symbol(&b.model).map(|i| crate::realnames::show(&i.name))).unwrap_or_else(|| hlx::pretty_model(&b.model))
}

/// A knob's display name and its range.
fn knob_meta(cat: Option<&Catalog>, b: &HlxBlock, knob: &str) -> (String, f64, f64) {
    let p = cat.and_then(|c| c.info_for_symbol(&b.model)).and_then(|i| i.params.iter().find(|p| p.id == knob).cloned());
    match p {
        Some(p) if p.max > p.min => (if p.name.is_empty() { knob.to_owned() } else { p.name }, p.min, p.max),
        Some(p) => (if p.name.is_empty() { knob.to_owned() } else { p.name }, 0.0, 1.0),
        None => (knob.to_owned(), 0.0, 1.0),
    }
}

/// The knobs of a block that a controller can move.
fn knobs_of(json: &serde_json::Value, b: &HlxBlock) -> Vec<String> {
    hlx::block(json, b)
        .map(|o| o.iter().filter(|(k, v)| !k.starts_with('@') && (v.is_number() || v.is_boolean())).map(|(k, _)| k.clone()).collect())
        .unwrap_or_default()
}

/// A "+ Add" menu: pick a block, then "On/Off" (when allowed) or one of its knobs.
/// Returns (block, Some(knob)) or (block, None) for on/off.
fn add_menu(
    ui: &mut egui::Ui,
    json: &serde_json::Value,
    cat: Option<&Catalog>,
    label: &str,
    allow_onoff: bool,
) -> Option<(HlxBlock, Option<String>)> {
    let mut out = None;
    ui.menu_button(RichText::new(label).color(accent()), |ui| {
        let blocks: Vec<HlxBlock> = hlx::blocks(json).into_iter().filter(|b| b.key.starts_with("block")).collect();
        if blocks.is_empty() {
            ui.label(RichText::new("Add effects to the chain first").color(MUTED));
        }
        for b in blocks {
            ui.menu_button(block_name(cat, &b), |ui| {
                egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                    if allow_onoff && ui.button(RichText::new("On / Off").strong()).clicked() {
                        out = Some((b.clone(), None));
                        ui.close_menu();
                    }
                    for k in knobs_of(json, &b) {
                        let (name, _, _) = knob_meta(cat, &b, &k);
                        if ui.button(name).clicked() {
                            out = Some((b.clone(), Some(k)));
                            ui.close_menu();
                        }
                    }
                });
            });
        }
    });
    out
}

/// One assigned knob: its name, the low and high ends, and a remove button.
fn knob_row(ui: &mut egui::Ui, json: &mut serde_json::Value, cat: Option<&Catalog>, a: &snapshots::Assignment, dirty: &mut bool) {
    let snapshots::Assignment { block: b, knob, src, lo, hi, cc } = a;
    let (knob, src, lo, hi, cc) = (knob.as_str(), *src, *lo, *hi, *cc);
    let (name, min, max) = knob_meta(cat, b, knob);
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{} › {}", block_name(cat, b), name)).strong());
        if src == Source::Snapshots {
            // One value per snapshot (pick a snapshot above and turn the knob to change its value).
            let model = hlx::block(json, b).and_then(|o| o.get("@model")).and_then(|v| v.as_str()).unwrap_or("").to_owned();
            let param = cat.and_then(|c| c.info_for_hlx(&model, false)).and_then(|i| i.params.iter().find(|p| p.id == knob)).cloned();
            let cur = snapshots::current(json);
            for (i, sname) in snapshots::list(json) {
                let v = if i == cur {
                    json.pointer(&format!("/data/tone/{}/{}/{knob}", b.dsp, b.key)).cloned()
                } else {
                    json.pointer(&format!("/data/tone/snapshot{i}/controllers/{}/{}/{knob}/@value", b.dsp, b.key)).cloned()
                };
                let shown = match v {
                    Some(serde_json::Value::Bool(x)) => {
                        if x {
                            "On".to_owned()
                        } else {
                            "Off".to_owned()
                        }
                    }
                    Some(n) => n.as_f64().map(|f| crate::catalog::format_value(param.as_ref(), f)).unwrap_or_else(|| "-".into()),
                    None => "-".into(),
                };
                let label = if sname.is_empty() { format!("{}", i + 1) } else { sname };
                ui.add_space(6.0);
                ui.label(RichText::new(label).small().color(MUTED));
                ui.label(RichText::new(shown).monospace().color(if i == cur { accent() } else { TEXT }));
            }
        }
        if src != Source::Snapshots {
            let (mut a, mut z) = (lo, hi);
            let step = ((max - min) / 200.0).max(0.001);
            ui.label(RichText::new("from").color(MUTED));
            let c1 = ui.add(egui::DragValue::new(&mut a).range(min..=max).speed(step)).changed();
            ui.label(RichText::new("to").color(MUTED));
            let c2 = ui.add(egui::DragValue::new(&mut z).range(min..=max).speed(step)).changed();
            let mut c = cc.unwrap_or(1);
            let c3 = if src == Source::Midi {
                ui.label(RichText::new("CC").color(MUTED));
                ui.add(egui::DragValue::new(&mut c).range(1..=127)).changed()
            } else {
                false
            };
            if c1 || c2 || c3 {
                snapshots::assign(json, b, knob, Some(src), a, z, Some(c));
                *dirty = true;
            }
        }
        if ui.small_button("✕").on_hover_text("Remove this assignment").clicked() {
            if src == Source::Snapshots {
                snapshots::release(json, b, knob);
            } else {
                snapshots::assign(json, b, knob, None, 0.0, 1.0, None);
            }
            *dirty = true;
        }
    });
}

pub fn panel(ui: &mut egui::Ui, json: &mut serde_json::Value, cat: Option<&Catalog>, dirty: &mut bool) {
    let fam = snapshots::family(json);
    let id = ui.id().with("controllers-tab");
    let mut tab: Tab = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    theme::card(ui, |ui| {
        ui.label(RichText::new("Controllers").size(17.0).strong());
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            for (t, label) in [
                (Tab::Footswitches, "Footswitches"),
                (Tab::Expression, "Expression"),
                (Tab::Snapshots, "Snapshots"),
                (Tab::Midi, "MIDI CC"),
                (Tab::Command, "Command Center"),
            ] {
                if ui.selectable_label(tab == t, label).clicked() {
                    tab = t;
                }
            }
            ui.add_space(4.0);
            theme::help(
                ui,
                match tab {
                    Tab::Footswitches => "Put knobs on a footswitch: each press makes the knob jump between its two values.",
                    Tab::Expression => "The pedal sweeps each knob from its first value (heel) to its second (toe).",
                    Tab::Snapshots => "Each snapshot keeps its own value for these knobs, and every block's on/off. Turning a knob while a snapshot is picked adds it here.",
                    Tab::Midi => "Move knobs from a MIDI controller: each knob listens to a CC number and sweeps between its two values.",
                    Tab::Command => "Commands the pedal sends when you press a switch: MIDI messages, presets or snapshots.",
                },
            );
        });
        ui.separator();
        let assignments = snapshots::all_assignments(json);
        match tab {
            Tab::Footswitches => {
                let switches = snapshots::all_footswitches(json);
                // The unit's switches at a glance: each one with what it does.
                let shown = fam.fs_count.min(if fam.name.starts_with("Helix") { 11 } else { fam.fs_count });
                let mut items: Vec<crate::editors::FootswitchItem> =
                    switches.iter().map(|(b, n, _)| (*n as usize, format!("{}: on/off", block_name(cat, b)), None)).collect();
                for a in &assignments {
                    if let Source::Footswitch(n) = a.src {
                        items.push((n as usize, format!("{} › {}", block_name(cat, &a.block), knob_meta(cat, &a.block, &a.knob).0), None));
                    }
                }
                crate::editors::footswitch_strip(ui, shown.max(0) as usize, &items);
                for n in 1..=fam.fs_count.min(if fam.name.starts_with("Helix") { 11 } else { fam.fs_count }) {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("FS {n}")).strong().color(accent()));
                        ui.add_space(8.0);
                        ui.vertical(|ui| {
                            let mut any = false;
                            for (b, _, momentary) in switches.iter().filter(|s| s.1 == n) {
                                any = true;
                                ui.horizontal(|ui| {
                                    ui.label(format!("{} › On / Off", block_name(cat, b)));
                                    let mut m = *momentary;
                                    if ui.checkbox(&mut m, "momentary").changed() {
                                        snapshots::set_footswitch(json, b, Some(n), m, &block_name(cat, b));
                                        *dirty = true;
                                    }
                                    if ui.small_button("✕").clicked() {
                                        snapshots::set_footswitch(json, b, None, false, "");
                                        *dirty = true;
                                    }
                                });
                            }
                            for a in assignments.iter().filter(|a| a.src == Source::Footswitch(n)) {
                                any = true;
                                knob_row(ui, json, cat, a, dirty);
                            }
                            if !any {
                                ui.label(RichText::new("unassigned").color(MUTED));
                            }
                            if let Some((b, knob)) = add_menu(ui, json, cat, "+ Add", true) {
                                match knob {
                                    None => snapshots::set_footswitch(json, &b, Some(n), false, &block_name(cat, &b)),
                                    Some(k) => {
                                        let (_, lo, hi) = knob_meta(cat, &b, &k);
                                        snapshots::assign(json, &b, &k, Some(Source::Footswitch(n)), lo, hi, None);
                                    }
                                }
                                *dirty = true;
                            }
                        });
                    });
                    ui.separator();
                }
            }
            Tab::Expression => {
                for n in 1..=fam.exp_count {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("EXP {n}")).strong().color(ORANGE));
                        ui.add_space(8.0);
                        ui.vertical(|ui| {
                            let list: Vec<_> = assignments.iter().filter(|a| a.src == Source::Exp(n)).collect();
                            if list.is_empty() {
                                ui.label(RichText::new("unassigned").color(MUTED));
                            }
                            for a in list {
                                knob_row(ui, json, cat, a, dirty);
                            }
                            if let Some((b, Some(k))) = add_menu(ui, json, cat, "+ Add knob", false) {
                                let (_, lo, hi) = knob_meta(cat, &b, &k);
                                snapshots::assign(json, &b, &k, Some(Source::Exp(n)), lo, hi, None);
                                *dirty = true;
                            }
                        });
                    });
                    ui.separator();
                }
            }
            Tab::Snapshots => {
                let list: Vec<_> = assignments.iter().filter(|a| a.src == Source::Snapshots).collect();
                if list.is_empty() {
                    ui.label(
                        RichText::new("No knob is stored per snapshot yet. Turn a knob while a snapshot is picked, or add one here.")
                            .color(MUTED),
                    );
                }
                for a in list {
                    knob_row(ui, json, cat, a, dirty);
                }
                if let Some((b, Some(k))) = add_menu(ui, json, cat, "+ Add knob", false) {
                    let (_, lo, hi) = knob_meta(cat, &b, &k);
                    snapshots::assign(json, &b, &k, Some(Source::Snapshots), lo, hi, None);
                    snapshots::knob_changed(json, &b, &k);
                    *dirty = true;
                }
            }
            Tab::Midi => {
                let list: Vec<_> = assignments.iter().filter(|a| a.src == Source::Midi).collect();
                if list.is_empty() {
                    ui.label(RichText::new("unassigned").color(MUTED));
                }
                for a in list {
                    knob_row(ui, json, cat, a, dirty);
                }
                if let Some((b, Some(k))) = add_menu(ui, json, cat, "+ Add knob", false) {
                    let (_, lo, hi) = knob_meta(cat, &b, &k);
                    snapshots::assign(json, &b, &k, Some(Source::Midi), lo, hi, Some(1));
                    *dirty = true;
                }
            }
            Tab::Command => {
                command_center_ui(ui, json, dirty);
            }
        }
    });
    ui.data_mut(|d| d.insert_temp(id, tab));
    let _ = Vec2::ZERO;
}

// ------------------------------------------------------------ Command Center ---

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

enum FieldSpec {
    Range { key: &'static str, label: &'static str, min: i64, max: i64 },
    Choice { key: &'static str, label: &'static str, options: Vec<(i64, String)> },
}

struct CmdSpec {
    id: i64,
    name: &'static str,
    switch_label: &'static str,
    momentary: bool,
    fields: Vec<FieldSpec>,
}

/// Longest footswitch label the pedal shows.
const FS_LABEL_MAX: usize = 16;

const MMC_MSGS: [&str; 8] = ["Stop", "Play", "Forward", "Rewind", "Pause", "Record", "Punch In", "Punch Out"];
const MODIFIERS: [&str; 5] = ["None", "Shift", "Alt", "Ctrl", "Win/Cmd"];
const LOOPER_FNS: [&str; 14] = [
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
const BEHAVIORS: [&str; 2] = ["Press/Release", "Press/Hold"];

fn channel_options() -> Vec<(i64, String)> {
    let mut out = vec![(0, "Base".into()), (1, "Base".into())];
    for ch in 2..=16 {
        out.push((ch, ch.to_string()));
    }
    out
}

fn enum_options(names: &[&str]) -> Vec<(i64, String)> {
    names.iter().enumerate().map(|(i, n)| (i as i64, (*n).to_owned())).collect()
}

fn note_options() -> Vec<(i64, String)> {
    const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    (0..=127)
        .map(|n| {
            let oct = (n / 12) - 1;
            let name = NAMES[(n % 12) as usize];
            (n, format!("{name}{oct} ({n})"))
        })
        .collect()
}

fn key_options() -> Vec<(i64, String)> {
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

fn preset_options() -> Vec<(i64, String)> {
    let mut out = vec![(0, "Next".into()), (1, "Previous".into())];
    for i in 0..126 {
        out.push(((i + 2) as i64, crate::proto::slot_label(i)));
    }
    out
}

fn snapshot_press_options(snap_count: usize) -> Vec<(i64, String)> {
    let mut out = vec![(0, "None".into()), (1, "Next".into()), (2, "Prev".into())];
    for i in 1..=snap_count.max(3) {
        out.push(((i + 2) as i64, format!("SNAPSHOT {i}")));
    }
    out
}

fn looper_press_options() -> Vec<(i64, String)> {
    let mut out = vec![(0, "None".into())];
    for (i, f) in LOOPER_FNS.iter().enumerate() {
        out.push(((i + 1) as i64, (*f).to_owned()));
    }
    out
}

fn specs_for(kind: SlotKind, device: i64, snap_count: usize) -> Vec<CmdSpec> {
    let is_stomp = device == 2162694;
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

fn read_slot_cmd(doc: &serde_json::Value, slot_key: &str, snap: usize) -> Option<serde_json::Map<String, serde_json::Value>> {
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

fn set_slot_cmd_type(doc: &mut serde_json::Value, slot: &CmdSlot, spec: Option<&CmdSpec>) {
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

fn set_slot_field(doc: &mut serde_json::Value, slot_key: &str, snap: usize, field: &str, value: serde_json::Value) {
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

fn command_center_ui(ui: &mut egui::Ui, json: &mut serde_json::Value, dirty: &mut bool) {
    let fam = snapshots::family(json);
    let device = snapshots::device(json);
    let cur_snap = snapshots::current(json);
    let snap_list = snapshots::list(json);
    let snap_name = snap_list.iter().find(|(i, _)| *i == cur_snap).map(|(_, n)| n.as_str()).unwrap_or("");
    let snap_badge = if snap_name.is_empty() || snap_name.eq_ignore_ascii_case(&format!("SNAPSHOT {}", cur_snap + 1)) {
        format!("Snapshot {}", cur_snap + 1)
    } else {
        format!("Snapshot {} · {snap_name}", cur_snap + 1)
    };

    ui.horizontal_wrapped(|ui| {
        theme::pill(ui, &snap_badge, accent());
        ui.label(
            RichText::new("A command's values are kept per snapshot, as on the pedal: these are the selected snapshot's.")
                .small()
                .color(MUTED),
        );
    });
    ui.add_space(8.0);

    let rows = slots_for_family(fam);
    let all_slots: Vec<CmdSlot> = rows.iter().flatten().cloned().collect();
    let sel_id = ui.id().with("cc-selected-slot");
    let mut selected_key: String = ui
        .data(|d| d.get_temp::<String>(sel_id))
        .filter(|k| all_slots.iter().any(|s| &s.key == k))
        .unwrap_or_else(|| "commandInst1".into());

    // Trigger grid (Instant 1..6, Footswitches, Expression pedals).
    for row in &rows {
        ui.horizontal_wrapped(|ui| {
            let mut prev_kind = None;
            for slot in row {
                if let Some(pk) = prev_kind {
                    if pk != slot.kind {
                        ui.add_space(10.0);
                    }
                }
                prev_kind = Some(slot.kind);
                let active_cmd = read_slot_cmd(json, &slot.key, cur_snap);
                let is_sel = slot.key == selected_key;
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(66.0, 52.0), egui::Sense::click());
                let p = ui.painter();
                let fill = if is_sel {
                    card_hi()
                } else if resp.hovered() {
                    crate::theme::panel()
                } else {
                    bg()
                };
                p.rect_filled(rect, egui::CornerRadius::same(8), fill);
                let stroke = if is_sel {
                    egui::Stroke::new(2.0_f32, accent())
                } else if active_cmd.is_some() {
                    egui::Stroke::new(1.0_f32, GOOD.linear_multiply(0.55))
                } else {
                    egui::Stroke::new(1.0_f32, line())
                };
                p.rect_stroke(rect, egui::CornerRadius::same(8), stroke, egui::StrokeKind::Inside);
                let tri_col = if active_cmd.is_some() { GOOD } else { egui::Color32::from_gray(75) };
                p.text(
                    rect.center_top() + Vec2::new(0.0, 12.0),
                    egui::Align2::CENTER_CENTER,
                    "▲",
                    egui::FontId::proportional(10.0),
                    tri_col,
                );
                p.text(
                    rect.center_bottom() + Vec2::new(0.0, -14.0),
                    egui::Align2::CENTER_CENTER,
                    &slot.short,
                    egui::FontId::proportional(13.0),
                    if is_sel || active_cmd.is_some() { TEXT } else { MUTED },
                );
                if resp.clicked() {
                    selected_key = slot.key.clone();
                }
            }
        });
        ui.add_space(4.0);
    }
    ui.data_mut(|d| d.insert_temp(sel_id, selected_key.clone()));

    ui.add_space(6.0);
    let Some(slot) = all_slots.into_iter().find(|s| s.key == selected_key) else { return };
    let specs = specs_for(slot.kind, device, fam.snapshot_count);
    let current_map = read_slot_cmd(json, &slot.key, cur_snap);
    let current_id = current_map.as_ref().and_then(|m| m.get("@command")).and_then(serde_json::Value::as_i64).unwrap_or(0);
    let current_spec = specs.iter().find(|s| s.id == current_id);

    // Detail card for the selected trigger.
    egui::Frame::new()
        .fill(crate::theme::panel())
        .stroke(egui::Stroke::new(1.0_f32, line()))
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(egui::Margin::same(14))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(&slot.title).size(15.0).strong());
                ui.add_space(12.0);
                ui.label(RichText::new("Command").color(MUTED));
                let shown = current_spec.map(|s| s.name).unwrap_or("None");
                egui::ComboBox::from_id_salt(format!("cc-type-{}", slot.key)).selected_text(shown).width(170.0).show_ui(ui, |ui| {
                    if ui.selectable_label(current_spec.is_none(), "None").clicked() && current_spec.is_some() {
                        set_slot_cmd_type(json, &slot, None);
                        *dirty = true;
                    }
                    for sp in &specs {
                        if ui.selectable_label(current_id == sp.id, sp.name).clicked() && current_id != sp.id {
                            set_slot_cmd_type(json, &slot, Some(sp));
                            *dirty = true;
                        }
                    }
                });
                if let (SlotKind::Switch, Some(map)) = (slot.kind, current_map.as_ref()) {
                    ui.add_space(12.0);
                    ui.label(RichText::new("Switch label").color(MUTED));
                    let mut lbl = map.get("@fs_label").and_then(serde_json::Value::as_str).unwrap_or("").to_owned();
                    if ui.add(egui::TextEdit::singleline(&mut lbl).desired_width(120.0).char_limit(FS_LABEL_MAX)).changed() {
                        set_slot_field(json, &slot.key, cur_snap, "@fs_label", serde_json::json!(lbl));
                        *dirty = true;
                    }
                    theme::name_hint(ui, &lbl, FS_LABEL_MAX);
                }
            });

            let Some(sp) = specs.iter().find(|s| s.id == current_id) else {
                return;
            };
            let Some(map) = read_slot_cmd(json, &slot.key, cur_snap) else {
                return;
            };
            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(10.0, 10.0);
                for field in &sp.fields {
                    egui::Frame::new()
                        .fill(card_bg())
                        .stroke(egui::Stroke::new(1.0_f32, line()))
                        .corner_radius(egui::CornerRadius::same(8))
                        .inner_margin(egui::Margin::symmetric(12, 10))
                        .show(ui, |ui| {
                            ui.set_width(185.0);
                            match field {
                                FieldSpec::Choice { key, label, options } => {
                                    let mut val =
                                        map.get(*key).and_then(serde_json::Value::as_i64).unwrap_or(if *key == "@channel" { 1 } else { 0 });
                                    let text =
                                        options.iter().find(|(v, _)| *v == val).map(|(_, n)| n.clone()).unwrap_or_else(|| val.to_string());
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new(*label).small().color(MUTED));
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            ui.label(RichText::new(&text).small().strong());
                                        });
                                    });
                                    ui.add_space(4.0);
                                    let mut changed = false;
                                    egui::ComboBox::from_id_salt(format!("cc-f-{}-{key}", slot.key))
                                        .selected_text(text)
                                        .width(175.0)
                                        .show_ui(ui, |ui| {
                                            // Avoid duplicate "Base" entry for channel 0 and 1.
                                            let mut seen_base = false;
                                            for (v, n) in options {
                                                if n == "Base" {
                                                    if seen_base {
                                                        continue;
                                                    }
                                                    seen_base = true;
                                                }
                                                let sel = *v == val || (n == "Base" && (val == 0 || val == 1) && *key == "@channel");
                                                if ui.selectable_label(sel, n).clicked() {
                                                    val = *v;
                                                    changed = true;
                                                }
                                            }
                                        });
                                    if changed {
                                        set_slot_field(json, &slot.key, cur_snap, key, serde_json::json!(val));
                                        *dirty = true;
                                    }
                                }
                                FieldSpec::Range { key, label, min, max } => {
                                    let mut val = map.get(*key).and_then(serde_json::Value::as_i64).unwrap_or(0).clamp(*min, *max);
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new(*label).small().color(MUTED));
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            if ui.add(egui::DragValue::new(&mut val).range(*min..=*max)).changed() {
                                                set_slot_field(json, &slot.key, cur_snap, key, serde_json::json!(val));
                                                *dirty = true;
                                            }
                                        });
                                    });
                                    ui.add_space(4.0);
                                    if ui.add(egui::Slider::new(&mut val, *min..=*max).show_value(false).trailing_fill(true)).changed() {
                                        set_slot_field(json, &slot.key, cur_snap, key, serde_json::json!(val));
                                        *dirty = true;
                                    }
                                }
                            }
                        });
                }
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
