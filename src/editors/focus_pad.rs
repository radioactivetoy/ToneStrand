//! Focus mode: an X-Y pad that steers several parameters of a block at once, next to the normal knobs.
//!
//! The five named tones of a model (the centre and the four corners) come from `tonestrand_core::focus`. Dragging the puck
//! evaluates the blend and writes the parameters into the preset, so the knob grid, drawn after the pad in the same frame, turns
//! with it. The logic that is not drawing (pad position and x,y, value conversion, send pacing, "edited by hand") is in small
//! functions at the top, with tests.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, OnceLock};

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use tonestrand_core::focus::{self, FocusModel, FocusSet, FocusValue, Kind as FocusKind, RegionKey};

use crate::device::Cmd;
use crate::devpreset::{DevicePreset, SlotView};
use crate::msgpack::Value;
use crate::theme::{self, *};

/// The pad's widest size and its share of the panel's width.
const PAD_MAX_W: f32 = 340.0;
const PAD_SHARE: f32 = 0.30;
/// The pad's height as a part of its width.
const PAD_ASPECT: f32 = 0.85;
/// The rows under the pad (readout, then Centre and Revert) with their gaps.
const BELOW_PAD: f32 = 16.0 + 26.0 + 12.0;
/// A tone name this close (in points) to the puck fades, so the puck never hides it.
const FADE_RADIUS: f32 = 40.0;
/// How faded a name is while the puck is near it.
const FADE_ALPHA: f32 = 0.35;
/// Darkening laid over the photo so it reads as a texture behind the controls.
const PHOTO_DIM: u8 = 120;
/// The most a photo is enlarged (1.0 = its own pixels).
const PHOTO_MAX_SCALE: f32 = 1.0;
/// How far (in points) a photo's edge fades into the card.
const PHOTO_FEATHER: f32 = 22.0;
/// Space between the pad's edge and its corner tones, so the dots and the puck stay inside.
const INSET: f32 = 34.0;
/// A click this close to a tone (in points) lands exactly on it.
const SNAP_RADIUS: f32 = 22.0;
/// The least time between two live sends while dragging, in seconds.
const FLUSH_STEP: f64 = 0.05;
/// Switch-like parameters (on/off, snapping choices) are sent while dragging too when true; when false only on release.
/// Off for now: they can click; to be decided by listening on the pedal (a later phase).
const SEND_SWITCHES_WHILE_DRAGGING: bool = false;

// ---------------------------------------------------------------- pure parts

/// The pad's width in a panel `full` points wide.
pub fn pad_width(full: f32) -> f32 {
    (full * PAD_SHARE).min(PAD_MAX_W).max(220.0)
}

/// The size of the pad for a width.
fn pad_size(w: f32) -> Vec2 {
    Vec2::new(w, (w * PAD_ASPECT).round())
}

/// The height of a block panel's body while it shows the pad: the pad, the rows under it and a margin.
pub fn panel_height(w: f32) -> f32 {
    pad_size(w).y + BELOW_PAD + 6.0
}

/// Whether a tone's name (its rectangle) is faded because the puck is close to it.
fn label_faded(puck: Option<Pos2>, label: Rect, radius: f32) -> bool {
    puck.is_some_and(|p| label.distance_to_pos(p) < radius)
}

