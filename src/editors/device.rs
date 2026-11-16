//! The editor for the pedal's own preset documents, and its controllers card.

use super::*;

// ------------------------------------------------------------ pedal preset ---

pub fn device_editor(ui: &mut egui::Ui, ed: DeviceEdit, selected: &mut Option<usize>, picker: &mut Option<PickerState>) {
    let DeviceEdit { preset, live, cmds, cat, dirty } = ed;
    if chain_part() {
        let slots = preset.slots();
        let mut toggle = None;
        let mut menu_change: Option<usize> = None;
        let mut menu_remove: Option<usize> = None;
        let mut dev_drop_cell: Option<(SlotDrag, i64, usize, usize)> = None;
        let dev_drop = &mut dev_drop_cell;

        // Each path: the main row between input and output, and a lower row
        // between split and merge when it holds anything.
        let mut paths: Vec<i64> = slots.iter().map(|s| s.path).collect();
        paths.dedup();
        for path in paths {
            let in_path: Vec<&SlotView> = slots.iter().filter(|s| s.path == path).collect();
            let pos = |k: Kind| in_path.iter().find(|s| s.kind == k).map(|s| s.position);
            let (inp, out, split, join) = (pos(Kind::Input), pos(Kind::Output), pos(Kind::Split), pos(Kind::Join));
            // The lower lane is the stretch between the split and merge slots; the
            // main lane is everything between input and output that is not in it.
            let split_slot = in_path.iter().find(|s| s.kind == Kind::Split).copied();
            let join_slot = in_path.iter().find(|s| s.kind == Kind::Join).copied();
            let lower: Vec<&SlotView> = match (split, join) {
                (Some(a), Some(b)) if a < b => in_path.iter().copied().filter(|s| s.position > a && s.position < b).collect(),
                _ => Vec::new(),
            };
            let main: Vec<&SlotView> = in_path
                .iter()
                .copied()
                .filter(|s| {
                    let inside = match (inp, out) {
                        (Some(a), Some(b)) => s.position > a && s.position < b,
                        _ => true,
                    };
                    inside
                        && !matches!(s.kind, Kind::Input | Kind::Output | Kind::Split | Kind::Join)
                        && !lower.iter().any(|l| l.global == s.global)
                })
                .take(chain::COLS)
                .collect();
            let lower_used = lower.iter().any(|s| s.kind != Kind::Empty);
            // How much of this path's DSP the blocks use.
            let used = cat.map(|c| slots_dsp(c, &in_path)).unwrap_or(0.0);
            let free = 100.0 - used;
            // Split and merge columns, from the main-lane slot each attaches before.
            let cols = main.len().clamp(1, chain::COLS);
            let col_before = |attach: Option<i64>| attach.map(|a| main.iter().filter(|s| (s.position as i64) < a).count());
            let split_col = split_slot.and_then(|s| col_before(preset.attach(s.path, s.position))).unwrap_or(0).min(cols.saturating_sub(1));
            let merge_col = join_slot.and_then(|s| col_before(preset.attach(s.path, s.position))).unwrap_or(cols).clamp(split_col, cols);
            let lower_selected = selected.is_some_and(|g| {
                lower.iter().any(|s| s.global == g) || split_slot.is_some_and(|s| s.global == g) || join_slot.is_some_and(|s| s.global == g)
            });
            // The lane shows while the path is split (the pedal's own flag), or holds or is being edited.
            let show_lower = !lower.is_empty() && (lower_used || lower_selected || preset.split_active(path));
            // On units whose two lanes share the slots, a full path has no room for a lower lane.
            let dev_fam = snapshots::family_of_device(crate::nam::unit());
            let path_full = in_path.iter().filter(|s| s.kind == Kind::Block).count() as i64 >= dev_fam.max_blocks;
            // The lower lane is drawn packed: its effects in order from the split, then one free place.
            // (The pedal keeps them in numbered slots with gaps; only their order matters.)
            let lower_view: Vec<&SlotView> = lower
                .iter()
                .copied()
                .filter(|s| s.kind != Kind::Empty)
                .chain(lower.iter().copied().find(|s| s.kind == Kind::Empty))
                .collect();
            let lower_end = (split_col + lower_view.len()).max(split_col + 1);

            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("PATH {}", path + 1)).small().strong().color(MUTED));
            });
            let in_global = in_path.iter().find(|s| s.kind == Kind::Input).map(|s| s.global);
            let out_global = in_path.iter().find(|s| s.kind == Kind::Output).map(|s| s.global);
            let to_cell = |s: &SlotView| -> chain::Cell {
                if s.kind == Kind::Empty {
                    return chain::Cell::Empty;
                }
                let (title, cname, colour) = s.model.map(|m| describe(cat, m)).unwrap_or((s.kind.label().to_owned(), String::new(), MUTED));
                // An amp carrying its cab reads "Amp+Cab" (a cab carrying a second one, "Dual Cab").
                let kind = crate::icons::kind_label(&cname, s.paired.is_some());
                let cname_for_icon = kind.clone();
                let subtitle = kind;
                // The NAM block shows the capture it plays.
                let detail = match (s.model, cat) {
                    (Some(m), Some(k)) if k.symbol(m).is_some_and(crate::nam::is_nam_symbol) => k
                        .info(m)
                        .and_then(|i| i.params.iter().position(|p| p.id == "MinFreq"))
                        .and_then(|ix| s.values.get(ix))
                        .and_then(|v| v.as_f64())
                        .map(nam_detail),
                    _ => None,
                };
                let symbol = s.model.zip(cat).and_then(|(m, k)| k.symbol(m)).unwrap_or_default().to_owned();
                chain::Cell::Block { title, subtitle, colour, on: s.enabled.unwrap_or(true), detail, category: cname_for_icon, symbol }
            };
            let lower_at = |col: usize| -> Option<&SlotView> { col.checked_sub(split_col).and_then(|i| lower_view.get(i).copied()) };
            let sel_now = match *selected {
                Some(g) if Some(g) == in_global => chain::Selected::Input,
                Some(g) if Some(g) == out_global => chain::Selected::Output,
                Some(g) if split_slot.is_some_and(|s| s.global == g) => chain::Selected::Split,
                Some(g) if join_slot.is_some_and(|s| s.global == g) => chain::Selected::Merge,
                Some(g) => match main.iter().position(|s| s.global == g) {
                    Some(c) => chain::Selected::Cell(0, c),
                    None => match lower_view.iter().position(|s| s.global == g) {
                        Some(i) => chain::Selected::Cell(1, i + split_col),
                        None => chain::Selected::None,
                    },
                },
                None => chain::Selected::None,
            };
            let diagram = chain::Chain {
                id: &format!("dev{path}"),
                main: main.iter().map(|s| to_cell(s)).collect(),
                lower: show_lower.then(|| (0..lower_end).map(|c| lower_at(c).map(to_cell).unwrap_or(chain::Cell::Empty)).collect()),
                split: split_col,
                merge: merge_col,
                selected: sel_now,
                can_add: cat.is_some(),
                can_extend_lower: false,
                can_add_lower: cat.is_some() && !path_full,
                cols,
                width: ui.available_width(),
                can_split: !path_full,
                unit: dev_fam.name,
            };
            let hits = egui::ScrollArea::horizontal().id_salt(format!("dchain-{path}")).show(ui, |ui| diagram.show(ui)).inner;
            let slot_at = |lane: usize, col: usize| -> Option<&SlotView> {
                if lane == 0 {
                    main.get(col).copied()
                } else {
                    lower_at(col)
                }
            };
            for hit in hits {
                match hit {
                    chain::Hit::Click(lane, col) => {
                        if let Some(s) = slot_at(lane, col) {
                            *selected = Some(s.global);
                            if s.kind == Kind::Empty && cat.is_some() {
                                *picker = Some(PickerState {
                                    block: s.wire_block(),
                                    slot: s.global,
                                    path: s.path,
                                    position: s.position,
                                    free,
                                    ..Default::default()
                                });
                            }
                        }
                    }
                    chain::Hit::DoubleClick(lane, col) | chain::Hit::Toggle(lane, col) => {
                        if let Some(s) = slot_at(lane, col).filter(|s| s.enabled.is_some()) {
                            toggle = Some(s.global);
                        }
                    }
                    chain::Hit::Change(lane, col) => menu_change = slot_at(lane, col).map(|s| s.global),
                    chain::Hit::Remove(lane, col) => menu_remove = slot_at(lane, col).map(|s| s.global),
                    chain::Hit::GapSplit(col) => {
                        // Split before this column, merging again one place later.
                        if let (Some(sp), Some(jp)) = (split_slot, join_slot) {
                            let attach_at = |c: usize| main.get(c).map(|s| s.position as i64).unwrap_or_else(|| out.unwrap_or(9) as i64);
                            let sa = attach_at(col);
                            let ja = attach_at((col + 1).min(cols));
                            // The pedal refuses a split that would sit after the merge, even for a
                            // moment: move whichever keeps the split before the merge first.
                            let old_join = preset.attach(jp.path, jp.position).unwrap_or(i64::MAX);
                            let join_first = sa >= old_join;
                            preset.set_attach(sp.path, sp.position, sa);
                            preset.set_attach(jp.path, jp.position, ja);
                            if live {
                                if join_first {
                                    cmds.push(Cmd::MoveJoin { block: 20 * jp.path + ja });
                                    cmds.push(Cmd::MoveSplit { block: 20 * sp.path + sa });
                                } else {
                                    cmds.push(Cmd::MoveSplit { block: 20 * sp.path + sa });
                                    cmds.push(Cmd::MoveJoin { block: 20 * jp.path + ja });
                                }
                            }
                            *dirty = true;
                            *selected = Some(sp.global);
                        }
                    }
                    chain::Hit::GapAdd(col) | chain::Hit::GapSendReturn(col, _) => {
                        // The free place nearest the line that was clicked.
                        // A return only goes at or after the line that was clicked: before it, it
                        // could land ahead of its own send.
                        let is_return = matches!(hit, chain::Hit::GapSendReturn(_, 2));
                        let free_at = (0..main.len())
                            .filter(|c| main[*c].kind == Kind::Empty && (!is_return || *c >= col))
                            .min_by_key(|c| (*c as i64 - col as i64).abs() * 2 + if *c < col { 1 } else { 0 });
                        match free_at.map(|c| main[c]) {
                            Some(s) => {
                                *selected = Some(s.global);
                                let (category, search) = match hit {
                                    chain::Hit::GapSendReturn(_, k) => ("Send/Return".to_owned(), loop_search(k)),
                                    _ => (String::new(), String::new()),
                                };
                                *picker = Some(PickerState {
                                    block: s.wire_block(),
                                    slot: s.global,
                                    path: s.path,
                                    position: s.position,
                                    free,
                                    category,
                                    search,
                                    ..Default::default()
                                });
                            }
                            None => {
                                let msg = if is_return {
                                    "No free place for a return after this point: put it further right, or free a place."
                                } else {
                                    "No free place on this path: remove or move an effect first."
                                };
                                note(ui.ctx(), msg);
                            }
                        }
                    }
                    chain::Hit::Drop(from, to) => {
                        if let (Some(a), Some(b)) = (slot_at(from.0, from.1), slot_at(to.0, to.1)) {
                            *dev_drop =
                                Some((SlotDrag { path: a.path, position: a.position, global: a.global }, b.path, b.position, b.global));
                        }
                    }
                    chain::Hit::ExtendLower => {
                        if let Some(s) = lower.get(lower_end.saturating_sub(split_col)) {
                            if let Some(jp) = join_slot.filter(|_| merge_col == lower_end) {
                                let next_col = (lower_end + 1).min(cols);
                                let attach = main.get(next_col).map(|s| s.position as i64).unwrap_or_else(|| out.unwrap_or(9) as i64);
                                preset.set_attach(jp.path, jp.position, attach);
                                if live {
                                    cmds.push(Cmd::MoveJoin { block: 20 * jp.path + attach });
                                }
                            }
                            *dirty = true;
                            *selected = Some(s.global);
                            if s.kind == Kind::Empty && cat.is_some() {
                                *picker = Some(PickerState {
                                    block: s.wire_block(),
                                    slot: s.global,
                                    path: s.path,
                                    position: s.position,
                                    free,
                                    ..Default::default()
                                });
                            }
                        }
                    }
                    chain::Hit::DropExtend(from) => {
                        if let (Some(a), Some(b)) = (slot_at(from.0, from.1), lower.get(lower_end.saturating_sub(split_col)).copied()) {
                            if let Some(jp) = join_slot.filter(|_| merge_col == lower_end) {
                                let next_col = (lower_end + 1).min(cols);
                                let attach = main.get(next_col).map(|s| s.position as i64).unwrap_or_else(|| out.unwrap_or(9) as i64);
                                preset.set_attach(jp.path, jp.position, attach);
                                if live {
                                    cmds.push(Cmd::MoveJoin { block: 20 * jp.path + attach });
                                }
                            }
                            *dev_drop =
                                Some((SlotDrag { path: a.path, position: a.position, global: a.global }, b.path, b.position, b.global));
                        }
                    }
                    chain::Hit::MoveSplit(col) => {
                        if let Some(sp) = split_slot {
                            let target = col.min(merge_col).min(cols.saturating_sub(1));
                            let attach = main.get(target).map(|s| s.position as i64).unwrap_or_else(|| out.unwrap_or(9) as i64);
                            preset.set_attach(sp.path, sp.position, attach);
                            if live {
                                cmds.push(Cmd::MoveSplit { block: 20 * sp.path + attach });
                            }
                            *selected = Some(sp.global);
                            *dirty = true;
                        }
                    }
                    chain::Hit::MoveMerge(col) => {
                        if let Some(jp) = join_slot {
                            let target = col.clamp(split_col, cols);
                            let attach = main.get(target).map(|s| s.position as i64).unwrap_or_else(|| out.unwrap_or(9) as i64);
                            preset.set_attach(jp.path, jp.position, attach);
                            if live {
                                cmds.push(Cmd::MoveJoin { block: 20 * jp.path + attach });
                            }
                            *selected = Some(jp.global);
                            *dirty = true;
                        }
                    }
                    chain::Hit::Input => *selected = in_global,
                    chain::Hit::Output => *selected = out_global,
                    chain::Hit::Split => *selected = split_slot.map(|s| s.global),
                    chain::Hit::Merge => *selected = join_slot.map(|s| s.global),
                }
            }
            ui.add_space(6.0);
        }
        // A block dropped on another position of its path moves there.
        if let Some((from, path, pos, global)) = dev_drop_cell.take() {
            if from.path == path && from.global != global {
                if live {
                    cmds.push(Cmd::MoveBlock { from: 20 * from.path + from.position as i64, to: 20 * path + pos as i64 });
                }
                preset.swap_slots(path, from.position, pos);
                *selected = Some(global);
                *dirty = true;
            }
        }
        if let Some(g) = menu_remove.filter(|g| *g < slots.len()) {
            let s = &slots[g];
            if s.kind == Kind::Block {
                if live {
                    cmds.push(Cmd::ClearBlock { block: s.wire_block() });
                }
                preset.clear_slot(s.path, s.position);
                *selected = Some(g);
                *dirty = true;
            }
        }
        if let (Some(g), Some(c)) = (menu_change.filter(|g| *g < slots.len()), cat) {
            *selected = Some(g);
            *picker = Some(change_picker(c, &slots, &slots[g]));
        }
        if let Some(g) = toggle {
            let s = &slots[g];
            let on = !s.enabled.unwrap_or(true);
            preset.set_enabled(s.path, s.position, on);
            *dirty = true;
            if live {
                cmds.push(Cmd::SetEnabled { block: s.wire_block(), enabled: on });
            }
        }
    }
    if !panel_part() {
        return;
    }
    let slots = preset.slots();
    // Nothing picked yet (just opened): show the amp's knobs, or else the first effect's.
    if selected.is_none() {
        let blocks = || slots.iter().enumerate().filter(|(_, s)| s.kind == Kind::Block && s.model.is_some());
        let is_amp = |s: &SlotView| cat.zip(s.model).is_some_and(|(c, m)| c.category_name(m).starts_with("Amp"));
        *selected = blocks().find(|(_, s)| is_amp(s)).or_else(|| blocks().next()).map(|(i, _)| i);
    }

    let Some(sel) = selected.filter(|s| *s < slots.len()) else {
        ui.add_space(20.0);
        ui.vertical_centered(|ui| ui.label(RichText::new("Pick a block in the chain to see its knobs.").color(MUTED)));
        return;
    };
    let s = slots[sel].clone();
    theme::card(ui, |ui| {
        // Header: the effect, its switch, and swapping or removing it.
        ui.horizontal(|ui| {
            if s.kind == Kind::Empty {
                block_header(ui, line(), "Empty slot", &format!("position {}", s.wire_block()));
            } else {
                let (name, cname, col) = s.model.map(|m| describe(cat, m)).unwrap_or((s.kind.label().to_owned(), "".into(), MUTED));
                let based = s.model.zip(cat).and_then(|(m, c)| c.symbol(m)).and_then(crate::modelinfo::based_on);
                let symbol = s.model.zip(cat).and_then(|(m, c)| c.symbol(m)).unwrap_or_default().to_owned();
                let kind = crate::icons::kind_label(&cname, s.paired.is_some());
                block_header_icon(ui, col, &kind, &symbol, &name, &based_line(&name, based));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(mut on) = s.enabled {
                    let label = if on { "  ON  " } else { "  OFF  " };
                    if ui.toggle_value(&mut on, label).changed() {
                        preset.set_enabled(s.path, s.position, on);
                        *dirty = true;
                        if live {
                            cmds.push(Cmd::SetEnabled { block: s.wire_block(), enabled: on });
                        }
                    }
                }
                if matches!(s.kind, Kind::Block | Kind::Empty) {
                    if s.kind == Kind::Block && ui.button(RichText::new("Remove").color(BAD)).clicked() {
                        if live {
                            cmds.push(Cmd::ClearBlock { block: s.wire_block() });
                        }
                        preset.clear_slot(s.path, s.position);
                        *dirty = true;
                    }
                    match cat {
                        Some(c) => {
                            let label = if s.kind == Kind::Empty { "Add effect..." } else { "Change effect..." };
                            if ui.button(label).clicked() {
                                *picker = Some(change_picker(c, &slots, &s));
                            }
                        }
                        None => {
                            ui.label(RichText::new("Effect list needs HX Edit's files").small().color(MUTED));
                        }
                    }
                }
            });
        });
        if s.kind == Kind::Empty {
            ui.label(RichText::new("Use Add effect to put something here.").color(MUTED));
            return;
        }
        if matches!(s.kind, Kind::Input | Kind::Output) {
            io_panel(ui, preset, &s, live, cmds, dirty);
            return;
        }
        if matches!(s.kind, Kind::Split | Kind::Join) {
            let in_path: Vec<&SlotView> = slots.iter().filter(|x| x.path == s.path).collect();
            let pos = |k: Kind| in_path.iter().find(|x| x.kind == k).map(|x| x.position);
            let (inp, out, sp_pos, jp_pos) = (pos(Kind::Input), pos(Kind::Output), pos(Kind::Split), pos(Kind::Join));
            let lower_positions: Vec<usize> = match (sp_pos, jp_pos) {
                (Some(a), Some(b)) if a < b => in_path.iter().filter(|x| x.position > a && x.position < b).map(|x| x.position).collect(),
                _ => Vec::new(),
            };
            let main_slots: Vec<&SlotView> = in_path
                .iter()
                .copied()
                .filter(|x| {
                    let inside = match (inp, out) {
                        (Some(a), Some(b)) => x.position > a && x.position < b,
                        _ => true,
                    };
                    inside
                        && !matches!(x.kind, Kind::Input | Kind::Output | Kind::Split | Kind::Join)
                        && !lower_positions.contains(&x.position)
                })
                .take(chain::COLS)
                .collect();
            let cols = main_slots.len().clamp(1, chain::COLS);
            let col_before = |attach: Option<i64>| attach.map(|a| main_slots.iter().filter(|x| (x.position as i64) < a).count());
            let sp_col = sp_pos.and_then(|p| col_before(preset.attach(s.path, p))).unwrap_or(0).min(cols.saturating_sub(1));
            let jp_col = jp_pos.and_then(|p| col_before(preset.attach(s.path, p))).unwrap_or(cols).clamp(sp_col, cols);
            let merge = s.kind == Kind::Join;
            let min_c = if merge { sp_col } else { 0 };
            let max_c = if merge { cols } else { jp_col.min(cols.saturating_sub(1)) };
            let mut cur = (if merge { jp_col } else { sp_col }).clamp(min_c, max_c);
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Position").color(MUTED));
                let mut moved = false;
                if ui.add_enabled(cur > min_c, egui::Button::new("◀").small()).on_hover_text("Move earlier in the chain").clicked() {
                    cur -= 1;
                    moved = true;
                }
                if ui
                    .add(egui::DragValue::new(&mut cur).range(min_c..=max_c))
                    .on_hover_text("Which chain position it sits before (or drag the dot on the path)")
                    .changed()
                {
                    moved = true;
                }
                if ui.add_enabled(cur < max_c, egui::Button::new("▶").small()).on_hover_text("Move later in the chain").clicked() {
                    cur += 1;
                    moved = true;
                }
                if moved {
                    let attach = main_slots.get(cur).map(|x| x.position as i64).unwrap_or_else(|| out.unwrap_or(9) as i64);
                    preset.set_attach(s.path, s.position, attach);
                    if live {
                        let block = 20 * s.path + attach;
                        cmds.push(if merge { Cmd::MoveJoin { block } } else { Cmd::MoveSplit { block } });
                    }
                    *dirty = true;
                }
            });
        }
        ui.add_space(8.0);
        if s.values.is_empty() {
            ui.label(RichText::new("This block has no knobs.").color(MUTED));
        }
        let sym = cat.zip(s.model).and_then(|(c, m)| c.symbol(m)).map(str::to_owned);
        // An amp + cab (or dual cab) shows the cab's photo under the amp's.
        let cab_sym = s.paired.zip(cat).and_then(|(m, c)| c.symbol(m)).map(str::to_owned);
        let side = sym.as_deref().is_some_and(|x| crate::modelinfo::has_side(ui, x, cab_sym.as_deref())) && ui.available_width() > 700.0;
        let full = ui.available_width();
        ui.horizontal_top(|ui| {
        if side {
            ui.vertical(|ui| {
                ui.set_width(SIDE_W);
                crate::modelinfo::side(ui, sym.as_deref().unwrap_or(""), SIDE_W, cab_sym.as_deref());
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
        // Which footswitch turns this block on and off.
        if s.kind == Kind::Block {
            let fam = snapshots::family_of_device(crate::nam::unit());
            let wire = s.wire_block();
            let current = preset.block_footswitch(wire);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Footswitch").color(MUTED));
                let mut pick = current.unwrap_or(0);
                egui::ComboBox::from_id_salt(("dev-fs", wire))
                    .selected_text(if pick == 0 { "None".to_owned() } else { format!("FS {pick}") })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut pick, 0, "None");
                        for n in 1..=fam.fs_count.max(0) as usize {
                            ui.selectable_value(&mut pick, n, format!("FS {n}"));
                        }
                    });
                if pick != current.unwrap_or(0) {
                    let label = s.model.and_then(|m| cat.map(|c| c.model_name(m))).unwrap_or_default();
                    preset.set_block_footswitch(wire, (pick > 0).then_some(pick), &label, s.enabled.unwrap_or(true));
                    *dirty = true;
                    if live {
                        if let Some(old) = current {
                            cmds.push(Cmd::UnassignFootswitch { block: wire, fs: old as i64 });
                        }
                        if pick > 0 {
                            cmds.push(Cmd::AssignFootswitch { block: wire, fs: pick as i64 });
                        }
                    }
                }
                theme::help(ui, "The footswitch that turns this block on and off. Knobs on footswitches, expression pedals, snapshots and MIDI are in the Controllers card below.");
            });
            ui.add_space(4.0);
        }
        knob_grid(ui, DeviceEdit { preset: &mut *preset, live, cmds: &mut *cmds, cat, dirty: &mut *dirty }, &s, sel, false);
        if let Some(cab) = s.paired {
            ui.add_space(10.0);
            ui.separator();
            let (name, _, col) = describe(cat, cab);
            let dual = s.model.zip(cat).is_some_and(|(m, c)| c.symbol(m).is_some_and(|x| x.starts_with("HD2_Cab")));
            let mut pick = None;
            ui.horizontal(|ui| {
                block_header(ui, col, &name, if dual { "second cab" } else { "cab in this block" });
                if let Some(c) = cat {
                    pick = cab_menu(ui, c, cab, dual);
                }
            });
            if let (Some(new), Some(c), Some(first)) = (pick, cat, s.model) {
                preset.set_paired_model(c, s.path, s.position, new);
                *dirty = true;
                if live {
                    cmds.push(Cmd::SetModel { block: s.wire_block(), model: first, paired: Some(new) });
                }
            }
            ui.add_space(4.0);
            knob_grid(ui, DeviceEdit { preset: &mut *preset, live, cmds: &mut *cmds, cat, dirty: &mut *dirty }, &s, sel, true);
        }
        });
        });
    });
}

