//! Focus mode: change an effect's character on an X-Y plane.
//!
//! Every model that has it comes with five named tones: the centre (`origin`, at (0, 0)) and four corners (x and y from -1 to +1).
//! A point on the plane blends the five, so one drag moves several parameters at once. This module is only the maths and the data
//! (no UI, no pedal): the editor draws the pad and sends what [`FocusModel::evaluate`] returns.
//!
//! The data is Line 6's (Helix Stadium Native's `focus-view-xy-defaults`, listed per model with a behaviour for each parameter:
//! `interpolate`, `quantize` or `exclude`). [`FocusSet::resolve`] maps it onto the pedal's symbols and the index each parameter has
//! in the pedal's value list; [`FocusSet::to_pack`] / [`FocusSet::from_pack`] keep the result compactly.

use std::collections::BTreeMap;
use std::io::{Read, Write};

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::catalog::Catalog;

/// The pack layout this build writes and reads.
pub const PACK_FORMAT: u32 = 1;

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/builtin_focus.rs"));
}

/// The tones built into ToneStrand: (Helix Stadium Native version they were made from, pack bytes). The newest folder in
/// `focus/` at build time; `None` when there was none.
pub fn builtin() -> Option<(&'static str, &'static [u8])> {
    embedded::BUILTIN
}

/// The built-in tones, or `None` (logged) when they are missing or unreadable.
pub fn load_builtin() -> Option<FocusSet> {
    let (_, bytes) = builtin()?;
    match FocusSet::from_pack(bytes) {
        Ok(set) => Some(set),
        Err(e) => {
            eprintln!("the built-in Focus mode tones could not be read: {e}");
            None
        }
    }
}

/// The five tones, in the order used by weights and regions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegionKey {
    Origin,
    BottomLeft,
    TopLeft,
    BottomRight,
    TopRight,
}

impl RegionKey {
    pub const ALL: [RegionKey; 5] =
        [RegionKey::Origin, RegionKey::BottomLeft, RegionKey::TopLeft, RegionKey::BottomRight, RegionKey::TopRight];

    /// Position in a weights array or a model's regions.
    pub fn slot(self) -> usize {
        self as usize
    }

    /// Where the tone sits on the plane: (x, y), each -1, 0 or 1.
    pub fn at(self) -> (f64, f64) {
        match self {
            RegionKey::Origin => (0.0, 0.0),
            RegionKey::BottomLeft => (-1.0, -1.0),
            RegionKey::TopLeft => (-1.0, 1.0),
            RegionKey::BottomRight => (1.0, -1.0),
            RegionKey::TopRight => (1.0, 1.0),
        }
    }

    /// The key Line 6's file uses.
    fn file_key(self) -> &'static str {
        match self {
            RegionKey::Origin => "origin",
            RegionKey::BottomLeft => "x_min_y_min",
            RegionKey::TopLeft => "x_min_y_max",
            RegionKey::BottomRight => "x_max_y_min",
            RegionKey::TopRight => "x_max_y_max",
        }
    }
}

/// How much each of the five tones counts at (x, y): in [`RegionKey::ALL`] order, adding up to 1. x and y are clamped to [-1, 1].
///
/// The plane is four quadrants. In a quadrant the point blends the centre, the quadrant's corner and the two edge midpoints
/// next to it (a midpoint is the average of two corners), so every point is smooth across the axes.
pub fn weights(x: f64, y: f64) -> [f64; 5] {
    let (cx, cy) = (x.clamp(-1.0, 1.0), y.clamp(-1.0, 1.0));
    let (u, v) = (cx.abs(), cy.abs());
    let w_origin = (1.0 - u) * (1.0 - v);
    let w_x = u * (1.0 - v);
    let w_y = (1.0 - u) * v;
    let w_corner = u * v;
    let [mut bl, mut tl, mut br, mut tr] = [0.0; 4];
    // The corner of this quadrant takes its own weight and half of each midpoint weight; the other half of the x midpoint goes to the
    // corner across the y axis, and the other half of the y midpoint to the corner across the x axis.
    match (cx >= 0.0, cy >= 0.0) {
        (true, true) => {
            tr += w_corner + 0.5 * w_x + 0.5 * w_y;
            br += 0.5 * w_x;
            tl += 0.5 * w_y;
        }
        (false, true) => {
            tl += w_corner + 0.5 * w_x + 0.5 * w_y;
            bl += 0.5 * w_x;
            tr += 0.5 * w_y;
        }
        (true, false) => {
            br += w_corner + 0.5 * w_x + 0.5 * w_y;
            tr += 0.5 * w_x;
            bl += 0.5 * w_y;
        }
        (false, false) => {
            bl += w_corner + 0.5 * w_x + 0.5 * w_y;
            tl += 0.5 * w_x;
            br += 0.5 * w_y;
        }
    }
    [w_origin, bl, tl, br, tr]
}

