//! Colours and spacing for the whole window.

use eframe::egui::{self, Color32, CornerRadius, Stroke};

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// Whether the light palette is in use (the dark one is the default).
static LIGHT: AtomicBool = AtomicBool::new(false);
pub fn is_light() -> bool {
    LIGHT.load(Ordering::Relaxed)
}
/// Pick the palette (call apply() afterwards so widgets follow).
pub fn set_light(on: bool) {
    LIGHT.store(on, Ordering::Relaxed);
}

/// One colour of each palette: (dark, light).
fn pick(dark: u32, light: u32) -> Color32 {
    let v = if is_light() { light } else { dark };
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// The base colours: neutral greys with thin borders, dark or light.
pub fn bg() -> Color32 {
    pick(0x101114, 0xeceef3)
}
pub fn panel() -> Color32 {
    pick(0x15161a, 0xf6f7fa)
}
pub fn card_bg() -> Color32 {
    pick(0x1b1d22, 0xffffff)
}
pub fn card_hi() -> Color32 {
    pick(0x25282f, 0xe6e9f0)
}
pub fn line() -> Color32 {
    pick(0x2a2d34, 0xcfd4de)
}
/// The fill of a hovered button.
pub fn hover_fill() -> Color32 {
    pick(0x353c4c, 0xd8deea)
}
pub fn ink() -> Color32 {
    pick(0xe6e9ef, 0x1a1d24)
}
pub fn muted() -> Color32 {
    pick(0x9aa2b6, 0x596176)
}
/// The accent as a fill (a button, the picked tab): on the light palette a deeper shade, so white
/// text sits on it and it stands out from the pale surfaces around it.
pub fn accent_fill() -> Color32 {
    if is_light() {
        let v = ACCENT_RGB.load(Ordering::Relaxed);
        let k = 0.62;
        Color32::from_rgb((((v >> 16) & 0xff) as f32 * k) as u8, (((v >> 8) & 0xff) as f32 * k) as u8, ((v & 0xff) as f32 * k) as u8)
    } else {
        accent()
    }
}

/// Text on the accent fill: dark or white, whichever reads better.
pub fn on_accent() -> Color32 {
    let dark = Color32::from_rgb(0x08, 0x1a, 0x18);
    if contrast(dark, accent_fill()) >= contrast(Color32::WHITE, accent_fill()) {
        dark
    } else {
        Color32::WHITE
    }
}

/// WCAG contrast ratio of two colours (1 to 21).
pub fn contrast(a: Color32, b: Color32) -> f32 {
    let lum = |c: Color32| {
        let f = |v: u8| {
            let v = v as f32 / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * f(c.r()) + 0.7152 * f(c.g()) + 0.0722 * f(c.b())
    };
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}
/// A faint wash over the surface under it: light on dark, dark on light (a hover or a divider).
pub fn wash(alpha: u8) -> Color32 {
    if is_light() {
        Color32::from_black_alpha(alpha)
    } else {
        Color32::from_white_alpha(alpha)
    }
}
/// The colour a palette-independent colour is drawn in on the light palette: darker, so it
/// reads on white.
fn for_palette(c: Color32) -> Color32 {
    if is_light() {
        let k = 0.78;
        Color32::from_rgb((c.r() as f32 * k) as u8, (c.g() as f32 * k) as u8, (c.b() as f32 * k) as u8)
    } else {
        c
    }
}
/// The accent colour, picked in Settings (teal by default).
static ACCENT_RGB: AtomicU32 = AtomicU32::new(0x2ec4b6);
pub const ACCENTS: [(u32, &str); 6] =
    [(0x2ec4b6, "Teal"), (0x4fb3ff, "Blue"), (0xc6e44a, "Lime"), (0xff5d8f, "Pink"), (0xff9f1c, "Orange"), (0xa879ff, "Violet")];
pub fn accent() -> Color32 {
    let v = ACCENT_RGB.load(Ordering::Relaxed);
    for_palette(Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
}
pub fn accent_rgb() -> u32 {
    ACCENT_RGB.load(Ordering::Relaxed)
}
/// Change the accent colour (call apply() afterwards so widgets follow).
pub fn set_accent(rgb: u32) {
    ACCENT_RGB.store(rgb & 0xffffff, Ordering::Relaxed);
}
pub fn orange() -> Color32 {
    for_palette(Color32::from_rgb(0xff, 0x9f, 0x1c))
}
pub fn pink() -> Color32 {
    for_palette(Color32::from_rgb(0xff, 0x5d, 0x8f))
}
pub fn blue() -> Color32 {
    for_palette(Color32::from_rgb(0x3a, 0x86, 0xff))
}
pub fn good() -> Color32 {
    for_palette(Color32::from_rgb(0x4c, 0xd1, 0x7a))
}
pub fn bad() -> Color32 {
    for_palette(Color32::from_rgb(0xff, 0x5c, 0x5c))
}

/// The one place a catalog colour becomes an egui colour.
pub fn rgb(c: crate::catalog::Rgb) -> Color32 {
    Color32::from_rgb(c.0, c.1, c.2)
}

pub fn family_colour(family: &str) -> Color32 {
    for_palette(family_base(family))
}

fn family_base(family: &str) -> Color32 {
    match family {
        "amp" => Color32::from_rgb(0xff, 0x8a, 0x3d),
        "cab" => Color32::from_rgb(0xc7, 0x8b, 0x5a),
        "drive" => Color32::from_rgb(0xff, 0xc1, 0x3d),
        "dynamics" => Color32::from_rgb(0x4c, 0xd1, 0x7a),
        "delay" => Color32::from_rgb(0x2e, 0xc4, 0xb6),
        "reverb" => Color32::from_rgb(0x3a, 0x86, 0xff),
        "modulation" => Color32::from_rgb(0xb3, 0x6b, 0xff),
        "filter" => Color32::from_rgb(0xff, 0x5d, 0x8f),
        "eq" => Color32::from_rgb(0x9a, 0xa5, 0xb8),
        "pitch" => Color32::from_rgb(0xd0, 0x9c, 0xff),
        "volume" => Color32::from_rgb(0x7f, 0x8c, 0xa3),
        "io" => Color32::from_rgb(0x55, 0x5e, 0x72),
        _ => Color32::from_rgb(0x6c, 0x7a, 0x99),
    }
}

/// A stable colour for a model number when its name is not known.
pub fn number_colour(n: i64) -> Color32 {
    const SET: [&str; 9] = ["amp", "drive", "dynamics", "delay", "reverb", "modulation", "filter", "pitch", "cab"];
    family_colour(SET[(n.unsigned_abs() as usize * 7) % SET.len()])
}

pub fn apply(ctx: &egui::Context) {
    let mut v = if is_light() { egui::Visuals::light() } else { egui::Visuals::dark() };
    v.panel_fill = panel();
    v.window_fill = panel();
    v.extreme_bg_color = bg();
    v.faint_bg_color = card_bg();
    v.override_text_color = Some(ink());
    v.selection.bg_fill = accent().linear_multiply(0.45);
    v.selection.stroke = Stroke::new(1.0_f32, accent());
    v.hyperlink_color = accent();
    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = CornerRadius::same(8);
    v.window_stroke = Stroke::new(1.0_f32, line());
    let r = CornerRadius::same(6);
    v.widgets.noninteractive.bg_fill = card_bg();
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, line());
    v.widgets.noninteractive.corner_radius = r;
    v.widgets.inactive.bg_fill = card_hi();
    v.widgets.inactive.weak_bg_fill = card_hi();
    v.widgets.inactive.corner_radius = r;
    v.widgets.hovered.bg_fill = hover_fill();
    v.widgets.hovered.weak_bg_fill = hover_fill();
    v.widgets.hovered.corner_radius = r;
    v.widgets.active.bg_fill = accent().linear_multiply(0.6);
    v.widgets.active.weak_bg_fill = accent().linear_multiply(0.6);
    v.widgets.active.corner_radius = r;
    v.widgets.open.corner_radius = r;
    // Buttons and links show a pointing hand, so what can be clicked is obvious.
    v.interact_cursor = Some(egui::CursorIcon::PointingHand);
    v.popup_shadow =
        egui::epaint::Shadow { offset: [0, 4], blur: 14, spread: 0, color: Color32::from_black_alpha(if is_light() { 50 } else { 110 }) };
    v.window_shadow =
        egui::epaint::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(if is_light() { 60 } else { 120 }) };
    ctx.set_visuals(v);

    ctx.style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(10.0, 5.0);
        s.spacing.interact_size.y = 26.0;
        s.spacing.tooltip_width = 320.0;
        s.interaction.tooltip_delay = 0.25;
        use egui::{FontId, TextStyle};
        s.text_styles.insert(TextStyle::Body, FontId::proportional(14.0));
        s.text_styles.insert(TextStyle::Button, FontId::proportional(14.0));
        s.text_styles.insert(TextStyle::Small, FontId::proportional(11.5));
        s.text_styles.insert(TextStyle::Heading, FontId::proportional(20.0));
        s.text_styles.insert(TextStyle::Monospace, FontId::monospace(13.0));
    });
}

