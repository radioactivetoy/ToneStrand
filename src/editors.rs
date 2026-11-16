//! The preset editor: the signal chain as tiles, and the selected block's
//! effect and knobs. Works on the pedal's live preset (every change is sent
//! at once), on presets read from a slot or a file, and on .hlx files.

use eframe::egui::{self, Color32, CornerRadius, RichText, Stroke, Vec2};

use crate::catalog::{self, Catalog, Param};
use crate::device::Cmd;
use crate::devpreset::{DevicePreset, Kind, SlotView};
use crate::chain;
use crate::hlx;
use crate::snapshots;
use crate::msgpack::Value;
use crate::theme::{self, *};

/// A small ring that fills as the path's DSP fills.
fn dsp_ring(ui: &mut egui::Ui, percent: f64) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(16.0), egui::Sense::hover());
    let c = rect.center();
    let p = ui.painter();
    p.circle_stroke(c, 6.5, Stroke::new(2.5_f32, line()));
    let frac = (percent / 100.0).clamp(0.0, 1.0) as f32;
    let col = if percent > 90.0 { BAD } else if percent > 70.0 { ORANGE } else { GOOD };
    let n = (frac * 40.0).ceil() as usize;
    let pts: Vec<egui::Pos2> = (0..=n)
        .map(|i| {
            let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * frac * i as f32 / n.max(1) as f32;
            c + Vec2::new(a.cos(), a.sin()) * 6.5
        })
        .collect();
    if pts.len() > 1 {
        p.add(egui::Shape::line(pts, Stroke::new(2.5_f32, col)));
    }
}

/// Names and ranges of the input's and output's knobs, in the pedal's order
/// (input: noise gate, threshold, decay; output: level, pan).
fn io_params(input: bool) -> Vec<(&'static str, Param)> {
    let p = |id: &str, name: &str, vt: i64, display: &str, min: f64, max: f64, default: f64| Param {
        id: id.into(),
        name: name.into(),
        value_type: vt,
        display: display.into(),
        min,
        max,
        default,
        choices: catalog::builtin_choices(display),
        spec: catalog::DisplaySpec::default(),
    };
    if input {
        vec![
            ("Noise Gate", p("noiseGate", "Noise Gate", 2, "off_on", 0.0, 1.0, 0.0)),
            ("Threshold", p("threshold", "Threshold", 1, "volume", -96.0, 0.0, -48.0)),
            ("Decay", p("decay", "Decay", 1, "time_ms", 0.001, 1.0, 0.1)),
        ]
    } else {
        vec![
            ("Level", p("gain", "Level", 1, "volume", -120.0, 20.0, 0.0)),
            ("Pan", p("pan", "Pan", 1, "pan", 0.0, 1.0, 0.5)),
        ]
    }
}

/// The settings of a path's input or output: where the sound comes from or goes, and its knobs.
fn io_panel(ui: &mut egui::Ui, preset: &mut DevicePreset, s: &SlotView, live: bool, cmds: &mut Vec<Cmd>, dirty: &mut bool) {
    let input = s.kind == Kind::Input;
    ui.horizontal(|ui| {
        ui.label(RichText::new(if input { "Source" } else { "Destination" }).color(MUTED));
        if let Some(route) = preset.io_route(s.path, s.position) {
            // The jacks by name, numbered as in .hlx files (the same numbers the pedal keeps).
            let fam = snapshots::family_of_device(crate::nam::unit());
            let mut names: Vec<(i64, String)> = route_names(fam, input).into_iter().map(|(v, n)| (v, n.to_owned())).collect();
            if !names.iter().any(|(v, _)| *v == route) {
                names.push((route, format!("Other ({route})")));
            }
            let shown = names.iter().find(|(v, _)| *v == route).map(|(_, n)| n.clone()).unwrap_or_default();
            let mut pick = route;
            egui::ComboBox::from_id_salt(("dev-route", s.global)).selected_text(shown).width(150.0).show_ui(ui, |ui| {
                for (v, n) in &names {
                    ui.selectable_value(&mut pick, *v, n);
                }
            });
            if pick != route {
                preset.set_io_route(s.path, s.position, pick);
                *dirty = true;
                if live {
                    cmds.push(Cmd::SetRoute { block: s.wire_block(), input, value: pick });
                }
            }
            theme::help(ui, if input { "Where this path's sound comes from." } else { "Where this path's sound goes." });
        }
    });
    ui.add_space(8.0);
    let names = io_params(input);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(6.0, 12.0);
        for (vi, v) in s.values.iter().enumerate() {
            let (name, meta) = names.get(vi).map(|(n, m)| (n.to_string(), Some(m.clone()))).unwrap_or((format!("Setting {}", vi + 1), None));
            let (nv, commit) = param_cell(ui, &name, meta.as_ref(), v);
            if let Some(nv) = &nv {
                preset.set_value(s.path, s.position, vi, nv.clone());
                *dirty = true;
            }
            if live && commit {
                if let Some(value) = preset.slots().get(s.global).and_then(|x| x.values.get(vi).cloned()) {
                    cmds.push(Cmd::SetParam { block: s.wire_block(), path: 0, index: vi as i64, value });
                }
            }
        }
    });
}

fn block_header(ui: &mut egui::Ui, colour: Color32, title: &str, extra: &str) {
    let (r, _) = ui.allocate_exact_size(Vec2::new(4.0, 26.0), egui::Sense::hover());
    ui.painter().rect_filled(r, CornerRadius::same(2), colour);
    ui.add_space(2.0);
    ui.label(RichText::new(title).size(19.0).strong().color(TEXT));
    if !extra.is_empty() {
        ui.label(RichText::new(extra).color(MUTED));
    }
}

/// A block panel's title with the effect type's icon (or its colour bar when the type has none).
fn block_header_icon(ui: &mut egui::Ui, colour: Color32, category: &str, symbol: &str, title: &str, extra: &str) {
    if crate::icons::show_model(ui, category, symbol, title, 34.0) {
        ui.add_space(4.0);
        ui.label(RichText::new(title).size(19.0).strong().color(TEXT));
        if !extra.is_empty() {
            ui.label(RichText::new(extra).color(MUTED));
        }
    } else {
        block_header(ui, colour, title, extra);
    }
}

/// "based on ..." under a block's name, unless the name already says it (real gear names).
fn based_line(name: &str, based: Option<String>) -> String {
    let Some(b) = based else { return String::new() };
    let (n, bl) = (name.to_lowercase(), b.to_lowercase());
    let first = bl.split_whitespace().next().unwrap_or("");
    if bl.contains(&n) || (first.len() > 2 && n.contains(first)) {
        return String::new();
    }
    format!("based on {b}")
}

/// Name and colour for a model number.
fn describe(cat: Option<&Catalog>, model: i64) -> (String, String, Color32) {
    match cat {
        Some(c) => {
            let (cname, col) = c.category(model).unwrap_or_else(|| ("Effect".into(), theme::number_colour(model)));
            (c.model_name(model), cname, col)
        }
        None => (format!("Model {model}"), "effect".into(), theme::number_colour(model)),
    }
}

/// How much of one DSP the catalog's load figures add up to (an estimate).
/// Calibrated on the person's 126 HX Stomp presets (all of which run on the pedal):
/// without split/merge, the fullest adds up to 77.6, which reads 97% at 80.
pub const DSP_BUDGET: f64 = 80.0;

/// Share of the path's DSP a model costs, in percent.
fn dsp_percent(cat: &Catalog, model: i64) -> f64 {
    cat.dsp_cost(model) / DSP_BUDGET * 100.0
}

/// The effect chooser for one of the pedal preset's slots: what it costs now comes back if replaced.
fn change_picker(c: &Catalog, slots: &[SlotView], s: &SlotView) -> PickerState {
    let own = s.model.map(|m| dsp_percent(c, m)).unwrap_or(0.0) + s.paired.map(|m| dsp_percent(c, m)).unwrap_or(0.0);
    let used: f64 = slots
        .iter()
        .filter(|x| x.path == s.path)
        .map(|x| x.model.map(|m| dsp_percent(c, m)).unwrap_or(0.0) + x.paired.map(|m| dsp_percent(c, m)).unwrap_or(0.0))
        .sum();
    let (cname, _) = s.model.and_then(|m| c.category(m)).unwrap_or_default();
    PickerState {
        block: s.wire_block(),
        slot: s.global,
        path: s.path,
        position: s.position,
        free: 100.0 - used + own,
        replacing: s.kind == Kind::Block,
        category: cname,
        ..Default::default()
    }
}