/// What a parameter holds, as the catalog types it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Float,
    Int,
    Bool,
}

/// One parameter Focus mode moves.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FocusParam {
    /// The pedal's parameter id (`Drive`, `HiCut`...).
    pub id: String,
    /// Where it sits in the block's value list on the pedal.
    pub index: usize,
    /// `quantize`: snaps to the nearest of the five tones' values instead of blending.
    pub quantize: bool,
    pub kind: Kind,
    /// The catalog's range, used to keep the result inside it.
    pub min: f64,
    pub max: f64,
}

/// One tone: its name and a value for each parameter of the model (in the model's `params` order; a switch is 0 or 1).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Region {
    pub name: String,
    pub values: Vec<f64>,
}

/// What is known about a pack besides its contents (kept next to it as JSON).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FocusMeta {
    pub format: u32,
    /// The Helix Stadium Native version the tones come from.
    pub stadium_version: String,
    pub created: String,
    /// Which effects list the parameter positions were taken from (HX Edit's files or the built-in list).
    pub catalog: String,
    pub files_read: usize,
    pub mapped: usize,
    pub stadium_only: usize,
    pub not_in_catalog: usize,
    pub params_left_out: usize,
}

/// A value to give a parameter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FocusValue {
    Float(f64),
    Int(i64),
    Bool(bool),
}

/// The five tones of one model (one symbol: a Mono and a Stereo version are two models) and the parameters they move.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FocusModel {
    pub name: String,
    pub category: String,
    pub params: Vec<FocusParam>,
    /// Five, in [`RegionKey::ALL`] order.
    pub regions: Vec<Region>,
}

impl FocusModel {
    pub fn region(&self, key: RegionKey) -> &Region {
        &self.regions[key.slot()]
    }

    /// The name of the tone at a corner or the centre.
    pub fn region_name(&self, key: RegionKey) -> &str {
        &self.region(key).name
    }

    /// The value every parameter takes at (x, y), as (index in the block's value list, value). At the centre and at the corners
    /// these are exactly the stored tones.
    pub fn evaluate(&self, x: f64, y: f64) -> Vec<(usize, FocusValue)> {
        let w = weights(x, y);
        self.params.iter().enumerate().map(|(i, p)| (p.index, self.value_of(i, p, &w))).collect()
    }

    fn value_of(&self, i: usize, p: &FocusParam, w: &[f64; 5]) -> FocusValue {
        let tones: [f64; 5] = std::array::from_fn(|k| self.regions[k].values.get(i).copied().unwrap_or(0.0));
        if p.kind == Kind::Bool {
            // A switch takes the value of the tone that counts most (the centre wins a tie).
            let best = (0..5).fold(0, |b, k| if w[k] > w[b] { k } else { b });
            return FocusValue::Bool(tones[best] != 0.0);
        }
        let blended: f64 = (0..5).map(|k| w[k] * tones[k]).sum();
        let v = if p.quantize {
            tones.iter().copied().fold(tones[0], |best, t| if (t - blended).abs() < (best - blended).abs() { t } else { best })
        } else {
            blended
        };
        let v = if p.max > p.min { v.clamp(p.min, p.max) } else { v };
        match p.kind {
            Kind::Int => FocusValue::Int(v.round() as i64),
            _ => FocusValue::Float(v),
        }
    }
}

