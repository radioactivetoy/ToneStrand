//! The preset editor: the signal chain as tiles, and the selected block's
//! effect and knobs. Works on the pedal's live preset (every change is sent
//! at once), on presets read from a slot or a file, and on .hlx files.

use eframe::egui::{self, Color32, CornerRadius, RichText, Stroke, Vec2};

use crate::catalog::{self, Catalog, Param};
use crate::chain;
use crate::device::Cmd;
use crate::devpreset::{DevicePreset, Kind, SlotView};
use crate::hlx;
use crate::msgpack::Value;
use crate::snapshots;
use crate::theme::{self, *};

mod device;
mod doc;
mod hlx_file;
mod pacing;
mod studio;

pub use device::{device_controllers, device_editor};
use doc::slots_dsp;
pub use doc::PresetDoc;
pub use hlx_file::hlx_editor;
pub use studio::{studio_strip_device, studio_strip_hlx};

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
        vec![("Level", p("gain", "Level", 1, "volume", -120.0, 20.0, 0.0)), ("Pan", p("pan", "Pan", 1, "pan", 0.0, 1.0, 0.5))]
    }
}

/// The picture beside an input's or output's settings, where an effect has its photo: a jack with an arrow
/// showing which way the sound goes.
fn io_tile(ui: &mut egui::Ui, input: bool, width: f32) {
    egui::Frame::new()
        .fill(Color32::from_rgb(0x10, 0x11, 0x14))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(egui::Margin::same(6))
        .show(ui, |ui| {
            ui.set_width(width - 12.0);
            let (rect, _) = ui.allocate_exact_size(Vec2::new(width - 24.0, 110.0), egui::Sense::hover());
            let p = ui.painter();
            let c = rect.center() + Vec2::new(if input { 14.0 } else { -14.0 }, -6.0);
            let steel = Color32::from_rgb(0x8a, 0x90, 0x9a);
            // The jack: a nut, its sleeve, and the hole.
            p.circle_filled(c, 30.0, Color32::from_rgb(0x2a, 0x2d, 0x33));
            p.circle_stroke(c, 30.0, Stroke::new(2.0_f32, steel.linear_multiply(0.55)));
            p.circle_filled(c, 21.0, steel.linear_multiply(0.75));
            p.circle_filled(c, 13.0, Color32::from_rgb(0x08, 0x09, 0x0b));
            p.circle_filled(c, 4.0, steel.linear_multiply(0.5));
            // An arrow into the jack (input) or out of it (output).
            let y = c.y;
            let (from, to) = if input { (c.x - 78.0, c.x - 38.0) } else { (c.x + 38.0, c.x + 78.0) };
            let col = accent();
            p.line_segment([egui::pos2(from, y), egui::pos2(to, y)], Stroke::new(3.0_f32, col));
            let tip = to;
            p.add(egui::Shape::convex_polygon(
                vec![egui::pos2(tip + 8.0, y), egui::pos2(tip - 2.0, y - 7.0), egui::pos2(tip - 2.0, y + 7.0)],
                col,
                Stroke::NONE,
            ));
            p.text(
                egui::pos2(rect.center().x, rect.bottom() - 8.0),
                egui::Align2::CENTER_CENTER,
                if input { "INPUT" } else { "OUTPUT" },
                egui::FontId::proportional(11.0),
                muted(),
            );
        });
}

/// An input's or output's panel, laid out like an effect's: the picture on the left, the settings on the right.
fn io_panel(ui: &mut egui::Ui, preset: &mut DevicePreset, s: &SlotView, live: bool, cmds: &mut Vec<Cmd>, dirty: &mut bool) {
    let input = s.kind == Kind::Input;
    let full = ui.available_width();
    let side = full > 700.0;
    let body_h = BODY_H;
    ui.horizontal_top(|ui| {
        ui.set_min_height(body_h);
        if side {
            ui.vertical(|ui| {
                ui.set_width(SIDE_W);
                io_tile(ui, input, SIDE_W);
            });
            ui.add_space(10.0);
            let (r, _) = ui.allocate_exact_size(Vec2::new(1.0, body_h), egui::Sense::hover());
            ui.painter().rect_filled(r, CornerRadius::ZERO, line());
            ui.add_space(10.0);
        }
        let rest = if side { full - SIDE_W - 21.0 } else { full };
        ui.vertical(|ui| {
            ui.set_max_width(rest);
            ui.set_min_width(rest.min(200.0));
            io_controls(ui, preset, s, live, cmds, dirty);
        });
    });
}

