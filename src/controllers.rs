//! The Controllers panel: every footswitch, expression pedal, snapshot and MIDI
//! assignment of a .hlx preset in one place, with ways to add, change and remove them.

use eframe::egui::{self, RichText, Vec2};

use crate::catalog::Catalog;
use crate::hlx::{self, HlxBlock};
use crate::snapshots::{self, Source};
use crate::theme::{self, *};

pub use crate::cmdslots::*;

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
            ui.label(RichText::new("Add effects to the chain first").color(muted()));
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
        let model = hlx::block(json, b).and_then(|o| o.get("@model")).and_then(|v| v.as_str()).unwrap_or("").to_owned();
        let param = cat.and_then(|c| c.info_for_hlx(&model, false)).and_then(|i| i.params.iter().find(|p| p.id == knob)).cloned();
        if src == Source::Snapshots {
            // One value per snapshot (pick a snapshot above and turn the knob to change its value).
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
                ui.label(RichText::new(label).small().color(muted()));
                ui.label(RichText::new(shown).monospace().color(if i == cur { accent() } else { ink() }));
            }
        }
        if src != Source::Snapshots {
            let (mut a, mut z) = (lo, hi);
            let step = ((max - min) / 200.0).max(0.001);
            ui.label(RichText::new("from").color(muted()));
            let c1 = ui
                .add(
                    egui::DragValue::new(&mut a)
                        .range(min..=max)
                        .speed(step)
                        .custom_formatter(|n, _| crate::catalog::format_value(param.as_ref(), n))
                        .custom_parser(|s| crate::catalog::parse_shown(param.as_ref(), s)),
                )
                .changed();
            ui.label(RichText::new("to").color(muted()));
            let c2 = ui
                .add(
                    egui::DragValue::new(&mut z)
                        .range(min..=max)
                        .speed(step)
                        .custom_formatter(|n, _| crate::catalog::format_value(param.as_ref(), n))
                        .custom_parser(|s| crate::catalog::parse_shown(param.as_ref(), s)),
                )
                .changed();
            let mut c = cc.unwrap_or(1);
            let c3 = if src == Source::Midi {
                ui.label(RichText::new("CC").color(muted()));
                ui.add(egui::DragValue::new(&mut c).range(1..=127)).changed()
            } else {
                false
            };
            if c1 || c2 || c3 {
                snapshots::assign(json, b, knob, Some(src), a, z, Some(c));
                *dirty = true;
            }
        }
        if ui.small_button("✖").on_hover_text("Remove this assignment").clicked() {
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
    let mut tab: Tab = ui.data(|d| d.get_temp(id)).unwrap_or_else(|| {
        match std::env::var("TONESTRAND_CTL_TAB").ok().and_then(|v| v.parse::<usize>().ok()) {
            Some(1) => Tab::Expression,
            Some(2) => Tab::Snapshots,
            Some(3) => Tab::Midi,
            Some(4) => Tab::Command,
            _ => Tab::Footswitches,
        }
    });
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
                                    if ui.small_button("✖").clicked() {
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
                                ui.label(RichText::new("unassigned").color(muted()));
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
                        ui.label(RichText::new(format!("EXP {n}")).strong().color(orange()));
                        ui.add_space(8.0);
                        ui.vertical(|ui| {
                            let list: Vec<_> = assignments.iter().filter(|a| a.src == Source::Exp(n)).collect();
                            if list.is_empty() {
                                ui.label(RichText::new("unassigned").color(muted()));
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
                            .color(muted()),
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
                    ui.label(RichText::new("unassigned").color(muted()));
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
                .color(muted()),
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
            let mut prev_pedal = false;
            for slot in row {
                if let Some(pk) = prev_kind {
                    if pk != slot.kind || (in_pedal_group(&slot.key) && !prev_pedal) {
                        ui.add_space(10.0);
                    }
                }
                prev_kind = Some(slot.kind);
                prev_pedal = in_pedal_group(&slot.key);
                let active_cmd = read_slot_cmd(json, &slot.key, cur_snap);
                let is_sel = slot.key == selected_key;
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(66.0, 52.0), egui::Sense::click());
                crate::theme::focus_ring(ui, &resp, 8);
                resp.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Button,
                        true,
                        format!("{}{}", slot.title, if active_cmd.is_some() { ", has a command" } else { ", empty" }),
                    )
                });
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
                    egui::Stroke::new(1.0_f32, good().linear_multiply(0.55))
                } else {
                    egui::Stroke::new(1.0_f32, line())
                };
                p.rect_stroke(rect, egui::CornerRadius::same(8), stroke, egui::StrokeKind::Inside);
                let tri_col = if active_cmd.is_some() { good() } else { muted().linear_multiply(0.5) };
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
                    if is_sel || active_cmd.is_some() { ink() } else { muted() },
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
                ui.label(RichText::new("Command").color(muted()));
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
                    ui.label(RichText::new("Switch label").color(muted()));
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
                                        ui.label(RichText::new(*label).small().color(muted()));
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
                                        ui.label(RichText::new(*label).small().color(muted()));
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

// ------------------------------------------------ Command Center on the pedal ---

/// The Command Center of a pedal document: what each trigger does, and a place to change it. On the
/// loaded preset a change goes to the pedal at once (Save on pedal keeps it); on a file or a slot's copy it
/// changes the document. Only the HX Stomp's layout is read so far.
pub fn device_command_center(
    ui: &mut egui::Ui,
    preset: &mut crate::devpreset::DevicePreset,
    live: bool,
    cmds: &mut Vec<crate::device::Cmd>,
    dirty: &mut bool,
) {
    let fam = snapshots::family_of_device(crate::nam::unit());
    let device = crate::nam::unit();
    let count = preset.command_count();
    let slots = unit_slots();
    {
        if count == 0 {
            ui.label(RichText::new("This preset has no Command Center.").color(muted()));
            return;
        }
        if count != slots.len() {
            ui.label(RichText::new("The Command Center of this unit isn't read here yet.").color(muted()));
            return;
        }
        if !layout_verified(fam) {
            ui.label(
                RichText::new(
                    "This unit's Command Center layout comes from the documentation and has not been checked on a pedal. Changes are written as for an HX Stomp; check them on the unit and tell us what differs.",
                )
                .small()
                .color(orange()),
            );
            ui.add_space(4.0);
        }
        // The values shown and edited are the selected snapshot's: the preset's own snapshot selector decides.
        let names = preset.snapshot_names();
        let snap = preset.current_snapshot().min(names.len().saturating_sub(1));
        let snap_name = names.get(snap).map(|n| n.trim()).unwrap_or("");
        ui.horizontal_wrapped(|ui| {
            theme::pill(
                ui,
                &if snap_name.is_empty() { format!("Snapshot {}", snap + 1) } else { format!("Snapshot {} · {snap_name}", snap + 1) },
                accent(),
            );
            ui.label(
                RichText::new("Values are per snapshot: change the snapshot with the snapshot bar. HX Preset is the same in all.")
                    .small()
                    .color(muted()),
            );
            theme::help(
                ui,
                "What the footswitches, expression pedals and instant commands do when this preset loads or they are pressed. The names are the Command Center's own: FS 1 to FS 5, whatever is printed on the pedal.\nOn the loaded preset a change goes to the pedal at once, and Save on pedal keeps it.\nThe wait is 0 to 20 (the pedal's own limit). Each snapshot keeps its own copy of a command's values.",
            );
        });
        ui.add_space(6.0);

        // Trigger grid arranged like the unit's panel (the same rows as a .hlx preset's Command Center tab);
        // a slot the rows don't place goes on a last row.
        let mut rows: Vec<Vec<usize>> = slots_for_family(fam)
            .iter()
            .map(|row| row.iter().filter_map(|s| slots.iter().position(|d| d.key == s.key)).collect::<Vec<usize>>())
            .collect();
        let left: Vec<usize> = (0..slots.len()).filter(|i| !rows.iter().flatten().any(|j| j == i)).collect();
        if !left.is_empty() {
            rows.push(left);
        }
        let sel_id = egui::Id::new("dev-cc-selected");
        let mut sel: usize = ui.data(|d| d.get_temp(sel_id)).unwrap_or(0usize).min(slots.len() - 1);
        for row in &rows {
            ui.horizontal_wrapped(|ui| {
                let mut prev_pedal = false;
                for (n, &i) in row.iter().enumerate() {
                    if n > 0 && in_pedal_group(&slots[i].key) && !prev_pedal {
                        ui.add_space(10.0);
                    }
                    prev_pedal = in_pedal_group(&slots[i].key);
                    let slot = &slots[i];
                    let in_use = preset.command(i).is_some_and(|(id, _)| id != 0);
                    let is_sel = i == sel;
                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(66.0, 52.0), egui::Sense::click());
                    crate::theme::focus_ring(ui, &resp, 8);
                    resp.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::WidgetType::Button,
                            true,
                            format!("{}{}", slot.title, if in_use { ", has a command" } else { ", empty" }),
                        )
                    });
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
                    } else if in_use {
                        egui::Stroke::new(1.0_f32, good().linear_multiply(0.55))
                    } else {
                        egui::Stroke::new(1.0_f32, line())
                    };
                    p.rect_stroke(rect, egui::CornerRadius::same(8), stroke, egui::StrokeKind::Inside);
                    p.text(
                        rect.center_top() + Vec2::new(0.0, 12.0),
                        egui::Align2::CENTER_CENTER,
                        "▲",
                        egui::FontId::proportional(10.0),
                        if in_use { good() } else { muted().linear_multiply(0.5) },
                    );
                    p.text(
                        rect.center_bottom() + Vec2::new(0.0, -14.0),
                        egui::Align2::CENTER_CENTER,
                        &slot.short,
                        egui::FontId::proportional(13.0),
                        if is_sel || in_use { ink() } else { muted() },
                    );
                    if resp.clicked() {
                        sel = i;
                    }
                }
            });
            ui.add_space(4.0);
        }
        ui.data_mut(|d| d.insert_temp(sel_id, sel));
        ui.add_space(6.0);

        // The selected trigger.
        let i = sel;
        let slot = &slots[i];
        let specs: Vec<CmdSpec> = specs_for(slot.kind, device, fam.snapshot_count).into_iter().filter(|s| s.id != 7 && s.id != 8).collect();
        let (id, base_args) = preset.command(i).unwrap_or((0, vec![0; 6]));
        let mut args = preset.snapshot_command_args(snap, i).filter(|_| id != 14).unwrap_or(base_args);
        let spec = specs.iter().find(|s| s.id == id);
        let name = if id == 0 { "None".to_owned() } else { spec.map(|s| s.name.to_owned()).unwrap_or_else(|| format!("command {id}")) };
        let mark = if UNVERIFIED.contains(&id) { " (not checked on a pedal)" } else { "" };
        egui::Frame::new()
            .fill(crate::theme::panel())
            .stroke(egui::Stroke::new(1.0_f32, line()))
            .corner_radius(egui::CornerRadius::same(8))
            .inner_margin(egui::Margin::same(14))
            .show(ui, |ui| {
                ui.label(RichText::new(&slot.title).strong().size(15.0));
                ui.add_space(4.0);
                let mut new_id = id;
                egui::Grid::new(("dev-cc-g0", i)).min_col_width(110.0).spacing([12.0, 8.0]).show(ui, |ui| {
                    ui.label(RichText::new("Command").color(muted()));
                    egui::ComboBox::from_id_salt(("dev-cc-type", i)).selected_text(format!("{name}{mark}")).width(220.0).show_ui(
                        ui,
                        |ui| {
                            if ui.selectable_label(id == 0, "None").clicked() {
                                new_id = 0;
                            }
                            for s in &specs {
                                let tag = if UNVERIFIED.contains(&s.id) { " (not checked on a pedal)" } else { "" };
                                if ui.selectable_label(id == s.id, format!("{}{tag}", s.name)).clicked() {
                                    new_id = s.id;
                                }
                            }
                        },
                    );
                    ui.end_row();
                });
                if new_id != id {
                    let start = device_defaults(new_id);
                    if preset.set_command(i, new_id, &start) {
                        *dirty = true;
                        if live {
                            cmds.push(crate::device::Cmd::SetCommand { slot: i, id: new_id, args: start });
                        }
                    }
                    return;
                }
                let Some(spec) = spec else {
                    if id == 0 {
                        ui.add_space(4.0);
                        ui.label(RichText::new("Nothing happens on this trigger.").color(muted()));
                    }
                    return;
                };
                ui.add_space(4.0);
                // Only a CC Toggle and a Hotkey have a Momentary switch (a CC Toggle can latch).
                if id == 2 || id == 12 {
                    if let Some(mut m) = preset.command_momentary(i) {
                        egui::Grid::new(("dev-cc-g1", i)).min_col_width(110.0).spacing([12.0, 8.0]).show(ui, |ui| {
                            ui.label(RichText::new("Momentary").color(muted()));
                            let resp = ui.checkbox(&mut m, "").on_hover_text(
                                "On: the value goes out while the switch is held and returns when it is released. Off: latching, each press toggles the command.",
                            );
                            ui.end_row();
                            if resp.changed() && preset.set_command_momentary(i, m) {
                                *dirty = true;
                                if live {
                                    cmds.push(crate::device::Cmd::SetMomentary { slot: i, momentary: m });
                                }
                            }
                        });
                    }
                }
                let mut changed = false;
                egui::Grid::new(("dev-cc-g2", i)).min_col_width(110.0).spacing([12.0, 8.0]).show(ui, |ui| {
                    for (f, field) in spec.fields.iter().enumerate() {
                        if args.len() <= f {
                            args.resize(f + 1, 0);
                        }
                        match field {
                            FieldSpec::Choice { label, options, .. } => {
                                let cur = args[f];
                                let text =
                                    options.iter().find(|(v, _)| *v == cur).map(|(_, n)| n.clone()).unwrap_or_else(|| format!("{cur}"));
                                ui.label(RichText::new(*label).color(muted()));
                                egui::ComboBox::from_id_salt(("dev-cc-field", i, f)).selected_text(text).width(220.0).show_ui(ui, |ui| {
                                    for (v, n) in options {
                                        if ui.selectable_label(cur == *v, n).clicked() && cur != *v {
                                            args[f] = *v;
                                            changed = true;
                                        }
                                    }
                                });
                            }
                            FieldSpec::Range { key, label, min, max } => {
                                let (label, max) = if *key == "@waittime" { ("Wait (0-20)", 20) } else { (*label, *max) };
                                let mut v = args[f].clamp(*min, max);
                                ui.label(RichText::new(label).color(muted()));
                                if ui.add(egui::DragValue::new(&mut v).range(*min..=max)).changed() {
                                    args[f] = v;
                                    changed = true;
                                }
                            }
                        }
                        ui.end_row();
                    }
                });
                if changed && preset.set_command_values(snap, i, &args) {
                    *dirty = true;
                    if live {
                        cmds.push(crate::device::Cmd::SetCommandValues { snapshot: snap, slot: i, args: args.clone() });
                    }
                }
            });
    }
}