/// What turning Line 6's file into a [`FocusSet`] left out, and why.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Report {
    /// Models in the file.
    pub models: usize,
    /// Helix Stadium-only models (the `Agoura` amps): not on this family of pedals.
    pub stadium_only: usize,
    /// Pedal symbols that got a pad.
    pub mapped: usize,
    /// Models whose files are not in the catalog (newer than it, or not on the pedal).
    pub unmapped: Vec<String>,
    /// Parameters left out: not a parameter of that model on the pedal, or no description in the catalog.
    pub skipped_params: Vec<String>,
    /// Values outside the catalog's range (kept as given; clamped when evaluated).
    pub out_of_range: Vec<String>,
}

/// Every model's five tones, by the pedal's symbol (`HD2_DistAlpacaRougeMono`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FocusSet {
    pub models: BTreeMap<String, FocusModel>,
    pub report: Report,
}

#[derive(Serialize)]
struct PackOut<'a> {
    format: u32,
    models: &'a BTreeMap<String, FocusModel>,
}

#[derive(Deserialize)]
struct PackIn {
    format: u32,
    models: BTreeMap<String, FocusModel>,
}

impl FocusSet {
    /// The tones of a model, by the pedal's symbol.
    pub fn model_for(&self, symbol: &str) -> Option<&FocusModel> {
        self.models.get(symbol)
    }

    /// Read Line 6's `focus_view_catalog.json` and map it onto the pedal: each model's files become symbols (`HD2_` or `L6SPB_`
    /// in front of the file name), each parameter the index it has in that symbol's value list, with its range and type from `cat`.
    pub fn resolve(stadium_json: &str, cat: &Catalog) -> Result<FocusSet, String> {
        let root: Json = serde_json::from_str(stadium_json.trim_start_matches('\u{feff}')).map_err(|e| format!("the focus file: {e}"))?;
        let categories = root.as_object().ok_or("the focus file is not a list of categories")?;
        let mut set = FocusSet::default();
        for (_, list) in categories {
            let Some(list) = list.as_array() else { continue };
            for m in list {
                let files: Vec<&str> =
                    m.get("files").and_then(Json::as_array).map(|a| a.iter().filter_map(Json::as_str).collect()).unwrap_or_default();
                set.add_model(m, &files, cat);
            }
        }
        set.report.mapped = set.models.len();
        Ok(set)
    }

