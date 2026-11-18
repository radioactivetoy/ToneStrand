//! Effect icons, cut from the sheet the person added (assets/block Images/Blocks_stadium.png):
//! - the line icon heading each column names a category (Distortion, Delay, Reverb...);
//! - the rounded tiles under it name its kinds (Fuzz, Boost, Overdrive...; Analog, Tape...).
//!
//! Each is cut once and kept as a texture. Line icons get their black turned into
//! transparency; tiles keep their own dark fill and lose only the black around them.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use eframe::egui::{self, Color32, ColorImage};

const SHEET: &[u8] = include_bytes!("../assets/block Images/Blocks_stadium.png");

/// Top of each tile row (tiles are about 55 px square).
const ROWS: [usize; 7] = [139, 228, 316, 404, 491, 579, 667];
const TILE: usize = 58;
/// Header line icons: centre x of each column, and the band they sit in.
const HEAD_TOP: usize = 42;
const HEAD: usize = 60;

/// A column of the sheet: its heading's centre, its tiles' left edge, and the kinds in its rows.
struct Column {
    centre: usize,
    left: Option<usize>,
    kinds: &'static [&'static str],
}

const COLUMNS: [(&str, Column); 19] = [
    ("input", Column { centre: 67, left: None, kinds: &[] }),
    ("none", Column { centre: 144, left: None, kinds: &[] }),
    ("amp", Column { centre: 223, left: Some(197), kinds: &["guitar amp", "bass amp", "amp clone", "amp+cab clone"] }),
    ("preamp", Column { centre: 305, left: Some(279), kinds: &["guitar pre", "bass pre", "mic pre", "clone"] }),
    ("cab", Column { centre: 388, left: Some(361), kinds: &["1x cab", "1 cab ir", "2x cab"] }),
    (
        "distortion",
        Column { centre: 466, left: Some(439), kinds: &["distortion", "boost", "overdrive", "fuzz", "bitcrusher", "megaphone", "clone"] },
    ),
    ("delay", Column { centre: 543, left: Some(516), kinds: &["analog", "digital", "tape", "multitap", "sustain", "special"] }),
    ("reverb", Column { centre: 621, left: Some(594), kinds: &["spaces", "spring", "plate", "special"] }),
    ("modulation", Column { centre: 699, left: Some(672), kinds: &["phaser", "tremolo", "flanger", "chorus", "rotary", "ring mod"] }),
    ("dynamics", Column { centre: 776, left: Some(749), kinds: &["compress", "3-band", "gate", "swell", "feedbacker"] }),
    ("eq", Column { centre: 854, left: Some(827), kinds: &["parametric", "graphic", "tilt", "acoustic"] }),
    ("pitch", Column { centre: 932, left: Some(905), kinds: &["pitch", "synth", "wham", "generator", "12-string"] }),
    ("wah", Column { centre: 1010, left: Some(982), kinds: &["wah", "filter"] }),
    ("volume", Column { centre: 1087, left: Some(1060), kinds: &["vol pedal", "gain", "pan"] }),
    ("looper", Column { centre: 1165, left: None, kinds: &[] }),
    ("fx loop", Column { centre: 1242, left: None, kinds: &[] }),
    ("split", Column { centre: 1319, left: None, kinds: &[] }),
    ("merge", Column { centre: 1388, left: None, kinds: &[] }),
    ("output", Column { centre: 1465, left: None, kinds: &[] }),
];