/// Opacity of a photo at `dist` points from its nearest edge: 0 at the edge, 1 from `width` inward, smooth in between.
fn feather(dist: f32, width: f32) -> f32 {
    let t = (dist / width.max(1e-3)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The photo as a mesh whose edges fade to nothing over `PHOTO_FEATHER`, with the dimming in the vertex colours (so the dim
/// fades too and no rectangle is left): 4x4 vertices, 9 quads.
fn photo_mesh(tex: &egui::TextureHandle, img: Rect) -> egui::Mesh {
    let f = PHOTO_FEATHER.min(img.width() / 2.0).min(img.height() / 2.0);
    let xs = [0.0, f, img.width() - f, img.width()];
    let ys = [0.0, f, img.height() - f, img.height()];
    let k = 1.0 - f32::from(PHOTO_DIM) / 255.0;
    let mut mesh = egui::Mesh::with_texture(tex.id());
    for y in ys {
        for x in xs {
            let a = feather(x.min(img.width() - x), f) * feather(y.min(img.height() - y), f);
            let c = (255.0 * k * a).round() as u8;
            let color = Color32::from_rgba_premultiplied(c, c, c, (255.0 * a).round() as u8);
            let uv = Pos2::new(x / img.width(), y / img.height());
            mesh.vertices.push(egui::epaint::Vertex { pos: img.min + Vec2::new(x, y), uv, color });
        }
    }
    for r in 0..3u32 {
        for c in 0..3u32 {
            let i = r * 4 + c;
            mesh.indices.extend_from_slice(&[i, i + 1, i + 5, i, i + 5, i + 4]);
        }
    }
    mesh
}

/// Where a whole picture of `tex` size goes in `rect`: centred at its own aspect ratio, never cropped or stretched, scaled down to fit
/// but never up past `PHOTO_MAX_SCALE` (so small art stays crisp).
fn contain_rect(tex: Vec2, rect: Rect) -> Rect {
    let (tw, th) = (tex.x.max(1.0), tex.y.max(1.0));
    let scale = (rect.width() / tw).min(rect.height() / th).min(PHOTO_MAX_SCALE);
    Rect::from_center_size(rect.center(), Vec2::new(tw * scale, th * scale))
}

/// The square the puck moves in: the pad without its border.
fn inner_rect(pad: Rect) -> Rect {
    pad.shrink(INSET)
}

/// Pad position to (x, y) in [-1, 1] (y up), clamped.
fn xy_at(inner: Rect, p: Pos2) -> (f64, f64) {
    let x = ((p.x - inner.left()) / inner.width().max(1.0)) as f64 * 2.0 - 1.0;
    let y = 1.0 - ((p.y - inner.top()) / inner.height().max(1.0)) as f64 * 2.0;
    (x.clamp(-1.0, 1.0), y.clamp(-1.0, 1.0))
}

/// (x, y) to a position on the pad.
fn pos_of(inner: Rect, x: f64, y: f64) -> Pos2 {
    Pos2::new(inner.left() + (x as f32 + 1.0) / 2.0 * inner.width(), inner.top() + (1.0 - y as f32) / 2.0 * inner.height())
}

/// The tone within `radius` points of `p`, if any: its exact (x, y).
fn snap_to_tone(inner: Rect, p: Pos2, radius: f32) -> Option<(f64, f64)> {
    RegionKey::ALL
        .iter()
        .map(|k| k.at())
        .map(|(x, y)| (x, y, pos_of(inner, x, y).distance(p)))
        .filter(|t| t.2 <= radius)
        .min_by(|a, b| a.2.total_cmp(&b.2))
        .map(|(x, y, _)| (x, y))
}

/// The tone that counts most at (x, y) (the centre wins a tie).
fn nearest_tone(x: f64, y: f64) -> RegionKey {
    let w = focus::weights(x, y);
    let best = (0..5).fold(0, |b, k| if w[k] > w[b] { k } else { b });
    RegionKey::ALL[best]
}

/// The value to store for a Focus result, in the type the parameter already has.
fn to_value(old: &Value, fv: FocusValue) -> Value {
    let num = match fv {
        FocusValue::Float(f) => f,
        FocusValue::Int(i) => i as f64,
        FocusValue::Bool(b) => f64::from(b),
    };
    match old {
        Value::Bool(_) => Value::Bool(num != 0.0),
        Value::Int(..) => Value::int(num.round() as i64),
        _ => Value::F32(num as f32),
    }
}

/// Whether two stored values are the same for our purposes.
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Bool(x), Value::Bool(y)) => x == y,
        _ => matches!((a.as_f64(), b.as_f64()), (Some(x), Some(y)) if (x - y).abs() < 1e-6),
    }
}

/// True when a value Focus last set has been changed since (a knob turned by hand).
fn edited_by_hand(applied: &[(usize, Value)], current: &[Value]) -> bool {
    applied.iter().any(|(i, v)| current.get(*i).is_some_and(|c| !same(c, v)))
}

/// Whether a live send is due: always at the end of a gesture, otherwise when `FLUSH_STEP` has passed since the last one.
fn should_flush(last: Option<f64>, now: f64, ended: bool) -> bool {
    ended || last.is_none_or(|t| now - t >= FLUSH_STEP)
}