/// The Controllers card of a pedal preset: every knob on an expression pedal,
/// footswitch, snapshots or MIDI, with its range (and CC), to change or remove,
/// and adding a knob. Edits are kept in the preset (written when it's saved or
/// sent); on the live preset they are also sent to the pedal at once.
pub fn device_controllers(
    ui: &mut egui::Ui,
    preset: &mut DevicePreset,
    live: bool,
    cmds: &mut Vec<Cmd>,
    cat: Option<&Catalog>,
    dirty: &mut bool,
) {
    let fam = snapshots::family_of_device(crate::nam::unit());
    let mut list = preset.controllers();
    let slots = preset.slots();
    let name_of_ctl = |c: i64| snapshots::Source::from_code(c, fam).map(|s| s.label()).unwrap_or_else(|| format!("Controller {c}"));
    let snap_names: Vec<String> =
        preset.snapshot_names().into_iter().enumerate().map(|(i, n)| if n.trim().is_empty() { format!("{}", i + 1) } else { n }).collect();
    // Effects whose on/off sits on a footswitch (kept apart from knob assignments).
    let fs_blocks: Vec<(usize, i64, String, bool)> = slots
        .iter()
        .filter(|s| s.kind == Kind::Block)
        .filter_map(|s| {
            let fs = preset.block_footswitch(s.wire_block())?;
            let name = s.model.map(|m| describe(cat, m).0).unwrap_or_else(|| "Block".into());
            Some((fs, s.wire_block(), name, s.enabled.unwrap_or(true)))
        })
        .collect();
    let mut changed = false;
    let mut remove: Option<usize> = None;
    theme::card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Controllers").size(17.0).strong());
            theme::help(
                ui,
                "Knobs moved by an expression pedal, a footswitch, snapshots or MIDI CC, with the range each sweeps. \
                 Changes are kept in the preset and written when you save or send it; on the live preset they go to the pedal at once.",
            );
            let total = list.len() + fs_blocks.len();
            ui.label(RichText::new(format!("{} assignment{}", total, if total == 1 { "" } else { "s" })).color(MUTED));
        });
        ui.add_space(4.0);
        // The same tabs as a .hlx preset's Controllers card.
        let tab_of = |code: i64| -> usize {
            match snapshots::Source::from_code(code, fam) {
                Some(snapshots::Source::Footswitch(_)) => 0,
                Some(snapshots::Source::Exp(_)) => 1,
                Some(snapshots::Source::Snapshots) => 2,
                Some(snapshots::Source::Midi) => 3,
                None => 4,
            }
        };
        let tab_id = egui::Id::new("dev-ctl-tab");
        let mut tab: usize = ui.data(|d| d.get_temp(tab_id)).unwrap_or(0);
        let count = |t: usize| list.iter().filter(|c| tab_of(c.controller) == t).count() + if t == 0 { fs_blocks.len() } else { 0 };
        let mut labels = vec![
            format!("Footswitches ({})", count(0)),
            format!("Expression ({})", count(1)),
            format!("Snapshots ({})", count(2)),
            format!("MIDI CC ({})", count(3)),
        ];
        if count(4) > 0 {
            labels.push(format!("Other ({})", count(4)));
        }
        tab = tab.min(labels.len() - 1);
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        if let Some(t) = theme::segmented(ui, &refs, tab) {
            tab = t;
        }
        ui.data_mut(|d| d.insert_temp(tab_id, tab));
        ui.add_space(4.0);
        if count(tab) == 0 {
            ui.label(
                RichText::new(match tab {
                    0 => "Nothing is on a footswitch.",
                    1 => "No knob is on an expression pedal.",
                    2 => "No knob changes with the snapshots.",
                    3 => "No knob is on a MIDI CC.",
                    _ => "Nothing here.",
                })
                .color(MUTED),
            );
        }
        // The pedal's switches at a glance: each one with what it does.
        if tab == 0 {
            let mut items: Vec<FootswitchItem> =
                fs_blocks.iter().map(|(fs, _, name, on)| (*fs, format!("{name}: on/off"), Some(*on))).collect();
            for c in list.iter().filter(|c| tab_of(c.controller) == 0) {
                let Some(snapshots::Source::Footswitch(n)) = snapshots::Source::from_code(c.controller, fam) else { continue };
                let slot = slots.iter().find(|s| s.wire_block() == c.wire);
                let model = slot.and_then(|s| if c.cab { s.paired } else { s.model });
                let block = model.map(|m| describe(cat, m).0).unwrap_or_else(|| format!("Position {}", c.wire));
                let knob = match (model, cat) {
                    (Some(m), Some(cat)) => cat.param(m, c.index).0,
                    _ => format!("setting {}", c.index + 1),
                };
                items.push((n as usize, format!("{block} › {knob}"), None));
            }
            let shown = if fam.name.starts_with("Helix") { fam.fs_count.min(11) } else { fam.fs_count };
            footswitch_strip(ui, shown.max(0) as usize, &items);
        }
        // Footswitches: which effects they switch on and off.
        if tab == 0 && !fs_blocks.is_empty() {
            let mut sorted = fs_blocks.clone();
            sorted.sort_by_key(|x| x.0);
            for (fs, wire, name, on) in sorted {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("FS {fs}")).strong().color(ORANGE));
                    ui.label(RichText::new(format!("{name}: on/off")).strong());
                    ui.label(RichText::new(if on { "(on now)" } else { "(off now)" }).small().color(MUTED));
                    if ui.small_button("✖").on_hover_text("Take this effect off the footswitch").clicked() {
                        preset.set_block_footswitch(wire, None, &name, on);
                        *dirty = true;
                        if live {
                            cmds.push(Cmd::UnassignFootswitch { block: wire, fs: fs as i64 });
                        }
                    }
                });
            }
            theme::help(ui, "To put an effect on a footswitch, pick it in the chain and choose its Footswitch in the block panel.");
            ui.add_space(4.0);
        }
        let mut order: Vec<usize> = (0..list.len()).filter(|&i| tab_of(list[i].controller) == tab).collect();
        order.sort_by_key(|&i| (list[i].controller, list[i].wire));
        let mut last_ctl = -1;
        for i in order {
            let c = list[i].clone();
            if c.controller != last_ctl {
                last_ctl = c.controller;
                ui.add_space(4.0);
                ui.label(RichText::new(name_of_ctl(c.controller)).strong().color(ORANGE));
            }
            let slot = slots.iter().find(|s| s.wire_block() == c.wire);
            let model = slot.and_then(|s| if c.cab { s.paired } else { s.model });
            let block_name = model
                .map(|m| describe(cat, m).0)
                .or_else(|| slot.map(|s| s.kind.label().to_owned()))
                .unwrap_or_else(|| format!("Position {}", c.wire));
            let (knob, meta) = match (model, cat) {
                (Some(m), Some(cat)) => cat.param(m, c.index),
                _ => (format!("setting {}", c.index + 1), None),
            };
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                match c.kind {
                    2 => {
                        ui.label(format!("{block_name}: on/off"));
                    }
                    1 => {
                        ui.label(RichText::new("a global setting").color(MUTED));
                    }
                    // Under snapshot control a knob has one value per snapshot, not a range.
                    _ if c.controller == fam.snapshots_ctrl => {
                        ui.label(RichText::new(format!("{block_name} › {knob}")).strong());
                        for (s, sname) in snap_names.iter().enumerate() {
                            let (on, v) = c.snaps.get(s).cloned().flatten().unwrap_or((false, Value::Nil));
                            ui.add_space(6.0);
                            ui.label(RichText::new(sname).small().color(MUTED));
                            if matches!(v, Value::Nil) {
                                ui.label(RichText::new("-").color(MUTED));
                                continue;
                            }
                            let r = ui.push_id(("snapv", i, s), |ui| bound_cell(ui, &v, meta.as_ref())).inner;
                            if let Some(nv) = r {
                                if let Some(slot) = list[i].snaps.get_mut(s) {
                                    *slot = Some((on, nv));
                                    changed = true;
                                }
                            }
                        }
                        if ui.small_button("✖").on_hover_text("Take this knob off snapshot control").clicked() {
                            remove = Some(i);
                        }
                    }
                    _ => {
                        ui.label(RichText::new(format!("{block_name} › {knob}")).strong());
                        ui.label(RichText::new("from").color(MUTED));
                        if let Some(v) = bound_cell(ui, &c.min, meta.as_ref()) {
                            list[i].min = v.clone();
                            changed = true;
                            if live {
                                cmds.push(Cmd::ControllerBound { max: false, block: c.wire, cab: c.cab, index: c.index as i64, value: v });
                            }
                        }
                        ui.label(RichText::new("to").color(MUTED));
                        if let Some(v) = bound_cell(ui, &c.max, meta.as_ref()) {
                            list[i].max = v.clone();
                            changed = true;
                            if live {
                                cmds.push(Cmd::ControllerBound { max: true, block: c.wire, cab: c.cab, index: c.index as i64, value: v });
                            }
                        }
                        if c.controller == fam.midi_ctrl {
                            ui.label(RichText::new("CC").color(MUTED));
                            let mut cc = c.cc;
                            if ui.add(egui::DragValue::new(&mut cc).range(1..=127)).changed() {
                                list[i].cc = cc;
                                changed = true;
                                if live {
                                    cmds.push(Cmd::ControllerCc { block: c.wire, cab: c.cab, index: c.index as i64, cc });
                                }
                            }
                        }
                        if ui.small_button("✖").on_hover_text("Take this knob off the controller").clicked() {
                            remove = Some(i);
                        }
                    }
                }
            });
        }
        // Adding a knob.
        ui.add_space(6.0);
        if let Some(cat) = cat {
            ui.menu_button("+ Add knob", |ui| {
                let sources: Vec<snapshots::Source> = match tab {
                    0 => (1..=fam.fs_count).map(snapshots::Source::Footswitch).collect(),
                    1 => (1..=fam.exp_count).map(snapshots::Source::Exp).collect(),
                    2 => vec![snapshots::Source::Snapshots],
                    3 => vec![snapshots::Source::Midi],
                    _ => {
                        let mut s = vec![snapshots::Source::Snapshots];
                        s.extend((1..=fam.exp_count).map(snapshots::Source::Exp));
                        s.extend((1..=fam.fs_count).map(snapshots::Source::Footswitch));
                        s.push(snapshots::Source::Midi);
                        s
                    }
                };
                for src in sources {
                    ui.menu_button(src.label(), |ui| {
                        for s in slots.iter().filter(|s| s.kind == Kind::Block) {
                            let Some(m) = s.model else { continue };
                            ui.menu_button(describe(Some(cat), m).0, |ui| {
                                for (k, (id, vt, _)) in cat.default_values(m).into_iter().enumerate() {
                                    let (label, meta) = cat.param(m, k);
                                    let label = if label.is_empty() { id } else { label };
                                    let taken = list.iter().any(|c| c.wire == s.wire_block() && c.index == k && !c.cab && c.kind == 0);
                                    if ui.add_enabled(!taken, egui::Button::new(label)).clicked() {
                                        let (lo, hi) = meta.as_ref().map(|p| (p.min, p.max)).unwrap_or((0.0, 1.0));
                                        let typed = |x: f64| match vt {
                                            2 => Value::Bool(x != 0.0),
                                            0 => Value::int(x.round() as i64),
                                            _ => Value::F32(x as f32),
                                        };
                                        let code = src.code(fam);
                                        let snaps = (0..preset.snapshot_names().len())
                                            .map(|_| {
                                                (code == fam.snapshots_ctrl)
                                                    .then(|| (false, preset.knob_value(s.wire_block(), k, false).unwrap_or(Value::Nil)))
                                            })
                                            .collect();
                                        let cc = if code == fam.midi_ctrl { 1 } else { 4 };
                                        if live {
                                            cmds.push(Cmd::AssignController {
                                                block: s.wire_block(),
                                                cab: false,
                                                index: k as i64,
                                                controller: code,
                                                cc,
                                            });
                                        } else {
                                            let mut c = crate::devpreset::Ctl::knob(code, s.wire_block(), k);
                                            (c.min, c.max, c.cc, c.snaps) = (typed(lo), typed(hi), cc, snaps);
                                            list.push(c);
                                            changed = true;
                                        }
                                        ui.close_menu();
                                    }
                                }
                            });
                        }
                    });
                }
            });
        }
        if !live && !list.iter().all(|c| c.kind == 0) {
            theme::help(ui, "Global settings and expression-pedal on/off assignments are kept as they are.");
        }
    });
    if let Some(i) = remove {
        let c = list.remove(i);
        changed = true;
        if live {
            cmds.push(Cmd::UnassignController { block: c.wire, cab: c.cab, index: c.index as i64 });
        }
    }
    if changed {
        // On the live preset, adding and removing come back with the pedal's own document.
        preset.set_controllers(&list);
        *dirty = true;
    }
}