/// The settings of a path's input or output: where the sound comes from or goes, and its knobs.
fn io_controls(ui: &mut egui::Ui, preset: &mut DevicePreset, s: &SlotView, live: bool, cmds: &mut Vec<Cmd>, dirty: &mut bool) {
    let input = s.kind == Kind::Input;
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(if input { "Source" } else { "Destination" }).color(muted()));
        if let Some(route) = preset.io_route(s.path, s.position) {
            // The jacks by name, numbered as in .hlx files (the same numbers the pedal keeps).
            let fam = snapshots::family_of_device(crate::nam::unit());
            let mut names: Vec<(i64, String)> =
                crate::routes::route_names(fam, input, s.path > 0).into_iter().map(|(v, n)| (v, n.to_owned())).collect();
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
        // The guitar input impedance and pad belong to the preset; they sit with the first path's input.
        if input && s.path == 0 {
            ui.add_space(14.0);
            device::guitar_input_controls(ui, preset, live, cmds, dirty);
        }
    });
    ui.add_space(8.0);
    let names = io_params(input);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(6.0, 12.0);
        for (vi, v) in s.values.iter().enumerate() {
            let (name, meta) =
                names.get(vi).map(|(n, m)| (n.to_string(), Some(m.clone()))).unwrap_or((format!("Setting {}", vi + 1), None));
            let (nv, commit) = param_cell(ui, &name, meta.as_ref(), v);
            if let Some(nv) = &nv {
                preset.set_value(s.path, s.position, vi, nv.clone());
                *dirty = true;
            }
            if live && pacing::live_send(ui, ("io", s.global, vi), nv.is_some(), commit) {
                if let Some(value) = preset.slots().get(s.global).and_then(|x| x.values.get(vi).cloned()) {
                    cmds.push(Cmd::SetParam { block: s.wire_block(), path: 0, index: vi as i64, value });
                }
            }
        }
    });
}

/// An input's or output's title with a small tile of its own where an effect has its icon: a jack and an
/// arrow, in the colour of the tile's border.
fn io_header(ui: &mut egui::Ui, input: bool, title: &str) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(34.0), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(6), Color32::from_rgb(0x10, 0x11, 0x14));
    p.rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.5_f32, accent()), egui::StrokeKind::Inside);
    let steel = Color32::from_rgb(0x8a, 0x90, 0x9a);
    let c = rect.center() + Vec2::new(if input { 5.0 } else { -5.0 }, 0.0);
    p.circle_filled(c, 9.0, steel.linear_multiply(0.75));
    p.circle_filled(c, 5.0, Color32::from_rgb(0x08, 0x09, 0x0b));
    let (from, to) = if input { (rect.left() + 4.0, c.x - 11.0) } else { (c.x + 11.0, rect.right() - 6.0) };
    p.line_segment([egui::pos2(from, c.y), egui::pos2(to, c.y)], Stroke::new(2.0_f32, accent()));
    p.add(egui::Shape::convex_polygon(
        vec![egui::pos2(to + 3.0, c.y), egui::pos2(to - 2.0, c.y - 3.5), egui::pos2(to - 2.0, c.y + 3.5)],
        accent(),
        Stroke::NONE,
    ));
    ui.add_space(4.0);
    ui.label(RichText::new(title).size(19.0).strong().color(ink()));
}

fn block_header(ui: &mut egui::Ui, colour: Color32, title: &str, extra: &str) {
    let (r, _) = ui.allocate_exact_size(Vec2::new(4.0, 26.0), egui::Sense::hover());
    ui.painter().rect_filled(r, CornerRadius::same(2), colour);
    ui.add_space(2.0);
    ui.label(RichText::new(title).size(19.0).strong().color(ink()));
    if !extra.is_empty() {
        ui.label(RichText::new(extra).color(muted()));
    }
}

/// A block panel's title with the effect type's icon (or its colour bar when the type has none).
fn block_header_icon(ui: &mut egui::Ui, colour: Color32, category: &str, symbol: &str, title: &str, extra: &str) {
    if crate::icons::show_model(ui, category, symbol, title, 34.0) {
        ui.add_space(4.0);
        ui.label(RichText::new(title).size(19.0).strong().color(ink()));
        if !extra.is_empty() {
            ui.label(RichText::new(extra).color(muted()));
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
            let (cname, col) =
                c.category(model).map(|(n, c)| (n, theme::rgb(c))).unwrap_or_else(|| ("Effect".into(), theme::number_colour(model)));
            (c.model_name(model), cname, col)
        }
        None => (format!("Model {model}"), "effect".into(), theme::number_colour(model)),
    }
}

pub use crate::catalog::DSP_BUDGET;

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
    let cname = s.model.and_then(|m| c.category(m)).map(|x| x.0).unwrap_or_default();
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
    /// The effect the pointer was last over (symbol and names; its description shows under the list).
    pub hover: Option<(String, String)>,
    /// The search box has been given the focus (once, when the window opens).
    pub focused: bool,
    /// For a .hlx file: how many FX loops its own unit has (else the connected unit's count).
    pub fx_loops: Option<i64>,
}

