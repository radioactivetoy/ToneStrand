//! Drawing a signal path as a diagram: input, eight positions and output on
//! the main line; when the path splits, a dot where it splits, a lower lane
//! underneath the positions it spans, and a dot where it merges again.

use eframe::egui::{self, Color32, CornerRadius, Pos2, Rect, RichText, Stroke, Vec2};

use crate::theme::*;

pub const COLS: usize = 8;
thread_local! {
    /// Block width: grows with the window so the chain uses the space it has.
    static BW: std::cell::Cell<f32> = const { std::cell::Cell::new(108.0) };
}
fn bw() -> f32 {
    BW.with(|c| c.get())
}
const IO_W: f32 = 56.0;
/// Block height.
fn bh() -> f32 {
    96.0
}
const GAP: f32 = 16.0;
const ROW_GAP: f32 = 26.0;

/// What sits in one cell of the diagram.
#[derive(Clone)]
pub enum Cell {
    Empty,
    /// `detail`: one more line under the name (e.g. the NAM capture it plays).
    /// category and symbol pick its icon.
    Block {
        title: String,
        subtitle: String,
        colour: Color32,
        on: bool,
        detail: Option<String>,
        category: String,
        symbol: String,
    },
}

/// Something the person did in the diagram.
pub enum Hit {
    /// Clicked a cell: lane (0 main, 1 lower) and column (0..8).
    Click(usize, usize),
    DoubleClick(usize, usize),
    /// Switch the block on or off (its light, or the right-click menu).
    Toggle(usize, usize),
    /// Right-click menu: choose another effect for it.
    Change(usize, usize),
    /// Right-click menu: take it out.
    Remove(usize, usize),
    /// The line before main-lane column `col`: split the path there.
    GapSplit(usize),
    /// The line before column `col`: add an effect there.
    GapAdd(usize),
    /// The line before column `col`: add an FX loop (0), a send (1) or a return (2) there.
    GapSendReturn(usize, u8),
    /// Dragged the block at (lane, col) onto (lane, col).
    Drop((usize, usize), (usize, usize)),
    Input,
    Output,
    Split,
    Merge,
    /// Dragged the split dot to sit before column `col`.
    MoveSplit(usize),
    /// Dragged the merge dot to sit before column `col`.
    MoveMerge(usize),
    /// The "+" after the lower lane: add an effect there and widen the lane.
    ExtendLower,
    /// A block dropped on that "+".
    DropExtend((usize, usize)),
}

#[derive(Clone, Copy)]
struct DragCell(usize, usize);

pub struct Chain<'a> {
    pub id: &'a str,
    pub main: Vec<Cell>,
    /// The lower lane, when the path splits: cells indexed by column (`split..lower.len()`).
    pub lower: Option<Vec<Cell>>,
    /// The split sits before column `split`, the merge before column `merge` (`cols` = before the output).
    pub split: usize,
    pub merge: usize,
    /// Which cell is selected: (lane, col), or the input/output/split/merge.
    pub selected: Selected,
    pub can_add: bool,
    pub can_extend_lower: bool,
    pub can_add_lower: bool,
    /// How many positions the main lane shows (the unit decides).
    pub cols: usize,
    /// The width the diagram may use (blocks grow to fill it).
    pub width: f32,
    /// A split can be added (the path has a free slot for the lower lane).
    pub can_split: bool,
    /// The unit's name, for the jack hints on sends and returns.
    pub unit: &'static str,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Selected {
    None,
    Cell(usize, usize),
    Input,
    Output,
    Split,
    Merge,
}

fn col_x(left: f32, col: usize) -> f32 {
    // Positions start one gap to the right of the IN box.
    left + IO_W + GAP + col as f32 * (bw() + GAP)
}

fn draw_block(ui: &egui::Ui, rect: Rect, cell: &Cell, selected: bool, hovered: bool, can_add: bool) {
    let p = ui.painter();
    match cell {
        Cell::Empty => {
            let small = Rect::from_center_size(rect.center(), Vec2::new(46.0, bh()));
            let col = if selected {
                accent()
            } else if hovered {
                muted()
            } else {
                // Visible enough to find, quiet enough not to compete with the effects.
                muted().gamma_multiply(0.4)
            };
            p.rect_filled(small, CornerRadius::same(8), bg());
            p.rect_stroke(small, CornerRadius::same(8), Stroke::new(1.0_f32, col), egui::StrokeKind::Inside);
            if can_add {
                p.text(small.center(), egui::Align2::CENTER_CENTER, "+", egui::FontId::proportional(20.0), col);
            }
        }
        Cell::Block { title, subtitle, colour, on, detail, category, symbol } => {
            let look =
                StudioBlock { title, subtitle, colour: *colour, on: *on, detail: detail.as_deref(), category, symbol, selected, hovered };
            draw_studio_block(ui, rect, look);
        }
    }
}

/// What a Studio effect box shows.
struct StudioBlock<'a> {
    title: &'a str,
    subtitle: &'a str,
    colour: Color32,
    on: bool,
    detail: Option<&'a str>,
    category: &'a str,
    symbol: &'a str,
    selected: bool,
    hovered: bool,
}

