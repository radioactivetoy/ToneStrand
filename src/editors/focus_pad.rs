//! Focus mode: an X-Y pad that steers several parameters of a block at once, next to the normal knobs.
//!
//! The five named tones of a model (the centre and the four corners) come from `tonestrand_core::focus`. Dragging the puck
//! evaluates the blend and writes the parameters into the preset, so the knob grid, drawn after the pad in the same frame, turns
//! with it. The logic that is not drawing (pad position and x,y, value conversion, send pacing, "edited by hand") is in small
//! functions at the top, with tests.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use tonestrand_core::focus::{self, FocusModel, FocusSet, FocusValue, Kind as FocusKind, Located, RegionKey, GHOST_MAX_ERROR};

use crate::device::Cmd;
use crate::devpreset::{DevicePreset, SlotView};
use crate::msgpack::Value;
use crate::theme::{self, *};

/// The pad's widest size and its share of the panel's width (the pad is bigger on touch, for a finger).
const PAD_MAX_W: f32 = 340.0;
const PAD_SHARE: f32 = 0.30;
const PAD_MAX_W_TOUCH: f32 = 640.0;
const PAD_SHARE_TOUCH: f32 = 0.55;
/// What the window's chrome above a block panel takes, for capping the pad's height on touch.
const CHROME_H: f32 = 410.0;
/// The pad's height as a part of its width.
const PAD_ASPECT: f32 = 0.85;
/// How faded a name is while the puck is near it.
const FADE_ALPHA: f32 = 0.35;
/// Darkening laid over the photo so it reads as a texture behind the controls.
const PHOTO_DIM: u8 = 120;
/// The most a photo is enlarged (1.0 = its own pixels).
const PHOTO_MAX_SCALE: f32 = 1.0;
/// How far (in points) a photo's edge fades into the card.
const PHOTO_FEATHER: f32 = 22.0;
/// Sizes that depend on the input: a finger is about 10 mm wide and hides the puck, so on touch everything is bigger.
#[derive(Clone, Copy)]
struct Look {
    /// Space between the pad's edge and its corner tones, so the rings and the puck stay inside.
    inset: f32,
    /// A click this close (in points) to a tone lands exactly on it.
    snap: f32,
    /// A tone name this close to the puck fades, so the puck never hides it.
    fade: f32,
    /// The text size of tone names, the caption and the readout.
    text: f32,
    /// The height of the caption and the readout rows.
    row_h: f32,
    puck: f32,
    /// The soft ring around the puck (on touch a clear halo around the fingertip).
    halo: f32,
    ring: f32,
    /// The least height of Centre and Revert.
    button_h: f32,
}

impl Look {
    fn new(touch: bool) -> Self {
        if touch {
            Self { inset: 40.0, snap: 32.0, fade: 48.0, text: 14.0, row_h: 20.0, puck: 14.0, halo: 26.0, ring: 7.0, button_h: 36.0 }
        } else {
            Self { inset: 34.0, snap: 22.0, fade: 40.0, text: 12.5, row_h: 16.0, puck: 11.0, halo: 19.0, ring: 5.0, button_h: 26.0 }
        }
    }

    /// What is under the pad: the caption row, the readout row and the buttons, with the gaps between them and the pad.
    fn below(self) -> f32 {
        self.row_h * 2.0 + self.button_h + 12.0
    }
}
/// The least time between two live sends while dragging, in seconds.
const FLUSH_STEP: f64 = 0.05;
/// Switch-like parameters (on/off, snapping choices) are sent while dragging too when true; when false only on release.
/// Off for now: they can click; to be decided by listening on the pedal (a later phase).
const SEND_SWITCHES_WHILE_DRAGGING: bool = false;

// ---------------------------------------------------------------- pure parts

/// The pad's width in a panel `full` points wide.
pub fn pad_width(full: f32, touch: bool) -> f32 {
    if touch {
        (full * PAD_SHARE_TOUCH).clamp(260.0, PAD_MAX_W_TOUCH)
    } else {
        (full * PAD_SHARE).clamp(220.0, PAD_MAX_W)
    }
}

/// The pad's size in a panel `full` wide, in a window `screen_h` high. On touch the height is capped to what the window leaves, so
/// the panel does not outgrow the screen (the pad is then wider than tall, which is fine).
pub fn pad_size(full: f32, screen_h: f32, touch: bool) -> Vec2 {
    let w = pad_width(full, touch);
    let mut h = (w * PAD_ASPECT).round();
    if touch {
        h = h.min((screen_h - CHROME_H - Look::new(true).below() - 6.0).max(200.0));
    }
    Vec2::new(w, h)
}