/// Take the values to send from the pending map (newest value per parameter). Switch-like ones (`deferred`) stay unless
/// `with_deferred`.
fn take_flush(pending: &mut BTreeMap<usize, Value>, deferred: impl Fn(usize) -> bool, with_deferred: bool) -> Vec<(usize, Value)> {
    let keys: Vec<usize> = pending.keys().copied().filter(|k| with_deferred || !deferred(*k)).collect();
    keys.into_iter().filter_map(|k| pending.remove(&k).map(|v| (k, v))).collect()
}

// ---------------------------------------------------------------- availability and mode memory

fn set() -> Option<&'static FocusSet> {
    static SET: OnceLock<Option<FocusSet>> = OnceLock::new();
    SET.get_or_init(focus::load_builtin).as_ref()
}

/// The model's Focus tones, when it has any (cabs, IRs and the standard EQs have none).
pub fn model_for(symbol: &str) -> Option<&'static FocusModel> {
    set()?.model_for(symbol)
}

fn modes() -> &'static Mutex<HashMap<String, bool>> {
    static MODES: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();
    MODES.get_or_init(Default::default)
}

/// Whether this model's panel shows Focus. Remembered while the program runs; `TONESTRAND_FOCUS=1` makes it the default (pictures).
// TODO: keep the choice in the settings (a later phase).
pub fn in_focus(symbol: &str) -> bool {
    let chosen = modes().lock().ok().and_then(|m| m.get(symbol).copied());
    chosen.unwrap_or_else(|| std::env::var("TONESTRAND_FOCUS").is_ok_and(|v| v == "1"))
}

pub fn set_focus(symbol: &str, on: bool) {
    if let Ok(mut m) = modes().lock() {
        m.insert(symbol.to_owned(), on);
    }
}

/// For pictures only: `TONESTRAND_FOCUS_AT=x,y` puts the puck there and applies it on a block's first frame.
fn preview_at() -> Option<(f64, f64)> {
    let v = std::env::var("TONESTRAND_FOCUS_AT").ok()?;
    let (x, y) = v.split_once(',')?;
    Some((x.trim().parse::<f64>().ok()?.clamp(-1.0, 1.0), y.trim().parse::<f64>().ok()?.clamp(-1.0, 1.0)))
}

// ---------------------------------------------------------------- the widget

/// What one block's pad remembers (in egui's temporary memory).
#[derive(Clone, Default)]
struct PadState {
    /// Where the puck is; none until it is first used.
    puck: Option<(f64, f64)>,
    /// The block's values at the model's parameters before the first move, for Revert.
    originals: Vec<(usize, Value)>,
    /// What Focus last wrote, to tell a knob turned by hand.
    applied: Vec<(usize, Value)>,
    /// Values not yet sent to the pedal (newest per parameter).
    pending: BTreeMap<usize, Value>,
    last_flush: Option<f64>,
    started: bool,
}

/// What the user did to the pad this frame.
#[derive(Clone, Copy)]
enum Gesture {
    /// Dragging: continuous.
    Move(f64, f64),
    /// A click, Centre or the preview: exact, and sent at once.
    Jump(f64, f64),
    Revert,
    /// The drag ended.
    Release,
}

fn current_values(preset: &DevicePreset, s: &SlotView) -> Vec<Value> {
    preset.slots().get(s.global).map(|x| x.values.clone()).unwrap_or_else(|| s.values.clone())
}

fn write(preset: &mut DevicePreset, s: &SlotView, st: &mut PadState, live: bool, dirty: &mut bool, index: usize, v: Value) {
    preset.set_value(s.path, s.position, index, v.clone());
    *dirty = true;
    if live {
        st.pending.insert(index, v);
    }
}

fn apply_at(preset: &mut DevicePreset, s: &SlotView, st: &mut PadState, model: &FocusModel, live: bool, dirty: &mut bool, x: f64, y: f64) {
    let cur = current_values(preset, s);
    if st.originals.is_empty() {
        st.originals = model.params.iter().filter_map(|p| cur.get(p.index).map(|v| (p.index, v.clone()))).collect();
    }
    st.puck = Some((x, y));
    st.applied.clear();
    for (index, fv) in model.evaluate(x, y) {
        let Some(old) = cur.get(index) else { continue };
        let new = to_value(old, fv);
        if !same(old, &new) {
            write(preset, s, st, live, dirty, index, new.clone());
        }
        st.applied.push((index, new));
    }
}