/// The effect box: a colourful gradient in the effect type's colour, its icon
/// tile with the type beside it, the name in bold at the bottom, and the on/off light.
fn draw_studio_block(ui: &egui::Ui, rect: Rect, look: StudioBlock) {
    let StudioBlock { title, subtitle, colour, on, detail, category, symbol, selected, hovered } = look;
    let p = ui.painter();
    // Sends, returns and FX loops aren't effects: they're drawn as a loop off the line.
    if category == "Send/Return" {
        return draw_loop_marker(ui, rect, title, on, selected, hovered);
    }
    let rad = 12.0;
    let r = CornerRadius::same(rad as u8);
    // A glow around the picked block, a soft ring on hover.
    if selected {
        p.rect_stroke(
            rect.expand(4.0),
            CornerRadius::same(15),
            Stroke::new(4.0_f32, accent().gamma_multiply(0.35)),
            egui::StrokeKind::Outside,
        );
    } else if hovered {
        p.rect_stroke(
            rect.expand(3.0),
            CornerRadius::same(14),
            Stroke::new(3.0_f32, colour.gamma_multiply(0.3)),
            egui::StrokeKind::Outside,
        );
    }
    // Flat, like the rest of the window: the effect type's colour (dull grey when off).
    let fill = if on { mix(colour, bg(), 0.18) } else { mix(panel(), card_hi(), 0.3) };
    p.rect_filled(rect, r, fill);
    let stroke = if selected {
        Stroke::new(2.0_f32, accent())
    } else {
        Stroke::new(1.0_f32, if on { colour.gamma_multiply(0.55) } else { crate::theme::line() })
    };
    p.rect_stroke(rect, r, stroke, egui::StrokeKind::Inside);
    // The icon tile, and the effect type (or the capture a NAM block plays) beside it.
    let icon = Rect::from_min_size(rect.left_top() + Vec2::new(9.0, 9.0), Vec2::splat(34.0));
    let mut text_x = 9.0;
    let tint = if on { Color32::WHITE } else { Color32::from_white_alpha(90) };
    let base = if crate::icons::is_amp_cab(category) { "Amp" } else { category };
    if crate::icons::for_model(ui.ctx(), base, symbol, title).is_some() {
        p.rect_filled(icon.expand(1.5), CornerRadius::same(9), Color32::from_black_alpha(if on { 90 } else { 40 }));
        crate::icons::paint_model(p, ui.ctx(), icon, category, symbol, title, tint);
        text_x = 50.0;
    }
    let side_w = (rect.width() - text_x - 22.0).max(20.0);
    let small = |s: &str, col: Color32, y: f32| {
        let mut job = egui::text::LayoutJob::simple(s.to_owned(), egui::FontId::proportional(11.0), col, side_w);
        job.wrap.max_rows = 1;
        let g = p.layout_job(job);
        p.galley(rect.left_top() + Vec2::new(text_x, y), g, col);
    };
    let bright_fill =
        on && crate::theme::is_light() && (0.299 * colour.r() as f32 + 0.587 * colour.g() as f32 + 0.114 * colour.b() as f32) > 140.0;
    let sub_col = if bright_fill {
        Color32::from_black_alpha(190)
    } else if on {
        Color32::from_white_alpha(200)
    } else {
        muted()
    };
    let wide = side_w >= 56.0;
    let pink = if on { Color32::from_rgb(255, 196, 230) } else { muted() };
    match (detail, wide) {
        (Some(d), true) => {
            small(subtitle, sub_col, 11.0);
            small(d, pink, 26.0);
        }
        (None, true) => small(subtitle, sub_col, 19.0),
        // Narrow box: the icon says the type; a NAM block's capture goes under the icon.
        (Some(d), false) => {
            let mut job = egui::text::LayoutJob::simple(d.to_owned(), egui::FontId::proportional(10.5), pink, rect.width() - 18.0);
            job.wrap.max_rows = 1;
            let g = p.layout_job(job);
            p.galley(rect.left_top() + Vec2::new(9.0, 46.0), g, pink);
        }
        (None, false) => {}
    }
    // The on/off light (clickable), glowing while on.
    let light = rect.right_top() + Vec2::new(-12.0, 18.0);
    if on {
        p.circle_filled(light, 7.5, good().gamma_multiply(0.3));
        p.circle_filled(light, 4.5, good());
    } else {
        p.circle_stroke(light, 4.5, Stroke::new(1.5_f32, Color32::from_gray(110)));
    }
    // The name, bold, at the bottom (two lines at most).
    // On a bright block (the light palette darkens colours only a little), dark text reads better.
    let bright =
        on && crate::theme::is_light() && (0.299 * colour.r() as f32 + 0.587 * colour.g() as f32 + 0.114 * colour.b() as f32) > 140.0;
    let tc = if bright {
        Color32::from_rgb(0x14, 0x17, 0x1c)
    } else if on {
        Color32::WHITE
    } else {
        muted()
    };
    // A long name steps down in size until it fits whole (the hover tip always has it in full);
    // the lane's other names follow it, so a row reads evenly.
    let rows = if detail.is_some() && !wide { 1 } else { 2 };
    let (g, _) = fit_name(p, title, tc, rect.width() - 20.0, rows, NAME_SIZE.with(|c| c.get()).unwrap_or(14.5));
    let at = Pos2::new(rect.left() + 10.0, rect.bottom() - 9.0 - g.size().y);
    // A faint shadow behind the name keeps it readable on bright colours.
    p.galley_with_override_text_color(
        at + Vec2::new(0.0, 1.0),
        g.clone(),
        if bright { Color32::from_white_alpha(110) } else { Color32::from_black_alpha(140) },
    );
    p.galley(at, g, tc);
    if !on {
        let pill = Rect::from_min_size(rect.right_top() + Vec2::new(-52.0, 11.0), Vec2::new(28.0, 15.0));
        p.rect_filled(pill, CornerRadius::same(4), card_hi());
        p.text(pill.center(), egui::Align2::CENTER_CENTER, "OFF", egui::FontId::proportional(9.5), muted());
    }
}