/// The estimated DSP use of each path of a .hlx preset, in percent (cabs riding with an amp included).
pub fn hlx_dsp_use(cat: &Catalog, json: &serde_json::Value) -> Vec<(String, f64)> {
    hlx::dsps(json)
        .into_iter()
        .map(|dsp| {
            let used: f64 = hlx::blocks(json)
                .iter()
                .filter(|b| b.dsp == dsp)
                .filter_map(|b| {
                    let stereo = hlx::block(json, b).and_then(|o| o.get("@stereo")).and_then(|v| v.as_bool()).unwrap_or(false);
                    cat.number_for_hlx(&b.model, stereo).map(|n| dsp_percent(cat, n))
                })
                .sum();
            (dsp, used)
        })
        .collect()
}

/// The "Add a block" window's state: which block it fills, and what the person is browsing.
#[derive(Clone, Default)]
pub struct PickerState {
    /// The pedal's slot number (`20 * path + position`, for live changes).
    pub block: i64,
    /// Index in `DevicePreset::slots()` (for selecting the slot in the editor).
    pub slot: usize,
    /// Where the block goes in the document: path and position.
    pub path: i64,
    pub position: usize,
    /// For a .hlx file: the DSP section and, when replacing, the block's key.
    pub hlx_dsp: Option<String>,
    pub hlx_key: Option<String>,
    pub free: f64,
    pub replacing: bool,
    pub category: String,
    pub sub: String,
    pub search: String,
}

/// The "Add a block" window: categories on the left, variants as tabs,
/// a search box, and each effect's DSP cost (greyed when it will not fit).
/// Returns where it goes, the model and the paired cab once one is picked.
pub fn picker_window(ctx: &egui::Context, state: &mut Option<PickerState>, cat: &Catalog) -> Option<(PickerState, i64, Option<i64>)> {
    let st = state.as_mut()?;
    let mut picked = None;
    let mut open = true;
    let all_groups = cat.picker_groups();
    let mut categories: Vec<(String, Color32, usize)> = Vec::new();
    for g in &all_groups {
        match categories.iter_mut().find(|c| c.0 == g.category) {
            Some(c) => c.2 += g.models.len(),
            None => categories.push((g.category.clone(), g.colour, g.models.len())),
        }
    }
    if st.category.is_empty() {
        st.category = categories.first().map(|c| c.0.clone()).unwrap_or_default();
    }
    egui::Window::new(if st.replacing { "Change the effect" } else { "Add a block" })
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size([760.0, 560.0])
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("position {}", st.block)).color(MUTED));
                let col = if st.free < 15.0 { BAD } else { GOOD };
                ui.label(RichText::new(format!("{:.0}% DSP free", st.free.max(0.0))).color(col));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add(egui::TextEdit::singleline(&mut st.search).hint_text("🔍 Search effects").desired_width(220.0));
                });
            });
            ui.separator();
            ui.horizontal_top(|ui| {
                // Categories.
                ui.vertical(|ui| {
                    ui.set_width(170.0);
                    egui::ScrollArea::vertical().id_salt("pick-cats").max_height(470.0).show(ui, |ui| {
                        for (name, col, count) in &categories {
                            let sel = *name == st.category && st.search.is_empty();
                            ui.horizontal(|ui| {
                                // The effect type's icon, or its colour for types without one (amps, cabs...).
                                if !crate::icons::show(ui, name, 22.0) {
                                    ui.add_space(9.0);
                                    let (r, _) = ui.allocate_exact_size(Vec2::new(4.0, 18.0), egui::Sense::hover());
                                    ui.painter().rect_filled(r, CornerRadius::same(2), *col);
                                    ui.add_space(9.0);
                                }
                                if ui.selectable_label(sel, format!("{name}  ")).clicked() {
                                    st.category = name.clone();
                                    st.sub.clear();
                                    st.search.clear();
                                }
                                ui.label(RichText::new(count.to_string()).small().color(MUTED));
                            });
                        }
                    });
                });
                ui.separator();
                ui.vertical(|ui| {
                    let search = st.search.to_lowercase();
                    let groups: Vec<&catalog::PickGroup> = if search.is_empty() {
                        all_groups.iter().filter(|g| g.category == st.category).collect()
                    } else {
                        all_groups.iter().collect()
                    };
                    // Variant tabs (Mono, Stereo, Legacy...).
                    if search.is_empty() && groups.len() > 1 {
                        if !groups.iter().any(|g| g.sub == st.sub) {
                            st.sub = groups[0].sub.clone();
                        }
                        ui.horizontal(|ui| {
                            for g in &groups {
                                let label = if g.sub.is_empty() { "All".to_owned() } else { format!("{}  {}", g.sub, g.models.len()) };
                                if ui.selectable_label(g.sub == st.sub, label).clicked() {
                                    st.sub = g.sub.clone();
                                }
                            }
                        });
                        ui.add_space(4.0);
                    }
                    egui::ScrollArea::vertical().id_salt("pick-models").max_height(440.0).auto_shrink([false, false]).show(ui, |ui| {
                        for g in groups.iter().filter(|g| !search.is_empty() || groups.len() == 1 || g.sub == st.sub) {
                            let loops = snapshots::family_of_device(crate::nam::unit()).fx_loops;
                            for m in g.models.iter().filter(|m| {
                                // Only the sends and returns this unit has jacks for.
                                !(g.category == "Send/Return" && loops < 4 && (m.name.contains('3') || m.name.contains('4')))
                            }).filter(|m| {
                                search.is_empty()
                                    || m.name.to_lowercase().contains(&search)
                                    || crate::realnames::real(&m.name).is_some_and(|r| r.to_lowercase().contains(&search))
                            }) {
                                let shown = crate::realnames::show(&m.name);
                                let with_cab = g.category.contains("Amp") && g.category.contains("Cab");
                                // A dual cab is two cabs in one block; it starts with the same cab twice,
                                // and the second can be changed on its own in the block's panel.
                                let dual = !with_cab && g.category.contains("Cab") && g.sub.to_lowercase().contains("dual");
                                let paired = if with_cab {
                                    cat.paired_cab(m.number)
                                } else if dual {
                                    Some(m.number)
                                } else {
                                    None
                                };
                                let cost = dsp_percent(cat, m.number) + paired.map(|p| dsp_percent(cat, p)).unwrap_or(0.0);
                                let fits = cost <= st.free + 0.01;
                                let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0), egui::Sense::click());
                                let p = ui.painter();
                                if resp.hovered() && fits {
                                    p.rect_filled(rect, CornerRadius::same(6), card_hi());
                                }
                                // The effect's own kind (Fuzz, Tape, Plate, Chorus...) from the Stadium icons;
                                // a colour square only when its type has none.
                                let sw = egui::Rect::from_min_size(rect.left_top() + Vec2::new(5.0, 5.0), Vec2::splat(24.0));
                                let sym_now = cat.symbol(m.number).unwrap_or("");
                                // Amps listed under "Amp+Cab" get the amp + cab icon.
                                let kind = if g.category.contains("Amp") && g.category.contains("Cab") { "Amp+Cab" } else { g.category.as_str() };
                                let tint = if fits { Color32::WHITE } else { Color32::from_white_alpha(90) };
                                if !crate::icons::paint_model(p, ui.ctx(), sw, kind, sym_now, &m.name, tint) {
                                    p.rect_filled(sw, CornerRadius::same(5), if fits { g.colour } else { g.colour.linear_multiply(0.35) });
                                }
                                let tc = if fits { TEXT } else { MUTED };
                                let title = if search.is_empty() || g.sub.is_empty() { shown.clone() } else { format!("{}  ·  {}", shown, g.sub) };
                                p.text(rect.left_center() + Vec2::new(38.0, 0.0), egui::Align2::LEFT_CENTER, title, egui::FontId::proportional(14.0), tc);
                                if cost > 0.0 {
                                    p.text(
                                        rect.right_center() - Vec2::new(10.0, 0.0),
                                        egui::Align2::RIGHT_CENTER,
                                        format!("{cost:.0}%"),
                                        egui::FontId::monospace(12.0),
                                        if fits { MUTED } else { BAD },
                                    );
                                }
                                let tip = if fits { "Click to use this effect" } else { "Not enough DSP left in this path" };
                                let other = crate::realnames::other(&m.name);
                                let sym = cat.symbol(m.number).map(str::to_owned);
                                let resp = resp.on_hover_ui(|ui| {
                                    ui.set_max_width(260.0);
                                    if let Some(s) = &sym {
                                        crate::modelinfo::hover(ui, s);
                                    }
                                    if let Some(o) = &other {
                                        ui.label(RichText::new(o).small());
                                    }
                                    ui.label(RichText::new(tip).small().color(MUTED));
                                });
                                if resp.clicked() && fits {
                                    picked = Some((st.clone(), m.number, paired));
                                }
                            }
                        }
                    });
                });
            });
        });
    if !open || picked.is_some() {
        *state = None;
    }
    picked
}

