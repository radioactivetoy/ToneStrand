//! The editor for .hlx files: chain, block panel, snapshots and controller assignment.

use super::*;

// ---------------------------------------------------------------- .hlx file ---

/// What is being dragged in a .hlx chain: a block by its DSP and key.
#[derive(Clone)]
struct HlxDrag {
    dsp: String,
    key: String,
}
/// The menu that puts a knob on a controller: snapshots, an expression pedal,
/// a footswitch or MIDI CC, with its low and high ends.
fn assign_menu(ui: &mut egui::Ui, json: &mut serde_json::Value, b: &hlx::HlxBlock, knob: &str, meta: Option<&Param>, dirty: &mut bool) {
    let fam = snapshots::family(json);
    let current = snapshots::controller(json, b, knob);
    let label = current.map(|c| c.0.short()).unwrap_or_else(|| "assign".into());
    let col = if current.is_some() { ORANGE } else { MUTED.gamma_multiply(0.6) };
    // Quiet until it means something: no frame when nothing is assigned, an orange tag when it is.
    {
        let w = &mut ui.style_mut().visuals.widgets;
        let fill = if current.is_some() { ORANGE.gamma_multiply(0.16) } else { Color32::TRANSPARENT };
        w.inactive.weak_bg_fill = fill;
        w.inactive.bg_fill = fill;
        w.inactive.bg_stroke = Stroke::NONE;
    }
    ui.menu_button(RichText::new(label).small().color(col), |ui| {
        ui.set_min_width(220.0);
        ui.label(RichText::new(format!("What moves {}", meta.map(|m| m.name.as_str()).unwrap_or(knob))).strong());
        let (lo, hi) = meta.filter(|m| m.max > m.min).map(|m| (m.min, m.max)).unwrap_or((0.0, 1.0));
        let (mut min, mut max) = current.map(|c| (c.1, c.2)).unwrap_or((lo, hi));
        let mut choice: Option<Option<snapshots::Source>> = None;
        if ui.selectable_label(current.is_none(), "Nothing").clicked() {
            choice = Some(None);
        }
        let mut options = vec![snapshots::Source::Snapshots, snapshots::Source::Exp(1), snapshots::Source::Exp(2)];
        for n in 1..=fam.fs_count.min(5) {
            options.push(snapshots::Source::Footswitch(n));
        }
        options.push(snapshots::Source::Midi);
        for o in options {
            if ui.selectable_label(current.map(|c| c.0) == Some(o), o.label()).clicked() {
                choice = Some(Some(o));
            }
        }
        if let Some((src, _, _, cc)) = current {
            if src != snapshots::Source::Snapshots {
                ui.separator();
                let mut changed = false;
                ui.horizontal(|ui| {
                    ui.label("Low");
                    changed |= ui.add(egui::DragValue::new(&mut min).range(lo..=hi).speed((hi - lo) / 200.0)).changed();
                    ui.label("High");
                    changed |= ui.add(egui::DragValue::new(&mut max).range(lo..=hi).speed((hi - lo) / 200.0)).changed();
                });
                let mut cc_v = cc.unwrap_or(1);
                if src == snapshots::Source::Midi {
                    ui.horizontal(|ui| {
                        ui.label("CC number");
                        changed |= ui.add(egui::DragValue::new(&mut cc_v).range(1..=127)).changed();
                    });
                }
                if changed {
                    snapshots::assign(json, b, knob, Some(src), min, max, Some(cc_v));
                    *dirty = true;
                }
            }
        }
        if let Some(c) = choice {
            match c {
                Some(snapshots::Source::Snapshots) => {
                    snapshots::assign(json, b, knob, Some(snapshots::Source::Snapshots), lo, hi, None);
                    snapshots::block_switched(json);
                    snapshots::knob_changed(json, b, knob);
                }
                Some(src) => {
                    if current.map(|x| x.0) == Some(snapshots::Source::Snapshots) {
                        snapshots::release(json, b, knob);
                    }
                    snapshots::assign(json, b, knob, Some(src), lo, hi, Some(1));
                }
                None => snapshots::release(json, b, knob),
            }
            *dirty = true;
            ui.close_menu();
        }
    });
}
const GUITAR_IN_Z: [&str; 9] = ["Auto", "10k Ohm", "22k Ohm", "32k Ohm", "70k Ohm", "90k Ohm", "136k Ohm", "230k Ohm", "1M Ohm"];
const GUITAR_PAD: [&str; 3] = ["Global", "Off", "On"];

/// A drop-down for a whole-number setting with named choices.
fn choice_combo(ui: &mut egui::Ui, id: &str, value: &mut i64, choices: &[(i64, &str)]) -> bool {
    let shown = choices.iter().find(|(v, _)| v == value).map(|(_, n)| n.to_string()).unwrap_or_else(|| value.to_string());
    let mut changed = false;
    egui::ComboBox::from_id_salt(id).selected_text(shown).width(200.0).show_ui(ui, |ui| {
        for (v, n) in choices {
            if ui.selectable_label(*value == *v, *n).clicked() {
                *value = *v;
                changed = true;
            }
        }
    });
    changed
}