/// The sheet column for a category name as the catalog gives it.
fn column_for(category: &str) -> Option<usize> {
    let c = category.to_lowercase();
    let key = if c.contains("preamp") {
        "preamp"
    } else if c.contains("amp") || c == "nam" {
        "amp"
    } else if c.contains("cab") || c.contains("impulse") || c == "ir" {
        "cab"
    } else if c.contains("distort") || c.contains("drive") || c.contains("fuzz") {
        "distortion"
    } else if c.contains("delay") {
        "delay"
    } else if c.contains("reverb") {
        "reverb"
    } else if c.contains("modulation") {
        "modulation"
    } else if c.contains("dynamic") {
        "dynamics"
    } else if c == "eq" || c.starts_with("eq") {
        "eq"
    } else if c.contains("pitch") || c.contains("synth") {
        "pitch"
    } else if c.contains("wah") || c.contains("filter") {
        "wah"
    } else if c.contains("vol") || c.contains("pan") {
        "volume"
    } else if c.contains("looper") {
        "looper"
    } else if c.contains("send") || c.contains("return") || c.contains("loop") {
        "fx loop"
    } else if c.contains("split") {
        "split"
    } else if c.contains("merge") || c.contains("mixer") {
        "merge"
    } else if c.contains("input") {
        "input"
    } else if c.contains("output") {
        "output"
    } else {
        return None;
    };
    COLUMNS.iter().position(|(k, _)| *k == key)
}

/// Which row of its column a model's kind is in, from words in its name and symbol.
fn row_for(column: &str, category: &str, symbol: &str, name: &str) -> usize {
    let s = format!("{} {} {}", symbol, name, category).to_lowercase();
    let has = |w: &[&str]| w.iter().any(|x| s.contains(x));
    match column {
        "amp" => {
            if has(&["bass", "ampeg", "svt", "cali 400", "agua"]) {
                1
            } else {
                0
            }
        }
        "preamp" => {
            if has(&["mic", "vintage pre", "studio"]) {
                2
            } else if has(&["bass", "ampeg", "svt"]) {
                1
            } else {
                0
            }
        }
        "cab" => {
            if has(&["withpan", "dual"]) {
                2
            } else if has(&["impulse", " ir", "ir "]) {
                1
            } else {
                0
            }
        }
        "distortion" => {
            if has(&["fuzz", "fuzzy", "triangle", "ram's", "rams", "arbitrator", "bighead", "octave"]) {
                3
            } else if has(&["boost", "kinky", "clean", "comp ", "tycoctavia"]) {
                1
            } else if has(&["bitcrush", "crush"]) {
                4
            } else if has(&["megaphone"]) {
                5
            } else if has(&["screamer", "drive", "scream", "808", "klon", "minotaur", "teemah", "compulsive", "valve"]) {
                2
            } else {
                0
            }
        }
        "delay" => {
            if has(&["tape", "echo", "cosmos"]) {
                2
            } else if has(&["analog", "bucket", "dm-2", "adriatic", "elephant", "bbd"]) {
                0
            } else if has(&["multi", "pass", "pattern", "ducked"]) {
                3
            } else if has(&["swell", "sustain", "harmony", "glitch", "reverse", "pitch", "particle", "crisscross", "dynamic"]) {
                5
            } else {
                1
            }
        }
        "reverb" => {
            if has(&["plate"]) {
                2
            } else if has(&["spring", "63 "]) {
                1
            } else if has(&[
                "shimmer",
                "particle",
                "ganymede",
                "glitz",
                "octo",
                "searchlight",
                "plateaux",
                "double tank",
                "cave",
                "dynamic",
            ]) {
                3
            } else {
                0
            }
        }
        "modulation" => {
            if has(&["phaser", "phase", "script", "vibe", "ubiquitous"]) {
                0
            } else if has(&["trem", "pattern", "bias", "harmonic", "opto"]) {
                1
            } else if has(&["flang", "jet"]) {
                2
            } else if has(&["rotary", "rotor", "122", "145", "leslie", "vibe"]) {
                4
            } else if has(&["ring"]) {
                5
            } else {
                3
            }
        }
        "dynamics" => {
            if has(&["gate", "noise", "horizon", "hard gate"]) {
                2
            } else if has(&["swell", "autoswell"]) {
                3
            } else if has(&["feedback"]) {
                4
            } else if has(&["3-band", "3band", "multiband", "multi band"]) {
                1
            } else {
                0
            }
        }
        "eq" => {
            if has(&["graphic", "10 band", "10band", "band eq", "mesa"]) {
                1
            } else if has(&["tilt", "low/high", "lowhigh"]) {
                2
            } else if has(&["acoustic", "sim"]) {
                3
            } else {
                0
            }
        }
        "pitch" => {
            if has(&["synth", "3 osc", "3osc", "attack", "bit", "buzz", "seismik", "tycoctavia"]) {
                1
            } else if has(&["wham", "whammy"]) {
                2
            } else if has(&["12", "twelve"]) {
                4
            } else if has(&["generator", "tone gen"]) {
                3
            } else {
                0
            }
        }
        "wah" => {
            if has(&["filter", "mutant", "mystery", "autofilter", "asheville", "slow"]) && !has(&["wah"]) {
                1
            } else {
                0
            }
        }
        "volume" => {
            if has(&["pan", "stereo width", "image"]) {
                2
            } else if has(&["gain"]) {
                1
            } else {
                0
            }
        }
        _ => 0,
    }
}