/// A round knob: drag up/down or left/right (hold Shift for fine steps), scroll
/// over it, or double-click to return to the default. Returns (changed, send now).
fn knob(ui: &mut egui::Ui, value: &mut f64, lo: f64, hi: f64, default: Option<f64>) -> (bool, bool) {
    let size = 46.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::click_and_drag());
    let span = (hi - lo).max(1e-9);
    *value = value.clamp(lo, hi);
    let before = *value;
    let mut commit = false;
    if resp.dragged() {
        let fine = if ui.input(|i| i.modifiers.shift) { 0.1 } else { 1.0 };
        let d = resp.drag_delta();
        let delta = (d.x - d.y) as f64;
        *value += delta * span / 150.0 * fine;
    }
    if resp.drag_stopped() {
        commit = true;
    }
    if resp.hovered() {
        let scroll = ui.input(|i| i.raw_scroll_delta.y);
        if scroll != 0.0 {
            *value += scroll.signum() as f64 * span / 50.0;
            commit = true;
        }
    }
    if resp.double_clicked() {
        if let Some(d) = default {
            *value = d;
            commit = true;
        }
    }
    *value = value.clamp(lo, hi);

    // Drawing: a 270-degree track, the filled part, and a pointer.
    let p = ui.painter();
    let c = rect.center();
    let r = size / 2.0 - 4.0;
    let start = std::f32::consts::PI * 0.75;
    let sweep = std::f32::consts::PI * 1.5;
    let frac = ((*value - lo) / span).clamp(0.0, 1.0) as f32;
    let arc = |from: f32, to: f32| -> Vec<egui::Pos2> {
        let n = 32;
        (0..=n)
            .map(|i| {
                let a = from + (to - from) * i as f32 / n as f32;
                c + Vec2::new(a.cos(), a.sin()) * r
            })
            .collect()
    };
    p.add(egui::Shape::line(arc(start, start + sweep), Stroke::new(4.0_f32, line())));
    let active = resp.hovered() || resp.dragged();
    let col = if active { accent() } else { accent().linear_multiply(0.85) };
    if frac > 0.0 {
        p.add(egui::Shape::line(arc(start, start + sweep * frac), Stroke::new(4.0_f32, col)));
    }
    p.circle_filled(c, r - 6.0, if active { card_hi() } else { panel() });
    let a = start + sweep * frac;
    p.line_segment([c + Vec2::new(a.cos(), a.sin()) * (r - 15.0), c + Vec2::new(a.cos(), a.sin()) * (r - 6.0)], Stroke::new(2.5_f32, TEXT));
    resp.on_hover_text("Drag up/down or left/right (Shift = fine), scroll, or double-click for the default");
    ((*value - before).abs() > span * 1e-6, commit)
}

/// One labelled control in the knob row.
fn param_cell(ui: &mut egui::Ui, name: &str, meta: Option<&Param>, v: &Value) -> (Option<Value>, bool) {
    let mut out = (None, false);
    let salt = meta.map(|m| m.id.as_str()).unwrap_or(name);
    ui.push_id(salt, |ui| {
        ui.allocate_ui_with_layout(Vec2::new(104.0, 104.0), egui::Layout::top_down(egui::Align::Center), |ui| {
            ui.set_width(104.0);
            ui.add(egui::Label::new(RichText::new(name).small().color(MUTED)).truncate());
            out = value_widget(ui, meta, v);
        });
    });
    out
}

/// One line naming the capture a NAM block plays, for the chain.
fn nam_detail(v: f64) -> String {
    let slot = v.trunc() as i64;
    if !(1..=128).contains(&slot) {
        return "no capture".into();
    }
    match crate::nam::ir_name(slot) {
        Some(n) => format!("{slot} · {n}"),
        None => format!("capture slot {slot}"),
    }
}

/// A pedal IR slot to play: for the NAM block (`nam`) captures first, for an IR block IRs first.
/// Returns the slot picked (from 1; 0 = empty, NAM only).
fn capture_picker(ui: &mut egui::Ui, cur: i64, nam: bool) -> Option<i64> {
    let lo = if nam { 0 } else { 1 };
    let names = crate::nam::ir_names();
    let shown = match crate::nam::ir_name(cur) {
        Some(n) if (1..=128).contains(&cur) => n,
        _ if (1..=128).contains(&cur) => format!("Slot {cur}"),
        _ => "<EMPTY>".to_owned(),
    };
    // A slot holding the other kind won't sound right.
    let warn = (1..=128).contains(&cur) && crate::nam::slot_is_capture(cur) == Some(!nam);
    let mut picked = None;
    let r = egui::ComboBox::from_id_salt(if nam { "nam-capture" } else { "ir-select" })
        .selected_text(RichText::new(&shown).color(if warn { ORANGE } else { TEXT }))
        .width(96.0)
        .height(420.0)
        .show_ui(ui, |ui| {
            ui.set_min_width(280.0);
            let captures: Vec<&(i64, String)> = names.iter().filter(|(s, _)| crate::nam::slot_is_capture(*s) == Some(true)).collect();
            let unknown: Vec<&(i64, String)> = names.iter().filter(|(s, _)| crate::nam::slot_is_capture(*s).is_none()).collect();
            let irs: Vec<&(i64, String)> = names.iter().filter(|(s, _)| crate::nam::slot_is_capture(*s) == Some(false)).collect();
            if names.is_empty() {
                ui.label(RichText::new("Connect the pedal to list its IR slots.\nOr type the slot number under the list.").small().color(MUTED));
            }
            let mut section = |ui: &mut egui::Ui, title: &str, list: &[&(i64, String)], col: Color32| {
                if list.is_empty() {
                    return;
                }
                ui.label(RichText::new(title).small().strong().color(MUTED));
                for (s, n) in list {
                    if ui.selectable_label(*s == cur, RichText::new(format!("{s:>3}  {n}")).monospace().color(col)).clicked() {
                        picked = Some(*s);
                    }
                }
                ui.add_space(4.0);
            };
            if nam {
                section(ui, "NAM CAPTURES", &captures, PINK);
                section(ui, "NOT SORTED YET", &unknown, TEXT);
                section(ui, "IRS (NOT CAPTURES - WON'T SOUND RIGHT)", &irs, MUTED);
                if ui.selectable_label(!(1..=128).contains(&cur), "<EMPTY>").clicked() {
                    picked = Some(0);
                }
            } else {
                section(ui, "IRS", &irs, TEXT);
                section(ui, "NOT SORTED YET", &unknown, TEXT);
                section(ui, "NAM CAPTURES (NOT IRS - WON'T SOUND RIGHT)", &captures, MUTED);
            }
        });
    let r = r.response.on_hover_text(match (warn, nam) {
        (true, true) => "This slot holds a plain IR, not a NAM capture",
        (true, false) => "This slot holds a NAM capture, not an IR",
        (false, true) => "The NAM capture this block plays (a User IR slot on the pedal)",
        (false, false) => "The IR this block plays (a User IR slot on the pedal)",
    });
    if r.hovered() {
        let scroll = ui.input(|i| i.raw_scroll_delta.y);
        if scroll != 0.0 {
            picked = Some((cur + if scroll > 0.0 { -1 } else { 1 }).clamp(lo, 128));
        }
    }
    let mut n = cur.clamp(lo, 128);
    ui.horizontal(|ui| {
        ui.add_space(18.0);
        ui.label(RichText::new("slot").small().color(MUTED));
        if ui.add(egui::DragValue::new(&mut n).range(lo..=128).speed(0.1)).changed() {
            picked = Some(n);
        }
    });
    picked.filter(|p| *p != cur)
}