/// The height of a block panel's body while it shows a pad of this size: the pad, the rows under it and a margin.
pub fn panel_height(pad: Vec2, touch: bool) -> f32 {
    pad.y + Look::new(touch).below() + 6.0
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
fn inner_rect(pad: Rect, inset: f32) -> Rect {
    pad.shrink(inset)
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

/// Whether a live send is due: always at the end of a gesture, otherwise when `FLUSH_STEP` has passed since the last one.
fn should_flush(last: Option<f64>, now: f64, ended: bool) -> bool {
    ended || last.is_none_or(|t| now - t >= FLUSH_STEP)
}

/// Whether a value is worth a round trip to the pedal: at the end of a gesture always, otherwise only when it differs from the last
/// one sent for that parameter by at least 0.1% of the parameter's range (a switch or a choice: when it is not the same).
fn worth_sending(last: Option<&Value>, new: &Value, range: f64, ended: bool) -> bool {
    let Some(last) = last else { return true };
    if ended {
        return true;
    }
    match (last.as_f64(), new.as_f64()) {
        (Some(a), Some(b)) => (a - b).abs() >= 0.001 * range,
        _ => !same(last, new),
    }
}

/// Take the values to send from the pending map (newest value per parameter). Switch-like ones (`deferred`) stay unless
/// `with_deferred`.
fn take_flush(pending: &mut BTreeMap<usize, Value>, deferred: impl Fn(usize) -> bool, with_deferred: bool) -> Vec<(usize, Value)> {
    let keys: Vec<usize> = pending.keys().copied().filter(|k| with_deferred || !deferred(*k)).collect();
    keys.into_iter().filter_map(|k| pending.remove(&k).map(|v| (k, v))).collect()
}

/// One knob of the grid: its value index, whether Focus moves it, and whether it differs from before the first pad gesture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct KnobSlot {
    pub index: usize,
    pub moved: bool,
    pub changed: bool,
}

/// The knobs in the order to show them: the ones Focus moves first, then the rest, each in its normal order. With no `focus`
/// indices (Knobs mode) every knob counts as moved and the order is the normal one. `changed` is set for a knob whose current
/// value differs from the one in `originals` (none before the first gesture and after Revert).
pub(super) fn knob_order(n: usize, focus: &[usize], originals: &[(usize, Value)], current: &[Value]) -> Vec<KnobSlot> {
    let slot = |index: usize, moved: bool| KnobSlot {
        index,
        moved,
        changed: originals.iter().any(|(i, v)| *i == index && current.get(index).is_some_and(|c| !same(c, v))),
    };
    if focus.is_empty() {
        return (0..n).map(|i| slot(i, true)).collect();
    }
    let first = (0..n).filter(|i| focus.contains(i)).map(|i| slot(i, true));
    let rest = (0..n).filter(|i| !focus.contains(i)).map(|i| slot(i, false));
    first.chain(rest).collect()
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

/// What the program knows about Focus on/off: the person's choices per effect (kept in the settings), the "open in Focus" default, and
/// whether a choice changed since the window last looked (to save the settings).
#[derive(Default)]
struct Modes {
    models: BTreeMap<String, bool>,
    default_on: bool,
    changed: bool,
}

fn modes() -> &'static Mutex<Modes> {
    static MODES: OnceLock<Mutex<Modes>> = OnceLock::new();
    MODES.get_or_init(Default::default)
}

/// Whether an effect shows Focus. The person's own choice for that effect wins, then `TONESTRAND_FOCUS=1` (pictures), then the
/// "open in Focus" setting, then Knobs.
fn focus_choice(explicit: Option<bool>, env_on: bool, default_on: bool) -> bool {
    explicit.unwrap_or(env_on || default_on)
}

/// Hand the saved choices over at start (not in a picture run: pictures never depend on the owner's settings).
pub fn load_choices(models: BTreeMap<String, bool>, default_on: bool) {
    if let Ok(mut m) = modes().lock() {
        m.models = models;
        m.default_on = default_on;
        m.changed = false;
    }
}

/// The Settings page changed "open in Focus".
pub fn set_default(on: bool) {
    if let Ok(mut m) = modes().lock() {
        m.default_on = on;
    }
}

/// The person's choices if one changed since the last call (to be saved), else None.
pub fn take_changed_choices() -> Option<BTreeMap<String, bool>> {
    let mut m = modes().lock().ok()?;
    std::mem::take(&mut m.changed).then(|| m.models.clone())
}

/// Whether this model's panel shows Focus.
pub fn in_focus(symbol: &str) -> bool {
    let (explicit, default_on) = modes().lock().map(|m| (m.models.get(symbol).copied(), m.default_on)).unwrap_or((None, false));
    focus_choice(explicit, std::env::var("TONESTRAND_FOCUS").is_ok_and(|v| v == "1"), default_on)
}

pub fn set_focus(symbol: &str, on: bool) {
    if let Ok(mut m) = modes().lock() {
        m.models.insert(symbol.to_owned(), on);
        m.changed = true;
    }
}

/// For pictures only: `TONESTRAND_FOCUS_AT=x,y` puts the puck there and applies it on a block's first frame.
fn preview_at() -> Option<(f64, f64)> {
    let v = std::env::var("TONESTRAND_FOCUS_AT").ok()?;
    let (x, y) = v.split_once(',')?;
    Some((x.trim().parse::<f64>().ok()?.clamp(-1.0, 1.0), y.trim().parse::<f64>().ok()?.clamp(-1.0, 1.0)))
}

// ---------------------------------------------------------------- the widget

fn pad_id(s: &SlotView, symbol: &str) -> egui::Id {
    egui::Id::new(("focus-pad", s.path, s.position, symbol))
}

/// What the knob grid beside the pad needs: the knobs the pad moves and the values before its first gesture.
pub(super) fn knobs_for(ctx: &egui::Context, s: &SlotView, symbol: &str, model: &FocusModel) -> super::FocusKnobs {
    let st: PadState = ctx.data(|d| d.get_temp(pad_id(s, symbol))).unwrap_or_default();
    super::FocusKnobs { moved: model.params.iter().map(|p| p.index).collect(), originals: st.originals }
}

/// The values of a block's Focus parameters (value index, value), in the model's order.
type Vals = Vec<(usize, Value)>;

/// One undoable step: the values of all Focus parameters before and after one gesture (a drag, a click, Centre, Revert).
#[derive(Clone, Debug, PartialEq)]
struct Step {
    before: Vals,
    after: Vals,
}

/// The pad's own undo and redo (the live preset has none of its own), per block per session.
#[derive(Clone, Debug, Default)]
struct History {
    undo: Vec<Step>,
    redo: Vec<Step>,
}

/// The most steps kept.
const HISTORY_MAX: usize = 100;

impl History {
    /// A finished gesture. A gesture that changed nothing is not a step; a new step drops what could be redone.
    fn push(&mut self, step: Step) -> bool {
        if same_vals(&step.before, &step.after) {
            return false;
        }
        self.undo.push(step);
        if self.undo.len() > HISTORY_MAX {
            self.undo.remove(0);
        }
        self.redo.clear();
        true
    }

    /// The values to go back to, and the step moves to the redo side.
    fn undo(&mut self) -> Option<Vals> {
        let step = self.undo.pop()?;
        let before = step.before.clone();
        self.redo.push(step);
        Some(before)
    }

    fn redo(&mut self) -> Option<Vals> {
        let step = self.redo.pop()?;
        let after = step.after.clone();
        self.undo.push(step);
        Some(after)
    }
}

/// Whether two lists of values are the same.
fn same_vals(a: &[(usize, Value)], b: &[(usize, Value)]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.0 == y.0 && same(&x.1, &y.1))
}