fn revert(preset: &mut DevicePreset, s: &SlotView, st: &mut PadState, live: bool, dirty: &mut bool) {
    let cur = current_values(preset, s);
    for (index, v) in std::mem::take(&mut st.originals) {
        if cur.get(index).is_some_and(|c| !same(c, &v)) {
            write(preset, s, st, live, dirty, index, v);
        }
    }
    st.puck = None;
    st.applied.clear();
}

/// The pad, its tone name and its buttons. Writes into the preset (so the knobs drawn after it show the change in the same frame)
/// and, on the live preset, sends the changed parameters in paced batches.
pub fn focus_pad(
    ui: &mut egui::Ui,
    preset: &mut DevicePreset,
    live: bool,
    cmds: &mut Vec<Cmd>,
    dirty: &mut bool,
    s: &SlotView,
    symbol: &str,
    model: &FocusModel,
    width: f32,
) {
    let id = ui.make_persistent_id(("focus-pad", s.path, s.position, symbol));
    let mut st: PadState = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    let mut gesture = None;
    if !st.started {
        st.started = true;
        if let Some((x, y)) = preview_at() {
            gesture = Some(Gesture::Jump(x, y));
        }
    }
    let by_hand = st.puck.is_some() && edited_by_hand(&st.applied, &current_values(preset, s));

    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        let (rect, resp) = ui.allocate_exact_size(pad_size(width), Sense::click_and_drag());
        let inner = inner_rect(rect);
        if resp.dragged() {
            if let Some(p) = resp.interact_pointer_pos() {
                let (x, y) = xy_at(inner, p);
                gesture = Some(Gesture::Move(x, y));
            }
        } else if resp.clicked() {
            if let Some(p) = resp.interact_pointer_pos() {
                let (x, y) = snap_to_tone(inner, p, SNAP_RADIUS).unwrap_or_else(|| xy_at(inner, p));
                gesture = Some(Gesture::Jump(x, y));
            }
        }
        if resp.drag_stopped() {
            gesture = gesture.or(Some(Gesture::Release));
        }
        if resp.dragged() || resp.is_pointer_button_down_on() {
            ui.ctx().request_repaint();
        }

        // The status line, then the buttons (their gestures count this frame too).
        let status_h = 16.0;
        let (srect, _) = ui.allocate_exact_size(Vec2::new(width, status_h), Sense::hover());
        let (centre, revert_it) = ui
            .horizontal(|ui| {
                let c = super::switch_button(ui, "Centre", false);
                let r = ui.add_enabled(!st.originals.is_empty(), egui::Button::new("Revert").min_size(Vec2::new(56.0, 26.0))).clicked();
                (c, r)
            })
            .inner;
        if centre {
            gesture = Some(Gesture::Jump(0.0, 0.0));
        }
        if revert_it {
            gesture = Some(Gesture::Revert);
        }

        let mut ended = false;
        match gesture {
            Some(Gesture::Move(x, y)) => apply_at(preset, s, &mut st, model, live, dirty, x, y),
            Some(Gesture::Jump(x, y)) => {
                apply_at(preset, s, &mut st, model, live, dirty, x, y);
                ended = true;
            }
            Some(Gesture::Revert) => {
                revert(preset, s, &mut st, live, dirty);
                ended = true;
            }
            Some(Gesture::Release) => ended = true,
            None => {}
        }
        let by_hand = by_hand && !matches!(gesture, Some(Gesture::Move(..) | Gesture::Jump(..) | Gesture::Revert));

        // Live sending: newest value per parameter, at most every FLUSH_STEP while dragging, everything at the end.
        if live && !st.pending.is_empty() {
            let now = ui.ctx().input(|i| i.time);
            if should_flush(st.last_flush, now, ended) {
                let deferred = |i: usize| model.params.iter().any(|p| p.index == i && (p.quantize || p.kind == FocusKind::Bool));
                for (index, value) in take_flush(&mut st.pending, deferred, ended || SEND_SWITCHES_WHILE_DRAGGING) {
                    cmds.push(Cmd::SetParam { block: s.wire_block(), path: 0, index: index as i64, value });
                }
                st.last_flush = Some(now);
            }
            if !st.pending.is_empty() {
                ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(FLUSH_STEP));
            }
        }

        paint_pad(ui, rect, inner, model, symbol, st.puck, by_hand);
        let (text, col) = match (st.puck, by_hand) {
            (None, _) => ("Drag to change the character".to_owned(), muted()),
            (Some(_), true) => ("Edited by hand".to_owned(), muted()),
            (Some((x, y)), false) => {
                let k = nearest_tone(x, y);
                let share = focus::weights(x, y)[RegionKey::ALL.iter().position(|r| *r == k).unwrap_or(0)] * 100.0;
                (format!("{}  {:.0}%", model.region_name(k), share), ink())
            }
        };
        ui.painter().text(srect.left_center(), Align2::LEFT_CENTER, text, FontId::proportional(12.5), col);
    });
    ui.data_mut(|d| d.insert_temp(id, st));
}