    /// Read the folder Helix Stadium Native keeps its tones in (`.../res/meta-data/focus-view-xy-defaults`: one folder per category,
    /// one `<File>.json` per model file) and map it like [`FocusSet::resolve`]. The file's name is the symbol without `HD2_` or
    /// `L6SPB_`. Returns the set and how many files were read.
    pub fn resolve_folder(root: &std::path::Path, cat: &Catalog) -> Result<(FocusSet, usize), String> {
        let mut set = FocusSet::default();
        let mut read = 0;
        let mut categories: Vec<_> = std::fs::read_dir(root).map_err(|e| format!("{}: {e}", root.display()))?.flatten().collect();
        categories.sort_by_key(|e| e.file_name());
        for dir in categories.into_iter().filter(|e| e.path().is_dir()) {
            let mut files: Vec<_> =
                std::fs::read_dir(dir.path()).map_err(|e| format!("{}: {e}", dir.path().display()))?.flatten().collect();
            files.sort_by_key(|e| e.file_name());
            for f in files {
                let path = f.path();
                let Some(stem) =
                    path.file_stem().and_then(|s| s.to_str()).filter(|_| path.extension().and_then(|e| e.to_str()) == Some("json"))
                else {
                    continue;
                };
                let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                let mut m: Json =
                    serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| format!("{}: {e}", path.display()))?;
                if let Some(obj) = m.as_object_mut() {
                    obj.insert("category".into(), Json::String(dir.file_name().to_string_lossy().into_owned()));
                }
                set.add_model(&m, &[stem], cat);
                read += 1;
            }
        }
        set.report.mapped = set.models.len();
        Ok((set, read))
    }

    /// One model of the file, with the model files it covers: counted, and kept when the catalog knows a symbol for one of the files.
    fn add_model(&mut self, m: &Json, files: &[&str], cat: &Catalog) {
        self.report.models += 1;
        let name = m.get("modelName").and_then(Json::as_str).unwrap_or("").to_owned();
        if files.iter().any(|f| f.contains("Agoura")) {
            self.report.stadium_only += 1;
            return;
        }
        let mut found = false;
        for file in files {
            let Some(symbol) = ["HD2_", "L6SPB_", ""].iter().map(|p| format!("{p}{file}")).find(|s| cat.sym.iter().any(|e| &e.symbol == s))
            else {
                continue;
            };
            if let Some(model) = build_model(m, &name, &symbol, cat, &mut self.report) {
                self.models.insert(symbol, model);
                found = true;
            }
        }
        if !found {
            self.report.unmapped.push(format!("{name} [{}]", files.join(", ")));
        }
    }

    /// The set as pack bytes (gzip of one JSON document; the same set gives the same bytes). The report is not kept.
    pub fn to_pack(&self) -> Result<Vec<u8>, String> {
        let json = serde_json::to_vec(&PackOut { format: PACK_FORMAT, models: &self.models }).map_err(|e| format!("focus pack: {e}"))?;
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
        gz.write_all(&json).map_err(|e| format!("focus pack: {e}"))?;
        gz.finish().map_err(|e| format!("focus pack: {e}"))
    }

    pub fn from_pack(bytes: &[u8]) -> Result<FocusSet, String> {
        let mut json = Vec::new();
        flate2::read::GzDecoder::new(bytes).read_to_end(&mut json).map_err(|e| format!("focus pack: {e}"))?;
        let p: PackIn = serde_json::from_slice(&json).map_err(|e| format!("focus pack: {e}"))?;
        if p.format != PACK_FORMAT {
            return Err(format!("focus pack: format {} is not the one this build reads ({PACK_FORMAT})", p.format));
        }
        Ok(FocusSet { models: p.models, report: Report::default() })
    }
}

/// One tone's value for a parameter: a number or a switch from the file; the centre's, then the catalog's default, if it has none.
fn tone_value(region: &Json, id: &str) -> Option<f64> {
    match region.get(id)? {
        Json::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        v => v.as_f64(),
    }
}