/// A send, return or FX loop on the chain: no effect box, the line itself leaves the chain.
/// An FX loop rises from the line, runs over a jack tag ("Loop 1") and comes back down;
/// a send rises with an arrow going out; a return comes down with an arrow coming in.
fn draw_loop_marker(ui: &egui::Ui, rect: Rect, title: &str, on: bool, selected: bool, hovered: bool) {
    let p = ui.painter();
    let col = if on { Color32::from_rgb(0x4f, 0xc9, 0xb4) } else { Color32::from_gray(95) };
    let stroke = Stroke::new(2.0_f32, col);
    let cy = rect.center().y;
    let top = rect.top() + 22.0;
    if selected || hovered {
        p.rect_filled(rect, CornerRadius::same(12), if selected { accent().gamma_multiply(0.12) } else { wash(6) });
        if selected {
            p.rect_stroke(rect, CornerRadius::same(12), Stroke::new(1.5_f32, accent()), egui::StrokeKind::Inside);
        }
    }
    let arrow = |tip: Pos2, up: bool| {
        let d = if up { -1.0 } else { 1.0 };
        p.add(egui::Shape::convex_polygon(vec![tip, tip + Vec2::new(-5.0, -7.0 * d), tip + Vec2::new(5.0, -7.0 * d)], col, Stroke::NONE));
    };
    let t = title.trim();
    // "Send 1/2" and "Loop 3/4" carry two jacks.
    let stereo = t.contains('/');
    let (kind, label) = if let Some(rest) = t.strip_prefix("FX Loop") {
        (0, format!("Loop{rest}"))
    } else if t.starts_with("Send") {
        (1, t.to_owned())
    } else {
        (2, t.to_owned())
    };
    let tag = |center: Pos2| {
        let galley = p.layout_no_wrap(label.clone(), egui::FontId::proportional(11.5), if on { ink() } else { muted() });
        let r = Rect::from_center_size(center, galley.size() + Vec2::new(14.0, 6.0));
        p.rect_filled(r, CornerRadius::same(6), bg());
        p.rect_stroke(r, CornerRadius::same(6), stroke, egui::StrokeKind::Inside);
        p.galley(r.center() - galley.size() / 2.0, galley, ink());
    };
    match kind {
        0 => {
            // A stereo loop (1/2, 3/4) is two loops side by side, one inside the other.
            let insets: &[f32] = if stereo { &[0.0, 7.0] } else { &[0.0] };
            for inset in insets {
                let (l, r) = (rect.left() + 18.0 + inset, rect.right() - 18.0 - inset);
                let top = top + inset * 0.6;
                p.line_segment([Pos2::new(l, cy), Pos2::new(l, top)], stroke);
                p.line_segment([Pos2::new(l, top), Pos2::new(r, top)], stroke);
                p.line_segment([Pos2::new(r, top), Pos2::new(r, cy)], stroke);
                arrow(Pos2::new(l, top + 14.0), true);
                arrow(Pos2::new(r, cy - 10.0), false);
            }
            tag(Pos2::new(rect.center().x, top));
            // The dry line under the loop is cut: everything goes out and back. A loop that is
            // off passes the signal straight through, so the line stays.
            if on {
                p.line_segment([Pos2::new(rect.left() + 20.0, cy), Pos2::new(rect.right() - 20.0, cy)], Stroke::new(4.0_f32, bg()));
            }
        }
        1 => {
            // A stereo send leaves on two lines.
            let xs: &[f32] = if stereo { &[-6.0, 6.0] } else { &[0.0] };
            for dx in xs {
                let x = rect.center().x + dx;
                p.line_segment([Pos2::new(x, cy), Pos2::new(x, top + 10.0)], stroke);
                arrow(Pos2::new(x, top + 8.0), true);
                p.circle_filled(Pos2::new(x, cy), 4.0, col);
            }
            tag(Pos2::new(rect.center().x, top - 4.0));
        }
        _ => {
            let xs: &[f32] = if stereo { &[-6.0, 6.0] } else { &[0.0] };
            for dx in xs {
                let x = rect.center().x + dx;
                p.line_segment([Pos2::new(x, top + 4.0), Pos2::new(x, cy - 6.0)], stroke);
                arrow(Pos2::new(x, cy - 4.0), false);
                p.circle_filled(Pos2::new(x, cy), 4.0, col);
            }
            tag(Pos2::new(rect.center().x, top - 4.0));
        }
    }
    // The on/off light where the effect boxes have it.
    let light = rect.right_top() + Vec2::new(-12.0, 18.0);
    if on {
        p.circle_filled(light, 4.0, good());
    } else {
        p.circle_stroke(light, 4.0, Stroke::new(1.5_f32, Color32::from_gray(110)));
        p.text(rect.center_bottom() + Vec2::new(0.0, -10.0), egui::Align2::CENTER_BOTTOM, "OFF", egui::FontId::proportional(10.0), muted());
    }
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// Radius of the round IN / OUT terminals.
const TERMINAL_R: f32 = 21.0;

fn io_box(ui: &egui::Ui, rect: Rect, text: &str, selected: bool, hovered: bool) {
    let p = ui.painter();
    // A small round terminal; click it for the input or output settings.
    let c = rect.center();
    p.circle_filled(c, TERMINAL_R, if hovered { card_hi() } else { panel() });
    p.circle_stroke(
        c,
        TERMINAL_R,
        Stroke::new(if selected { 2.0_f32 } else { 1.0_f32 }, if selected { accent() } else { crate::theme::line() }),
    );
    p.text(c, egui::Align2::CENTER_CENTER, text, egui::FontId::monospace(11.5), if hovered || selected { ink() } else { muted() });
}

impl Chain<'_> {
    pub fn show(&self, ui: &mut egui::Ui) -> Vec<Hit> {
        let cols = self.cols;
        let lower_end = self.lower.as_ref().map(|l| l.len().max(self.split)).unwrap_or(0);
        let total_cols = cols.max(lower_end);
        let mut hits = Vec::new();
        let rows = if self.lower.is_some() { 2 } else { 1 };
        // Blocks share the visible width (inside a sideways scroll area, the clip is what's seen).
        let view = self.width;
        let fit = (view - 8.0 - 2.0 * IO_W - (total_cols as f32 + 1.0) * GAP) / total_cols.max(1) as f32;
        BW.with(|c| c.set(fit.clamp(86.0, 170.0)));
        let width = 2.0 * IO_W + (total_cols as f32 + 1.0) * GAP + total_cols as f32 * bw();
        let height = rows as f32 * bh() + (rows - 1) as f32 * ROW_GAP + 8.0;
        let (area, _) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::hover());
        let left = area.left();
        let top = area.top() + 4.0;
        let main_y = top + bh() / 2.0;
        let low_y = top + bh() + ROW_GAP + bh() / 2.0;
        let mid_y = (main_y + low_y) / 2.0;
        let line = Stroke::new(2.0_f32, muted());
        let in_rect = Rect::from_min_size(Pos2::new(left, top), Vec2::new(IO_W, bh()));
        let out_rect = Rect::from_min_size(Pos2::new(col_x(left, total_cols), top), Vec2::new(IO_W, bh()));

        // The main line, cleanly connecting the right edge of IN to the left edge of OUT.
        let p = ui.painter().clone();
        let (line_from, line_to) = (in_rect.center().x + TERMINAL_R, out_rect.center().x - TERMINAL_R);
        p.line_segment([Pos2::new(line_from, main_y), Pos2::new(line_to, main_y)], line);

        // The lower lane and its connections.
        let merge_visual_col = if self.merge >= cols { total_cols } else { self.merge };
        let splay = if merge_visual_col == self.split { 5.0 } else { 0.0 };
        let lx_start = col_x(left, self.split) - GAP / 2.0;
        let lx_end = col_x(left, lower_end) - GAP / 2.0;
        let sx = lx_start - splay;
        let mx = col_x(left, merge_visual_col) - GAP / 2.0 + splay;
        if self.lower.is_some() {
            if (sx - lx_start).abs() < 1.0 {
                p.line_segment([Pos2::new(sx, main_y), Pos2::new(sx, low_y)], line);
            } else {
                p.line_segment([Pos2::new(sx, main_y), Pos2::new(sx, mid_y)], line);
                p.line_segment([Pos2::new(sx, mid_y), Pos2::new(lx_start, mid_y)], line);
                p.line_segment([Pos2::new(lx_start, mid_y), Pos2::new(lx_start, low_y)], line);
            }
            if lx_end <= mx + 1.0 {
                p.line_segment([Pos2::new(lx_start, low_y), Pos2::new(mx, low_y)], line);
                p.line_segment([Pos2::new(mx, low_y), Pos2::new(mx, main_y)], line);
            } else {
                p.line_segment([Pos2::new(lx_start, low_y), Pos2::new(lx_end, low_y)], line);
                p.line_segment([Pos2::new(lx_end, low_y), Pos2::new(lx_end, mid_y)], line);
                p.line_segment([Pos2::new(lx_end, mid_y), Pos2::new(mx, mid_y)], line);
                p.line_segment([Pos2::new(mx, mid_y), Pos2::new(mx, main_y)], line);
            }
        }

        let cell_rect = |lane: usize, col: usize| {
            let y = if lane == 0 { main_y } else { low_y };
            Rect::from_center_size(Pos2::new(col_x(left, col) + bw() / 2.0, y), Vec2::new(bw(), bh()))
        };

        // Input and output.
        let r = ui.interact(in_rect, ui.id().with((self.id, "in")), egui::Sense::click());
        io_box(ui, in_rect, "IN", self.selected == Selected::Input, r.hovered());
        crate::theme::focus_ring(ui, &r, 20);
        r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Input of this path"));
        if r.clicked() {
            hits.push(Hit::Input);
        }
        let r = ui.interact(out_rect, ui.id().with((self.id, "out")), egui::Sense::click());
        io_box(ui, out_rect, "OUT", self.selected == Selected::Output, r.hovered());
        crate::theme::focus_ring(ui, &r, 20);
        r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Output of this path"));
        if r.clicked() {
            hits.push(Hit::Output);
        }

        // The line between two places on the main lane: click it to split there, add an effect,
        // add a send/return, or move the split or merge there. (Registered before the blocks,
        // so a block always wins where they touch.)
        if self.can_add {
            for c in 0..=cols {
                let x = if c == cols && cols < total_cols { col_x(left, cols) - GAP / 2.0 } else { col_x(left, c) - GAP / 2.0 };
                let rect = Rect::from_center_size(Pos2::new(x, main_y), Vec2::new(GAP + 10.0, 26.0));
                let pid = ui.id().with((self.id, "gap", c));
                let r = ui.interact(rect, pid.with("hit"), egui::Sense::click());
                let open = ui.memory(|m| m.is_popup_open(pid));
                if r.hovered() || r.has_focus() || open {
                    p.circle_filled(rect.center(), 8.0, accent_fill());
                    p.text(rect.center(), egui::Align2::CENTER_CENTER, "+", egui::FontId::proportional(13.0), on_accent());
                }
                if r.clicked() {
                    ui.memory_mut(|m| m.toggle_popup(pid));
                }
                r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Add or split here"));
                let r = r.on_hover_text("Click to split here, add an effect or a send/return");
                egui::popup_below_widget(ui, pid, &r, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
                    ui.set_min_width(230.0);
                    let mut chosen = None;
                    if self.lower.is_none() {
                        let b = ui.add_enabled(self.can_split, egui::Button::new("Split here (a parallel lane)"));
                        if b.on_disabled_hover_text("No free slot left on this path for a parallel lane").clicked() {
                            chosen = Some(Hit::GapSplit(c));
                        }
                    } else {
                        if ui.button("Move the split here").clicked() {
                            chosen = Some(Hit::MoveSplit(c));
                        }
                        if ui.button("Move the merge here").clicked() {
                            chosen = Some(Hit::MoveMerge(c));
                        }
                    }
                    if ui.button("Add an effect here...").clicked() {
                        chosen = Some(Hit::GapAdd(c));
                    }
                    ui.separator();
                    if ui
                        .button("Add an FX loop here (send + return)...")
                        .on_hover_text("One block that sends out and returns: the usual way to add pedals in a loop")
                        .clicked()
                    {
                        chosen = Some(Hit::GapSendReturn(c, 0));
                    }
                    if ui.button("Add a send here...").clicked() {
                        chosen = Some(Hit::GapSendReturn(c, 1));
                    }
                    if ui.button("Add a return here...").on_hover_text("Put it after its send").clicked() {
                        chosen = Some(Hit::GapSendReturn(c, 2));
                    }
                    if let Some(h) = chosen {
                        hits.push(h);
                        ui.memory_mut(|m| m.close_popup());
                    }
                });
            }
        }

        // Cells of both lanes.
        let mut lanes: Vec<(usize, &Vec<Cell>)> = vec![(0, &self.main)];
        if let Some(l) = &self.lower {
            lanes.push((1, l));
        }
        for (lane, cells) in lanes {
            // One name size for the whole lane, so a row of effects reads evenly.
            let lane_size = cells
                .iter()
                .filter_map(|c| match c {
                    Cell::Block { title, category, .. } if category != "Send/Return" => {
                        Some(fit_name(ui.painter(), title, ink(), bw() - 20.0, 2, 14.5).1)
                    }
                    _ => None,
                })
                .fold(14.5_f32, f32::min)
                // A floor: short names stay readable; only a long one goes smaller on its own.
                .max(12.5);
            NAME_SIZE.with(|c| c.set(Some(lane_size)));
            let range = if lane == 0 { 0..cols } else { self.split..lower_end };
            // The lower lane shows its effects and one free place after them; past that the
            // line runs on to the merge without empty boxes.
            let lower_last = if lane == 1 {
                cells.iter().rposition(|c| matches!(c, Cell::Block { .. })).map(|i| i + 1).unwrap_or(self.split).max(self.split)
            } else {
                usize::MAX
            };
            for col in range {
                let cell = cells.get(col).cloned().unwrap_or(Cell::Empty);
                if lane == 1 && col > lower_last && matches!(cell, Cell::Empty) {
                    continue;
                }
                let rect = cell_rect(lane, col);
                let is_block = matches!(cell, Cell::Block { .. });
                let sense = if is_block { egui::Sense::click_and_drag() } else { egui::Sense::click() };
                let r = ui.interact(rect, ui.id().with((self.id, lane, col)), sense);
                draw_block(ui, rect, &cell, self.selected == Selected::Cell(lane, col), r.hovered(), self.can_add);
                crate::theme::focus_ring(ui, &r, 12);
                r.widget_info(|| {
                    let text = match &cell {
                        Cell::Block { title, subtitle, on, .. } => format!("{title}, {subtitle}, {}", if *on { "on" } else { "off" }),
                        Cell::Empty => "Empty place, press Enter to add an effect".to_owned(),
                    };
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, true, text)
                });
                if is_block {
                    r.dnd_set_drag_payload(DragCell(lane, col));
                }
                if r.dnd_hover_payload::<DragCell>().is_some() {
                    ui.painter().rect_stroke(
                        rect.expand(2.0),
                        CornerRadius::same(11),
                        Stroke::new(2.0_f32, orange()),
                        egui::StrokeKind::Outside,
                    );
                }
                if let Some(from) = r.dnd_release_payload::<DragCell>() {
                    if (from.0, from.1) != (lane, col) {
                        hits.push(Hit::Drop((from.0, from.1), (lane, col)));
                    }
                }
                // The light in the corner switches the block on or off.
                let mut light_hit = false;
                if let Cell::Block { on, .. } = &cell {
                    let light = Rect::from_center_size(rect.right_top() + Vec2::new(-12.0, 18.0), Vec2::splat(18.0));
                    let lr = ui.interact(light, ui.id().with((self.id, lane, col, "light")), egui::Sense::click());
                    crate::theme::focus_ring(ui, &lr, 9);
                    lr.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, *on, "Block on"));
                    if lr.hovered() {
                        ui.painter().circle_stroke(light.center(), 7.0, Stroke::new(1.5_f32, ink()));
                    }
                    if lr.clicked() {
                        light_hit = true;
                        hits.push(Hit::Toggle(lane, col));
                    }
                    lr.on_hover_text(if *on { "On - click to switch off" } else { "Off - click to switch on" });
                }
                if light_hit {
                } else if r.double_clicked() {
                    hits.push(Hit::DoubleClick(lane, col));
                } else if r.clicked() {
                    hits.push(Hit::Click(lane, col));
                }
                // A right-click selects the block too, so its knobs show beside the menu that opens.
                if r.secondary_clicked() && matches!(&cell, Cell::Block { .. }) {
                    hits.push(Hit::Click(lane, col));
                }
                r.context_menu(|ui| {
                    if let Cell::Block { on, .. } = &cell {
                        if ui.button(if *on { "Switch off" } else { "Switch on" }).clicked() {
                            hits.push(Hit::Toggle(lane, col));
                            ui.close_menu();
                        }
                        if self.can_add && ui.button("Change effect...").clicked() {
                            hits.push(Hit::Change(lane, col));
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.button(RichText::new("Remove").color(bad())).clicked() {
                            hits.push(Hit::Remove(lane, col));
                            ui.close_menu();
                        }
                    } else if self.can_add {
                        if ui.button("Add effect...").clicked() {
                            hits.push(Hit::Click(lane, col));
                            ui.close_menu();
                        }
                    } else {
                        ui.label("Empty position");
                    }
                });
                if let Cell::Block { title, .. } = &cell {
                    let jacks = loop_jacks(self.unit, title).map(|j| format!("\n{j}")).unwrap_or_default();
                    r.on_hover_text(format!("{title}{jacks}\nClick to edit · light or Space: on/off · right-click for more · drag to move\nSelected: Delete removes, Enter changes the effect, arrows pick the next block"));
                } else if self.can_add {
                    r.on_hover_text("Empty - click to add an effect, or drop a block here");
                }
            }
        }

        // A return before its own send can't work: say so under it.
        // Both lanes count: the lower lane's columns line up with the main lane's.
        let named: Vec<(usize, usize, String)> = [(0usize, Some(&self.main)), (1, self.lower.as_ref())]
            .into_iter()
            .flat_map(|(lane, cells)| {
                cells.into_iter().flat_map(move |cells| {
                    cells.iter().enumerate().filter_map(move |(c, cell)| match cell {
                        Cell::Block { title, category, .. } if category == "Send/Return" => Some((lane, c, title.trim().to_owned())),
                        _ => None,
                    })
                })
            })
            .collect();
        for (lane, c, t) in &named {
            if let Some(n) = t.strip_prefix("Return") {
                let send = format!("Send{n}");
                if named.iter().any(|(_, c2, t2)| c2 > c && *t2 == send) {
                    let r = cell_rect(*lane, *c);
                    p.text(
                        r.center_bottom() + Vec2::new(0.0, 4.0),
                        egui::Align2::CENTER_TOP,
                        "before its send",
                        egui::FontId::proportional(10.5),
                        bad(),
                    );
                }
            }
        }

        // Keys for the selected block: Space on/off, Delete removes, Enter changes it,
        // arrows pick the next block.
        if let Selected::Cell(lane, col) = self.selected {
            let cells = if lane == 0 { Some(&self.main) } else { self.lower.as_ref() };
            let is_block = |l: usize, c: usize| {
                let cs = if l == 0 { Some(&self.main) } else { self.lower.as_ref() };
                cs.and_then(|cs| cs.get(c)).is_some_and(|x| matches!(x, Cell::Block { .. }))
            };
            if cells.and_then(|c| c.get(col)).is_some_and(|c| matches!(c, Cell::Block { .. })) && !ui.ctx().wants_keyboard_input() {
                let (space, del, enter, left, right, up, down) = ui.input(|i| {
                    (
                        i.key_pressed(egui::Key::Space),
                        i.key_pressed(egui::Key::Delete),
                        i.key_pressed(egui::Key::Enter),
                        i.key_pressed(egui::Key::ArrowLeft),
                        i.key_pressed(egui::Key::ArrowRight),
                        i.key_pressed(egui::Key::ArrowUp),
                        i.key_pressed(egui::Key::ArrowDown),
                    )
                });
                if space {
                    hits.push(Hit::Toggle(lane, col));
                }
                if del {
                    hits.push(Hit::Remove(lane, col));
                }
                if enter && self.can_add {
                    hits.push(Hit::Change(lane, col));
                }
                let width = total_cols.max(cols);
                if left {
                    if let Some(c) = (0..col).rev().find(|c| is_block(lane, *c)) {
                        hits.push(Hit::Click(lane, c));
                    }
                }
                if right {
                    if let Some(c) = (col + 1..width).find(|c| is_block(lane, *c)) {
                        hits.push(Hit::Click(lane, c));
                    }
                }
                if (up && lane == 1) || (down && lane == 0 && self.lower.is_some()) {
                    let other = 1 - lane;
                    let near = (0..width).filter(|c| is_block(other, *c)).min_by_key(|c| (*c as i64 - col as i64).abs());
                    if let Some(c) = near {
                        hits.push(Hit::Click(other, c));
                    }
                }
            }
        }

        // One more "+" just after the lower lane, so it can hold more effects.
        if self.lower.is_some() && self.can_extend_lower {
            let ex_x = if lower_end < total_cols { col_x(left, lower_end) + bw() / 2.0 } else { col_x(left, lower_end) + IO_W / 2.0 };
            let rect = Rect::from_center_size(Pos2::new(ex_x, low_y), Vec2::new(46.0, bh() - 16.0));
            let r = ui.interact(rect, ui.id().with((self.id, "extend")), egui::Sense::click());
            let col = if r.hovered() || r.dnd_hover_payload::<DragCell>().is_some() { accent() } else { crate::theme::line() };
            ui.painter().rect_filled(rect, CornerRadius::same(8), bg());
            ui.painter().rect_stroke(rect, CornerRadius::same(8), Stroke::new(1.0_f32, col), egui::StrokeKind::Inside);
            if self.can_add_lower {
                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "+", egui::FontId::proportional(18.0), col);
            }
            if let Some(from) = r.dnd_release_payload::<DragCell>() {
                hits.push(Hit::DropExtend((from.0, from.1)));
            }
            let tip = if self.can_add_lower {
                "Add another effect to the parallel lane (or drop a block here)"
            } else {
                "Drop a main-lane block here to move it onto the parallel lane"
            };
            if r.on_hover_text(tip).clicked() && self.can_add_lower {
                hits.push(Hit::ExtendLower);
            }
        }

        // The split and merge dots, drawn last so they sit on the lines.
        if self.lower.is_some() {
            for (is_split, x, sel) in [(true, sx, Selected::Split), (false, mx, Selected::Merge)] {
                let c = Pos2::new(x, main_y);
                let rect = Rect::from_center_size(c, Vec2::splat(22.0));
                let id = ui.id().with((self.id, if is_split { "split" } else { "merge" }));
                let r = ui.interact(rect, id, egui::Sense::click_and_drag());
                let active = self.selected == sel || r.hovered() || r.dragged();
                ui.painter().circle_filled(
                    c,
                    if active { 8.0 } else { 6.5 },
                    if self.selected == sel || r.dragged() { accent() } else { muted() },
                );
                ui.painter().circle_stroke(c, 7.5, Stroke::new(2.0_f32, bg()));
                if r.dragged() {
                    if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                        let range = if is_split { 0..=cols.saturating_sub(1) } else { self.split..=cols };
                        let x_of = |col: usize| {
                            let vcol = if !is_split && col >= cols { total_cols } else { col };
                            col_x(left, vcol) - GAP / 2.0
                        };
                        if let Some(best) = range.min_by(|&a, &b| (x_of(a) - pos.x).abs().total_cmp(&(x_of(b) - pos.x).abs())) {
                            let target_c = Pos2::new(x_of(best), main_y);
                            ui.painter().circle_stroke(target_c, 9.5, Stroke::new(1.5_f32, accent()));
                            let cur = if is_split { self.split } else { self.merge };
                            if best != cur {
                                hits.push(if is_split { Hit::MoveSplit(best) } else { Hit::MoveMerge(best) });
                            }
                        }
                    }
                }
                let tip = if is_split {
                    "Split: click for settings, or drag left/right to move"
                } else {
                    "Merge: click for settings, or drag left/right to move"
                };
                let r = r.on_hover_text(tip);
                if r.clicked() || r.drag_started() {
                    hits.push(if is_split { Hit::Split } else { Hit::Merge });
                }
            }
        }
        hits
    }
}