/// Where a tone's name sits: inside its corner of the pad (left corners left-aligned, right ones right-aligned, top at the top, bottom
/// at the bottom), the centre one just below the centre ring. `size` is the pill's size.
fn label_rect(rect: Rect, at: Pos2, x: f64, y: f64, size: Vec2) -> Rect {
    let m = 6.0;
    let ring = 12.0;
    let (min_x, min_y) = if x < 0.0 {
        (rect.left() + m, if y > 0.0 { rect.top() + m } else { rect.bottom() - m - size.y })
    } else if x > 0.0 {
        (rect.right() - m - size.x, if y > 0.0 { rect.top() + m } else { rect.bottom() - m - size.y })
    } else {
        (at.x - size.x / 2.0, at.y + ring)
    };
    Rect::from_min_size(Pos2::new(min_x, min_y), size)
}

fn paint_pad(ui: &egui::Ui, rect: Rect, inner: Rect, model: &FocusModel, symbol: &str, puck: Option<(f64, f64)>, by_hand: bool) {
    let p = ui.painter();
    let round = CornerRadius::same(8);
    p.rect_filled(rect, round, Color32::from_rgb(0x10, 0x11, 0x14));
    // The model's whole photo, centred at its own aspect ratio (never cropped or stretched), dimmed; the rest stays the dark card.
    if let Some(tex) = crate::modelinfo::photo(ui.ctx(), symbol) {
        let img = contain_rect(tex.size_vec2(), rect);
        p.add(egui::Shape::mesh(photo_mesh(&tex, img)));
    }
    p.rect_stroke(rect, round, Stroke::new(1.0_f32, line()), egui::StrokeKind::Inside);
    // Cross-hair through the centre.
    let faint = Stroke::new(1.0_f32, Color32::from_white_alpha(40));
    let c = inner.center();
    p.line_segment([Pos2::new(c.x, inner.top()), Pos2::new(c.x, inner.bottom())], faint);
    p.line_segment([Pos2::new(inner.left(), c.y), Pos2::new(inner.right(), c.y)], faint);

    let puck_at = puck.map(|(x, y)| pos_of(inner, x, y));
    let leading = puck.filter(|_| !by_hand).map(|(x, y)| nearest_tone(x, y));
    let font = FontId::proportional(12.5);
    for key in RegionKey::ALL {
        let (x, y) = key.at();
        let at = pos_of(inner, x, y);
        p.circle_stroke(at, 5.0, Stroke::new(1.5_f32, Color32::from_white_alpha(170)));
        let lead = leading == Some(key);
        let colour = if lead { accent() } else { Color32::from_gray(0xE4) };
        let galley = p.layout_no_wrap(model.region_name(key).to_owned(), font.clone(), colour);
        let pill = label_rect(rect, at, x, y, galley.size() + Vec2::new(12.0, 6.0));
        let fade = if label_faded(puck_at, pill, FADE_RADIUS) { FADE_ALPHA } else { 1.0 };
        p.rect_filled(pill, CornerRadius::same(6), Color32::from_black_alpha(150).gamma_multiply(fade));
        if lead {
            p.rect_stroke(pill, CornerRadius::same(6), Stroke::new(1.0_f32, accent().gamma_multiply(fade)), egui::StrokeKind::Inside);
        }
        p.galley_with_override_text_color(pill.center() - galley.size() / 2.0, galley, colour.gamma_multiply(fade));
    }
    if let Some(at) = puck_at {
        let (fill, ring) = if by_hand { (theme::muted().gamma_multiply(0.7), muted()) } else { (accent_fill(), Color32::WHITE) };
        p.circle_filled(at, 19.0, fill.gamma_multiply(0.25));
        p.circle_stroke(at, 19.0, Stroke::new(1.0_f32, fill.gamma_multiply(0.5)));
        p.circle_filled(at, 11.0, fill);
        p.circle_stroke(at, 11.0, Stroke::new(2.0_f32, Color32::from_black_alpha(200)));
        p.circle_stroke(at, 12.5, Stroke::new(1.0_f32, ring));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pad() -> Rect {
        Rect::from_min_size(Pos2::new(100.0, 50.0), Vec2::new(300.0, 255.0))
    }

    #[test]
    fn position_and_xy_round_trip() {
        let inner = inner_rect(pad());
        for (x, y) in [(0.0, 0.0), (-1.0, -1.0), (1.0, 1.0), (0.25, -0.5)] {
            let (rx, ry) = xy_at(inner, pos_of(inner, x, y));
            assert!((rx - x).abs() < 1e-5 && (ry - y).abs() < 1e-5);
        }
        // Y points up, and outside the pad is clamped.
        assert_eq!(xy_at(inner, Pos2::new(-500.0, -500.0)), (-1.0, 1.0));
        assert_eq!(xy_at(inner, Pos2::new(900.0, 900.0)), (1.0, -1.0));
    }

    #[test]
    fn snapping_lands_exactly_on_a_tone() {
        let inner = inner_rect(pad());
        let near_corner = pos_of(inner, 1.0, 1.0) + Vec2::new(-6.0, 5.0);
        assert_eq!(snap_to_tone(inner, near_corner, SNAP_RADIUS), Some((1.0, 1.0)));
        assert_eq!(snap_to_tone(inner, pos_of(inner, 0.0, 0.0) + Vec2::new(3.0, 3.0), SNAP_RADIUS), Some((0.0, 0.0)));
        assert_eq!(snap_to_tone(inner, pos_of(inner, 0.5, 0.5), SNAP_RADIUS), None);
    }

    #[test]
    fn non_square_rectangle_maps_both_ways() {
        let inner = inner_rect(pad());
        assert!(inner.width() != inner.height());
        for (x, y) in [(0.0, 0.0), (-1.0, 1.0), (1.0, -1.0), (0.3, 0.7)] {
            let (rx, ry) = xy_at(inner, pos_of(inner, x, y));
            assert!((rx - x).abs() < 1e-5 && (ry - y).abs() < 1e-5);
        }
        // The centre is the middle of the rectangle, and the corners are its corners.
        assert_eq!(pos_of(inner, 0.0, 0.0), inner.center());
        assert_eq!(pos_of(inner, -1.0, 1.0), inner.left_top());
        assert_eq!(pos_of(inner, 1.0, -1.0), inner.right_bottom());
    }

    #[test]
    fn snap_radius_is_generous_but_not_endless() {
        let inner = inner_rect(pad());
        let corner = pos_of(inner, -1.0, -1.0);
        assert_eq!(snap_to_tone(inner, corner + Vec2::new(SNAP_RADIUS - 1.0, 0.0), SNAP_RADIUS), Some((-1.0, -1.0)));
        assert_eq!(snap_to_tone(inner, corner + Vec2::new(SNAP_RADIUS + 1.0, 0.0), SNAP_RADIUS), None);
    }

    #[test]
    fn a_name_fades_only_when_the_puck_is_close() {
        let label = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(80.0, 20.0));
        assert!(label_faded(Some(Pos2::new(50.0, 20.0)), label, FADE_RADIUS));
        assert!(label_faded(Some(Pos2::new(90.0 + FADE_RADIUS - 1.0, 20.0)), label, FADE_RADIUS));
        assert!(!label_faded(Some(Pos2::new(90.0 + FADE_RADIUS + 1.0, 20.0)), label, FADE_RADIUS));
        assert!(!label_faded(None, label, FADE_RADIUS));
    }

    #[test]
    fn the_photo_edge_fades_smoothly_and_symmetrically() {
        assert_eq!(feather(0.0, 20.0), 0.0);
        assert_eq!(feather(20.0, 20.0), 1.0);
        assert_eq!(feather(500.0, 20.0), 1.0);
        assert!((feather(10.0, 20.0) - 0.5).abs() < 1e-6);
        assert!(feather(5.0, 20.0) < feather(10.0, 20.0));
        // Same fade from either side of a 100-wide photo.
        let at = |x: f32| feather(x.min(100.0 - x), 20.0);
        assert!((at(7.0) - at(93.0)).abs() < 1e-6);
    }

    #[test]
    fn contain_shows_the_whole_picture_centred_and_never_enlarged() {
        let rect = Rect::from_min_size(Pos2::new(10.0, 20.0), Vec2::new(300.0, 255.0));
        // Bigger than the pad: scaled down to fit, aspect kept, centred.
        let r = contain_rect(Vec2::new(600.0, 200.0), rect);
        assert!((r.width() - 300.0).abs() < 1e-3 && (r.width() / r.height() - 3.0).abs() < 1e-4);
        assert!(r.center().distance(rect.center()) < 1e-4);
        let r = contain_rect(Vec2::new(200.0, 600.0), rect);
        assert!((r.height() - 255.0).abs() < 1e-3 && (r.width() / r.height() - 1.0 / 3.0).abs() < 1e-4);
        // Smaller than the pad: native size, not enlarged.
        let r = contain_rect(Vec2::new(100.0, 80.0), rect);
        assert_eq!(r.size(), Vec2::new(100.0, 80.0) * PHOTO_MAX_SCALE);
        assert!(rect.contains_rect(r));
    }

    #[test]
    fn the_pad_is_bigger_in_a_wide_panel_and_the_panel_grows_to_hold_it() {
        assert_eq!(pad_width(1500.0), PAD_MAX_W);
        assert!(pad_width(800.0) < PAD_MAX_W && pad_width(800.0) >= 220.0);
        let w = pad_width(1500.0);
        assert!(panel_height(w) > super::super::BODY_H + 100.0);
        assert!(panel_height(w) > pad_size(w).y);
    }

    #[test]
    fn nearest_tone_names_the_corner() {
        assert_eq!(nearest_tone(0.0, 0.0), RegionKey::Origin);
        assert_eq!(nearest_tone(-0.9, 0.9), RegionKey::TopLeft);
        assert_eq!(nearest_tone(0.9, -0.9), RegionKey::BottomRight);
    }

    #[test]
    fn values_keep_the_stored_type() {
        assert_eq!(to_value(&Value::Bool(false), FocusValue::Bool(true)), Value::Bool(true));
        assert_eq!(to_value(&Value::Bool(true), FocusValue::Float(0.0)), Value::Bool(false));
        assert_eq!(to_value(&Value::int(3), FocusValue::Int(7)), Value::int(7));
        assert_eq!(to_value(&Value::int(3), FocusValue::Float(4.6)), Value::int(5));
        assert_eq!(to_value(&Value::F32(0.1), FocusValue::Float(0.5)), Value::F32(0.5));
        assert_eq!(to_value(&Value::F32(0.1), FocusValue::Bool(true)), Value::F32(1.0));
    }

    #[test]
    fn edited_by_hand_sees_a_changed_knob() {
        let applied = vec![(0, Value::F32(0.5)), (2, Value::Bool(true))];
        let same_now = vec![Value::F32(0.5), Value::F32(9.0), Value::Bool(true)];
        assert!(!edited_by_hand(&applied, &same_now));
        let knob_moved = vec![Value::F32(0.6), Value::F32(9.0), Value::Bool(true)];
        assert!(edited_by_hand(&applied, &knob_moved));
        let switch_flipped = vec![Value::F32(0.5), Value::F32(9.0), Value::Bool(false)];
        assert!(edited_by_hand(&applied, &switch_flipped));
        assert!(!edited_by_hand(&[], &knob_moved));
    }

    #[test]
    fn flush_is_paced_and_always_at_the_end() {
        assert!(should_flush(None, 1.0, false));
        assert!(!should_flush(Some(1.0), 1.02, false));
        assert!(should_flush(Some(1.0), 1.06, false));
        assert!(should_flush(Some(1.0), 1.001, true));
    }

    #[test]
    fn take_flush_keeps_switches_until_the_end() {
        let mut pending: BTreeMap<usize, Value> = [(1, Value::F32(0.2)), (4, Value::Bool(true)), (7, Value::F32(0.9))].into();
        let first = take_flush(&mut pending, |i| i == 4, false);
        assert_eq!(first.iter().map(|x| x.0).collect::<Vec<_>>(), vec![1, 7]);
        assert_eq!(pending.len(), 1);
        let last = take_flush(&mut pending, |i| i == 4, true);
        assert_eq!(last, vec![(4, Value::Bool(true))]);
        assert!(pending.is_empty());
    }
}