/// A stored value as a number for `locate` (a switch is 0 or 1).
fn num(v: &Value) -> f64 {
    match v {
        Value::Bool(b) => f64::from(*b),
        _ => v.as_f64().unwrap_or(0.0),
    }
}

/// The block's values at the model's parameters.
fn focus_values(model: &FocusModel, cur: &[Value]) -> Vals {
    model.params.iter().filter_map(|p| cur.get(p.index).map(|v| (p.index, v.clone()))).collect()
}

/// What one block's pad remembers (in egui's temporary memory).
#[derive(Clone, Default)]
struct PadState {
    /// The last known place of the puck (the preferred spot when the values are located again, so it does not jump).
    pos: Option<(f64, f64)>,
    /// True while the values are exactly what the last gesture wrote: the solid puck. A knob turned since, a reload or an undo from
    /// outside makes it a ghost or nothing (see [`shown`]).
    solid: bool,
    /// Where the current values sit on the pad, when they were last looked at.
    located: Option<Located>,
    /// The values `located` was computed from, to find out when they change (as numbers: cheap to compare).
    seen: Vec<(usize, f64)>,
    /// The block's values at the model's parameters before the first gesture of the session, for Revert.
    originals: Vals,
    history: History,
    /// The values when the current drag began.
    drag_before: Option<Vals>,
    /// Values not yet sent to the pedal (newest per parameter).
    pending: BTreeMap<usize, Value>,
    last_flush: Option<f64>,
    /// The last value sent to the pedal for each parameter in this gesture, to skip changes too small to matter.
    last_sent: BTreeMap<usize, Value>,
    started: bool,
}

/// What the pad shows for the puck.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Shown {
    /// A gesture put the values here: the solid puck.
    Solid(f64, f64),
    /// The values (set elsewhere, or saved from the pad) match a point of the pad: an outlined ring.
    Ghost(f64, f64),
    /// Set by hand: no puck; the closest point is only used to name the closest tone.
    Hand(f64, f64),
    Nothing,
}

/// What to show for the state of a pad (a pure rule, tested).
fn shown(solid: bool, pos: Option<(f64, f64)>, located: Option<Located>) -> Shown {
    match (solid, pos, located) {
        (true, Some((x, y)), _) => Shown::Solid(x, y),
        (_, _, Some(l)) if l.error <= GHOST_MAX_ERROR => Shown::Ghost(l.x, l.y),
        (_, _, Some(l)) => Shown::Hand(l.x, l.y),
        _ => Shown::Nothing,
    }
}

/// What the user did to the pad this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Gesture {
    /// Dragging: continuous.
    Move(f64, f64),
    /// A click, Centre or the preview: exact, and sent at once.
    Jump(f64, f64),
    Revert,
    Undo,
    Redo,
    /// The drag ended.
    Release,
}

/// What the pad's controls report in one frame.
#[derive(Clone, Copy, Default)]
struct Input {
    /// Where the pointer is while it drags on the pad.
    dragging: Option<Pos2>,
    /// Where a click on the pad landed.
    clicked: Option<Pos2>,
    drag_stopped: bool,
    centre: bool,
    revert: bool,
    undo: bool,
    redo: bool,
}