/// Name sizes tried in turn, largest first; the smallest may use a third line.
const NAME_SIZES: [(f32, usize); 5] = [(14.5, 0), (13.5, 0), (12.5, 0), (11.5, 0), (10.5, 1)];

thread_local! {
    /// The size the effect names of the lane being drawn share (set by `Chain::show`).
    static NAME_SIZE: std::cell::Cell<Option<f32>> = const { std::cell::Cell::new(None) };
}

/// Lay out an effect's name at the largest size (no bigger than `max_size`) at which it fits
/// whole in `rows` lines (the smallest size gets one more when `rows` is 2). Returns the text
/// and the size used.
fn fit_name(
    p: &egui::Painter,
    title: &str,
    colour: Color32,
    width: f32,
    rows: usize,
    max_size: f32,
) -> (std::sync::Arc<egui::Galley>, f32) {
    let mut last = None;
    for (size, extra) in NAME_SIZES.into_iter().filter(|(s, _)| *s <= max_size) {
        let mut job = egui::text::LayoutJob::simple(title.to_owned(), egui::FontId::proportional(size), colour, width);
        job.wrap.max_rows = rows + if rows == 2 { extra } else { 0 };
        job.wrap.break_anywhere = false;
        let laid = p.layout_job(job);
        let fits = !laid.elided;
        last = Some((laid, size));
        if fits {
            break;
        }
    }
    last.expect("at least one size was tried")
}