/// The "Add a block" window: categories on the left, variants as tabs,
/// a search box, and each effect's DSP cost (greyed when it will not fit).
/// Returns where it goes, the model and the paired cab once one is picked.
pub fn picker_window(ctx: &egui::Context, state: &mut Option<PickerState>, cat: &Catalog) -> Option<(PickerState, i64, Option<i64>)> {
    let st = state.as_mut()?;
    let enter = ctx.input(|i| i.key_pressed(egui::Key::Enter));
    let mut picked = None;
    let mut open = true;
    let all_groups = cat.picker_groups();
    let mut categories: Vec<(String, Color32, usize)> = Vec::new();
    for g in &all_groups {
        match categories.iter_mut().find(|c| c.0 == g.category) {
            Some(c) => c.2 += g.models.len(),
            None => categories.push((g.category.clone(), theme::rgb(g.colour), g.models.len())),
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
                ui.label(RichText::new(format!("Path {}, slot {}", st.path + 1, st.position + 1)).color(muted()));
                let col = if st.free < 15.0 { bad() } else { good() };
                ui.label(RichText::new(format!("{:.0}% DSP free", st.free.max(0.0))).color(col));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // A visible box, and the cursor in it from the start: type to search,
                    // Enter takes the first match, Esc closes.
                    let r = theme::search_box(ui, &mut st.search, "Search effects", 210.0);
                    if !st.focused {
                        r.request_focus();
                        st.focused = true;
                    }
                    if r.changed() {
                        st.hover = None;
                    }
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
                                if ui.selectable_label(sel, name.as_str()).clicked() {
                                    st.category = name.clone();
                                    st.sub.clear();
                                    st.search.clear();
                                    st.hover = None;
                                }
                                // The counts line up on the right.
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    ui.label(RichText::new(count.to_string()).small().color(muted()));
                                });
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
                                let tip = match g.sub.as_str() {
                                    "Mono" => "Mono versions: one signal in, one out",
                                    "Stereo" => "Stereo versions: two signals in and out",
                                    "Legacy" => "Older versions of these effects",
                                    _ => "",
                                };
                                let r = ui.selectable_label(g.sub == st.sub, label);
                                let r = if tip.is_empty() { r } else { r.on_hover_text(tip) };
                                if r.clicked() {
                                    st.sub = g.sub.clone();
                                }
                            }
                        });
                        ui.add_space(4.0);
                    }
                    // The first effect that fits: Enter takes it while searching.
                    let mut first: Option<(i64, Option<i64>)> = None;
                    let mut first_pane: Option<(String, String)> = None;
                    egui::ScrollArea::vertical().id_salt("pick-models").max_height(340.0).auto_shrink([false, false]).show(ui, |ui| {
                        for g in groups.iter().filter(|g| !search.is_empty() || groups.len() == 1 || g.sub == st.sub) {
                            let loops = st.fx_loops.unwrap_or_else(|| snapshots::family_of_device(crate::nam::unit()).fx_loops);
                            for m in g
                                .models
                                .iter()
                                .filter(|m| {
                                    // Only the sends and returns this unit has jacks for.
                                    g.category != "Send/Return" || loop_numbers(&m.name).into_iter().all(|n| n <= loops)
                                })
                                .filter(|m| {
                                    search.is_empty()
                                        || m.name.to_lowercase().contains(&search)
                                        || crate::realnames::real(&m.name).is_some_and(|r| r.to_lowercase().contains(&search))
                                })
                            {
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
                                crate::theme::focus_ring(ui, &resp, 6);
                                resp.widget_info(|| {
                                    egui::WidgetInfo::labeled(egui::WidgetType::Button, true, crate::realnames::show(&m.name))
                                });
                                let p = ui.painter();
                                if resp.hovered() && fits {
                                    p.rect_filled(rect, CornerRadius::same(6), card_hi());
                                }
                                // The names for the details pane: the one in the list and its other name.
                                let pane_names = match crate::realnames::other(&m.name) {
                                    Some(o) => format!("{}  ·  {o}", crate::realnames::show(&m.name)),
                                    None => crate::realnames::show(&m.name),
                                };
                                if first.is_none() && fits {
                                    first = Some((m.number, paired));
                                    first_pane = cat.symbol(m.number).map(|s| (s.to_owned(), pane_names.clone()));
                                    if !search.is_empty() {
                                        p.rect_stroke(
                                            rect,
                                            CornerRadius::same(6),
                                            Stroke::new(1.0_f32, accent().gamma_multiply(0.6)),
                                            egui::StrokeKind::Inside,
                                        );
                                    }
                                }
                                if resp.hovered() {
                                    st.hover = cat.symbol(m.number).map(|s| (s.to_owned(), pane_names.clone()));
                                }
                                // The effect's own kind (Fuzz, Tape, Plate, Chorus...) from the Stadium icons;
                                // a colour square only when its type has none.
                                let sw = egui::Rect::from_min_size(rect.left_top() + Vec2::new(5.0, 5.0), Vec2::splat(24.0));
                                let sym_now = cat.symbol(m.number).unwrap_or("");
                                // Amps listed under "Amp+Cab" get the amp + cab icon.
                                let kind =
                                    if g.category.contains("Amp") && g.category.contains("Cab") { "Amp+Cab" } else { g.category.as_str() };
                                let tint = if fits { Color32::WHITE } else { Color32::from_white_alpha(90) };
                                if !crate::icons::paint_model(p, ui.ctx(), sw, kind, sym_now, &m.name, tint) {
                                    p.rect_filled(
                                        sw,
                                        CornerRadius::same(5),
                                        if fits { theme::rgb(g.colour) } else { theme::rgb(g.colour).linear_multiply(0.35) },
                                    );
                                }
                                let tc = if fits { ink() } else { muted() };
                                let title =
                                    if search.is_empty() || g.sub.is_empty() { shown.clone() } else { format!("{}  ·  {}", shown, g.sub) };
                                p.text(
                                    rect.left_center() + Vec2::new(38.0, 0.0),
                                    egui::Align2::LEFT_CENTER,
                                    title,
                                    egui::FontId::proportional(14.0),
                                    tc,
                                );
                                if cost > 0.0 {
                                    p.text(
                                        rect.right_center() - Vec2::new(10.0, 0.0),
                                        egui::Align2::RIGHT_CENTER,
                                        format!("{cost:.0}%"),
                                        egui::FontId::monospace(12.0),
                                        if fits { muted() } else { bad() },
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
                                    ui.label(RichText::new(tip).small().color(muted()));
                                });
                                if resp.clicked() && fits {
                                    picked = Some((st.clone(), m.number, paired));
                                }
                            }
                        }
                    });
                    // Enter takes the first match of a search.
                    if enter && !search.is_empty() && picked.is_none() {
                        if let Some((n, p)) = first {
                            picked = Some((st.clone(), n, p));
                        }
                    }
                    // What the effect under the pointer is, in a few lines.
                    ui.separator();
                    let shown = st.hover.clone().or(first_pane);
                    ui.allocate_ui(Vec2::new(ui.available_width(), 84.0), |ui| {
                        ui.set_min_height(84.0);
                        match shown.as_ref() {
                            Some((sym, names)) => crate::modelinfo::summary(ui, sym, names),
                            None => {
                                ui.label(RichText::new("Point at an effect to see what it is.").small().color(muted()));
                            }
                        }
                    });
                });
            });
        });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        open = false;
    }
    if !open || picked.is_some() {
        *state = None;
    }
    picked
}