/// What the input asks for. Nothing asked, nothing done: opening Focus, switching block or reloading never writes anything.
fn decide(input: Input, inner: Rect, snap: f32) -> Option<Gesture> {
    let mut g = None;
    if let Some(p) = input.dragging {
        let (x, y) = xy_at(inner, p);
        g = Some(Gesture::Move(x, y));
    } else if let Some(p) = input.clicked {
        let (x, y) = snap_to_tone(inner, p, snap).unwrap_or_else(|| xy_at(inner, p));
        g = Some(Gesture::Jump(x, y));
    }
    if input.drag_stopped {
        g = g.or(Some(Gesture::Release));
    }
    if input.centre {
        g = Some(Gesture::Jump(0.0, 0.0));
    }
    if input.revert {
        g = Some(Gesture::Revert);
    }
    if input.undo {
        g = Some(Gesture::Undo);
    }
    if input.redo {
        g = Some(Gesture::Redo);
    }
    g
}

// The window's undo groups a pad gesture into one step: these flags tell it when a gesture begins and while it lasts.
static GESTURE_START: AtomicBool = AtomicBool::new(false);
static GESTURE_ACTIVE: AtomicBool = AtomicBool::new(false);

/// True from the start of a pad gesture until the window has made it an undo step (it calls [`gesture_started`] then).
pub fn gesture_start_pending() -> bool {
    GESTURE_START.load(Ordering::Relaxed)
}

/// The window made the step.
pub fn gesture_started() {
    GESTURE_START.store(false, Ordering::Relaxed);
}