/// The ToneStrand logos (the person's own artwork, in assets/logos).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Logo {
    /// The helix mark from the square logo, for the top bar.
    MarkSquare,
    /// The helix with "ToneStrand" under it.
    WithName,
    /// The square logo, for the window icon.
    Square,
}

pub fn logo_bytes(which: Logo) -> &'static [u8] {
    match which {
        Logo::MarkSquare => include_bytes!("../assets/logos/mark-square.png"),
        Logo::WithName => include_bytes!("../assets/logos/logo-name.png"),
        Logo::Square => include_bytes!("../assets/logos/icon-256.png"),
    }
}

/// A logo as a texture (decoded once).
pub fn logo(ctx: &egui::Context, which: Logo) -> Option<egui::TextureHandle> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<Logo, Option<egui::TextureHandle>>>> = OnceLock::new();
    let mut c = CACHE.get_or_init(|| Mutex::new(HashMap::new())).lock().ok()?;
    c.entry(which)
        .or_insert_with(|| {
            let img = crate::modelinfo::decode(logo_bytes(which))?;
            Some(ctx.load_texture(format!("logo-{}", which as u8), img, egui::TextureOptions::LINEAR))
        })
        .clone()
}

/// The square logo as the window's icon.
pub fn window_icon() -> Option<egui::IconData> {
    let img = crate::modelinfo::decode(logo_bytes(Logo::Square))?;
    let [w, h] = img.size;
    let rgba = img.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
    Some(egui::IconData { rgba, width: w as u32, height: h as u32 })
}