/// A round knob: drag up/down or left/right (hold Shift for fine steps), scroll
/// over it, or double-click to return to the default. Returns (changed, send now).
fn knob(ui: &mut egui::Ui, label: &str, value: &mut f64, lo: f64, hi: f64, default: Option<f64>) -> (bool, bool) {
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
    // From the keyboard: arrows change it (Shift for fine steps), Page Up and Down in big steps, Home and End go to
    // the ends, Delete or Backspace to the default. The arrows stay with the knob while it has the focus.
    if resp.has_focus() {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(resp.id, egui::EventFilter { horizontal_arrows: true, vertical_arrows: true, ..Default::default() })
        });
        let step = |ui: &egui::Ui| if ui.input(|i| i.modifiers.shift) { span / 500.0 } else { span / 50.0 };
        let press = |ui: &mut egui::Ui, key: egui::Key| {
            ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, key) || i.consume_key(egui::Modifiers::SHIFT, key))
        };
        let s = step(ui);
        let mut moved = false;
        for key in [egui::Key::ArrowUp, egui::Key::ArrowRight] {
            if press(ui, key) {
                *value += s;
                moved = true;
            }
        }
        for key in [egui::Key::ArrowDown, egui::Key::ArrowLeft] {
            if press(ui, key) {
                *value -= s;
                moved = true;
            }
        }
        if press(ui, egui::Key::PageUp) {
            *value += span / 10.0;
            moved = true;
        }
        if press(ui, egui::Key::PageDown) {
            *value -= span / 10.0;
            moved = true;
        }
        if press(ui, egui::Key::Home) {
            *value = lo;
            moved = true;
        }
        if press(ui, egui::Key::End) {
            *value = hi;
            moved = true;
        }
        if let Some(d) = default {
            if press(ui, egui::Key::Delete) || press(ui, egui::Key::Backspace) {
                *value = d;
                moved = true;
            }
        }
        commit |= moved;
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
    p.line_segment(
        [c + Vec2::new(a.cos(), a.sin()) * (r - 15.0), c + Vec2::new(a.cos(), a.sin()) * (r - 6.0)],
        Stroke::new(2.5_f32, ink()),
    );
    crate::theme::focus_ring(ui, &resp, 24);
    let shown = *value;
    resp.widget_info(|| egui::WidgetInfo::slider(true, shown, label));
    resp.on_hover_text("Drag up/down or left/right (Shift = fine), scroll, or double-click for the default. With the keyboard: arrows (Shift = fine), Page Up/Down, Home/End, Delete for the default");
    ((*value - before).abs() > span * 1e-6, commit)
}