fn sheet() -> Option<&'static ColorImage> {
    static SHEET_IMG: OnceLock<Option<ColorImage>> = OnceLock::new();
    SHEET_IMG.get_or_init(|| crate::modelinfo::decode(SHEET)).as_ref()
}

fn crop(img: &ColorImage, x0: usize, y0: usize, w: usize, h: usize) -> ColorImage {
    let [iw, ih] = img.size;
    let mut px = Vec::with_capacity(w * h);
    for y in y0..y0 + h {
        for x in x0..x0 + w {
            px.push(if x < iw && y < ih { img.pixels[y * iw + x] } else { Color32::BLACK });
        }
    }
    ColorImage { size: [w, h], pixels: px }
}

/// A line icon: brightness becomes opacity, so it sits on any background.
fn line_icon(img: ColorImage) -> ColorImage {
    let pixels = img
        .pixels
        .iter()
        .map(|c| {
            let [r, g, b, _] = c.to_srgba_unmultiplied();
            let m = r.max(g).max(b);
            if m < 24 {
                return Color32::TRANSPARENT;
            }
            let k = 255.0 / m as f32;
            Color32::from_rgba_unmultiplied((r as f32 * k) as u8, (g as f32 * k) as u8, (b as f32 * k) as u8, m)
        })
        .collect();
    ColorImage { size: img.size, pixels }
}

/// A tile: the black around it (reached from the edges without crossing its border) becomes transparent.
fn tile_icon(mut img: ColorImage) -> ColorImage {
    let [w, h] = img.size;
    let dark = |c: Color32| {
        let [r, g, b, _] = c.to_srgba_unmultiplied();
        r.max(g).max(b) < 14
    };
    let mut seen = vec![false; w * h];
    let mut stack: Vec<usize> = Vec::new();
    for x in 0..w {
        stack.push(x);
        stack.push((h - 1) * w + x);
    }
    for y in 0..h {
        stack.push(y * w);
        stack.push(y * w + w - 1);
    }
    while let Some(i) = stack.pop() {
        if seen[i] || !dark(img.pixels[i]) {
            continue;
        }
        seen[i] = true;
        img.pixels[i] = Color32::TRANSPARENT;
        let (x, y) = (i % w, i / w);
        if x > 0 {
            stack.push(i - 1);
        }
        if x + 1 < w {
            stack.push(i + 1);
        }
        if y > 0 {
            stack.push(i - w);
        }
        if y + 1 < h {
            stack.push(i + w);
        }
    }
    soften_edge(&mut img);
    img
}