/// Where a Send, Return or FX Loop block's signal goes on the unit's jacks, from the
/// units' rear and side panels. None for a unit whose panel isn't described here.
pub fn loop_jacks(unit: &str, title: &str) -> Option<String> {
    let t = title.trim();
    let is_send = t.starts_with("Send");
    let is_return = t.starts_with("Return");
    let is_loop = t.starts_with("FX Loop");
    if !(is_send || is_return || is_loop) {
        return None;
    }
    let nums: Vec<char> = t.chars().filter(char::is_ascii_digit).collect();
    let pair = nums.len() > 1;
    let n = nums.iter().map(char::to_string).collect::<Vec<_>>().join(" and ");
    let what = if is_send {
        "send"
    } else if is_return {
        "return"
    } else {
        "send and return"
    };
    match unit {
        "Helix Floor" | "Helix Rack" => {
            let kind = if pair { "a stereo pair: two adjacent jack pairs" } else { "one jack pair" };
            Some(format!("Jacks: SENDS/RETURNS {n} on the rear panel ({what}; {kind})"))
        }
        "Helix LT" => Some(format!(
            "Jacks: {} {n} on the rear panel{}",
            if is_return {
                "RETURN"
            } else if is_send {
                "SEND"
            } else {
                "SEND and RETURN"
            },
            if pair { " (the stereo pair)" } else { "" }
        )),
        "HX Stomp" => Some("Jacks: the stereo SEND jack and the RETURN / AUX IN jacks (L/MONO, RIGHT) on the sides".to_owned()),
        "HX Stomp XL" => Some("Jacks: the stereo SEND jack and the RETURN / AUX IN jacks (L/MONO, RIGHT) on the rear panel".to_owned()),
        "HX Effects" if nums.iter().all(|c| matches!(c, '1' | '2')) => {
            Some(format!("Jacks: the SEND and RETURN jacks of row {n} (marked 1 and 2) on the rear panel"))
        }
        "HX Effects" => Some("Jacks: the rear panel marks SEND / RETURN rows 1 and 2 only".to_owned()),
        _ => None,
    }
}

#[cfg(test)]
mod jack_tests {
    use super::loop_jacks;

    #[test]
    fn jack_hints_follow_the_units_panels() {
        assert!(loop_jacks("Helix Floor", "Send 3").unwrap().contains("SENDS/RETURNS 3"));
        assert!(loop_jacks("Helix Rack", "Return 3/4").unwrap().contains("3 and 4"));
        assert!(loop_jacks("Helix LT", "Return 2").unwrap().contains("RETURN 2"));
        assert!(loop_jacks("HX Stomp", "FX Loop 1").unwrap().contains("SEND"));
        assert!(loop_jacks("HX Effects", "Send 2").unwrap().contains("row 2"));
        assert!(loop_jacks("HX Stomp XL", "Return 1").unwrap().contains("rear"));
        assert!(loop_jacks("Helix LT", "Delay").is_none());
    }
}