/// True if a pad drag went on this frame (read once per frame: it is cleared).
pub fn gesture_active() -> bool {
    GESTURE_ACTIVE.swap(false, Ordering::Relaxed)
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

/// Write these values (where they differ from the block's).
fn write_values(preset: &mut DevicePreset, s: &SlotView, st: &mut PadState, live: bool, dirty: &mut bool, vals: Vals) {
    let cur = current_values(preset, s);
    for (index, v) in vals {
        if cur.get(index).is_some_and(|c| !same(c, &v)) {
            write(preset, s, st, live, dirty, index, v);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn apply_at(preset: &mut DevicePreset, s: &SlotView, st: &mut PadState, model: &FocusModel, live: bool, dirty: &mut bool, x: f64, y: f64) {
    let cur = current_values(preset, s);
    if st.originals.is_empty() {
        st.originals = focus_values(model, &cur);
    }
    for (index, fv) in model.evaluate(x, y) {
        let Some(old) = cur.get(index) else { continue };
        let new = to_value(old, fv);
        if !same(old, &new) {
            write(preset, s, st, live, dirty, index, new);
        }
    }
    st.pos = Some((x, y));
    st.solid = true;
    st.located = Some(Located { x, y, error: 0.0 });
    st.seen = focus_values(model, &current_values(preset, s)).iter().map(|(i, v)| (*i, num(v))).collect();
}

/// Look where the block's current values sit on the pad (when they changed since the last look).
fn relocate(st: &mut PadState, model: &FocusModel, cur: &[Value]) {
    let nums: Vec<(usize, f64)> = focus_values(model, cur).iter().map(|(i, v)| (*i, num(v))).collect();
    if nums == st.seen {
        return;
    }
    st.solid = false;
    st.located = model.locate(&nums, st.pos);
    if let Some(l) = st.located {
        st.pos = Some((l.x, l.y));
    }
    st.seen = nums;
}

/// For pictures only: `TONESTRAND_FOCUS_GHOST=x,y` writes the values at that point into the block as if the preset had been saved
/// from the pad: no gesture, no undo step.
fn ghost_at() -> Option<(f64, f64)> {
    let v = std::env::var("TONESTRAND_FOCUS_GHOST").ok()?;
    let (x, y) = v.split_once(',')?;
    Some((x.trim().parse::<f64>().ok()?.clamp(-1.0, 1.0), y.trim().parse::<f64>().ok()?.clamp(-1.0, 1.0)))
}

/// The pad, its tone name and its buttons. Writes into the preset (so the knobs drawn after it show the change in the same frame)
/// and, on the live preset, sends the changed parameters in paced batches. Nothing is written until the person clicks or drags on
/// the pad or presses a button.
#[allow(clippy::too_many_arguments)]
pub fn focus_pad(
    ui: &mut egui::Ui,
    preset: &mut DevicePreset,
    live: bool,
    cmds: &mut Vec<Cmd>,
    dirty: &mut bool,
    s: &SlotView,
    symbol: &str,
    model: &FocusModel,
    size: Vec2,
) {
    let look = Look::new(theme::touch());
    let id = pad_id(s, symbol);
    let mut st: PadState = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    let mut input = Input::default();
    let mut preview = None;
    if !st.started {
        st.started = true;
        // Pictures only (environment variables): a puck placed on the first frame, or values written as if saved from the pad.
        preview = preview_at();
        if let Some((x, y)) = ghost_at().filter(|_| preview.is_none()) {
            let cur = current_values(preset, s);
            for (index, fv) in model.evaluate(x, y) {
                if let Some(old) = cur.get(index) {
                    preset.set_value(s.path, s.position, index, to_value(old, fv));
                }
            }
        }
    }
    relocate(&mut st, model, &current_values(preset, s));

    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
        let inner = inner_rect(rect, look.inset);
        if resp.dragged() {
            input.dragging = resp.interact_pointer_pos();
            GESTURE_ACTIVE.store(true, Ordering::Relaxed);
        } else if resp.clicked() {
            input.clicked = resp.interact_pointer_pos();
        }
        input.drag_stopped = resp.drag_stopped();
        if resp.drag_started() {
            GESTURE_START.store(true, Ordering::Relaxed);
        }
        if resp.dragged() || resp.is_pointer_button_down_on() {
            ui.ctx().request_repaint();
        }

        // The caption (the centre tone's name), the readout, then the buttons (their gestures count this frame too).
        let (crect, _) = ui.allocate_exact_size(Vec2::new(size.x, look.row_h), Sense::hover());
        let (srect, _) = ui.allocate_exact_size(Vec2::new(size.x, look.row_h), Sense::hover());
        let now_vals = focus_values(model, &current_values(preset, s));
        ui.horizontal(|ui| {
            let button = |ui: &mut egui::Ui, text: &str, enabled: bool| {
                ui.add_enabled(
                    enabled,
                    egui::Button::new(RichText::new(text).color(ink()))
                        .min_size(Vec2::new(48.0, look.button_h))
                        .corner_radius(CornerRadius::same(6))
                        .fill(card_hi())
                        .stroke(Stroke::new(1.0_f32, line())),
                )
                .clicked()
            };
            input.centre = button(ui, "Centre", true);
            input.revert = button(ui, "Revert", !st.originals.is_empty() && !same_vals(&st.originals, &now_vals));
            if live {
                input.undo = button(ui, "Undo", !st.history.undo.is_empty());
                input.redo = button(ui, "Redo", !st.history.redo.is_empty());
            }
        });
        let mut gesture = decide(input, inner, look.snap);
        if let Some((x, y)) = preview {
            gesture = Some(Gesture::Jump(x, y));
        }
        if matches!(gesture, Some(Gesture::Jump(..) | Gesture::Revert | Gesture::Undo | Gesture::Redo)) {
            GESTURE_START.store(true, Ordering::Relaxed);
        }

        let mut ended = false;
        match gesture {
            Some(Gesture::Move(x, y)) => {
                if st.drag_before.is_none() {
                    st.last_sent.clear();
                    st.drag_before = Some(now_vals.clone());
                }
                apply_at(preset, s, &mut st, model, live, dirty, x, y);
            }
            Some(Gesture::Jump(x, y)) => {
                apply_at(preset, s, &mut st, model, live, dirty, x, y);
                let after = focus_values(model, &current_values(preset, s));
                st.history.push(Step { before: now_vals.clone(), after });
                ended = true;
            }
            Some(Gesture::Revert) => {
                let originals = st.originals.clone();
                write_values(preset, s, &mut st, live, dirty, originals);
                let after = focus_values(model, &current_values(preset, s));
                st.history.push(Step { before: now_vals.clone(), after });
                st.solid = false;
                st.seen.clear();
                ended = true;
            }
            Some(Gesture::Undo) => {
                if let Some(v) = st.history.undo() {
                    write_values(preset, s, &mut st, live, dirty, v);
                }
                st.solid = false;
                st.seen.clear();
                ended = true;
            }
            Some(Gesture::Redo) => {
                if let Some(v) = st.history.redo() {
                    write_values(preset, s, &mut st, live, dirty, v);
                }
                st.solid = false;
                st.seen.clear();
                ended = true;
            }
            Some(Gesture::Release) => ended = true,
            None => {}
        }
        // A drag is one step: from where it began to where it ended.
        if matches!(gesture, Some(Gesture::Release)) || (st.drag_before.is_some() && !resp.dragged()) {
            if let Some(before) = st.drag_before.take() {
                let after = focus_values(model, &current_values(preset, s));
                st.history.push(Step { before, after });
            }
        }
        relocate(&mut st, model, &current_values(preset, s));

        // Live sending: newest value per parameter, at most every FLUSH_STEP while dragging, everything at the end.
        if live && !st.pending.is_empty() {
            let now = ui.ctx().input(|i| i.time);
            if should_flush(st.last_flush, now, ended) {
                let deferred = |i: usize| model.params.iter().any(|p| p.index == i && (p.quantize || p.kind == FocusKind::Bool));
                for (index, value) in take_flush(&mut st.pending, deferred, ended || SEND_SWITCHES_WHILE_DRAGGING) {
                    let range =
                        model.params.iter().find(|p| p.index == index).map_or(1.0, |p| if p.max > p.min { p.max - p.min } else { 1.0 });
                    if worth_sending(st.last_sent.get(&index), &value, range, ended) {
                        st.last_sent.insert(index, value.clone());
                        cmds.push(Cmd::SetParam { block: s.wire_block(), path: 0, index: index as i64, value });
                    } else {
                        // Not worth a round trip yet: keep it for the next flush (and the end of the gesture sends it).
                        st.pending.insert(index, value);
                    }
                }
                st.last_flush = Some(now);
            }
            if !st.pending.is_empty() {
                ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(FLUSH_STEP));
            }
        }

        let what = shown(st.solid, st.pos, st.located);
        paint_pad(ui, rect, inner, model, symbol, what, look);
        let dominant = |x: f64, y: f64| {
            let k = nearest_tone(x, y);
            let share = focus::weights(x, y)[RegionKey::ALL.iter().position(|r| *r == k).unwrap_or(0)] * 100.0;
            (model.region_name(k).to_owned(), share)
        };
        let (text, col) = match what {
            Shown::Solid(x, y) => {
                let (n, share) = dominant(x, y);
                (format!("{n}  {share:.0}%"), ink())
            }
            Shown::Ghost(x, y) => {
                let (n, share) = dominant(x, y);
                (format!("On the pad: {n} {share:.0}%"), muted())
            }
            Shown::Hand(x, y) => (format!("Set by hand: closest tone {}", model.region_name(nearest_tone(x, y))), muted()),
            Shown::Nothing => ("Drag to change the character".to_owned(), muted()),
        };
        ui.painter().text(srect.left_center(), Align2::LEFT_CENTER, text, FontId::proportional(look.text), col);
        // "Centre: <name>", the name in the accent colour while the centre is the leading tone.
        let centre_leads = matches!(what, Shown::Solid(x, y) | Shown::Ghost(x, y) if nearest_tone(x, y) == RegionKey::Origin);
        let font = FontId::proportional(look.text);
        let head = ui.painter().layout_no_wrap("Centre: ".to_owned(), font.clone(), muted());
        let name = ui.painter().layout_no_wrap(
            model.region_name(RegionKey::Origin).to_owned(),
            font,
            if centre_leads { accent() } else { muted() },
        );
        let at = crect.left_center() - Vec2::new(0.0, head.size().y / 2.0);
        let name_at = at + Vec2::new(head.size().x, 0.0);
        ui.painter().galley(at, head, muted());
        ui.painter().galley(name_at, name, muted());
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

fn paint_pad(ui: &egui::Ui, rect: Rect, inner: Rect, model: &FocusModel, symbol: &str, what: Shown, look: Look) {
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

    let at_puck = match what {
        Shown::Solid(x, y) | Shown::Ghost(x, y) => Some((x, y)),
        _ => None,
    };
    let puck_at = at_puck.map(|(x, y)| pos_of(inner, x, y));
    let leading = at_puck.map(|(x, y)| nearest_tone(x, y));
    let font = FontId::proportional(look.text);
    for key in RegionKey::ALL {
        let (x, y) = key.at();
        let at = pos_of(inner, x, y);
        p.circle_stroke(at, look.ring, Stroke::new(1.5_f32, Color32::from_white_alpha(170)));
        // The centre's name is under the pad (it was hard to read on the photo, under the puck).
        if key == RegionKey::Origin {
            continue;
        }
        let lead = leading == Some(key);
        let colour = if lead { accent() } else { Color32::from_gray(0xE4) };
        let galley = p.layout_no_wrap(model.region_name(key).to_owned(), font.clone(), colour);
        let pill = label_rect(rect, at, x, y, galley.size() + Vec2::new(12.0, 6.0));
        let fade = if label_faded(puck_at, pill, look.fade) { FADE_ALPHA } else { 1.0 };
        p.rect_filled(pill, CornerRadius::same(6), Color32::from_black_alpha(150).gamma_multiply(fade));
        if lead {
            p.rect_stroke(pill, CornerRadius::same(6), Stroke::new(1.0_f32, accent().gamma_multiply(fade)), egui::StrokeKind::Inside);
        }
        p.galley_with_override_text_color(pill.center() - galley.size() / 2.0, galley, colour.gamma_multiply(fade));
    }
    if let Some(at) = puck_at {
        let halo_w = if look.halo > 20.0 { 2.0_f32 } else { 1.0 };
        if matches!(what, Shown::Solid(..)) {
            let fill = accent_fill();
            p.circle_filled(at, look.halo, fill.gamma_multiply(0.25));
            p.circle_stroke(at, look.halo, Stroke::new(halo_w, fill.gamma_multiply(0.6)));
            p.circle_filled(at, look.puck, fill);
            p.circle_stroke(at, look.puck, Stroke::new(2.0_f32, Color32::from_black_alpha(200)));
            p.circle_stroke(at, look.puck + 1.5, Stroke::new(1.0_f32, Color32::WHITE));
        } else {
            // A ghost: the same puck as an outline only, so it reads as "the values are here" and not "you put it here".
            p.circle_stroke(at, look.halo, Stroke::new(halo_w, accent().gamma_multiply(0.5)));
            p.circle_filled(at, look.puck, Color32::from_black_alpha(60));
            p.circle_stroke(at, look.puck, Stroke::new(3.0_f32, Color32::from_black_alpha(200)));
            p.circle_stroke(at, look.puck, Stroke::new(2.0_f32, accent()));
        }
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
        let inner = inner_rect(pad(), Look::new(false).inset);
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
        let inner = inner_rect(pad(), Look::new(false).inset);
        let near_corner = pos_of(inner, 1.0, 1.0) + Vec2::new(-6.0, 5.0);
        assert_eq!(snap_to_tone(inner, near_corner, Look::new(false).snap), Some((1.0, 1.0)));
        assert_eq!(snap_to_tone(inner, pos_of(inner, 0.0, 0.0) + Vec2::new(3.0, 3.0), Look::new(false).snap), Some((0.0, 0.0)));
        assert_eq!(snap_to_tone(inner, pos_of(inner, 0.5, 0.5), Look::new(false).snap), None);
    }

    #[test]
    fn non_square_rectangle_maps_both_ways() {
        let inner = inner_rect(pad(), Look::new(false).inset);
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
        let inner = inner_rect(pad(), Look::new(false).inset);
        let corner = pos_of(inner, -1.0, -1.0);
        assert_eq!(snap_to_tone(inner, corner + Vec2::new(Look::new(false).snap - 1.0, 0.0), Look::new(false).snap), Some((-1.0, -1.0)));
        assert_eq!(snap_to_tone(inner, corner + Vec2::new(Look::new(false).snap + 1.0, 0.0), Look::new(false).snap), None);
    }

    #[test]
    fn a_name_fades_only_when_the_puck_is_close() {
        let label = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(80.0, 20.0));
        assert!(label_faded(Some(Pos2::new(50.0, 20.0)), label, Look::new(false).fade));
        assert!(label_faded(Some(Pos2::new(90.0 + Look::new(false).fade - 1.0, 20.0)), label, Look::new(false).fade));
        assert!(!label_faded(Some(Pos2::new(90.0 + Look::new(false).fade + 1.0, 20.0)), label, Look::new(false).fade));
        assert!(!label_faded(None, label, Look::new(false).fade));
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
        assert_eq!(pad_width(1500.0, false), PAD_MAX_W);
        assert!(pad_width(800.0, false) < PAD_MAX_W && pad_width(800.0, false) >= 220.0);
        let pad = pad_size(1500.0, 1000.0, false);
        assert_eq!(pad.x, PAD_MAX_W);
        assert!(panel_height(pad, false) > super::super::BODY_H + 100.0);
        assert!(panel_height(pad, false) > pad.y);
    }

    #[test]
    fn the_knobs_focus_moves_come_first_in_their_normal_order() {
        let cur = vec![Value::F32(0.1), Value::F32(0.2), Value::F32(0.3), Value::F32(0.4), Value::F32(0.5)];
        let o = knob_order(5, &[3, 1], &[], &cur);
        assert_eq!(
            o.iter().map(|k| (k.index, k.moved)).collect::<Vec<_>>(),
            vec![(1, true), (3, true), (0, false), (2, false), (4, false)]
        );
        assert!(o.iter().all(|k| !k.changed), "no dots before the first gesture");
        // Knobs mode (no Focus indices): the normal order, everything counts as moved.
        let plain = knob_order(5, &[], &[], &cur);
        assert_eq!(plain.iter().map(|k| k.index).collect::<Vec<_>>(), vec![0, 1, 2, 3, 4]);
        assert!(plain.iter().all(|k| k.moved));
    }

    #[test]
    fn a_knob_that_differs_from_before_the_first_gesture_is_marked() {
        let originals = vec![(1, Value::F32(0.2)), (3, Value::F32(0.4))];
        let cur = vec![Value::F32(0.1), Value::F32(0.9), Value::F32(0.3), Value::F32(0.4), Value::F32(0.5)];
        let o = knob_order(5, &[1, 3], &originals, &cur);
        let changed: Vec<usize> = o.iter().filter(|k| k.changed).map(|k| k.index).collect();
        assert_eq!(changed, vec![1], "knob 3 is back at its original, knob 0 was never captured");
        // After Revert the originals are gone: no dots.
        assert!(knob_order(5, &[1, 3], &[], &cur).iter().all(|k| !k.changed));
    }

    #[test]
    fn the_saved_choice_beats_the_picture_variable_beats_the_default() {
        // (explicit, TONESTRAND_FOCUS, focus_default) -> Focus?
        assert!(!focus_choice(None, false, false), "Knobs by default");
        assert!(focus_choice(None, false, true), "the setting");
        assert!(focus_choice(None, true, false), "the picture variable");
        assert!(focus_choice(Some(true), false, false), "a saved Focus");
        assert!(!focus_choice(Some(false), true, true), "a saved Knobs beats both");
        assert!(focus_choice(Some(true), false, false));
    }

    #[test]
    fn opening_focus_asks_for_nothing() {
        let inner = inner_rect(pad(), Look::new(false).inset);
        // No pointer, no button: no gesture, so nothing is written, sent or marked dirty.
        assert_eq!(decide(Input::default(), inner, Look::new(false).snap), None);
        // A drag, a click (snapping to a tone) and the buttons each ask for something.
        let p = pos_of(inner, 0.5, 0.5);
        assert!(matches!(decide(Input { dragging: Some(p), ..Default::default() }, inner, 22.0), Some(Gesture::Move(..))));
        let near_corner = pos_of(inner, 1.0, 1.0) + Vec2::new(-4.0, 4.0);
        assert_eq!(decide(Input { clicked: Some(near_corner), ..Default::default() }, inner, 22.0), Some(Gesture::Jump(1.0, 1.0)));
        assert_eq!(decide(Input { centre: true, ..Default::default() }, inner, 22.0), Some(Gesture::Jump(0.0, 0.0)));
        assert_eq!(decide(Input { revert: true, ..Default::default() }, inner, 22.0), Some(Gesture::Revert));
        assert_eq!(decide(Input { undo: true, ..Default::default() }, inner, 22.0), Some(Gesture::Undo));
        assert_eq!(decide(Input { drag_stopped: true, ..Default::default() }, inner, 22.0), Some(Gesture::Release));
    }

    #[test]
    fn the_puck_state_follows_the_rule() {
        let near = Located { x: 0.2, y: -0.4, error: GHOST_MAX_ERROR / 2.0 };
        let far = Located { x: -0.1, y: 0.6, error: GHOST_MAX_ERROR * 5.0 };
        // A gesture's own values: solid, wherever the fit says.
        assert_eq!(shown(true, Some((0.5, 0.5)), Some(far)), Shown::Solid(0.5, 0.5));
        // Not from a gesture: a ghost when it fits, none (set by hand) when it does not.
        assert_eq!(shown(false, Some((0.5, 0.5)), Some(near)), Shown::Ghost(0.2, -0.4));
        assert_eq!(shown(false, None, Some(far)), Shown::Hand(-0.1, 0.6));
        assert_eq!(shown(false, None, None), Shown::Nothing);
    }

    fn step(before: f32, after: f32) -> Step {
        Step { before: vec![(0, Value::F32(before))], after: vec![(0, Value::F32(after))] }
    }

    #[test]
    fn undo_and_redo_walk_the_steps_and_a_new_gesture_drops_the_redo() {
        let mut h = History::default();
        assert!(h.undo().is_none() && h.redo().is_none());
        assert!(h.push(step(0.1, 0.5)));
        assert!(h.push(step(0.5, 0.9)));
        assert!(!h.push(step(0.9, 0.9)), "a gesture that changed nothing is not a step");
        assert_eq!(h.undo(), Some(vec![(0, Value::F32(0.5))]));
        assert_eq!(h.undo(), Some(vec![(0, Value::F32(0.1))]));
        assert!(h.undo().is_none());
        assert_eq!(h.redo(), Some(vec![(0, Value::F32(0.5))]));
        // A new gesture starts a new line: nothing to redo.
        assert!(h.push(step(0.5, 0.7)));
        assert!(h.redo().is_none());
        assert_eq!(h.undo(), Some(vec![(0, Value::F32(0.5))]));
    }

    #[test]
    fn revert_is_a_step_that_can_be_undone() {
        let mut h = History::default();
        h.push(step(0.1, 0.8)); // a gesture
        h.push(step(0.8, 0.1)); // Revert: back to the originals
        assert_eq!(h.undo(), Some(vec![(0, Value::F32(0.8))]), "undoing Revert brings the gesture back");
        assert_eq!(h.redo(), Some(vec![(0, Value::F32(0.1))]));
    }

    #[test]
    fn touch_makes_everything_bigger() {
        let (d, t) = (Look::new(false), Look::new(true));
        assert_eq!((d.snap, d.puck, d.inset, d.fade, d.button_h), (22.0, 11.0, 34.0, 40.0, 26.0));
        assert_eq!((t.snap, t.puck, t.inset, t.fade, t.button_h, t.text), (32.0, 14.0, 40.0, 48.0, 36.0, 14.0));
        assert!(t.halo >= 26.0 && t.halo > t.puck * 1.5);
        assert!(t.below() > d.below());
        // Width: min(640, 55%), at least 260 on touch; the desktop is as before.
        assert_eq!(pad_width(1080.0, true), 1080.0 * 0.55);
        assert_eq!(pad_width(2000.0, true), 640.0);
        assert_eq!(pad_width(400.0, true), 260.0);
        assert_eq!(pad_width(1080.0, false), 324.0);
    }

    #[test]
    fn on_touch_the_pad_height_is_capped_to_the_window() {
        // 1280x800 tablet: the pad would be 0.85 x 518 = 440 tall, the window leaves less.
        let tall = pad_size(1080.0, 800.0, true);
        assert!(tall.y < tall.x * PAD_ASPECT);
        assert!(panel_height(tall, true) + CHROME_H <= 800.0 + 1.0);
        // A huge window: no cap. A tiny one: never under 200.
        assert_eq!(pad_size(1080.0, 3000.0, true).y, (1080.0f32 * 0.55 * PAD_ASPECT).round());
        assert_eq!(pad_size(1080.0, 300.0, true).y, 200.0);
        // The desktop pad is never capped.
        assert_eq!(pad_size(1500.0, 300.0, false).y, (340.0f32 * PAD_ASPECT).round());
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
    fn flush_is_paced_and_always_at_the_end() {
        assert!(should_flush(None, 1.0, false));
        assert!(!should_flush(Some(1.0), 1.02, false));
        assert!(should_flush(Some(1.0), 1.06, false));
        assert!(should_flush(Some(1.0), 1.001, true));
    }

    #[test]
    fn a_tiny_change_is_not_sent_but_the_end_of_a_gesture_always_sends() {
        let sent = Value::F32(0.5);
        assert!(worth_sending(None, &Value::F32(0.5), 1.0, false), "never sent: send");
        assert!(!worth_sending(Some(&sent), &Value::F32(0.5004), 1.0, false), "0.04% of the range");
        assert!(worth_sending(Some(&sent), &Value::F32(0.502), 1.0, false), "0.2% of the range");
        assert!(!worth_sending(Some(&sent), &Value::F32(0.5004), 1.0, false));
        assert!(worth_sending(Some(&sent), &Value::F32(0.5004), 1.0, true), "the end always sends");
        // The range scales the limit: 0.05 of a 0-100 range is small, of a 0-1 range it is big.
        assert!(!worth_sending(Some(&Value::F32(50.0)), &Value::F32(50.05), 100.0, false));
        assert!(worth_sending(Some(&Value::F32(0.5)), &Value::F32(0.55), 1.0, false));
        // A switch is sent when it changes, and not when it is the same.
        assert!(worth_sending(Some(&Value::Bool(false)), &Value::Bool(true), 1.0, false));
        assert!(!worth_sending(Some(&Value::Bool(true)), &Value::Bool(true), 1.0, false));
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