fn build_model(m: &Json, name: &str, symbol: &str, cat: &Catalog, report: &mut Report) -> Option<FocusModel> {
    let entry = cat.sym.iter().find(|e| e.symbol == symbol)?;
    let info = cat.info_for_symbol(symbol)?;
    let regions = m.get("regions")?;
    let origin = regions.get(RegionKey::Origin.file_key())?;
    let mut params = Vec::new();
    let mut values: Vec<Vec<f64>> = vec![Vec::new(); 5];
    for (id, p) in m.get("params")?.as_object()? {
        let behaviour = p.get("behavior").and_then(Json::as_str).unwrap_or("interpolate");
        if behaviour == "exclude" {
            continue;
        }
        let (Some(index), Some(meta)) = (entry.params.iter().position(|x| x == id), info.params.iter().find(|x| &x.id == id)) else {
            report.skipped_params.push(format!("{symbol} {id}"));
            continue;
        };
        let kind = match meta.value_type {
            2 => Kind::Bool,
            0 => Kind::Int,
            _ => Kind::Float,
        };
        let centre = tone_value(origin, id).unwrap_or(meta.default);
        for key in RegionKey::ALL {
            let v = regions.get(key.file_key()).and_then(|r| tone_value(r, id)).unwrap_or(centre);
            let eps = (meta.max - meta.min).abs() * 1e-5; // the file holds 32-bit numbers: 19.899999618 is the range's 19.9
            if meta.max > meta.min && (v < meta.min - eps || v > meta.max + eps) && kind != Kind::Bool {
                report.out_of_range.push(format!("{symbol} {id} {} = {v} (range {} to {})", key.file_key(), meta.min, meta.max));
            }
            values[key.slot()].push(v);
        }
        params.push(FocusParam { id: id.clone(), index, quantize: behaviour == "quantize", kind, min: meta.min, max: meta.max });
    }
    if params.is_empty() {
        return None;
    }
    let regions = RegionKey::ALL
        .iter()
        .zip(values)
        .map(|(key, values)| Region {
            name: regions.get(key.file_key()).and_then(|r| r.get("name")).and_then(Json::as_str).unwrap_or("").to_owned(),
            values,
        })
        .collect();
    let category = m.get("category").and_then(Json::as_str).unwrap_or("").to_owned();
    Some(FocusModel { name: name.to_owned(), category, params, regions })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::test_support::mini_hx_edit_dir;

    fn region(name: &str, values: &[f64]) -> Region {
        Region { name: name.into(), values: values.to_vec() }
    }

    /// Five tones for a float, a snapping choice and a switch.
    fn model() -> FocusModel {
        let p = |id: &str, index, quantize, kind, min, max| FocusParam { id: id.into(), index, quantize, kind, min, max };
        FocusModel {
            name: "Test".into(),
            category: "amp".into(),
            params: vec![
                p("Drive", 0, false, Kind::Float, 0.0, 1.0),
                p("Mode", 2, true, Kind::Int, 0.0, 6.0),
                p("Boost", 5, false, Kind::Bool, 0.0, 1.0),
            ],
            regions: vec![
                region("Home", &[0.5, 1.0, 0.0]),
                region("Soft", &[0.1, 0.0, 0.0]),
                region("Edge", &[0.3, 3.0, 1.0]),
                region("Deep", &[0.7, 5.0, 0.0]),
                region("Wild", &[1.0, 6.0, 1.0]),
            ],
        }
    }

    fn drive(m: &FocusModel, x: f64, y: f64) -> f64 {
        match m.evaluate(x, y)[0].1 {
            FocusValue::Float(f) => f,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn weights_add_up_to_one_everywhere() {
        for i in -20..=20 {
            for j in -20..=20 {
                let w = weights(i as f64 / 20.0, j as f64 / 20.0);
                assert!((w.iter().sum::<f64>() - 1.0).abs() < 1e-12, "({i},{j}) {w:?}");
                assert!(w.iter().all(|x| *x >= 0.0));
            }
        }
    }

    #[test]
    fn the_centre_and_the_corners_are_exactly_their_own_tone() {
        for key in RegionKey::ALL {
            let (x, y) = key.at();
            let mut want = [0.0; 5];
            want[key.slot()] = 1.0;
            assert_eq!(weights(x, y), want, "{key:?}");
        }
        // Outside the plane counts as its edge.
        assert_eq!(weights(5.0, -3.0), weights(1.0, -1.0));
    }

    #[test]
    fn the_middle_of_an_edge_is_half_and_half_of_its_two_corners() {
        assert_eq!(weights(1.0, 0.0), [0.0, 0.0, 0.0, 0.5, 0.5]); // right edge: bottom right and top right
        assert_eq!(weights(-1.0, 0.0), [0.0, 0.5, 0.5, 0.0, 0.0]);
        assert_eq!(weights(0.0, 1.0), [0.0, 0.0, 0.5, 0.0, 0.5]); // top edge: top left and top right
        assert_eq!(weights(0.0, -1.0), [0.0, 0.5, 0.0, 0.5, 0.0]);
    }

    #[test]
    fn evaluating_at_the_centre_and_corners_gives_the_stored_values() {
        let m = model();
        for key in RegionKey::ALL {
            let (x, y) = key.at();
            let got = m.evaluate(x, y);
            let r = m.region(key);
            assert_eq!(got[0], (0, FocusValue::Float(r.values[0])), "{key:?}");
            assert_eq!(got[1], (2, FocusValue::Int(r.values[1] as i64)), "{key:?}");
            assert_eq!(got[2], (5, FocusValue::Bool(r.values[2] != 0.0)), "{key:?}");
        }
        assert_eq!(m.region_name(RegionKey::TopRight), "Wild");
    }

    #[test]
    fn a_blended_value_moves_smoothly_with_the_puck() {
        let m = model();
        // Halfway from the centre to the top right: the centre, the right midpoint and the top midpoint (0.5 weight each of their corners).
        let half = drive(&m, 0.5, 0.5);
        let by_hand = 0.25 * 0.5 + 0.25 * (0.5 * 1.0 + 0.5 * 0.7) + 0.25 * (0.5 * 1.0 + 0.5 * 0.3) + 0.25 * 1.0;
        assert!((half - by_hand).abs() < 1e-12, "{half} {by_hand}");
        // No jumps: a step of 0.01 changes the value by little (the range is 1.0), across the axes too.
        let mut worst: f64 = 0.0;
        for i in -100..100 {
            for j in -100..=100 {
                let (x, y) = (i as f64 / 100.0, j as f64 / 100.0);
                worst = worst.max((drive(&m, x + 0.01, y) - drive(&m, x, y)).abs()).max((drive(&m, y, x + 0.01) - drive(&m, y, x)).abs());
            }
        }
        assert!(worst < 0.03, "largest step {worst}");
    }

    #[test]
    fn a_choice_snaps_to_one_of_the_five_values_and_a_switch_to_the_nearest_tone() {
        let m = model();
        let held = [1.0, 0.0, 3.0, 5.0, 6.0];
        for i in -10..=10 {
            for j in -10..=10 {
                let got = m.evaluate(i as f64 / 10.0, j as f64 / 10.0);
                let FocusValue::Int(n) = got[1].1 else { panic!() };
                assert!(held.contains(&(n as f64)), "({i},{j}) gave {n}");
            }
        }
        // Close to the top right the switch is that tone's (on), close to the centre it is the centre's (off).
        assert_eq!(m.evaluate(0.9, 0.9)[2].1, FocusValue::Bool(true));
        assert_eq!(m.evaluate(0.1, 0.0)[2].1, FocusValue::Bool(false));
    }

    #[test]
    fn a_value_never_leaves_the_catalogs_range() {
        let mut m = model();
        m.regions[4].values[0] = 9.0; // a tone outside the range
        assert_eq!(drive(&m, 1.0, 1.0), 1.0);
    }

    /// A made-up Stadium file for the made-up catalog: one model that maps, one Stadium-only, one the catalog lacks.
    const STADIUM: &str = r#"{
      "amp": [
        {"modelName": "Test Amp", "category": "amp", "files": ["TS_TestAmp"],
         "params": {"TS_Drive": {"id": 1, "behavior": "interpolate"}, "TS_Mode": {"id": 2, "behavior": "quantize"},
                    "AmpCabX": {"id": 3, "behavior": "exclude"}, "Nonexistent": {"id": 4, "behavior": "interpolate"}},
         "regions": {
           "origin": {"name": "Home", "TS_Drive": 0.5, "TS_Mode": 1, "AmpCabX": 0.0, "Nonexistent": 1.0},
           "x_min_y_min": {"name": "Soft", "TS_Drive": 0.1, "TS_Mode": 0, "AmpCabX": 0.0, "Nonexistent": 1.0},
           "x_min_y_max": {"name": "Edge", "TS_Drive": 0.3, "TS_Mode": 2, "AmpCabX": 0.0, "Nonexistent": 1.0},
           "x_max_y_min": {"name": "Deep", "TS_Drive": 0.7, "AmpCabX": 0.0, "Nonexistent": 1.0},
           "x_max_y_max": {"name": "Wild", "TS_Drive": 1.5, "TS_Mode": 2, "AmpCabX": 0.0, "Nonexistent": 1.0}}},
        {"modelName": "Big Amp", "category": "amp", "files": ["AgouraBigAmp"], "params": {}, "regions": {}}
      ],
      "delay": [
        {"modelName": "New Delay", "category": "delay", "files": ["DelayNewMono"], "params": {"Time": {"id": 1, "behavior": "interpolate"}}, "regions": {"origin": {"name": "x", "Time": 0.1}}}
      ]
    }"#;

    #[test]
    fn a_stadium_file_maps_onto_the_catalogs_symbols_and_indices() {
        let cat = Catalog::load(&mini_hx_edit_dir()).unwrap();
        let set = FocusSet::resolve(STADIUM, &cat).unwrap();
        assert_eq!(set.report.models, 3);
        assert_eq!(set.report.stadium_only, 1);
        assert_eq!(set.report.mapped, 1);
        assert_eq!(set.report.unmapped, vec!["New Delay [DelayNewMono]".to_owned()]);
        assert_eq!(set.report.skipped_params, vec!["TS_TestAmp Nonexistent".to_owned()], "excluded ones are left out without comment");
        assert_eq!(set.report.out_of_range.len(), 1, "{:?}", set.report.out_of_range);

        let m = set.model_for("TS_TestAmp").unwrap();
        assert_eq!(
            m.params.iter().map(|p| (p.id.as_str(), p.index, p.quantize, p.kind)).collect::<Vec<_>>(),
            vec![("TS_Drive", 0, false, Kind::Float), ("TS_Mode", 1, true, Kind::Int)]
        );
        assert_eq!(m.region_name(RegionKey::TopLeft), "Edge");
        // A tone with no value for a parameter takes the centre's (TS_Mode in "Deep" is 1, the centre's).
        assert_eq!(m.region(RegionKey::BottomRight).values, vec![0.7, 1.0]);
        assert_eq!(
            m.evaluate(1.0, 1.0),
            vec![(0, FocusValue::Float(1.0)), (1, FocusValue::Int(2))],
            "the Drive of 1.5 is held to the range"
        );
    }

    #[test]
    fn a_pack_reads_back_the_same_and_the_same_set_gives_the_same_bytes() {
        let cat = Catalog::load(&mini_hx_edit_dir()).unwrap();
        let set = FocusSet::resolve(STADIUM, &cat).unwrap();
        let bytes = set.to_pack().unwrap();
        assert_eq!(bytes, set.to_pack().unwrap());
        let back = FocusSet::from_pack(&bytes).unwrap();
        assert_eq!(back.models, set.models);
        assert!(FocusSet::from_pack(b"not a pack").is_err());
    }
}

#[cfg(test)]
mod real_data {
    use super::*;

    /// Not part of the normal run (Line 6's file is not in the repository): `TONESTRAND_FOCUS_FILE=<focus_view_catalog.json>
    /// cargo test -p tonestrand-core focus::real_data -- --nocapture` maps it onto the best catalog found and prints what was left out.
    #[test]
    fn report_for_a_real_file() {
        let Some(path) = std::env::var_os("TONESTRAND_FOCUS_FILE") else { return };
        let text = std::fs::read_to_string(&path).expect("the focus file");
        let cat = crate::catalog::load_best(None).expect("an effects list (HX Edit's files or the built-in one)");
        let set = FocusSet::resolve(&text, &cat).unwrap();
        let r = &set.report;
        println!(
            "models in the file {}, Stadium-only {}, pads on the pedal {} (by symbol), not in the catalog {}",
            r.models,
            r.stadium_only,
            r.mapped,
            r.unmapped.len()
        );
        println!("parameters left out {}, values outside the catalog's range {}", r.skipped_params.len(), r.out_of_range.len());
        for l in r.skipped_params.iter().take(15) {
            println!("  skipped: {l}");
        }
        for l in r.out_of_range.iter().take(15) {
            println!("  range: {l}");
        }
        let mut by_cat: BTreeMap<&str, usize> = BTreeMap::new();
        for m in set.models.values() {
            *by_cat.entry(m.category.as_str()).or_default() += 1;
        }
        println!("pads by category: {by_cat:?}");
        let pack = set.to_pack().unwrap();
        println!("pack: {} bytes", pack.len());
        assert!(FocusSet::from_pack(&pack).unwrap().models == set.models);
    }
}

#[cfg(test)]
mod folder_tests {
    use super::*;
    use crate::catalog::test_support::mini_hx_edit_dir;

    /// A made-up Stadium folder: `amp/TS_TestAmp.json`, `amp/AgouraThing.json` and a note that is not a model.
    pub fn stadium_folder() -> std::path::PathBuf {
        let dir = mini_hx_edit_dir().join("focus-view-xy-defaults");
        std::fs::create_dir_all(dir.join("amp")).unwrap();
        std::fs::write(
            dir.join("amp").join("TS_TestAmp.json"),
            "\u{feff}{\"modelName\": \"Test Amp\", \"params\": {\"TS_Drive\": {\"id\": 1, \"behavior\": \"interpolate\"}}, \"regions\": {\
             \"origin\": {\"name\": \"Home\", \"TS_Drive\": 0.5}, \"x_min_y_min\": {\"name\": \"A\", \"TS_Drive\": 0.1}, \"x_min_y_max\": {\"name\": \"B\", \"TS_Drive\": 0.2},\
             \"x_max_y_min\": {\"name\": \"C\", \"TS_Drive\": 0.3}, \"x_max_y_max\": {\"name\": \"D\", \"TS_Drive\": 0.4}}}",
        )
        .unwrap();
        std::fs::write(dir.join("amp").join("AgouraThing.json"), "{\"modelName\": \"Thing\", \"params\": {}, \"regions\": {}}").unwrap();
        std::fs::write(dir.join("amp").join("notes.txt"), "not a model").unwrap();
        dir
    }

    #[test]
    fn a_stadium_folder_is_read_file_by_file() {
        let cat = Catalog::load(&mini_hx_edit_dir()).unwrap();
        let (set, read) = FocusSet::resolve_folder(&stadium_folder(), &cat).unwrap();
        assert_eq!(read, 2, "the two .json files, not the note");
        assert_eq!(set.report.stadium_only, 1);
        let m = set.model_for("TS_TestAmp").unwrap();
        assert_eq!(m.category, "amp");
        assert_eq!(m.region_name(RegionKey::BottomRight), "C");
        assert!(FocusSet::resolve_folder(&std::env::temp_dir().join("no-such-focus-folder"), &cat).is_err());
    }
}

#[cfg(test)]
mod builtin_tests {
    use super::*;

    /// The tones built into the program hang together: five regions, a value per parameter in each, no two parameters in one place.
    #[test]
    fn the_built_in_tones_are_well_formed() {
        let Some((version, _)) = builtin() else { return }; // a build with no focus/ folder
        let set = load_builtin().unwrap_or_else(|| panic!("the built-in tones ({version}) could not be read"));
        assert!(set.models.len() > 400, "{} models", set.models.len());
        for (symbol, m) in &set.models {
            assert_eq!(m.regions.len(), 5, "{symbol}");
            assert!(!m.params.is_empty(), "{symbol}");
            for r in &m.regions {
                assert_eq!(r.values.len(), m.params.len(), "{symbol} {}", r.name);
                assert!(!r.name.is_empty(), "{symbol} has a tone with no name");
                assert!(r.values.iter().all(|v| v.is_finite()), "{symbol}");
            }
            let mut idx: Vec<usize> = m.params.iter().map(|p| p.index).collect();
            idx.sort_unstable();
            idx.dedup();
            assert_eq!(idx.len(), m.params.len(), "{symbol}: two parameters in one place");
            // The centre and the corners come back exactly.
            for key in RegionKey::ALL {
                let (x, y) = key.at();
                assert_eq!(m.evaluate(x, y).len(), m.params.len(), "{symbol}");
            }
        }
    }
}