/// The sheet's tiles were drawn on black, so the pixels along a tile's outer edge are the tile's border colour
/// mixed with that black. Left as they are they show as a dark band around the tile on a coloured block. Turn
/// them into the border colour with a matching transparency (the mix undone), two pixels deep.
fn soften_edge(img: &mut ColorImage) {
    let [w, h] = img.size;
    let at = |x: usize, y: usize| y * w + x;
    let clear: Vec<bool> = img.pixels.iter().map(|p| p.a() == 0).collect();
    // How far each opaque pixel is from the transparent outside (1 = touching it), up to 2.
    let mut ring = vec![0u8; w * h];
    for depth in 1..=2u8 {
        for y in 0..h {
            for x in 0..w {
                let i = at(x, y);
                if clear[i] || ring[i] != 0 {
                    continue;
                }
                let touches = |dx: i32, dy: i32| {
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    nx >= 0
                        && ny >= 0
                        && (nx as usize) < w
                        && (ny as usize) < h
                        && (clear[at(nx as usize, ny as usize)] || (depth == 2 && ring[at(nx as usize, ny as usize)] == 1))
                };
                if [(-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (1, -1), (-1, 1), (1, 1)].iter().any(|(dx, dy)| touches(*dx, *dy)) {
                    ring[i] = depth;
                }
            }
        }
    }
    let bright = |p: Color32| {
        let [r, g, b, _] = p.to_srgba_unmultiplied();
        r.max(g).max(b) as f32
    };
    let src = img.pixels.clone();
    for y in 0..h {
        for x in 0..w {
            let i = at(x, y);
            if ring[i] == 0 {
                continue;
            }
            // The border colour: the brightest pixel a few steps in that is not itself on the edge.
            let mut reference = 0.0f32;
            for dy in -3i32..=3 {
                for dx in -3i32..=3 {
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx < 0 || ny < 0 || nx as usize >= w || ny as usize >= h {
                        continue;
                    }
                    let j = at(nx as usize, ny as usize);
                    if !clear[j] && ring[j] == 0 {
                        reference = reference.max(bright(src[j]));
                    }
                }
            }
            let own = bright(src[i]);
            if reference < 40.0 || own >= reference {
                continue;
            }
            let a = (own / reference).clamp(0.0, 1.0);
            let [r, g, b, _] = src[i].to_srgba_unmultiplied();
            let up = |c: u8| (c as f32 / a.max(0.05)).min(255.0) as u8;
            img.pixels[i] = Color32::from_rgba_unmultiplied(up(r), up(g), up(b), (a * 255.0) as u8);
        }
    }
}

fn texture(ctx: &egui::Context, key: String, make: impl FnOnce() -> Option<ColorImage>) -> Option<egui::TextureHandle> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<egui::TextureHandle>>>> = OnceLock::new();
    let mut c = CACHE.get_or_init(|| Mutex::new(HashMap::new())).lock().ok()?;
    c.entry(key.clone()).or_insert_with(|| make().map(|img| ctx.load_texture(key, img, egui::TextureOptions::LINEAR))).clone()
}

/// The line icon heading a category's column.
pub fn for_category(ctx: &egui::Context, category: &str) -> Option<egui::TextureHandle> {
    let col = column_for(category)?;
    let centre = COLUMNS[col].1.centre;
    texture(ctx, format!("fx-head-{col}"), || Some(line_icon(crop(sheet()?, centre - HEAD / 2, HEAD_TOP, HEAD, HEAD))))
}

/// The tile for a model: its category's column, the row of its kind (the first when unsure).
/// Categories without tiles (loopers, splits...) fall back to their line icon.
pub fn for_model(ctx: &egui::Context, category: &str, symbol: &str, name: &str) -> Option<egui::TextureHandle> {
    let col = column_for(category)?;
    let (key, c) = &COLUMNS[col];
    let Some(left) = c.left else { return for_category(ctx, category) };
    let row = row_for(key, category, symbol, name).min(c.kinds.len().saturating_sub(1)).min(ROWS.len() - 1);
    texture(ctx, format!("fx-tile-{col}-{row}"), || Some(tile_icon(crop(sheet()?, left - 1, ROWS[row] - 2, TILE, TILE))))
}

/// A tile picked by column and row (for things that aren't models: an IR, a NAM capture).
fn tile_at(ctx: &egui::Context, key: &str, row: usize) -> Option<egui::TextureHandle> {
    let col = COLUMNS.iter().position(|(k, _)| *k == key)?;
    let left = COLUMNS[col].1.left?;
    texture(ctx, format!("fx-tile-{col}-{row}"), || Some(tile_icon(crop(sheet()?, left - 1, ROWS[row] - 2, TILE, TILE))))
}

/// The "1 Cab IR" tile, for impulse responses.
pub fn ir(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    tile_at(ctx, "cab", 1)
}

/// The "Amp Clone" tile, for NAM captures.
pub fn capture(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    tile_at(ctx, "amp", 2)
}