/// A rounded card around some content.
pub fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(card_bg())
        .stroke(Stroke::new(1.0_f32, line()))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// A small upper-case section title.
pub fn section(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text.to_uppercase()).small().strong().color(muted()));
}

/// A small round "?" that explains something when hovered, instead of a line of text on screen.
pub fn help(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
    let hot = resp.hovered();
    let p = ui.painter();
    let c = rect.center();
    if hot {
        p.circle_filled(c, 7.5, accent().linear_multiply(0.25));
    }
    p.circle_stroke(c, 7.0, Stroke::new(1.2_f32, if hot { accent() } else { muted().linear_multiply(0.8) }));
    p.text(
        c + egui::vec2(0.0, 0.5),
        egui::Align2::CENTER_CENTER,
        "?",
        egui::FontId::proportional(10.5),
        if hot { accent() } else { muted() },
    );
    resp.on_hover_ui(|ui| {
        ui.set_max_width(300.0);
        ui.label(text);
    })
}

/// Beside a name that goes to the pedal: how much of its limit is used ("12/16"), and, when
/// the pedal will show it differently (accents lost, odd symbols dropped, cut short), how.
pub fn name_hint(ui: &mut egui::Ui, text: &str, max: usize) {
    let n = text.chars().count();
    ui.label(egui::RichText::new(format!("{n}/{max}")).small().monospace().color(if n >= max { orange() } else { muted() }));
    let shown = crate::session::pedal_name(text, max);
    if shown != crate::session::clean(text) {
        ui.label(egui::RichText::new(format!("reads \"{shown}\" on the pedal")).small().color(muted()));
    }
}

/// A section title with a "?" beside it.
pub fn section_help(ui: &mut egui::Ui, title: &str, text: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        section(ui, title);
        help(ui, text);
    });
}

/// A row of page tabs drawn as one rounded segmented control. Returns the clicked index.
pub fn segmented(ui: &mut egui::Ui, labels: &[&str], current: usize) -> Option<usize> {
    let mut clicked = None;
    egui::Frame::new()
        .fill(panel())
        .stroke(Stroke::new(1.0_f32, line()))
        .corner_radius(CornerRadius::same(9))
        .inner_margin(egui::Margin::same(3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (i, l) in labels.iter().enumerate() {
                    let on = i == current;
                    let text = egui::RichText::new(*l).size(14.5).strong().color(if on { on_accent() } else { muted() });
                    let b = egui::Button::new(text)
                        .fill(if on { accent_fill() } else { Color32::TRANSPARENT })
                        .stroke(Stroke::NONE)
                        .corner_radius(CornerRadius::same(7))
                        .min_size(egui::vec2(78.0, 26.0));
                    if ui.add(b).clicked() && !on {
                        clicked = Some(i);
                    }
                }
            });
        });
    clicked
}