/// One knob, switch or chooser for a value of the pedal's document.
/// Returns the new value and whether it should be sent now (released or clicked).
fn value_widget(ui: &mut egui::Ui, meta: Option<&Param>, v: &Value) -> (Option<Value>, bool) {
    // The NAM block's capture and an IR block's IR Select: a pedal IR slot, picked by name.
    if let Some(nam) = meta.and_then(|m| match m.display.as_str() { "nam_slot" => Some(true), "ir_select" => Some(false), _ => None }) {
        let cur = v.as_f64().unwrap_or(0.0).trunc() as i64;
        let mut out = (None, false);
        ui.add_space(10.0);
        if let Some(slot) = capture_picker(ui, cur, nam) {
            let nv = match v {
                Value::Int(_, f) => Value::Int(slot, *f),
                Value::F64(_) => Value::F64(slot as f64),
                _ => Value::F32(slot as f32),
            };
            out = (Some(nv), true);
        }
        return out;
    }
    // A few fixed choices in HX Edit (its "segmented" control): a cab mic's Angle is 0° or 45°.
    // Only when there really are two settings: a Ratio (2:1 ... 20:1) stays a knob.
    let two_state = |m: &Param| {
        if m.id.contains("Angle") || m.name.contains("Angle") {
            return true;
        }
        let (a, b) = (catalog::format_value(Some(m), m.min), catalog::format_value(Some(m), m.max));
        [0.25, 0.5, 0.75].iter().all(|t| {
            let s = catalog::format_value(Some(m), m.min + (m.max - m.min) * t);
            s == a || s == b
        })
    };
    if let Some(m) = meta.filter(|m| m.spec.segmented && m.max > m.min && !matches!(v, Value::Bool(_)) && two_state(m)) {
        let cur = v.as_f64().unwrap_or(m.min);
        let mut out = (None, false);
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for target in [m.min, m.max] {
                let label = crate::catalog::format_value(Some(m), target);
                let on = (cur - target).abs() < (m.max - m.min) / 4.0;
                if ui.selectable_label(on, label).clicked() && !on {
                    let nv = match v {
                        Value::Int(..) => Value::int(target.round() as i64),
                        _ => Value::F32(target as f32),
                    };
                    out = (Some(nv), true);
                }
            }
        });
        return out;
    }
    match v {
        Value::Bool(b) => {
            let mut bb = *b;
            let choices = meta.map(|m| m.choices.as_slice()).unwrap_or(&[]);
            if choices.len() >= 2 && !matches!(choices[0].as_str(), "Off" | "OFF" | "False") {
                let shown = &choices[if bb { 1 } else { 0 }];
                let mut changed = false;
                ui.add_space(10.0);
                egui::ComboBox::from_id_salt("bool-choice")
                    .selected_text(shown.as_str())
                    .width(94.0)
                    .show_ui(ui, |ui| {
                        for (idx, lbl) in choices.iter().take(2).enumerate() {
                            let target = idx == 1;
                            if ui.selectable_label(bb == target, lbl).clicked() && bb != target {
                                bb = target;
                                changed = true;
                            }
                        }
                    });
                if changed {
                    return (Some(Value::Bool(bb)), true);
                }
            } else {
                let (off_lbl, on_lbl) = if choices.len() >= 2 {
                    (choices[0].as_str(), choices[1].as_str())
                } else {
                    ("Off", "On")
                };
                let label = if bb { format!("  {on_lbl}  ") } else { format!("  {off_lbl}  ") };
                ui.add_space(10.0);
                let r = ui.toggle_value(&mut bb, label);
                if r.changed() {
                    return (Some(Value::Bool(bb)), true);
                }
            }
        }
        Value::Int(n, fmt) => {
            let mut x = *n;
            if let Some(m) = meta.filter(|m| !m.choices.is_empty()) {
                let base = m.min.round() as i64;
                let max_v = (m.max.round() as i64).max(base + m.choices.len() as i64 - 1);
                let hi_v = (m.max.round() as i64).min(max_v).max(base);
                x = x.clamp(base, hi_v);
                let shown = m.choice_label(x).unwrap_or("—").to_owned();
                let mut changed = false;
                ui.add_space(10.0);
                let r = egui::ComboBox::from_id_salt("int-choice")
                    .selected_text(shown)
                    .width(94.0)
                    .show_ui(ui, |ui| {
                        for (idx, lbl) in m.choices.iter().enumerate() {
                            let val = base + idx as i64;
                            if val > hi_v {
                                break;
                            }
                            if ui.selectable_label(x == val, lbl).clicked() && x != val {
                                x = val;
                                changed = true;
                            }
                        }
                    })
                    .response;
                if r.hovered() {
                    let scroll = ui.input(|i| i.raw_scroll_delta.y);
                    if scroll != 0.0 {
                        let next = (x + if scroll > 0.0 { -1 } else { 1 }).clamp(base, hi_v);
                        if next != x {
                            x = next;
                            changed = true;
                        }
                    }
                }
                if changed {
                    return (Some(Value::Int(x, *fmt)), true);
                }
            } else {
                let mut dv = egui::DragValue::new(&mut x).speed(0.1);
                if let Some(m) = meta.filter(|m| m.max > m.min) {
                    dv = dv.range(m.min as i64..=m.max as i64);
                }
                ui.add_space(10.0);
                let r = ui.add(dv);
                if r.changed() {
                    let commit = r.drag_stopped() || !r.dragged();
                    return (Some(Value::Int(x, *fmt)), commit);
                } else if r.drag_stopped() {
                    return (None, true);
                }
            }
        }
        Value::F32(_) | Value::F64(_) => {
            if let Some(m) = meta.filter(|m| m.value_type == 0 && !m.choices.is_empty()) {
                let cur = v.as_f64().unwrap_or(m.default).round() as i64;
                if let (Some(Value::Int(nx, _)), commit) = value_widget(ui, Some(m), &Value::int(cur)) {
                    let nv = match v {
                        Value::F64(_) => Value::F64(nx as f64),
                        _ => Value::F32(nx as f32),
                    };
                    return (Some(nv), commit);
                }
                return (None, false);
            }
            let mut f = v.as_f64().unwrap_or(0.0);
            if let Some(m) = meta {
                if m.min == 0.0 && m.max == 1.0 && (m.spec.scale - 10.0).abs() < 1e-6 && f > 1.0 && f <= 10.0 {
                    f /= 10.0;
                }
            }
            let (lo, hi) = match meta {
                Some(m) if m.max > m.min => (m.min, m.max),
                _ if (0.0..=1.0).contains(&f) => (0.0, 1.0),
                _ => (0.0, 10.0_f64.max(f.abs())),
            };
            let default = meta.map(|m| m.default);
            let (mut changed, mut commit) = knob(ui, &mut f, lo, hi, default);
            // Click the number to type a value.
            let edit_id = ui.id().with("num-edit");
            let editing: Option<(String, bool)> = ui.data(|d| d.get_temp(edit_id));
            if let Some((mut text, started)) = editing {
                let r = ui.add(
                    egui::TextEdit::singleline(&mut text)
                        .desired_width(90.0)
                        .horizontal_align(egui::Align::Center)
                        .font(egui::TextStyle::Monospace),
                );
                if !started {
                    r.request_focus();
                }
                if r.lost_focus() || (started && !r.has_focus()) {
                    ui.data_mut(|d| d.remove::<(String, bool)>(edit_id));
                    if !ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        if let Some(nf) = catalog::parse_typed(meta, &text, lo, hi) {
                            f = nf;
                            changed = true;
                            commit = true;
                        }
                    }
                } else {
                    ui.data_mut(|d| d.insert_temp(edit_id, (text, true)));
                }
            } else {
                let shown = catalog::format_value(meta, f);
                let r = ui
                    .add(egui::Label::new(RichText::new(&shown).monospace().color(TEXT)).sense(egui::Sense::click()))
                    .on_hover_cursor(egui::CursorIcon::Text)
                    .on_hover_text("Click to type a value");
                if r.clicked() {
                    ui.data_mut(|d| d.insert_temp(edit_id, (shown, false)));
                }
            }
            if changed {
                let nv = match v {
                    Value::F64(_) => Value::F64(f),
                    _ => Value::F32(f as f32),
                };
                return (Some(nv), commit);
            } else if commit {
                return (None, true);
            }
        }
        other => {
            ui.label(RichText::new(other.summary()).small().color(MUTED));
        }
    }
    (None, false)
}