/// A category's line icon in a square of `size` points. Returns whether it drew.
pub fn show(ui: &mut egui::Ui, category: &str, size: f32) -> bool {
    match for_category(ui.ctx(), category) {
        Some(t) => {
            ui.add(egui::Image::new(&t).fit_to_exact_size(egui::Vec2::splat(size)));
            true
        }
        None => false,
    }
}

/// Whether a category label is the amp-with-its-cab kind (see `kind_label`).
pub fn is_amp_cab(category: &str) -> bool {
    category.eq_ignore_ascii_case("Amp+Cab")
}

/// The type to show for a block: an amp carrying its own cab is "Amp+Cab", a cab carrying a
/// second cab is "Dual Cab"; anything else keeps its category.
pub fn kind_label(category: &str, paired: bool) -> String {
    let c = category.to_lowercase();
    if paired && c.contains("cab") && !c.contains("amp") {
        "Dual Cab".into()
    } else if paired && c.contains("amp") && !c.contains("preamp") {
        "Amp+Cab".into()
    } else {
        category.to_owned()
    }
}

/// Paint a model's tile into `rect`. An amp + cab gets the amp's tile with a small cab tile on
/// its lower right corner, so it reads differently from a plain amp. Returns whether it drew.
pub fn paint_model(
    p: &egui::Painter,
    ctx: &egui::Context,
    rect: egui::Rect,
    category: &str,
    symbol: &str,
    name: &str,
    tint: Color32,
) -> bool {
    let uv = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::Pos2::new(1.0, 1.0));
    if is_amp_cab(category) {
        let Some(amp) = for_model(ctx, "Amp", symbol, name) else { return false };
        p.image(amp.id(), rect, uv, tint);
        if let Some(cab) = tile_at(ctx, "cab", 0) {
            let s = rect.width() * 0.56;
            let badge = egui::Rect::from_min_size(rect.right_bottom() - egui::vec2(s * 0.82, s * 0.82), egui::vec2(s, s));
            p.circle_filled(badge.center(), s * 0.5, Color32::from_black_alpha(170));
            p.image(cab.id(), badge, uv, tint);
        }
        return true;
    }
    match for_model(ctx, category, symbol, name) {
        Some(t) => {
            p.image(t.id(), rect, uv, tint);
            true
        }
        None => false,
    }
}

/// A model's tile in a square of `size` points. Returns whether it drew.
pub fn show_model(ui: &mut egui::Ui, category: &str, symbol: &str, name: &str, size: f32) -> bool {
    if for_model(ui.ctx(), if is_amp_cab(category) { "Amp" } else { category }, symbol, name).is_none() {
        return false;
    }
    let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(size), egui::Sense::hover());
    // The icons are drawn for a dark ground: on the light palette they get a dark tile.
    if crate::theme::is_light() {
        ui.painter().rect_filled(rect.expand(1.5), egui::CornerRadius::same(8), Color32::from_rgb(0x20, 0x22, 0x28));
    }
    paint_model(ui.painter(), ui.ctx(), rect, category, symbol, name, Color32::WHITE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sheet_cuts_into_icons() {
        let img = sheet().expect("sheet decodes");
        assert_eq!(img.size, [1524, 839]);
        // A tile keeps its border and its inside; the black around it goes.
        let t = tile_icon(crop(img, 671, ROWS[0] - 2, TILE, TILE));
        assert_eq!(t.pixels[0].a(), 0, "corner outside the tile is transparent");
        assert!(t.pixels[(TILE / 2) * TILE + TILE / 2].a() > 200, "middle of the tile stays");
        assert_eq!(column_for("Delay").map(|i| COLUMNS[i].0), Some("delay"));
        assert_eq!(column_for("Volume/Pan").map(|i| COLUMNS[i].0), Some("volume"));
        assert_eq!(row_for("distortion", "Distortion", "HD2_DistScream808", "Scream 808"), 2);
        assert_eq!(row_for("modulation", "Modulation", "HD2_ChorusPlastiChorus", "PlastiChorus"), 3);
        assert_eq!(row_for("reverb", "Reverb", "HD2_ReverbPlate", "Plate"), 2);
    }
}