pub fn hlx_editor(
    ui: &mut egui::Ui,
    json: &mut serde_json::Value,
    selected: &mut Option<usize>,
    dirty: &mut bool,
    cat: Option<&Catalog>,
    picker: &mut Option<PickerState>,
) {
    // Unit and block counts; tempo and snapshots sit in the top bar.

    let name_of = |model: &str| -> (String, String, Color32) {
        let fam = hlx::family(model);
        match cat {
            Some(c) => {
                let name = c.info_for_symbol(model).map(|i| crate::realnames::show(&i.name)).unwrap_or_else(|| hlx::pretty_model(model));
                let (cname, col) = c.category_of_symbol(model).unwrap_or((fam.to_owned(), family_colour(fam)));
                (name, cname, col)
            }
            None => (hlx::pretty_model(model), fam.to_owned(), family_colour(fam)),
        }
    };

    // What the unit can do: parallel lanes and paths.
    let unit = snapshots::family(json);
    // A Helix file with only Path 1 still shows Path 2, ready to fill.
    hlx::ensure_paths(json, unit.paths);
    if unit.lanes {
        let defaults = |m: &str| cat.and_then(|c| c.number_for_hlx(m, false).map(|n| c.default_values(n))).unwrap_or_default();
        for dsp in hlx::dsps(json) {
            hlx::ensure_split(json, &dsp, &defaults);
            hlx::fit_lanes(json, &dsp, unit.positions, unit.max_blocks, unit.shared_slots);
        }
    }

    // The blocks shown in the chain; cabs that ride with an amp are shown with it.
    let all = hlx::blocks(json);
    let riding: Vec<String> = all
        .iter()
        .filter_map(|b| hlx::block(json, b).and_then(|o| o.get("@cab")).and_then(|v| v.as_str()).map(|c| format!("{}/{c}", b.dsp)))
        .collect();
    let blocks: Vec<hlx::HlxBlock> = all.into_iter().filter(|b| !riding.contains(&format!("{}/{}", b.dsp, b.key))).collect();
    let dsps: Vec<String> = hlx::dsps(json).into_iter().take(unit.paths).collect();
    // Only a path that is really split shows its lower lane.
    let splits: Vec<Option<(String, i64, i64)>> = dsps.iter().map(|d| hlx::split_of(json, d).filter(|_| hlx::split_active(json, d))).collect();
    let costs: Vec<f64> = blocks
        .iter()
        .map(|b| {
            let own = cat
                .and_then(|c| c.number_for_hlx(&b.model, hlx::block(json, b).and_then(|o| o.get("@stereo")).and_then(|v| v.as_bool()).unwrap_or(false)).map(|n| dsp_percent(c, n)))
                .unwrap_or(0.0);
            // A cab riding with an amp (or a dual cab's second cab) costs DSP too.
            let riding = hlx::block(json, b)
                .and_then(|o| o.get("@cab"))
                .and_then(|v| v.as_str())
                .and_then(|k| json.pointer(&format!("/data/tone/{}/{k}/@model", b.dsp)))
                .and_then(|m| m.as_str())
                .and_then(|m| cat.and_then(|c| c.number_for_hlx(m, false)).zip(cat))
                .map(|(n, c)| dsp_percent(c, n))
                .unwrap_or(0.0);
            own + riding
        })
        .collect();
    let cost_of = |i: usize| costs.get(i).copied().unwrap_or(0.0);
    let mut toggle = None;
    let mut menu_change: Option<usize> = None;
    let mut menu_remove: Option<usize> = None;
    let mut drop_to: Option<(HlxDrag, i64, i64)> = None;
    let mut extend_dsp: Option<(String, f64)> = None;
    let mut drop_extend: Option<HlxDrag> = None;
    let mut move_marker: Option<(String, bool, i64)> = None;
    let mut gap_split: Option<(String, i64, usize)> = None;
    if chain_part() {
    for (di, dsp) in dsps.iter().enumerate() {
        let used: f64 = blocks.iter().enumerate().filter(|(_, b)| &b.dsp == dsp).map(|(i, _)| cost_of(i)).sum();
        let count = hlx::block_count(json, dsp);
        let full = count >= unit.max_blocks;
        let main_cols = hlx::main_cols_for(json, dsp, unit.positions, unit.max_blocks, unit.shared_slots);
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("PATH {}", di + 1)).small().strong().color(MUTED));
        });
        // The path as a diagram: main line, and when it splits the lower lane
        // starting at the split dot.
        let split = splits.get(di).cloned().flatten().filter(|_| unit.lanes);
        let cell_of = |lane: i64, col: i64| -> chain::Cell {
            match blocks.iter().find(|b| &b.dsp == dsp && b.path == lane && b.position == col && b.key.starts_with("block")) {
                Some(b) => {
                    let (title, cname, colour) = name_of(&b.model);
                    // An amp carrying its cab (`@cab`) reads "Amp+Cab"; a dual cab "Dual Cab".
                    let has_cab = hlx::block(json, b).and_then(|o| o.get("@cab")).is_some();
                    let subtitle = crate::icons::kind_label(&cname, has_cab);
                    let detail = crate::nam::is_nam_symbol(&b.model)
                        .then(|| hlx::block(json, b).and_then(|o| o.get("MinFreq")).and_then(|v| v.as_f64()).map(nam_detail))
                        .flatten();
                    chain::Cell::Block { title, category: subtitle.clone(), subtitle, colour, on: b.enabled.unwrap_or(true), detail, symbol: b.model.clone() }
                }
                None => chain::Cell::Empty,
            }
        };
        let sel_now = match *selected {
            Some(c) if c == 10_000 + di => chain::Selected::Input,
            Some(c) if c == 20_000 + di => chain::Selected::Output,
            Some(c) if c == 40_000 + di => chain::Selected::Split,
            Some(c) if c == 50_000 + di => chain::Selected::Merge,
            Some(i) if i < blocks.len() && blocks[i].dsp == *dsp => chain::Selected::Cell(blocks[i].path as usize, blocks[i].position as usize),
            _ => chain::Selected::None,
        };
        let sp_col = split.as_ref().map(|s| s.1.clamp(0, main_cols.max(0)) as usize).unwrap_or(0);
        let jp_col = split.as_ref().map(|s| s.2.clamp(sp_col as i64, main_cols) as usize).unwrap_or(main_cols as usize);
        let lower_slots = hlx::lower_slots_of(json, dsp).max(1);
        let lower_end = (sp_col + lower_slots as usize).min(unit.positions as usize);
        let main_blocks = blocks.iter().filter(|b| &b.dsp == dsp && b.path == 0 && b.key.starts_with("block")).count() as i64;
        let max_lower = if unit.shared_slots { (unit.max_blocks - 1).max(1) } else { unit.positions };
        let can_extend_lower = split.is_some()
            && lower_slots < max_lower
            && (sp_col as i64 + lower_slots) < unit.positions;
        let can_add_lower = can_extend_lower
            && cat.is_some()
            && !full
            && (!unit.shared_slots || main_blocks < main_cols);
        let diagram = chain::Chain {
            id: dsp,
            main: (0..main_cols).map(|c| cell_of(0, c)).collect(),
            lower: split.as_ref().map(|_| (0..lower_end).map(|c| cell_of(1, c as i64)).collect()),
            split: sp_col,
            merge: jp_col,
            selected: sel_now,
            can_add: cat.is_some() && !full,
            can_extend_lower,
            can_add_lower,
            cols: main_cols as usize,
            width: ui.available_width(),
            can_split: !full,
        };
        let hits = egui::ScrollArea::horizontal().id_salt(format!("chain-{dsp}")).show(ui, |ui| diagram.show(ui)).inner;
        for hit in hits {
            let index_at = |lane: usize, col: usize| {
                blocks.iter().position(|b| &b.dsp == dsp && b.path == lane as i64 && b.position == col as i64 && b.key.starts_with("block"))
            };
            match hit {
                chain::Hit::Click(lane, col) => match index_at(lane, col) {
                    Some(i) => *selected = Some(i),
                    None if cat.is_some() && !full => {
                        *picker = Some(PickerState {
                            path: lane as i64,
                            position: col,
                            hlx_dsp: Some(dsp.clone()),
                            free: 100.0 - used,
                            ..Default::default()
                        })
                    }
                    None => {}
                },
                chain::Hit::DoubleClick(lane, col) | chain::Hit::Toggle(lane, col) => {
                    if let Some(i) = index_at(lane, col) {
                        toggle = Some(i);
                    }
                }
                chain::Hit::Change(lane, col) => menu_change = index_at(lane, col),
                chain::Hit::Remove(lane, col) => menu_remove = index_at(lane, col),
                chain::Hit::GapSplit(col) => {
                    if unit.lanes {
                        gap_split = Some((dsp.clone(), col as i64, di));
                    }
                }
                chain::Hit::GapAdd(col) | chain::Hit::GapSendReturn(col, _) => {
                    let send_return = match hit {
                        chain::Hit::GapSendReturn(_, ret) => Some(ret),
                        _ => None,
                    };
                    // The free place on the main lane nearest the line that was clicked.
                    let free_at = (0..main_cols.max(0) as usize)
                        .filter(|c| index_at(0, *c).is_none())
                        .min_by_key(|c| (*c as i64 - col as i64).abs() * 2 + if *c < col { 1 } else { 0 });
                    if let (Some(c), false) = (free_at, full) {
                        *picker = Some(PickerState {
                            path: 0,
                            position: c,
                            hlx_dsp: Some(dsp.clone()),
                            free: 100.0 - used,
                            category: if send_return.is_some() { "Send/Return".into() } else { String::new() },
                            search: send_return.map(loop_search).unwrap_or_default(),
                            ..Default::default()
                        });
                    } else {
                        note(ui.ctx(), "No free place on this path: remove or move an effect first.");
                    }
                }
                chain::Hit::Drop(from, to) => {
                    if let Some(i) = index_at(from.0, from.1) {
                        drop_to = Some((HlxDrag { dsp: dsp.clone(), key: blocks[i].key.clone() }, to.0 as i64, to.1 as i64));
                    }
                }
                chain::Hit::ExtendLower => {
                    if can_add_lower {
                        extend_dsp = Some((dsp.clone(), 100.0 - used));
                    }
                }
                chain::Hit::DropExtend(from) => {
                    if let Some(i) = index_at(from.0, from.1) {
                        drop_extend = Some(HlxDrag { dsp: dsp.clone(), key: blocks[i].key.clone() });
                    }
                }
                chain::Hit::MoveSplit(col) => {
                    *selected = Some(40_000 + di);
                    move_marker = Some((dsp.clone(), false, col as i64));
                }
                chain::Hit::MoveMerge(col) => {
                    *selected = Some(50_000 + di);
                    move_marker = Some((dsp.clone(), true, col as i64));
                }
                chain::Hit::Input => *selected = Some(10_000 + di),
                chain::Hit::Output => *selected = Some(20_000 + di),
                chain::Hit::Split => *selected = Some(40_000 + di),
                chain::Hit::Merge => *selected = Some(50_000 + di),
            }
        }
        ui.add_space(6.0);
    }
    if let Some((dsp, free)) = extend_dsp {
        if let Some(col) = hlx::extend_lower(json, &dsp, unit.positions, unit.max_blocks, unit.shared_slots) {
            *dirty = true;
            *picker = Some(PickerState {
                path: 1,
                position: col,
                hlx_dsp: Some(dsp),
                free,
                ..Default::default()
            });
        }
    }
    if let Some(p) = drop_extend {
        if let Some(b) = hlx::blocks(json).into_iter().find(|b| b.dsp == p.dsp && b.key == p.key) {
            hlx::move_to_extend_lower(json, &b, unit.positions, unit.max_blocks, unit.shared_slots);
            *dirty = true;
        }
    }
    if let Some((dsp, at, di)) = gap_split {
        let defaults = |m: &str| cat.and_then(|c| c.number_for_hlx(m, false).map(|n| c.default_values(n))).unwrap_or_default();
        hlx::add_split_at(json, &dsp, Some(at), &defaults);
        *selected = Some(40_000 + di);
        *dirty = true;
    }
    if let Some((dsp, merge, pos)) = move_marker {
        hlx::move_split_or_join(json, &dsp, merge, pos, unit.positions, unit.max_blocks, unit.shared_slots);
        *dirty = true;
    }
    if let Some(b) = menu_change.and_then(|i| blocks.get(i).map(|b| (i, b))).filter(|_| cat.is_some()) {
        let (i, b) = b;
        let used: f64 = blocks.iter().enumerate().filter(|(_, x)| x.dsp == b.dsp).map(|(k, _)| cost_of(k)).sum();
        *selected = Some(i);
        *picker = Some(PickerState {
            path: b.path,
            position: b.position as usize,
            hlx_dsp: Some(b.dsp.clone()),
            hlx_key: Some(b.key.clone()),
            free: 100.0 - used + cost_of(i),
            replacing: true,
            category: cat.and_then(|c| c.category_of_symbol(&b.model)).map(|x| x.0).unwrap_or_default(),
            ..Default::default()
        });
    }
    if let Some(b) = menu_remove.and_then(|i| blocks.get(i)) {
        hlx::remove_block(json, b);
        *dirty = true;
        *selected = None;
        return;
    }
    if let Some(i) = toggle {
        if let Some(obj) = blocks.get(i).and_then(|b| hlx::block_mut(json, b)) {
            let on = obj.get("@enabled").and_then(|v| v.as_bool()).unwrap_or(true);
            obj.insert("@enabled".into(), serde_json::Value::Bool(!on));
            *dirty = true;
        }
        snapshots::block_switched(json);
    }
    // A block dropped on another position moves there (swapping with what was there).
    if let Some((p, lane, pos)) = drop_to {
        if let Some(b) = blocks.iter().find(|b| b.dsp == p.dsp && b.key == p.key) {
            hlx::move_to(json, b, lane, pos);
            hlx::fit_lanes(json, &p.dsp, unit.positions, unit.max_blocks, unit.shared_slots);
            *dirty = true;
        }
    }
    }
    if !panel_part() {
        return;
    }

    // Split or merge settings.
    if let Some(code) = selected.filter(|s| (40_000..60_000).contains(s)) {
        let merge = code >= 50_000;
        let di = code % 10_000;
        let Some(dsp) = dsps.get(di).cloned() else { return };
        let Some((model, sp, jp)) = hlx::split_of(json, &dsp) else { return };
        let mut removed = false;
        theme::card(ui, |ui| {
            ui.horizontal(|ui| {
                block_header(ui, family_colour("io"), if merge { "Merge" } else { "Split" }, "");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .button(RichText::new("Remove split").color(BAD))
                        .on_hover_text("Back to a single lane; lower-lane blocks move to free places on the main lane")
                        .clicked()
                    {
                        hlx::remove_split(json, &dsp);
                        *dirty = true;
                        removed = true;
                    }
                });
            });
            if removed {
                return;
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if !merge {
                    ui.label(RichText::new("Type").color(MUTED));
                    let current = hlx::SPLITS.iter().find(|s| s.0 == model).map(|s| s.1).unwrap_or("Split");
                    egui::ComboBox::from_id_salt(format!("split-{dsp}")).selected_text(current).show_ui(ui, |ui| {
                        for (m, n) in hlx::SPLITS {
                            if ui.selectable_label(m == model, n).clicked() {
                                let params = cat.and_then(|c| c.number_for_hlx(m, false).map(|x| c.default_values(x))).unwrap_or_default();
                                hlx::set_split_model(json, &dsp, m, &params);
                                *dirty = true;
                            }
                        }
                    });
                    ui.add_space(12.0);
                }
                ui.label(RichText::new("Position").color(MUTED));
                let main_cols = hlx::main_cols_for(json, &dsp, unit.positions, unit.max_blocks, unit.shared_slots);
                let lower_slots = hlx::lower_slots_of(json, &dsp).max(1);
                let min_p = if merge { sp } else { 0 };
                let max_p = if merge {
                    main_cols
                } else {
                    jp.min(main_cols).min(unit.positions - lower_slots).max(0)
                };
                let mut p = (if merge { jp } else { sp }).clamp(min_p, max_p);
                let mut moved = false;
                if ui.add_enabled(p > min_p, egui::Button::new("◀").small()).on_hover_text("Move earlier in the chain").clicked() {
                    p -= 1;
                    moved = true;
                }
                if ui.add(egui::DragValue::new(&mut p).range(min_p..=max_p)).on_hover_text("Which chain position it sits before (or drag the dot on the path)").changed() {
                    moved = true;
                }
                if ui.add_enabled(p < max_p, egui::Button::new("▶").small()).on_hover_text("Move later in the chain").clicked() {
                    p += 1;
                    moved = true;
                }
                if moved {
                    hlx::move_split_or_join(json, &dsp, merge, p, unit.positions, unit.max_blocks, unit.shared_slots);
                    *dirty = true;
                }
            });
            ui.add_space(8.0);
            let model_id = if merge { hlx::JOIN_MODEL.to_owned() } else { model.clone() };
            let info = cat.and_then(|c| c.info_for_symbol(&model_id)).cloned();
            let Some(obj) = hlx::split_mut(json, &dsp, merge) else { return };
            let keys: Vec<String> = obj.keys().filter(|k| !k.starts_with('@')).cloned().collect();
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(6.0, 12.0);
                for k in keys {
                    let meta = info.as_ref().and_then(|i| i.params.iter().find(|p| p.id == k)).cloned();
                    let label = meta.as_ref().map(|m| m.name.clone()).filter(|s| !s.is_empty()).unwrap_or_else(|| k.clone());
                    let Some(v) = obj.get_mut(&k) else { continue };
                    let val = match &*v {
                        serde_json::Value::Bool(b) => Value::Bool(*b),
                        serde_json::Value::Number(x) if x.is_i64() => Value::int(x.as_i64().unwrap_or(0)),
                        other => Value::F64(other.as_f64().unwrap_or(0.0)),
                    };
                    if let (Some(nv), _) = param_cell(ui, &label, meta.as_ref(), &val) {
                        *v = match nv {
                            Value::Bool(b) => serde_json::Value::Bool(b),
                            Value::Int(i, _) => serde_json::json!(i),
                            other => serde_json::json!(other.as_f64().unwrap_or(0.0)),
                        };
                        *dirty = true;
                    }
                }
            });
        });
        return;
    }
    // Input or output settings.
    if let Some(code) = selected.filter(|s| (10_000..30_000).contains(s)) {
        let input = code < 20_000;
        let di = code % 10_000;
        let Some(dsp) = dsps.get(di).cloned() else { return };
        let fam = snapshots::family(json);
        theme::card(ui, |ui| {
            ui.horizontal(|ui| block_header(ui, line(), if input { "Input" } else { "Output" }, ""));
            ui.add_space(6.0);
            // Guitar input impedance and pad live with the preset's global settings.
            if input {
                let global = json.pointer_mut("/data/tone/global").and_then(|g| g.as_object_mut());
                if let Some(g) = global {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Guitar In-Z").color(MUTED));
                        let mut z = g.get("@guitarinputZ").and_then(|v| v.as_i64()).unwrap_or(0);
                        let choices: Vec<(i64, &str)> = GUITAR_IN_Z.iter().enumerate().map(|(i, n)| (i as i64, *n)).collect();
                        if choice_combo(ui, "inz", &mut z, &choices) {
                            g.insert("@guitarinputZ".into(), serde_json::json!(z));
                            *dirty = true;
                        }
                        ui.add_space(12.0);
                        ui.label(RichText::new("Guitar Pad").color(MUTED));
                        let mut pad = g.get("@guitarpad").and_then(|v| v.as_i64()).unwrap_or(0);
                        for (i, n) in GUITAR_PAD.iter().enumerate() {
                            if ui.selectable_label(pad == i as i64, *n).clicked() {
                                pad = i as i64;
                                g.insert("@guitarpad".into(), serde_json::json!(pad));
                                *dirty = true;
                            }
                        }
                    });
                    ui.add_space(4.0);
                }
            }
            let Some(obj) = hlx::io_mut(json, &dsp, input) else {
                ui.label(RichText::new("This preset has no settings here.").color(MUTED));
                return;
            };
            let route_key = if input { "@input" } else { "@output" };
            let mut route = obj.get(route_key).and_then(|v| v.as_i64()).unwrap_or(1);
            ui.horizontal(|ui| {
                ui.label(RichText::new(if input { "Source" } else { "Destination" }).color(MUTED));
                if choice_combo(ui, &format!("route-{dsp}-{input}"), &mut route, &route_names(fam, input)) {
                    obj.insert(route_key.into(), serde_json::json!(route));
                    *dirty = true;
                }
            });
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(6.0, 12.0);
                for (label, meta) in io_params(input) {
                    let Some(v) = obj.get_mut(&meta.id) else { continue };
                    let val = match &*v {
                        serde_json::Value::Bool(b) => Value::Bool(*b),
                        other => Value::F64(other.as_f64().unwrap_or(0.0)),
                    };
                    if let (Some(nv), _) = param_cell(ui, label, Some(&meta), &val) {
                        *v = match nv {
                            Value::Bool(b) => serde_json::Value::Bool(b),
                            other => serde_json::json!(other.as_f64().unwrap_or(0.0)),
                        };
                        *dirty = true;
                    }
                }
            });
        });
        return;
    }
    let Some(sel) = selected.filter(|s| *s < blocks.len()) else {
        ui.label(RichText::new("Pick a block above to see its knobs, or click + to add an effect.").color(MUTED));
        return;
    };
    let b = blocks[sel].clone();
    let (name, cname, col) = name_of(&b.model);
    let is_stereo = hlx::block(json, &b).and_then(|o| o.get("@stereo")).and_then(|v| v.as_bool()).unwrap_or(false);
    let info = cat.and_then(|c| c.info_for_hlx(&b.model, is_stereo)).cloned();
    let mut removed = false;
    let mut switched = false;
    let fam = snapshots::family(json);
    theme::card(ui, |ui| {
        ui.horizontal(|ui| {
            let kind = crate::icons::kind_label(&cname, hlx::block(json, &b).and_then(|o| o.get("@cab")).is_some());
            block_header_icon(ui, col, &kind, &b.model, &name, &based_line(&name, crate::modelinfo::based_on(&b.model)));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(obj) = hlx::block_mut(json, &b) {
                    let mut on = obj.get("@enabled").and_then(|v| v.as_bool()).unwrap_or(true);
                    let label = if on { "  ON  " } else { "  OFF  " };
                    if ui.toggle_value(&mut on, label).on_hover_text("On/off is kept in the current snapshot").changed() {
                        obj.insert("@enabled".into(), serde_json::Value::Bool(on));
                        *dirty = true;
                        switched = true;
                    }
                }
                if ui.button(RichText::new("Remove").color(BAD)).clicked() {
                    hlx::remove_block(json, &b);
                    *dirty = true;
                    removed = true;
                    *selected = None;
                }
                if cat.is_some() && ui.button("Change effect...").clicked() {
                    let used: f64 = blocks.iter().enumerate().filter(|(_, x)| x.dsp == b.dsp).map(|(i, _)| cost_of(i)).sum();
                    *picker = Some(PickerState {
                        path: b.path,
                        position: b.position as usize,
                        hlx_dsp: Some(b.dsp.clone()),
                        hlx_key: Some(b.key.clone()),
                        free: 100.0 - used + cost_of(sel),
                        replacing: true,
                        category: cat.and_then(|c| c.category_of_symbol(&b.model)).map(|x| x.0).unwrap_or_default(),
                        ..Default::default()
                    });
                }
            });
        });
        if removed {
            return;
        }
        ui.add_space(8.0);
        // An amp + cab (or dual cab) shows the cab's photo under the amp's.
        let cab_sym: Option<String> = hlx::block(json, &b)
            .and_then(|o| o.get("@cab"))
            .and_then(|v| v.as_str())
            .and_then(|k| json.pointer(&format!("/data/tone/{}/{k}/@model", b.dsp)))
            .and_then(|m| m.as_str())
            .map(str::to_owned);
        let side = crate::modelinfo::has_side(ui, &b.model, cab_sym.as_deref()) && ui.available_width() > 700.0;
        let full = ui.available_width();
        ui.horizontal_top(|ui| {
        if side {
            ui.vertical(|ui| {
                ui.set_width(SIDE_W);
                crate::modelinfo::side(ui, &b.model, SIDE_W, cab_sym.as_deref());
            });
            ui.add_space(10.0);
            let (r, _) = ui.allocate_exact_size(Vec2::new(1.0, 150.0), egui::Sense::hover());
            ui.painter().rect_filled(r, CornerRadius::ZERO, line());
            ui.add_space(10.0);
        }
        let rest = if side { full - SIDE_W - 21.0 } else { full };
        ui.vertical(|ui| {
        ui.set_max_width(rest);
        ui.set_min_width(rest.min(200.0));
        // Footswitch for this block's on/off.
        ui.horizontal(|ui| {
            ui.label(RichText::new("Footswitch").color(MUTED));
            let current = snapshots::footswitch(json, &b);
            let mut fs = current.map(|c| c.0).unwrap_or(0);
            let mut choices: Vec<(i64, String)> = vec![(0, "None".into())];
            for n in 1..=fam.fs_count.min(5) {
                choices.push((n, format!("FS {n}")));
            }
            let named: Vec<(i64, &str)> = choices.iter().map(|(v, n)| (*v, n.as_str())).collect();
            let mut momentary = current.map(|c| c.1).unwrap_or(false);
            let mut changed = choice_combo(ui, &format!("fs-{}-{}", b.dsp, b.key), &mut fs, &named);
            if fs > 0 {
                changed |= ui.checkbox(&mut momentary, "Momentary").on_hover_text("On only while the switch is held").changed();
            }
            if changed {
                snapshots::set_footswitch(json, &b, (fs > 0).then_some(fs), momentary, &name);
                *dirty = true;
            }
            theme::help(ui, "The footswitch that turns this block on and off. Momentary keeps it on only while you hold the switch.");
        });
        ui.add_space(8.0);
        let normalize_block = |obj: &mut serde_json::Map<String, serde_json::Value>, minfo: Option<&catalog::ModelInfo>| {
            let Some(mi) = minfo else { return };
            for p in &mi.params {
                if p.id.starts_with('@') && !matches!(p.id.as_str(), "@mic" | "@trails") {
                    continue;
                }
                if let Some(v) = obj.get_mut(&p.id) {
                    if let Some(f) = v.as_f64() {
                        if p.value_type == 1 && p.max > p.min {
                            let mut nf = f;
                            if p.min == 0.0 && p.max == 1.0 && nf > 1.0 && nf <= 10.0 {
                                nf /= 10.0;
                            }
                            nf = nf.clamp(p.min, p.max);
                            if (nf - f).abs() > 1e-9 {
                                *v = serde_json::json!(nf);
                            }
                        }
                    }
                } else {
                    let dv = match p.value_type {
                        2 => serde_json::Value::Bool(p.default == 1.0),
                        0 => serde_json::json!(p.default.round() as i64),
                        _ => serde_json::json!(p.default),
                    };
                    obj.insert(p.id.clone(), dv);
                }
            }
        };
        if let Some(obj) = hlx::block_mut(json, &b) {
            normalize_block(obj, info.as_ref());
        }
        let ordered_keys = |obj: &serde_json::Map<String, serde_json::Value>, minfo: Option<&catalog::ModelInfo>| -> Vec<String> {
            let mut out = Vec::new();
            if let Some(mi) = minfo {
                for p in &mi.params {
                    if (!p.id.starts_with('@') || matches!(p.id.as_str(), "@mic" | "@trails")) && obj.contains_key(&p.id) {
                        out.push(p.id.clone());
                    }
                }
            }
            for k in obj.keys() {
                if !k.starts_with('@') && !out.contains(k) {
                    out.push(k.clone());
                }
            }
            out
        };
        let keys: Vec<String> = hlx::block(json, &b).map(|o| ordered_keys(o, info.as_ref())).unwrap_or_default();
        let mut moved: Option<(String, serde_json::Value)> = None;
        let to_msg_val = |v: &serde_json::Value, meta: Option<&Param>| -> Option<Value> {
            match (v, meta.map(|m| m.value_type)) {
                (serde_json::Value::Bool(x), _) => Some(Value::Bool(*x)),
                (serde_json::Value::Number(x), Some(2)) => Some(Value::Bool(x.as_f64().unwrap_or(0.0) != 0.0)),
                (serde_json::Value::Number(x), Some(0)) => Some(Value::int(x.as_f64().unwrap_or(0.0).round() as i64)),
                (serde_json::Value::Number(x), Some(1)) => Some(Value::F64(x.as_f64().unwrap_or(0.0))),
                (serde_json::Value::Number(x), _) if x.is_i64() || x.is_u64() => Some(Value::int(x.as_i64().unwrap_or(0))),
                (serde_json::Value::Number(x), _) => Some(Value::F64(x.as_f64().unwrap_or(0.0))),
                _ => None,
            }
        };
        let per_row = ((ui.available_width() + 6.0) / 116.0).floor().max(1.0) as usize;
        for row in keys.chunks(per_row) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(6.0, 12.0);
            for k in row.iter() {
                let meta = info.as_ref().and_then(|i| i.params.iter().find(|p| &p.id == k)).cloned();
                let label = meta.as_ref().map(|m| m.name.clone()).filter(|s| !s.is_empty()).unwrap_or_else(|| k.clone());
                ui.vertical(|ui| {
                    ui.set_width(104.0);
                    let Some(v) = hlx::block_mut(json, &b).and_then(|o| o.get_mut(k)) else { return };
                    match to_msg_val(v, meta.as_ref()) {
                        Some(val) => {
                            if let (Some(nv), _) = param_cell(ui, &label, meta.as_ref(), &val) {
                                let old = v.clone();
                                *v = match nv {
                                    Value::Bool(x) => serde_json::Value::Bool(x),
                                    Value::Int(i, _) => serde_json::json!(i),
                                    other => serde_json::json!(other.as_f64().unwrap_or(0.0)),
                                };
                                *dirty = true;
                                moved = Some((k.clone(), old));
                            }
                            ui.vertical_centered(|ui| assign_menu(ui, json, &b, k, meta.as_ref(), dirty));
                        }
                        None => {
                            ui.label(RichText::new(&label).small().color(MUTED));
                            if let serde_json::Value::String(s) = v {
                                if ui.add(egui::TextEdit::singleline(s).desired_width(100.0)).changed() {
                                    *dirty = true;
                                }
                            } else {
                                ui.label(RichText::new(v.to_string()).small().color(MUTED));
                            }
                        }
                    }
                });
            }
        });
        }
        if let Some((k, old)) = moved {
            snapshots::knob_moved(json, &b, &k, &old);
        }
        // If this block carries a paired cab (`@cab: "cab0"`), show the cab's controls below.
        let cab_key = hlx::block(json, &b).and_then(|o| o.get("@cab")).and_then(|v| v.as_str()).map(str::to_owned);
        if let Some(ck) = cab_key {
            let cab_block = hlx::HlxBlock {
                dsp: b.dsp.clone(),
                key: ck,
                model: String::new(),
                path: b.path,
                position: b.position,
                enabled: Some(true),
            };
            if let Some(cab_model) = hlx::block(json, &cab_block).and_then(|o| o.get("@model")).and_then(|v| v.as_str()).map(str::to_owned) {
                let (cab_name, _, cab_col) = name_of(&cab_model);
                let cab_info = cat.and_then(|c| c.info_for_hlx(&cab_model, false)).cloned();
                let cab_keys: Vec<String> = hlx::block(json, &cab_block).map(|o| ordered_keys(o, cab_info.as_ref())).unwrap_or_default();
                if !cab_keys.is_empty() {
                    ui.add_space(10.0);
                    ui.separator();
                    let dual = b.model.starts_with("HD2_Cab");
                    let mut pick = None;
                    ui.horizontal(|ui| {
                        block_header(ui, cab_col, &cab_name, if dual { "second cab" } else { "cab in this block" });
                        if let (Some(c), Some(cur)) = (cat, cat.and_then(|c| c.number_for_hlx(&cab_model, false))) {
                            pick = cab_menu(ui, c, cur, dual);
                        }
                    });
                    // A new second cab: its model, with its own default knobs.
                    if let (Some(new), Some(c)) = (pick, cat) {
                        if let (Some((hlx_model, _)), Some(obj)) = (c.hlx_model(new), hlx::block_mut(json, &cab_block)) {
                            obj.retain(|k, _| k.starts_with('@'));
                            obj.insert("@model".into(), serde_json::Value::String(hlx_model));
                            for (id, vt, d) in c.default_values(new) {
                                let v = match vt {
                                    2 => serde_json::Value::Bool(d != 0.0),
                                    0 => serde_json::json!(d.round() as i64),
                                    _ => serde_json::json!(d),
                                };
                                obj.insert(id, v);
                            }
                            *dirty = true;
                        }
                    }
                    ui.add_space(4.0);
                    let mut cab_moved: Option<(String, serde_json::Value)> = None;
                    ui.push_id(&cab_block.key, |ui| {
                        let per_row = ((ui.available_width() + 6.0) / 116.0).floor().max(1.0) as usize;
                        for row in cab_keys.chunks(per_row) {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing = Vec2::new(6.0, 12.0);
                            for k in row.iter() {
                                let meta = cab_info.as_ref().and_then(|i| i.params.iter().find(|p| &p.id == k)).cloned();
                                let label = meta.as_ref().map(|m| m.name.clone()).filter(|s| !s.is_empty()).unwrap_or_else(|| k.clone());
                                ui.vertical(|ui| {
                                    ui.set_width(104.0);
                                    let Some(v) = hlx::block_mut(json, &cab_block).and_then(|o| o.get_mut(k)) else { return };
                                    if let Some(val) = to_msg_val(v, meta.as_ref()) {
                                        if let (Some(nv), _) = param_cell(ui, &label, meta.as_ref(), &val) {
                                            let old = v.clone();
                                            *v = match nv {
                                                Value::Bool(x) => serde_json::Value::Bool(x),
                                                Value::Int(i, _) => serde_json::json!(i),
                                                other => serde_json::json!(other.as_f64().unwrap_or(0.0)),
                                            };
                                            *dirty = true;
                                            cab_moved = Some((k.clone(), old));
                                        }
                                        ui.vertical_centered(|ui| assign_menu(ui, json, &cab_block, k, meta.as_ref(), dirty));
                                    }
                                });
                            }
                        });
                        }
                    });
                    if let Some((k, old)) = cab_moved {
                        snapshots::knob_moved(json, &cab_block, &k, &old);
                    }
                }
            }
        }
        });
        });
    });
    if switched {
        snapshots::block_switched(json);
    }
}