// ------------------------------------------------------------ pedal preset ---

#[allow(clippy::too_many_arguments)]
pub fn device_editor(
    ui: &mut egui::Ui,
    preset: &mut DevicePreset,
    selected: &mut Option<usize>,
    dirty: &mut bool,
    live: bool,
    cmds: &mut Vec<Cmd>,
    cat: Option<&Catalog>,
    picker: &mut Option<PickerState>,
) {
    if chain_part() {
    // Tempo and snapshots (in the Studio look they sit in the top bar).
    if !crate::chain::studio() {
    ui.horizontal_wrapped(|ui| {
        if let Some(fw) = preset.firmware() {
            theme::pill(ui, &format!("firmware {fw}"), MUTED);
        }
        if let Some(mut t) = preset.tempo() {
            ui.label(RichText::new("Tempo").color(MUTED));
            if ui.add(egui::DragValue::new(&mut t).range(30.0..=240.0).speed(0.5).suffix(" bpm")).changed() {
                preset.set_tempo(t);
                *dirty = true;
            }
        }
        let snaps = preset.snapshot_names();
        if !snaps.is_empty() {
            ui.add_space(12.0);
            ui.label(RichText::new("Snapshots").color(MUTED));
            let count = snaps.len();
            let names = snaps.clone();
            for (i, mut n) in snaps.into_iter().enumerate() {
                let num = ui.add(egui::Button::new(format!("{}", i + 1)).small());
                let num = if live { num.on_hover_text("Switch to this snapshot. Right-click to copy another snapshot into it.") } else { num.on_hover_text("Right-click to copy another snapshot into this one") };
                if live && num.clicked() {
                    cmds.push(Cmd::Snapshot(i as i64));
                }
                num.context_menu(|ui| {
                    if live {
                        ui.label(RichText::new("Copying works on the offline copy.\nSave a copy of this preset first.").small().color(MUTED));
                        return;
                    }
                    ui.label(RichText::new(format!("Make snapshot {} the same as", i + 1)).small().color(MUTED));
                    for j in (0..count).filter(|j| *j != i) {
                        let from = names.get(j).filter(|s| !s.trim().is_empty()).cloned().unwrap_or_else(|| format!("Snapshot {}", j + 1));
                        if ui.button(format!("{}: {from}", j + 1)).clicked() {
                            if preset.copy_snapshot_states(j, i) {
                                *dirty = true;
                            }
                            ui.close_menu();
                        }
                    }
                });
                let r = ui.add(egui::TextEdit::singleline(&mut n).desired_width(80.0).char_limit(10));
                if r.changed() {
                    preset.set_snapshot_name(i, &n);
                    *dirty = true;
                }
                if live && (r.lost_focus() || (r.changed() && ui.input(|inp| inp.key_pressed(egui::Key::Enter)))) {
                    cmds.push(Cmd::RenameSnapshot { index: i as i64, name: n.clone() });
                }
            }
        }
    });
    ui.horizontal(|ui| {
        if live {
            theme::pill(ui, "LIVE", GOOD);
            theme::help(ui, "Changes are heard on the pedal at once. \"Save on pedal\" keeps them in the preset.");
        } else {
            theme::pill(ui, "OFFLINE COPY", MUTED);
            theme::help(ui, "Edit knobs and switches here, then save or send it. To swap effects, edit the live preset.");
        }
    });
    ui.add_space(10.0);
    theme::section(ui, "Signal chain");
    ui.add_space(4.0);
    }

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
                inside && !matches!(s.kind, Kind::Input | Kind::Output | Kind::Split | Kind::Join) && !lower.iter().any(|l| l.global == s.global)
            })
            .take(chain::COLS)
            .collect();
        let lower_used = lower.iter().any(|s| s.kind != Kind::Empty);
        // How much of this path's DSP the blocks use.
        let used = cat
            .map(|c| {
                in_path
                    .iter()
                    .map(|s| s.model.map(|m| dsp_percent(c, m)).unwrap_or(0.0) + s.paired.map(|m| dsp_percent(c, m)).unwrap_or(0.0))
                    .sum::<f64>()
            })
            .unwrap_or(0.0);
        let free = 100.0 - used;
        // Split and merge columns, from the main-lane slot each attaches before.
        let cols = main.len().clamp(1, chain::COLS);
        let col_before = |attach: Option<i64>| attach.map(|a| main.iter().filter(|s| (s.position as i64) < a).count());
        let split_col = split_slot.and_then(|s| col_before(preset.attach(s.path, s.position))).unwrap_or(0).min(cols.saturating_sub(1));
        let merge_col = join_slot
            .and_then(|s| col_before(preset.attach(s.path, s.position)))
            .unwrap_or(cols)
            .clamp(split_col, cols);
        let lower_selected = selected.is_some_and(|g| lower.iter().any(|s| s.global == g) || split_slot.is_some_and(|s| s.global == g) || join_slot.is_some_and(|s| s.global == g));
        // The lane shows while the path is split (the pedal's own flag), or holds or is being edited.
        let show_lower = !lower.is_empty() && (lower_used || lower_selected || preset.split_active(path));
        // On units whose two lanes share the slots, a full path has no room for a lower lane.
        let dev_fam = snapshots::family_of_device(crate::nam::unit());
        let path_full = in_path.iter().filter(|s| s.kind == Kind::Block).count() as i64 >= dev_fam.max_blocks;
        let last_used_lower = lower
            .iter()
            .rposition(|s| s.kind != Kind::Empty)
            .map(|i| split_col + i + 1)
            .unwrap_or(split_col + 1);
        let _ = last_used_lower;
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
            // In the Studio look the path's DSP sits in the preset bar.
            if cat.is_some() && !crate::chain::studio() {
                dsp_ring(ui, used);
                let col = if used > 90.0 { BAD } else if used > 70.0 { ORANGE } else { GOOD };
                ui.label(RichText::new(format!("DSP {used:.0}%")).small().strong().color(col))
                    .on_hover_text("How much of the pedal's processing this path uses (estimated from HX Edit's figures)");
            }
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
                            *picker = Some(PickerState { block: s.wire_block(), slot: s.global, path: s.path, position: s.position, free, ..Default::default() });
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
                    let free_at = (0..main.len())
                        .filter(|c| main[*c].kind == Kind::Empty)
                        .min_by_key(|c| (*c as i64 - col as i64).abs() * 2 + if *c < col { 1 } else { 0 });
                    match free_at.map(|c| main[c]) {
                        Some(s) => {
                            *selected = Some(s.global);
                            let (category, search) = match hit {
                                chain::Hit::GapSendReturn(_, k) => ("Send/Return".to_owned(), loop_search(k)),
                                _ => (String::new(), String::new()),
                            };
                            *picker = Some(PickerState { block: s.wire_block(), slot: s.global, path: s.path, position: s.position, free, category, search, ..Default::default() });
                        }
                        None => {
                            note(ui.ctx(), "No free place on this path: remove or move an effect first.");
                        }
                    }
                }
                chain::Hit::Drop(from, to) => {
                    if let (Some(a), Some(b)) = (slot_at(from.0, from.1), slot_at(to.0, to.1)) {
                        *dev_drop = Some((SlotDrag { path: a.path, position: a.position, global: a.global }, b.path, b.position, b.global));
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
                            *picker = Some(PickerState { block: s.wire_block(), slot: s.global, path: s.path, position: s.position, free, ..Default::default() });
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
                        *dev_drop = Some((SlotDrag { path: a.path, position: a.position, global: a.global }, b.path, b.position, b.global));
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
        if !show_lower && !lower.is_empty() && cat.is_some() && !crate::chain::studio() {
            if ui.small_button("+ Split into two lanes").on_hover_text("Show the lower lane to add effects to it").clicked() {
                // Place the split before the first free main position and the merge two later.
                let first_free = main.iter().position(|s| s.kind == Kind::Empty).unwrap_or(0);
                if let (Some(sp), Some(jp)) = (split_slot, join_slot) {
                    let attach_at = |col: usize| main.get(col).map(|s| s.position as i64).unwrap_or_else(|| out.unwrap_or(9) as i64);
                    let sa = attach_at(first_free);
                    let ja = attach_at(first_free + 2);
                    preset.set_attach(sp.path, sp.position, sa);
                    preset.set_attach(jp.path, jp.position, ja);
                    if live {
                        cmds.push(Cmd::MoveSplit { block: 20 * sp.path + sa });
                        cmds.push(Cmd::MoveJoin { block: 20 * jp.path + ja });
                    }
                    *dirty = true;
                }
                *selected = split_slot.map(|s| s.global);
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

    if !crate::chain::studio() {
        ui.add_space(8.0);
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
                    inside && !matches!(x.kind, Kind::Input | Kind::Output | Kind::Split | Kind::Join) && !lower_positions.contains(&x.position)
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
                if ui.add(egui::DragValue::new(&mut cur).range(min_c..=max_c)).on_hover_text("Which chain position it sits before (or drag the dot on the path)").changed() {
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
        knob_grid(ui, preset, &s, sel, false, live, cmds, cat, dirty);
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
            knob_grid(ui, preset, &s, sel, true, live, cmds, cat, dirty);
        }
        });
        });
    });
}