/// One labelled control in the knob row.
fn param_cell(ui: &mut egui::Ui, name: &str, meta: Option<&Param>, v: &Value) -> (Option<Value>, bool) {
    let mut out = (None, false);
    let salt = meta.map(|m| m.id.as_str()).unwrap_or(name);
    ui.push_id(salt, |ui| {
        ui.allocate_ui_with_layout(Vec2::new(104.0, 104.0), egui::Layout::top_down(egui::Align::Center), |ui| {
            ui.set_width(104.0);
            ui.add(egui::Label::new(RichText::new(name).small().color(muted())).truncate());
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
        .selected_text(RichText::new(&shown).color(if warn { orange() } else { ink() }))
        .width(96.0)
        .height(420.0)
        .show_ui(ui, |ui| {
            ui.set_min_width(280.0);
            let captures: Vec<&(i64, String)> = names.iter().filter(|(s, _)| crate::nam::slot_is_capture(*s) == Some(true)).collect();
            let unknown: Vec<&(i64, String)> = names.iter().filter(|(s, _)| crate::nam::slot_is_capture(*s).is_none()).collect();
            let irs: Vec<&(i64, String)> = names.iter().filter(|(s, _)| crate::nam::slot_is_capture(*s) == Some(false)).collect();
            if names.is_empty() {
                ui.label(
                    RichText::new("Connect the pedal to list its IR slots.\nOr type the slot number under the list.")
                        .small()
                        .color(muted()),
                );
            }
            let mut section = |ui: &mut egui::Ui, title: &str, list: &[&(i64, String)], col: Color32| {
                if list.is_empty() {
                    return;
                }
                ui.label(RichText::new(title).small().strong().color(muted()));
                for (s, n) in list {
                    if ui.selectable_label(*s == cur, RichText::new(format!("{s:>3}  {n}")).monospace().color(col)).clicked() {
                        picked = Some(*s);
                    }
                }
                ui.add_space(4.0);
            };
            if nam {
                section(ui, "NAM CAPTURES", &captures, pink());
                section(ui, "NOT SORTED YET", &unknown, ink());
                section(ui, "IRS (NOT CAPTURES - WON'T SOUND RIGHT)", &irs, muted());
                if ui.selectable_label(!(1..=128).contains(&cur), "<EMPTY>").clicked() {
                    picked = Some(0);
                }
            } else {
                section(ui, "IRS", &irs, ink());
                section(ui, "NOT SORTED YET", &unknown, ink());
                section(ui, "NAM CAPTURES (NOT IRS - WON'T SOUND RIGHT)", &captures, muted());
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
        ui.label(RichText::new("slot").small().color(muted()));
        if ui.add(egui::DragValue::new(&mut n).range(lo..=128).speed(0.1)).changed() {
            picked = Some(n);
        }
    });
    picked.filter(|p| *p != cur)
}

/// One knob, switch or chooser for a value of the pedal's document.
/// Returns the new value and whether it should be sent now (released or clicked).
/// A switch or one of a few fixed choices, drawn as a button in both states: filled with the accent when on, and a framed button (not bare
/// text that seems to float) when off. Returns true when clicked.
fn switch_button(ui: &mut egui::Ui, text: &str, on: bool) -> bool {
    let label = RichText::new(text).color(if on { on_accent() } else { ink() });
    let mut b = egui::Button::new(label).min_size(Vec2::new(56.0, 26.0)).corner_radius(CornerRadius::same(6));
    b = if on { b.fill(accent_fill()) } else { b.fill(card_hi()).stroke(Stroke::new(1.0_f32, line())) };
    ui.add(b).clicked()
}

fn value_widget(ui: &mut egui::Ui, meta: Option<&Param>, v: &Value) -> (Option<Value>, bool) {
    // The NAM block's capture and an IR block's IR Select: a pedal IR slot, picked by name.
    if let Some(nam) = meta.and_then(|m| match m.display.as_str() {
        "nam_slot" => Some(true),
        "ir_select" => Some(false),
        _ => None,
    }) {
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
                if switch_button(ui, &label, on) && !on {
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
                egui::ComboBox::from_id_salt("bool-choice").selected_text(shown.as_str()).width(94.0).show_ui(ui, |ui| {
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
                let (off_lbl, on_lbl) = if choices.len() >= 2 { (choices[0].as_str(), choices[1].as_str()) } else { ("Off", "On") };
                let label = if bb { on_lbl } else { off_lbl };
                ui.add_space(10.0);
                if switch_button(ui, label, bb) {
                    return (Some(Value::Bool(!bb)), true);
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
                if let Some(m) = meta {
                    dv = dv
                        .custom_formatter(|n, _| crate::catalog::format_value(Some(m), n))
                        .custom_parser(|s| crate::catalog::parse_shown(Some(m), s).map(f64::round));
                }
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
            let label = meta.map(|m| m.name.as_str()).unwrap_or("Value");
            let (mut changed, mut commit) = knob(ui, label, &mut f, lo, hi, default);
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
                    .add(egui::Label::new(RichText::new(&shown).monospace().color(ink())).sense(egui::Sense::click()))
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
            ui.label(RichText::new(other.summary()).small().color(muted()));
        }
    }
    (None, false)
}

/// What the effect list is filtered to for an FX loop (0), a send (1) or a return (2).
/// The loop numbers a Send/Return/FX Loop name mentions ("Send 3/4" -> [3, 4]).
fn loop_numbers(name: &str) -> Vec<i64> {
    name.chars().filter_map(|c| c.to_digit(10)).map(i64::from).collect()
}

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
/// 2 only the selected block's panel, 4 only a one-row strip of chips (one per block).
/// The panel is docked at the bottom.
static PART: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
pub fn set_part(p: u8) {
    PART.store(p, std::sync::atomic::Ordering::Relaxed);
}
fn chain_part() -> bool {
    let p = PART.load(std::sync::atomic::Ordering::Relaxed);
    p != 2 && p != 4
}
fn panel_part() -> bool {
    let p = PART.load(std::sync::atomic::Ordering::Relaxed);
    p != 1 && p != 4
}
fn strip_part() -> bool {
    PART.load(std::sync::atomic::Ordering::Relaxed) == 4
}

/// One tappable block in the chip strip. `sel` is what the editor's `selected` holds for this block.
pub struct Chip {
    pub sel: usize,
    pub name: String,
    pub colour: Color32,
    /// The block's column in the chain (empty slots count), used to place a lane's split point.
    pub col: usize,
}

/// One line of the strip: a lane of a path ("1A" the main lane, "1B" the lower lane after a split, "2A"...).
/// `label` is empty when the preset is a single plain lane.
pub struct ChipRow {
    pub label: String,
    pub chips: Vec<Chip>,
    /// For a lower (B) row: how many chips of the path's A row come before the split point, so the row
    /// can start under the A chip where the split happens. 0 for an A row.
    pub split_col: usize,
}

/// Group blocks given as (path index, lane, chip) into the strip's rows: 1A, 1B, 2A, 2B in that order,
/// leaving out lanes with no blocks. A lone 1A row gets no label. Chips keep their given order within a row.
/// `splits` gives, per path index, the chain column where its lane splits; a B row records how many of its
/// A row's chips sit before that column (never more than the A row has).
pub fn chip_rows(items: impl IntoIterator<Item = (usize, usize, Chip)>, splits: &[(usize, usize)]) -> Vec<ChipRow> {
    let mut by: std::collections::BTreeMap<(usize, usize), Vec<Chip>> = std::collections::BTreeMap::new();
    for (path, lane, chip) in items {
        by.entry((path, lane)).or_default().push(chip);
    }
    let lone_plain = by.len() == 1 && by.contains_key(&(0, 0));
    let split_cols: Vec<((usize, usize), usize)> = by
        .keys()
        .filter(|(_, lane)| *lane != 0)
        .map(|&(path, lane)| {
            let at = splits.iter().find(|(p, _)| *p == path).map_or(0, |s| s.1);
            let n = by.get(&(path, 0)).map_or(0, |a| a.iter().filter(|c| c.col < at).count());
            ((path, lane), n)
        })
        .collect();
    by.into_iter()
        .map(|((path, lane), chips)| ChipRow {
            label: if lone_plain { String::new() } else { format!("{}{}", path + 1, if lane == 0 { 'A' } else { 'B' }) },
            chips,
            split_col: split_cols.iter().find(|(k, _)| *k == (path, lane)).map_or(0, |x| x.1),
        })
        .collect()
}

const CHIP_TEXT: f32 = 12.0;
/// Side padding inside a chip, and the gap between chips.
const CHIP_PAD: f32 = 6.0;
const CHIP_GAP: f32 = 4.0;
/// A chip is at most this wide; a longer name is cut short with an ellipsis.
const CHIP_MAX_W: f32 = 100.0;

fn chip_font() -> egui::FontId {
    egui::FontId::proportional(CHIP_TEXT)
}

/// The name as it fits a chip: whole if it fits `CHIP_MAX_W`, else cut short with "…".
fn fit_chip_name(ui: &egui::Ui, name: &str) -> String {
    let max_text = CHIP_MAX_W - 2.0 * CHIP_PAD;
    let width = |s: &str| ui.fonts(|f| f.layout_no_wrap(s.to_owned(), chip_font(), Color32::WHITE).size().x);
    if width(name) <= max_text {
        return name.to_owned();
    }
    let chars: Vec<char> = name.chars().collect();
    for keep in (0..chars.len()).rev() {
        let cand = format!("{}…", chars[..keep].iter().collect::<String>().trim_end());
        if width(&cand) <= max_text {
            return cand;
        }
    }
    "…".to_owned()
}

/// The rows of block chips, sideways-scrolling together as one. True when a chip was tapped.
pub fn chip_strip(ui: &mut egui::Ui, rows: &[ChipRow], selected: &mut Option<usize>) -> bool {
    if rows.is_empty() {
        ui.label(RichText::new("No blocks").color(muted()));
        return false;
    }
    let mut tapped = false;
    let labelled = rows.iter().any(|r| !r.label.is_empty());
    egui::ScrollArea::horizontal().id_salt("chip-strip").auto_shrink([false, true]).show(ui, |ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            // Left x of each A row's chips (and its right end), so a B row can start under its split.
            let mut a_xs: Vec<(char, Vec<f32>, f32)> = Vec::new();
            for row in rows {
                let is_b = row.label.ends_with('B');
                let key = row.label.chars().next().unwrap_or(' ');
                let start = if is_b {
                    a_xs.iter().find(|(k, _, _)| *k == key).map(|(_, xs, end)| xs.get(row.split_col).copied().unwrap_or(*end))
                } else {
                    None
                };
                let mut xs: Vec<f32> = Vec::new();
                let mut end = 0.0_f32;
                ui.horizontal(|ui| {
                    if labelled {
                        let (rect, _) = ui.allocate_exact_size(Vec2::new(26.0, 32.0), egui::Sense::hover());
                        ui.painter().text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            &row.label,
                            egui::FontId::proportional(13.0),
                            muted(),
                        );
                    }
                    if let Some(x) = start {
                        let gap = x - ui.cursor().min.x;
                        if gap > 0.0 {
                            ui.add_space(gap);
                        }
                    }
                    ui.spacing_mut().item_spacing.x = CHIP_GAP;
                    for chip in &row.chips {
                        let shown = fit_chip_name(ui, &chip.name);
                        let text_w = ui.fonts(|f| f.layout_no_wrap(shown.clone(), chip_font(), Color32::WHITE).size().x);
                        let size = Vec2::new(text_w + 2.0 * CHIP_PAD, 32.0);
                        let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
                        let resp = resp.on_hover_text(chip.name.as_str());
                        xs.push(rect.min.x);
                        end = rect.max.x + ui.spacing().item_spacing.x;
                        let is_sel = *selected == Some(chip.sel);
                        let p = ui.painter();
                        p.rect_filled(rect, CornerRadius::same(8), chip.colour);
                        if is_sel {
                            p.rect_stroke(rect, CornerRadius::same(8), Stroke::new(2.0_f32, accent()), egui::StrokeKind::Inside);
                        } else if resp.hovered() {
                            p.rect_stroke(rect, CornerRadius::same(8), Stroke::new(1.0_f32, theme::wash(90)), egui::StrokeKind::Inside);
                        }
                        let bright = theme::is_light()
                            && (0.299 * chip.colour.r() as f32 + 0.587 * chip.colour.g() as f32 + 0.114 * chip.colour.b() as f32) > 140.0;
                        let tc = if bright { Color32::from_rgb(0x14, 0x17, 0x1c) } else { Color32::WHITE };
                        p.text(rect.center(), egui::Align2::CENTER_CENTER, &shown, chip_font(), tc);
                        if resp.clicked() {
                            *selected = Some(chip.sel);
                            tapped = true;
                        }
                    }
                });
                if !is_b {
                    a_xs.push((key, xs, end));
                }
            }
        });
    });
    tapped
}

#[cfg(test)]
mod part_tests {
    use super::*;

    #[test]
    fn part_flags() {
        set_part(4);
        assert!(!chain_part() && !panel_part() && strip_part());
        set_part(1);
        assert!(chain_part() && !panel_part() && !strip_part());
        set_part(2);
        assert!(!chain_part() && panel_part() && !strip_part());
        set_part(0);
        assert!(chain_part() && panel_part() && !strip_part());
    }

    fn chip(n: usize) -> Chip {
        Chip { sel: n, name: format!("b{n}"), colour: Color32::GRAY, col: n }
    }

    #[test]
    fn rows_follow_paths_and_lanes() {
        let rows = chip_rows([(0, 0, chip(0)), (1, 0, chip(3)), (0, 1, chip(2)), (0, 0, chip(1))], &[]);
        let labels: Vec<&str> = rows.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(labels, ["1A", "1B", "2A"]);
        assert_eq!(rows[0].chips.iter().map(|c| c.sel).collect::<Vec<_>>(), [0, 1]);
        let plain = chip_rows([(0, 0, chip(0)), (0, 0, chip(1))], &[]);
        assert_eq!(plain.len(), 1);
        assert!(plain[0].label.is_empty());
        assert!(chip_rows([], &[]).is_empty());
    }

    #[test]
    fn lower_row_knows_its_split_column() {
        let items = || [(0, 0, chip(0)), (0, 0, chip(1)), (0, 0, chip(2)), (0, 0, chip(3)), (0, 1, chip(9))];
        // Split before column 3: three A chips come first.
        assert_eq!(chip_rows(items(), &[(0, 3)])[1].split_col, 3);
        // Past the last A chip: clamps to the A count.
        assert_eq!(chip_rows(items(), &[(0, 40)])[1].split_col, 4);
        // Empty slots count as columns: chips at columns 0, 1, 5 with a split at column 4.
        let gap = [(0, 0, Chip { col: 5, ..chip(2) }), (0, 0, chip(0)), (0, 0, chip(1)), (0, 1, chip(9))];
        assert_eq!(chip_rows(gap, &[(0, 4)])[1].split_col, 2);
        assert_eq!(chip_rows(items(), &[])[1].split_col, 0);
        assert_eq!(chip_rows(items(), &[(0, 3)])[0].split_col, 0);
    }
}

/// Width of the photo-and-notes column beside a block's knobs.
const SIDE_W: f32 = 210.0;

/// The height of a block panel's body: the same for an input, an output and every effect, so the editor
/// doesn't jump as blocks are picked; an amp with a cab (or a dual cab) gets two of them.
const BODY_H: f32 = 190.0;

/// One section of a block panel: the photo tile on the left, a divider, the settings on the right, all at the
/// panel's body height. An amp with a cab has two of these, one above the other.
fn body_section(ui: &mut egui::Ui, side: bool, full: f32, h: f32, photo: Option<&str>, content: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal_top(|ui| {
        ui.set_min_height(h);
        if side {
            ui.vertical(|ui| {
                ui.set_width(SIDE_W);
                crate::modelinfo::photo_tile(ui, photo, SIDE_W, h);
            });
            ui.add_space(10.0);
            let (r, _) = ui.allocate_exact_size(Vec2::new(1.0, h), egui::Sense::hover());
            ui.painter().rect_filled(r, CornerRadius::ZERO, line());
            ui.add_space(10.0);
        }
        let rest = if side { full - SIDE_W - 21.0 } else { full };
        ui.vertical(|ui| {
            ui.set_max_width(rest);
            ui.set_min_width(rest.min(200.0));
            content(ui);
        });
    });
}
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

/// What editing a pedal preset needs: the document, whether it is the live one (changes
/// also go to the pedal as `cmds`), the effect catalog, and the unsaved-changes flag.
pub struct DeviceEdit<'a> {
    pub preset: &'a mut DevicePreset,
    pub live: bool,
    pub cmds: &'a mut Vec<Cmd>,
    pub cat: Option<&'a Catalog>,
    pub dirty: &'a mut bool,
}

/// The knobs of a block (or of its paired cab), two columns wide.
/// The vertical gap between two rows of knobs, so a row's value text does not touch the next row's names.
pub(crate) const ROW_GAP: f32 = 16.0;

fn knob_grid(ui: &mut egui::Ui, ed: DeviceEdit, s: &SlotView, sel: usize, cab: bool) {
    let DeviceEdit { preset, live, cmds, cat, dirty } = ed;
    let values = if cab { preset.paired_values(s.path, s.position) } else { s.values.clone() };
    let model = if cab { s.paired } else { s.model };
    // For splits, merges, inputs and outputs the parameter names come from the same table.
    let per_row = ((ui.available_width() + 6.0) / 116.0).floor().max(1.0) as usize;
    let mut all: Vec<(usize, &Value)> = values.iter().enumerate().collect();
    // A single cab keeps its Mic last in the pedal's list, but it is the first control of a cab block (and in HX Edit): show it first.
    if let (Some(m), Some(c)) = (model, cat) {
        if let Some(at) = all.iter().position(|(vi, _)| c.param(m, *vi).1.is_some_and(|p| p.id == "@mic")) {
            let mic = all.remove(at);
            all.insert(0, mic);
        }
    }
    for (row_no, row) in all.chunks(per_row).enumerate() {
        if row_no > 0 {
            ui.add_space(ROW_GAP);
        }
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
                let hidden = model.zip(cat).is_some_and(|(m, c)| c.is_hidden_value(m, vi));
                // A hidden value writes the pedal's edit buffer: it goes on commit only, never paced mid-drag.
                let go = live && if hidden { commit } else { pacing::live_send(ui, ("blk", s.global, cab, vi), nv.is_some(), commit) };
                if go {
                    let cur = if cab {
                        preset.paired_values(s.path, s.position).get(vi).cloned()
                    } else {
                        preset.slots().get(sel).and_then(|x| x.values.get(vi).cloned())
                    };
                    if let Some(value) = cur {
                        // Key 26 picks the block's own knobs (0) or its cab's (1). The Mic and Trails are not settable that way.
                        cmds.push(if hidden {
                            Cmd::SetHidden { block: s.wire_block(), cab, index: vi as i64, value }
                        } else {
                            Cmd::SetParam { block: s.wire_block(), path: cab as i64, index: vi as i64, value }
                        });
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
                    .custom_formatter(|n, _| crate::catalog::format_value(meta, n))
                    .custom_parser(|s| crate::catalog::parse_shown(meta, s)),
            );
            r.changed().then(|| if int { Value::int(x.round() as i64) } else { Value::F32(x as f32) })
        }
    }
}

/// What is being dragged in a pedal preset's chain: a slot.
#[derive(Clone, Copy)]
struct SlotDrag {
    path: i64,
    position: usize,
    global: usize,
}

/// What one footswitch does, for [`footswitch_strip`]: (switch number from 1, text, on now).
pub type FootswitchItem = (usize, String, Option<bool>);

/// A row of footswitch cells, "FS 1" to "FS n", each listing what it switches (an effect's
/// on/off, with its state, or a knob it moves); a free switch says so.
pub fn footswitch_strip(ui: &mut egui::Ui, count: usize, items: &[FootswitchItem]) {
    if count == 0 {
        return;
    }
    let gap = 6.0;
    let w = ((ui.available_width() - gap * (count as f32 - 1.0)) / count as f32).max(84.0);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        for n in 1..=count {
            let mine: Vec<&FootswitchItem> = items.iter().filter(|i| i.0 == n).collect();
            ui.allocate_ui(Vec2::new(w, 0.0), |ui| {
                egui::Frame::new()
                    .fill(card_bg())
                    .stroke(Stroke::new(1.0_f32, if mine.is_empty() { line() } else { orange().gamma_multiply(0.5) }))
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(egui::Margin::symmetric(8, 6))
                    .show(ui, |ui| {
                        ui.set_width(w - 18.0);
                        ui.set_min_height(48.0);
                        // (the strip itself is a row, so each cell stacks its own lines)
                        ui.vertical(|ui| {
                            ui.label(RichText::new(format!("FS {n}")).strong().color(if mine.is_empty() { muted() } else { orange() }));
                            if mine.is_empty() {
                                ui.label(RichText::new("free").small().color(muted()));
                            }
                            for (_, text, on) in mine {
                                ui.horizontal(|ui| {
                                    if let Some(on) = on {
                                        let (r, _) = ui.allocate_exact_size(Vec2::splat(8.0), egui::Sense::hover());
                                        if *on {
                                            ui.painter().circle_filled(r.center(), 3.5, good());
                                        } else {
                                            ui.painter().circle_stroke(r.center(), 3.0, Stroke::new(1.0_f32, muted()));
                                        }
                                    }
                                    ui.add(egui::Label::new(RichText::new(text).small()).truncate());
                                });
                            }
                        });
                    });
            });
        }
    });
    ui.add_space(6.0);
}
