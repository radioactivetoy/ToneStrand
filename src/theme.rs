//! Colours and spacing for the whole window.

use eframe::egui::{self, Color32, CornerRadius, Stroke};

/// The base colours. The Studio look uses the mockup's darker, neutral greys with thin
/// borders; the classic look keeps the bluer greys it had.
fn studio_palette() -> bool {
    crate::chain::studio()
}
pub fn bg() -> Color32 {
    if studio_palette() { Color32::from_rgb(0x10, 0x11, 0x14) } else { Color32::from_rgb(0x12, 0x14, 0x1a) }
}
pub fn panel() -> Color32 {
    if studio_palette() { Color32::from_rgb(0x15, 0x16, 0x1a) } else { Color32::from_rgb(0x19, 0x1c, 0x24) }
}
pub fn card_bg() -> Color32 {
    if studio_palette() { Color32::from_rgb(0x1b, 0x1d, 0x22) } else { Color32::from_rgb(0x21, 0x25, 0x30) }
}
pub fn card_hi() -> Color32 {
    if studio_palette() { Color32::from_rgb(0x25, 0x28, 0x2f) } else { Color32::from_rgb(0x2a, 0x2f, 0x3c) }
}
pub fn line() -> Color32 {
    if studio_palette() { Color32::from_rgb(0x2a, 0x2d, 0x34) } else { Color32::from_rgb(0x30, 0x35, 0x44) }
}
pub const TEXT: Color32 = Color32::from_rgb(0xe6, 0xe9, 0xef);
pub const MUTED: Color32 = Color32::from_rgb(0x8b, 0x93, 0xa7);
/// The accent colour, picked in the top bar (teal by default).
static ACCENT_RGB: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x2ec4b6);
pub const ACCENTS: [(u32, &str); 6] = [(0x2ec4b6, "Teal"), (0x4fb3ff, "Blue"), (0xc6e44a, "Lime"), (0xff5d8f, "Pink"), (0xff9f1c, "Orange"), (0xa879ff, "Violet")];
pub fn accent() -> Color32 {
    let v = ACCENT_RGB.load(std::sync::atomic::Ordering::Relaxed);
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}
pub fn accent_rgb() -> u32 {
    ACCENT_RGB.load(std::sync::atomic::Ordering::Relaxed)
}
/// Change the accent colour (call apply() afterwards so widgets follow).
pub fn set_accent(rgb: u32) {
    ACCENT_RGB.store(rgb & 0xffffff, std::sync::atomic::Ordering::Relaxed);
}
pub const ORANGE: Color32 = Color32::from_rgb(0xff, 0x9f, 0x1c);
pub const PINK: Color32 = Color32::from_rgb(0xff, 0x5d, 0x8f);
pub const BLUE: Color32 = Color32::from_rgb(0x3a, 0x86, 0xff);
pub const GOOD: Color32 = Color32::from_rgb(0x4c, 0xd1, 0x7a);
pub const BAD: Color32 = Color32::from_rgb(0xff, 0x5c, 0x5c);

pub fn family_colour(family: &str) -> Color32 {
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
    let mut v = egui::Visuals::dark();
    v.panel_fill = panel();
    v.window_fill = panel();
    v.extreme_bg_color = bg();
    v.faint_bg_color = card_bg();
    v.override_text_color = Some(TEXT);
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
    v.widgets.hovered.bg_fill = Color32::from_rgb(0x35, 0x3c, 0x4c);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(0x35, 0x3c, 0x4c);
    v.widgets.hovered.corner_radius = r;
    v.widgets.active.bg_fill = accent().linear_multiply(0.6);
    v.widgets.active.weak_bg_fill = accent().linear_multiply(0.6);
    v.widgets.active.corner_radius = r;
    v.widgets.open.corner_radius = r;
    // Buttons and links show a pointing hand, so what can be clicked is obvious.
    v.interact_cursor = Some(egui::CursorIcon::PointingHand);
    v.popup_shadow = egui::epaint::Shadow { offset: [0, 4], blur: 14, spread: 0, color: Color32::from_black_alpha(110) };
    v.window_shadow = egui::epaint::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(120) };
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
    ui.label(egui::RichText::new(text.to_uppercase()).small().strong().color(MUTED));
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
    p.circle_stroke(c, 7.0, Stroke::new(1.2_f32, if hot { accent() } else { MUTED.linear_multiply(0.8) }));
    p.text(c + egui::vec2(0.0, 0.5), egui::Align2::CENTER_CENTER, "?", egui::FontId::proportional(10.5), if hot { accent() } else { MUTED });
    resp.on_hover_ui(|ui| {
        ui.set_max_width(300.0);
        ui.label(text);
    })
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
                    let text = egui::RichText::new(*l).size(14.5).strong().color(if on { Color32::from_rgb(0x08, 0x1a, 0x18) } else { MUTED });
                    let b = egui::Button::new(text)
                        .fill(if on { accent() } else { Color32::TRANSPARENT })
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
            ui.label(egui::RichText::new(text).small().strong().color(Color32::from_rgb(colour.r() / 2 + 128, colour.g() / 2 + 128, colour.b() / 2 + 128)));
        });
}

pub fn primary_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(text).strong().color(Color32::from_rgb(0x08, 0x1a, 0x18)))
            .fill(accent())
            .corner_radius(CornerRadius::same(6)),
    )
}