/// One snapshot chip of the Studio top bar ("1 CLEAN"); `on` when it is the current one.
fn snap_chip(ui: &mut egui::Ui, n: usize, name: &str, on: bool) -> egui::Response {
    let label = if name.trim().is_empty() { format!("{n}") } else { format!("{n}  {}", name.trim()) };
    let text = RichText::new(label).monospace().size(12.5).color(if on { Color32::from_rgb(8, 22, 26) } else { TEXT });
    ui.add(egui::Button::new(text).fill(if on { accent() } else { Color32::TRANSPARENT }).stroke(Stroke::NONE).min_size(Vec2::new(0.0, 24.0)))
}

/// The Studio top bar's preset strip for a .hlx preset: BPM and the snapshot chips
/// (click to recall; right-click to rename, copy, paste, duplicate or delete; + adds one).
pub fn studio_strip_hlx(ui: &mut egui::Ui, json: &mut serde_json::Value, dirty: &mut bool) {
    let list = snapshots::list(json);
    let cur = snapshots::current(json);
    let fam = snapshots::family(json);
    let clip_id = egui::Id::new("tonestrand-snapshot-clipboard");
    let clip: Option<(String, serde_json::Value)> = ui.data(|d| d.get_temp(clip_id));
    let mut action: Option<(&str, usize)> = None;
    egui::Frame::new().fill(bg()).corner_radius(CornerRadius::same(7)).inner_margin(egui::Margin::same(2)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (i, name) in &list {
                let r = snap_chip(ui, i + 1, name, *i == cur).on_hover_text("Recall this snapshot. Right-click to rename, copy, paste, duplicate or delete.");
                if r.clicked() && *i != cur {
                    snapshots::select(json, *i);
                    *dirty = true;
                }
                r.context_menu(|ui| {
                    let mut n = name.clone();
                    ui.horizontal(|ui| {
                        ui.label("Name");
                        if ui.add(egui::TextEdit::singleline(&mut n).desired_width(110.0).char_limit(10)).changed() {
                            snapshots::rename(json, *i, &n);
                            *dirty = true;
                        }
                    });
                    ui.separator();
                    if ui.button("Copy").clicked() {
                        action = Some(("copy", *i));
                        ui.close_menu();
                    }
                    let paste = clip.as_ref().map(|(f, _)| format!("Paste {f} here")).unwrap_or_else(|| "Paste".into());
                    if ui.add_enabled(clip.is_some(), egui::Button::new(paste)).clicked() {
                        action = Some(("paste", *i));
                        ui.close_menu();
                    }
                    if ui.add_enabled(list.len() < fam.snapshot_count, egui::Button::new("Duplicate")).clicked() {
                        action = Some(("duplicate", *i));
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.add_enabled(list.len() > 1, egui::Button::new(RichText::new("Delete").color(BAD))).clicked() {
                        action = Some(("delete", *i));
                        ui.close_menu();
                    }
                });
            }
            if list.len() < fam.snapshot_count && ui.small_button("+").on_hover_text("Add a snapshot (a copy of the current one)").clicked() {
                if let Some(n) = snapshots::add(json) {
                    snapshots::select(json, n);
                    *dirty = true;
                }
            }
        });
    });
    match action {
        Some(("copy", i)) => {
            if let Some(v) = snapshots::copy(json, i) {
                let name = list.iter().find(|(x, _)| *x == i).map(|(_, n)| n.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| format!("Snapshot {}", i + 1));
                ui.data_mut(|d| d.insert_temp(clip_id, (name, v)));
            }
        }
        Some(("paste", i)) => {
            if let Some((_, v)) = &clip {
                snapshots::paste(json, i, v);
                *dirty = true;
            }
        }
        Some(("duplicate", i)) => {
            if i != cur {
                snapshots::select(json, i);
            }
            if let Some(n) = snapshots::add(json) {
                snapshots::select(json, n);
            }
            *dirty = true;
        }
        Some(("delete", i)) => {
            if snapshots::delete(json, i) {
                *dirty = true;
            }
        }
        _ => {}
    }
}

/// The Studio top bar's snapshot chips for a pedal preset: on the live preset a click
/// switches the pedal; right-click renames it or copies another snapshot into it.
pub fn studio_strip_device(ui: &mut egui::Ui, preset: &mut DevicePreset, dirty: &mut bool, live: bool, cmds: &mut Vec<Cmd>) {
    let names = preset.snapshot_names();
    if names.is_empty() {
        return;
    }
    let cur = preset.current_snapshot();
    egui::Frame::new().fill(bg()).corner_radius(CornerRadius::same(7)).inner_margin(egui::Margin::same(2)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (i, name) in names.iter().enumerate() {
                let tip = if live { "Switch the pedal to this snapshot. Right-click to rename it." } else { "Right-click to rename it or copy another snapshot into it." };
                let r = snap_chip(ui, i + 1, name, i == cur).on_hover_text(tip);
                if live && r.clicked() && i != cur {
                    cmds.push(Cmd::Snapshot(i as i64));
                }
                r.context_menu(|ui| {
                    let mut n = name.clone();
                    ui.horizontal(|ui| {
                        ui.label("Name");
                        let te = ui.add(egui::TextEdit::singleline(&mut n).desired_width(110.0).char_limit(10));
                        if te.changed() {
                            preset.set_snapshot_name(i, &n);
                            *dirty = true;
                        }
                        if live && te.lost_focus() {
                            cmds.push(Cmd::RenameSnapshot { index: i as i64, name: n.clone() });
                        }
                    });
                    if !live {
                        ui.separator();
                        ui.label(RichText::new(format!("Make snapshot {} the same as", i + 1)).small().color(MUTED));
                        for j in (0..names.len()).filter(|j| *j != i) {
                            let from = names.get(j).filter(|s| !s.trim().is_empty()).cloned().unwrap_or_else(|| format!("Snapshot {}", j + 1));
                            if ui.button(format!("{}: {from}", j + 1)).clicked() {
                                if preset.copy_snapshot_states(j, i) {
                                    *dirty = true;
                                }
                                ui.close_menu();
                            }
                        }
                    }
                });
            }
        });
    });
}

/// Each path of a .hlx preset: (blocks used, blocks it can hold, estimated DSP %).
pub fn hlx_path_meters(cat: Option<&Catalog>, json: &serde_json::Value) -> Vec<(usize, usize, Option<f64>)> {
    let unit = snapshots::family(json);
    let dsp = cat.map(|c| hlx_dsp_use(c, json));
    hlx::dsps(json)
        .into_iter()
        .take(unit.paths)
        .map(|d| {
            let used = dsp.as_ref().and_then(|v| v.iter().find(|(k, _)| *k == d).map(|(_, u)| *u));
            (hlx::block_count(json, &d) as usize, unit.max_blocks as usize, used)
        })
        .collect()
}

/// Each path of a pedal preset that holds blocks: (blocks used, blocks it can hold, DSP %).
pub fn device_path_meters(cat: Option<&Catalog>, preset: &DevicePreset) -> Vec<(usize, usize, Option<f64>)> {
    let fam = snapshots::family_of_device(crate::nam::unit());
    let slots = preset.slots();
    (0..fam.paths as i64)
        .map(|p| {
            let in_path: Vec<&SlotView> = slots.iter().filter(|s| s.path == p).collect();
            let count = in_path.iter().filter(|s| s.kind == Kind::Block).count();
            let used = cat.map(|c| {
                in_path
                    .iter()
                    .map(|s| s.model.map(|m| dsp_percent(c, m)).unwrap_or(0.0) + s.paired.map(|m| dsp_percent(c, m)).unwrap_or(0.0))
                    .sum::<f64>()
            });
            (count, fam.max_blocks as usize, used)
        })
        .collect()
}