/// A rounded coloured tag.
pub fn pill(ui: &mut egui::Ui, text: &str, colour: Color32) {
    egui::Frame::new()
        .fill(colour.linear_multiply(0.18))
        .stroke(Stroke::new(1.0_f32, colour.linear_multiply(0.7)))
        .corner_radius(CornerRadius::same(9))
        .inner_margin(egui::Margin::symmetric(8, 2))
        .show(ui, |ui| {
            let shown = if is_light() {
                Color32::from_rgb(colour.r() / 2, colour.g() / 2, colour.b() / 2)
            } else {
                Color32::from_rgb(colour.r() / 2 + 128, colour.g() / 2 + 128, colour.b() / 2 + 128)
            };
            ui.label(egui::RichText::new(text).small().strong().color(shown));
        });
}

pub fn primary_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(text).strong().color(on_accent())).fill(accent_fill()).corner_radius(CornerRadius::same(6)),
    )
}

/// A one-line search box with a visible frame (egui's own frame vanishes on this background).
pub fn search_box(ui: &mut egui::Ui, text: &mut String, hint: &str, width: f32) -> egui::Response {
    egui::Frame::new()
        .fill(bg())
        .stroke(Stroke::new(1.0_f32, line()))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(8, 3))
        .show(ui, |ui| ui.add(egui::TextEdit::singleline(text).frame(false).hint_text(hint).desired_width(width)))
        .inner
}

/// A small painted folder (the font's folder symbol draws as an unclear speck).
pub fn folder_icon(ui: &mut egui::Ui, colour: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 11.0), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(egui::Rect::from_min_size(rect.min, egui::vec2(6.0, 4.0)), CornerRadius::same(1), colour);
    p.rect_filled(egui::Rect::from_min_max(rect.min + egui::vec2(0.0, 2.0), rect.max), CornerRadius::same(2), colour);
}

/// Whether a theme setting ("dark", "light", "system") means the light palette now.
pub fn wants_light(setting: &str, ctx: &egui::Context) -> bool {
    match setting {
        "light" => true,
        "system" => ctx.system_theme() == Some(egui::Theme::Light),
        _ => false,
    }
}

/// Whether the person is moving around with the keyboard (set by Tab or an arrow key, cleared by a mouse click):
/// a focus ring is drawn only then, so clicking does not leave rings behind.
pub fn track_keyboard_nav(ctx: &egui::Context) {
    use egui::Key;
    let (keys, click) = ctx.input(|i| {
        (
            [Key::Tab, Key::ArrowUp, Key::ArrowDown, Key::ArrowLeft, Key::ArrowRight, Key::Home, Key::End]
                .iter()
                .any(|k| i.key_pressed(*k)),
            i.pointer.any_pressed(),
        )
    });
    let id = egui::Id::new("keyboard-nav");
    if click {
        ctx.data_mut(|d| d.insert_temp(id, false));
    } else if keys {
        ctx.data_mut(|d| d.insert_temp(id, true));
    }
}

/// Whether the last thing the person did was press a navigation key.
pub fn keyboard_nav(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(egui::Id::new("keyboard-nav"))).unwrap_or(false)
}

/// A ring around a widget that is drawn by hand, when it has the keyboard focus and the person is using the
/// keyboard. Widgets egui draws itself show their own focus.
pub fn focus_ring(ui: &egui::Ui, resp: &egui::Response, radius: u8) {
    if resp.has_focus() && keyboard_nav(ui.ctx()) {
        ui.painter().rect_stroke(
            resp.rect.expand(2.0),
            CornerRadius::same(radius),
            Stroke::new(2.0_f32, accent()),
            egui::StrokeKind::Outside,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_reads_on_every_surface_of_both_palettes() {
        for light in [false, true] {
            set_light(light);
            for surface in [bg(), panel(), card_bg(), card_hi()] {
                assert!(contrast(ink(), surface) >= 7.0, "ink on a surface, light={light}");
                assert!(contrast(muted(), surface) >= 4.5, "muted on a surface, light={light}");
            }
            // Text on the accent colour (a primary button, the picked tab), for every accent.
            for (rgb, name) in ACCENTS {
                set_accent(rgb);
                assert!(contrast(on_accent(), accent_fill()) >= 4.5, "{name} accent, light={light}");
            }
        }
        set_light(false);
        set_accent(0x2ec4b6);
    }
}
