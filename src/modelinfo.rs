//! Model photos and descriptions (from Line 6's firmware
//! release notes): what each amp, cab and effect is based on, a short
//! description, notable players, and its picture.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use eframe::egui::{self, RichText, Vec2};
use serde::Deserialize;

use crate::theme::*;

include!(concat!(env!("OUT_DIR"), "/photos.rs"));

const DATA: &str = include_str!("../assets/models/models.json");

#[derive(Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct Info {
    pub image: Option<String>,
    pub based_on: Option<String>,
    pub description: Option<String>,
    pub notable_users: Option<String>,
}

#[derive(Deserialize)]
struct File {
    models: HashMap<String, Info>,
}

fn table() -> &'static HashMap<String, Info> {
    static T: OnceLock<HashMap<String, Info>> = OnceLock::new();
    T.get_or_init(|| serde_json::from_str::<File>(DATA).map(|f| f.models).unwrap_or_default())
}

/// The entry for a model symbol (`HD2_AmpUSDeluxeNrm`, `...Mono`, `...Stereo`, `...WithPan`).
pub fn info(symbol: &str) -> Option<&'static Info> {
    if crate::nam::is_nam_symbol(symbol) {
        return None; // the NAM A2 Lite block is not the stock model it replaces
    }
    let t = table();
    let mut s = symbol.to_owned();
    for _ in 0..3 {
        if let Some(i) = t.get(&s) {
            return Some(i);
        }
        let next = s.strip_suffix("Mono").or_else(|| s.strip_suffix("Stereo")).or_else(|| s.strip_suffix("WithPan")).map(str::to_owned);
        match next {
            Some(n) => s = n,
            None => break,
        }
    }
    None
}

fn photo_bytes(name: &str) -> Option<&'static [u8]> {
    // Brackets in file names were stored as underscores.
    let stored = name.replace(['(', ')'], "_");
    PHOTOS.iter().find(|(n, _)| *n == stored).map(|(_, b)| *b)
}

pub fn decode(bytes: &[u8]) -> Option<egui::ColorImage> {
    let mut d = png::Decoder::new(bytes);
    d.set_transformations(png::Transformations::normalize_to_color8());
    let mut r = d.read_info().ok()?;
    let mut buf = vec![0; r.output_buffer_size()];
    let f = r.next_frame(&mut buf).ok()?;
    let px = &buf[..f.buffer_size()];
    let (w, h) = (f.width as usize, f.height as usize);
    let rgba: Vec<u8> = match f.color_type {
        png::ColorType::Rgba => px.to_vec(),
        png::ColorType::Rgb => px.chunks(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => px.chunks(2).flat_map(|c| [c[0], c[0], c[0], c[1]]).collect(),
        png::ColorType::Grayscale => px.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        _ => return None,
    };
    (rgba.len() == w * h * 4).then(|| egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba))
}

/// The photo of a model as a texture (decoded once).
pub fn photo(ctx: &egui::Context, symbol: &str) -> Option<egui::TextureHandle> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<egui::TextureHandle>>>> = OnceLock::new();
    let name = info(symbol)?.image.clone()?;
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut c = cache.lock().ok()?;
    c.entry(name.clone())
        .or_insert_with(|| {
            let img = decode(photo_bytes(&name)?)?;
            Some(ctx.load_texture(format!("model-{name}"), img, egui::TextureOptions::LINEAR))
        })
        .clone()
}

fn image(ui: &mut egui::Ui, tex: &egui::TextureHandle, max: Vec2) {
    let size = tex.size_vec2();
    let scale = (max.x / size.x).min(max.y / size.y).min(1.5);
    ui.add(egui::Image::new(tex).fit_to_exact_size(size * scale));
}

/// "Based on ..." for a model, when known.
pub fn based_on(symbol: &str) -> Option<String> {
    info(symbol).and_then(|i| i.based_on.clone())
}

/// Whether there is a photo or a description to show beside the knobs (for an amp + cab,
/// the cab's photo counts too).
pub fn has_side(ui: &egui::Ui, symbol: &str, cab: Option<&str>) -> bool {
    info(symbol).is_some_and(|i| i.description.is_some() || photo(ui.ctx(), symbol).is_some())
        || cab.is_some_and(|c| photo(ui.ctx(), c).is_some())
}

/// A narrow column beside the knobs: the photo (with the cab's photo under it for an amp + cab
/// or a dual cab), a few lines about the model, and more on demand.
pub fn side(ui: &mut egui::Ui, symbol: &str, width: f32, cab: Option<&str>) {
    let framed = |ui: &mut egui::Ui, t: &egui::TextureHandle, h: f32| {
        egui::Frame::new().fill(bg()).corner_radius(egui::CornerRadius::same(8)).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
            ui.set_width(width - 12.0);
            ui.vertical_centered(|ui| image(ui, t, Vec2::new(width - 24.0, h)));
        });
        ui.add_space(6.0);
    };
    let cab_photo = cab.and_then(|c| photo(ui.ctx(), c));
    if let Some(t) = photo(ui.ctx(), symbol) {
        framed(ui, &t, if cab_photo.is_some() { 90.0 } else { 110.0 });
    }
    if let Some(t) = &cab_photo {
        framed(ui, t, 80.0);
    }
    let Some(i) = info(symbol) else { return };
    if let Some(d) = i.description.as_deref().and_then(|d| d.split("\n\n").next()) {
        let short: String = if d.chars().count() > 150 { d.chars().take(150).collect::<String>() + "…" } else { d.to_owned() };
        ui.add(egui::Label::new(RichText::new(short).small().color(MUTED)).wrap());
        egui::CollapsingHeader::new(RichText::new("More").small()).id_salt(("about", symbol)).default_open(false).show(ui, |ui| {
            if let Some(d) = &i.description {
                ui.label(RichText::new(d).small());
            }
            if let Some(n) = &i.notable_users {
                ui.add_space(4.0);
                ui.label(RichText::new("Notable players").small().strong().color(MUTED));
                ui.label(RichText::new(n).small());
            }
            ui.label(RichText::new("Source: Line 6 release notes").small().color(MUTED));
        });
    }
}

/// A tooltip for the effect picker: photo, based-on line and the first lines of the description.
pub fn hover(ui: &mut egui::Ui, symbol: &str) {
    let Some(i) = info(symbol) else { return };
    if let Some(t) = photo(ui.ctx(), symbol) {
        image(ui, &t, Vec2::new(220.0, 120.0));
    }
    if let Some(b) = &i.based_on {
        ui.label(RichText::new(format!("Based on {b}")).strong());
    }
    if let Some(d) = i.description.as_deref().and_then(|d| d.split("\n\n").next()) {
        let short: String = if d.chars().count() > 260 { d.chars().take(260).collect::<String>() + "…" } else { d.to_owned() };
        ui.label(RichText::new(short).small().color(MUTED));
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn data_and_photos_load() {
        let t = super::table();
        assert!(t.len() > 500, "models: {}", t.len());
        let i = super::info("HD2_AmpSoupPro").expect("Soup Pro");
        assert!(i.based_on.as_deref().unwrap_or("").contains("Supro"));
        assert!(super::info("HD2_AmpSoupProMono").is_some());
        let img = i.image.as_deref().unwrap();
        assert!(super::decode(super::photo_bytes(img).unwrap()).is_some());
        let found = t.values().filter_map(|i| i.image.as_deref()).filter(|n| super::photo_bytes(n).is_some()).count();
        assert!(found > 400, "photos found: {found}");
    }
}