/// What the effect list is filtered to for an FX loop (0), a send (1) or a return (2).
fn loop_search(kind: u8) -> String {
    match kind {
        1 => "Send".into(),
        2 => "Return".into(),
        _ => "FX Loop".into(),
    }
}

/// A short message for the person, shown for a few seconds above the chain.
pub fn note(ctx: &egui::Context, msg: &str) {
    let now = ctx.input(|i| i.time);
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("editor-note"), (msg.to_owned(), now)));
}

/// The message from note(), while it's recent.
pub fn current_note(ctx: &egui::Context) -> Option<String> {
    let now = ctx.input(|i| i.time);
    ctx.data(|d| d.get_temp::<(String, f64)>(egui::Id::new("editor-note"))).filter(|(_, t)| now - t < 6.0).map(|(m, _)| m)
}

/// Which part of an editor to draw: 0 everything, 1 only the chain (and what's above it),
/// 2 only the selected block's panel. The Studio look draws the panel docked at the bottom.
static PART: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
pub fn set_part(p: u8) {
    PART.store(p, std::sync::atomic::Ordering::Relaxed);
}
fn chain_part() -> bool {
    PART.load(std::sync::atomic::Ordering::Relaxed) != 2
}
fn panel_part() -> bool {
    PART.load(std::sync::atomic::Ordering::Relaxed) != 1
}

/// Width of the photo-and-notes column beside a block's knobs.
const SIDE_W: f32 = 210.0;

/// A "Change cab" menu for the second model of a block: the cabs of the same kind
/// (dual cabs for a dual cab's second cab, single mic cabs for an amp's cab; legacy
/// cabs for legacy ones). Returns the cab picked.
fn cab_menu(ui: &mut egui::Ui, cat: &Catalog, current: i64, dual: bool) -> Option<i64> {
    let sym = cat.symbol(current).unwrap_or("").to_owned();
    let legacy = !sym.starts_with("HD2_CabMicIr");
    let mut picked = None;
    ui.menu_button("Change cab...", |ui| {
        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
            for g in cat.picker_groups().iter().filter(|g| g.category == "Cab") {
                let sub = g.sub.to_lowercase();
                let fits = sub.contains("legacy") == legacy && (sub.contains("dual") == dual || legacy);
                if !fits {
                    continue;
                }
                for m in &g.models {
                    // A dual cab's second cab must be a dual (with pan) cab too.
                    if dual && !legacy && !cat.symbol(m.number).is_some_and(|x| x.ends_with("WithPan")) {
                        continue;
                    }
                    if ui.selectable_label(m.number == current, crate::realnames::show(&m.name)).clicked() {
                        picked = Some(m.number);
                        ui.close_menu();
                    }
                }
            }
        });
    })
    .response
    .on_hover_text(if dual { "Pick the second cab of this dual cab on its own" } else { "Pick another cab for this amp" });
    picked.filter(|p| *p != current)
}

/// The knobs of a block (or of its paired cab), two columns wide.
#[allow(clippy::too_many_arguments)]
fn knob_grid(
    ui: &mut egui::Ui,
    preset: &mut DevicePreset,
    s: &SlotView,
    sel: usize,
    cab: bool,
    live: bool,
    cmds: &mut Vec<Cmd>,
    cat: Option<&Catalog>,
    dirty: &mut bool,
) {
    let values = if cab { preset.paired_values(s.path, s.position) } else { s.values.clone() };
    let model = if cab { s.paired } else { s.model };
    // For splits, merges, inputs and outputs the parameter names come from the same table.
    let per_row = ((ui.available_width() + 6.0) / 116.0).floor().max(1.0) as usize;
    let all: Vec<(usize, &Value)> = values.iter().enumerate().collect();
    for row in all.chunks(per_row) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(6.0, 12.0);
        for &(vi, v) in row.iter() {
            let (name, meta) = match (model, cat) {
                (Some(m), Some(c)) => c.param(m, vi),
                _ => (format!("Setting {}", vi + 1), None),
            };
            let (nv, commit) = param_cell(ui, &name, meta.as_ref(), v);
            if let Some(nv) = &nv {
                if cab {
                    preset.set_paired_value(s.path, s.position, vi, nv.clone());
                } else {
                    preset.set_value(s.path, s.position, vi, nv.clone());
                }
                *dirty = true;
            }
            if live && commit {
                let cur = if cab {
                    preset.paired_values(s.path, s.position).get(vi).cloned()
                } else {
                    preset.slots().get(sel).and_then(|x| x.values.get(vi).cloned())
                };
                if let Some(value) = cur {
                    // Key 26 picks the block's own knobs (0) or its cab's (1).
                    cmds.push(Cmd::SetParam { block: s.wire_block(), path: cab as i64, index: vi as i64, value });
                }
            }
        }
    });
    }
}

/// Edit a controller bound (min or max) keeping the value's type; Some(new) when changed.
fn bound_cell(ui: &mut egui::Ui, v: &Value, meta: Option<&crate::catalog::Param>) -> Option<Value> {
    match v {
        Value::Bool(b) => {
            let mut x = *b;
            ui.checkbox(&mut x, "").changed().then_some(Value::Bool(x))
        }
        _ => {
            let mut x = v.as_f64().unwrap_or(0.0);
            let (lo, hi) = meta.map(|m| (m.min, m.max)).filter(|(a, b)| b > a).unwrap_or((0.0, 1.0));
            let int = matches!(v, Value::Int(..));
            let r = ui.add(
                egui::DragValue::new(&mut x)
                    .range(lo..=hi)
                    .speed(if int { 0.2 } else { (hi - lo) / 200.0 })
                    .custom_formatter(|n, _| crate::catalog::format_value(meta, n)),
            );
            r.changed().then(|| if int { Value::int(x.round() as i64) } else { Value::F32(x as f32) })
        }
    }
}

/// The Controllers card of a pedal preset: every knob on an expression pedal,
/// footswitch, snapshots or MIDI, with its range (and CC), to change or remove,
/// and adding a knob. Edits are kept in the preset (written when it's saved or
/// sent); on the live preset they are also sent to the pedal at once.
pub fn device_controllers(ui: &mut egui::Ui, preset: &mut DevicePreset, live: bool, cmds: &mut Vec<Cmd>, cat: Option<&Catalog>, dirty: &mut bool) {
    let fam = snapshots::family_of_device(crate::nam::unit());
    let mut list = preset.controllers();
    let slots = preset.slots();
    let name_of_ctl = |c: i64| snapshots::Source::from_code(c, fam).map(|s| s.label()).unwrap_or_else(|| format!("Controller {c}"));
    let snap_names: Vec<String> = preset
        .snapshot_names()
        .into_iter()
        .enumerate()
        .map(|(i, n)| if n.trim().is_empty() { format!("{}", i + 1) } else { n })
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
            ui.label(RichText::new(format!("{} assignment{}", list.len(), if list.len() == 1 { "" } else { "s" })).color(MUTED));
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
            ui.label(RichText::new(match tab {
                0 => "Nothing is on a footswitch.",
                1 => "No knob is on an expression pedal.",
                2 => "No knob changes with the snapshots.",
                3 => "No knob is on a MIDI CC.",
                _ => "Nothing here.",
            }).color(MUTED));
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
                                            .map(|_| (code == fam.snapshots_ctrl).then(|| (false, preset.knob_value(s.wire_block(), k, false).unwrap_or(Value::Nil))))
                                            .collect();
                                        let cc = if code == fam.midi_ctrl { 1 } else { 4 };
                                        if live {
                                            cmds.push(Cmd::AssignController { block: s.wire_block(), cab: false, index: k as i64, controller: code, cc });
                                        } else {
                                            list.push(crate::devpreset::Ctl::new_knob(code, s.wire_block(), k, false, typed(lo), typed(hi), cc, snaps));
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

// ---------------------------------------------------------------- .hlx file ---

/// What is being dragged in a .hlx chain: a block by its DSP and key.
#[derive(Clone)]
struct HlxDrag {
    dsp: String,
    key: String,
}

/// What is being dragged in a pedal preset's chain: a slot.
#[derive(Clone, Copy)]
struct SlotDrag {
    path: i64,
    position: usize,
    global: usize,
}

/// The snapshot buttons: pick one to hear and edit it; its name; add another.
/// Knobs moved and blocks switched while a snapshot is picked are kept in it.
fn snapshot_bar(ui: &mut egui::Ui, json: &mut serde_json::Value, dirty: &mut bool) {
    let list = snapshots::list(json);
    let cur = snapshots::current(json);
    let fam = snapshots::family(json);
    theme::card(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("SNAPSHOTS").small().strong().color(MUTED));
            theme::help(ui, "Each snapshot is a variation of this preset. Knobs you turn now are stored in the picked snapshot (they get a SNAP tag), and every block's on/off is stored too.");
            let clip_id = egui::Id::new("tonestrand-snapshot-clipboard");
            let clip: Option<(String, serde_json::Value)> = ui.data(|d| d.get_temp(clip_id));
            let mut action: Option<(&str, usize)> = None;
            for (i, name) in &list {
                let on = *i == cur;
                let label = if name.is_empty() { format!("Snapshot {}", i + 1) } else { name.clone() };
                let text = RichText::new(&label).strong().color(if on { Color32::from_rgb(8, 26, 24) } else { TEXT });
                let b = egui::Button::new(text).fill(if on { accent() } else { card_hi() }).min_size(Vec2::new(96.0, 28.0));
                let r = ui.add(b);
                r.context_menu(|ui| {
                    if ui.button("Copy").clicked() {
                        action = Some(("copy", *i));
                        ui.close_menu();
                    }
                    let paste = match &clip {
                        Some((from, _)) => format!("Paste {from} here"),
                        None => "Paste".to_owned(),
                    };
                    if ui.add_enabled(clip.is_some(), egui::Button::new(paste)).clicked() {
                        action = Some(("paste", *i));
                        ui.close_menu();
                    }
                    if ui.add_enabled(list.len() < fam.snapshot_count, egui::Button::new("Duplicate")).clicked() {
                        action = Some(("duplicate", *i));
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.add_enabled(list.len() > 1, egui::Button::new(RichText::new("Delete").color(BAD))).clicked() {
                        action = Some(("delete", *i));
                        ui.close_menu();
                    }
                });
                if r.on_hover_text("Recall this snapshot; changes you make now are kept in it. Right-click to copy, paste, duplicate or delete.").clicked() && !on {
                    snapshots::select(json, *i);
                    *dirty = true;
                }
            }
            if list.len() < fam.snapshot_count && ui.button("+ Snapshot").on_hover_text("Add a snapshot that starts as a copy of this one").clicked() {
                if let Some(n) = snapshots::add(json) {
                    snapshots::select(json, n);
                    *dirty = true;
                }
            }
            if let Some((_, name)) = list.iter().find(|(i, _)| *i == cur) {
                let mut n = name.clone();
                ui.add_space(12.0);
                ui.label(RichText::new("Name").color(MUTED));
                if ui.add(egui::TextEdit::singleline(&mut n).desired_width(110.0).char_limit(10)).changed() {
                    snapshots::rename(json, cur, &n);
                    *dirty = true;
                }
                if ui.small_button("Copy").on_hover_text("Copy this snapshot's settings, to paste into another").clicked() {
                    action = Some(("copy", cur));
                }
                if let Some((from, _)) = &clip {
                    if ui.small_button("Paste").on_hover_text(format!("Put {from}'s settings into this snapshot")).clicked() {
                        action = Some(("paste", cur));
                    }
                }
                if list.len() > 1 && ui.small_button(RichText::new("Delete").color(BAD)).on_hover_text("Take this snapshot out").clicked() {
                    action = Some(("delete", cur));
                }
            }
            match action {
                Some(("copy", i)) => {
                    if let Some(v) = snapshots::copy(json, i) {
                        let name = list.iter().find(|(x, _)| *x == i).map(|(_, n)| n.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| format!("Snapshot {}", i + 1));
                        ui.data_mut(|d| d.insert_temp(clip_id, (name, v)));
                    }
                }
                Some(("paste", i)) => {
                    if let Some((_, v)) = &clip {
                        snapshots::paste(json, i, v);
                        *dirty = true;
                    }
                }
                Some(("duplicate", i)) => {
                    if i != cur {
                        snapshots::select(json, i);
                    }
                    if let Some(n) = snapshots::add(json) {
                        snapshots::select(json, n);
                    }
                    *dirty = true;
                }
                Some(("delete", i)) => {
                    if snapshots::delete(json, i) {
                        *dirty = true;
                    }
                }
                _ => {}
            }
        });
    });
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

/// Names of input sources and output destinations for a pedal family.
fn route_names(fam: snapshots::Family, input: bool) -> Vec<(i64, &'static str)> {
    let stomp = fam.name.starts_with("HX Stomp");
    if input && stomp {
        vec![(0, "None"), (1, "Main L/R"), (4, "Return L/R"), (6, "USB 5/6")]
    } else if input {
        [
            "None", "Multi (Guitar, Aux, Variax)", "Guitar", "Aux", "Variax", "Variax Magnetics", "Mic", "Return 1", "Return 2", "Return 3",
            "Return 4", "Return 1/2", "Return 3/4", "S/PDIF", "USB 3/4", "USB 5/6", "USB 7/8",
        ]
        .iter()
        .enumerate()
        .map(|(i, n)| (i as i64, *n))
        .collect()
    } else if stomp {
        vec![(0, "None"), (1, "Multi (Main L/R, Send, USB)")]
    } else {
        [
            "None", "Multi (1/4\", XLR, Digital, USB 1/2)", "Path 2A", "Path 2B", "Path 2A+B", "1/4\"", "XLR", "Send 1/2", "Send 3/4", "Digital",
            "USB 1/2", "USB 3/4", "USB 5/6",
        ]
        .iter()
        .enumerate()
        .map(|(i, n)| (i as i64, *n))
        .collect()
    }
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
    // Tempo, unit and snapshots (in the Studio look they sit in the top bar).
    if chain_part() && !crate::chain::studio() {
    ui.horizontal_wrapped(|ui| {
        if let Some(mut t) = hlx::tempo(json) {
            ui.label(RichText::new("Tempo").color(MUTED));
            if ui.add(egui::DragValue::new(&mut t).range(30.0..=240.0).speed(0.5).suffix(" bpm")).changed() {
                hlx::set_tempo(json, t);
                *dirty = true;
            }
            ui.add_space(12.0);
        }
        // Which unit the preset is for; it decides lanes, paths, snapshots and switches.
        // Shown here, chosen with Unit at the top (where presets are converted and checked).
        let device = snapshots::device(json);
        theme::pill(ui, &format!("for {}", crate::units::unit_name(device)), MUTED);
        let _ = dirty;
    });
    ui.add_space(8.0);
    snapshot_bar(ui, json, dirty);
    ui.add_space(10.0);
    theme::section_help(
        ui,
        "Signal chain",
        "Click a block to edit its knobs, double-click to switch it on or off, and drag it onto another position to move it. Click + to add an effect.",
    );
    ui.add_space(2.0);
    }

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
            // In the Studio look the path's DSP and block count sit in the preset bar.
            if crate::chain::studio() {
                return;
            }
            if cat.is_some() {
                dsp_ring(ui, used);
                let col = if used > 90.0 { BAD } else if used > 70.0 { ORANGE } else { GOOD };
                ui.label(RichText::new(format!("DSP {:.0}%", used.abs())).small().strong().color(col));
            }
            let bcol = if full { ORANGE } else { MUTED };
            ui.label(RichText::new(format!("·  {count}/{} blocks", unit.max_blocks)).small().color(bcol));
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
        if unit.lanes && split.is_none() && (!unit.shared_slots || main_blocks < unit.max_blocks) && !crate::chain::studio() {
            if ui.small_button("+ Split into two lanes").on_hover_text("Add a split and a merge; the lower lane appears under the main one").clicked() {
                let defaults = |m: &str| cat.and_then(|c| c.number_for_hlx(m, false).map(|n| c.default_values(n))).unwrap_or_default();
                hlx::add_split(json, dsp, &defaults);
                *selected = Some(40_000 + di);
                *dirty = true;
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

    if !crate::chain::studio() {
        ui.add_space(8.0);
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
