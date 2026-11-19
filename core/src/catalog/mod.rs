//! The effect catalog: model names, categories and knob names and ranges.
//!
//! These are read at start-up from the person's own HX Edit installation
//! (Line 6's files stay where they are; nothing is copied into this app):
//! - `Helix.sym`: every model symbol in the pedal's numbering (position = the
//!   model number the pedal stores), with its parameters in the pedal's order.
//! - `*.models`: each model's display name and its parameters' names, types,
//!   ranges and how to show them.
//! - `HX_ModelCatalog.json`: categories, their colours and sub-groups, for the
//!   model picker.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::Value as Json;

mod pack;
pub use pack::{read_pack_meta, PackMeta, PACK_FORMAT};
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
pub mod version;

#[cfg(test)]
mod version_tests {
    use super::version::{is_version, version_key};

    #[test]
    fn versions_sort_as_numbers() {
        let mut v = vec!["3.100", "3.82", "3.9", "4.0", "3.9.1"];
        v.sort_by_key(|n| version_key(n));
        assert_eq!(v, ["3.9", "3.9.1", "3.82", "3.100", "4.0"]);
    }

    #[test]
    fn only_dotted_numbers_are_versions() {
        for ok in ["3.82", "4", "3.9.1"] {
            assert!(is_version(ok), "{ok}");
        }
        for bad in ["", ".", "3.", ".3", "3..8", "../x", "3.8a", "3*8", "3.8 ", "v3.8"] {
            assert!(!is_version(bad), "{bad:?}");
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DisplayPiece {
    pub lower: f64,
    pub upper: f64,
    pub mult: f64,
    pub format: Option<String>,
    pub format_units: Option<String>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DisplaySpec {
    pub scale: f64,
    pub int_offset: i64,
    pub min_val: Option<f64>,
    pub max_val: Option<f64>,
    pub format: Option<String>,
    pub format_units: Option<String>,
    pub pieces: Vec<DisplayPiece>,
    /// HX Edit shows it as a few fixed choices (its "segmented" control), e.g. a mic Angle of 0° or 45°.
    pub segmented: bool,
}

impl Default for DisplaySpec {
    fn default() -> Self {
        Self {
            scale: 1.0,
            int_offset: 0,
            min_val: None,
            max_val: None,
            format: None,
            format_units: None,
            pieces: Vec::new(),
            segmented: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Param {
    pub id: String,
    pub name: String,
    /// 0 whole number / choice, 1 decimal, 2 on/off.
    pub value_type: i64,
    pub display: String,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    /// Explanatory labels for discrete/enum parameters (indexed from `min as i64`).
    pub choices: Vec<String>,
    /// How continuous values are scaled and formatted for display.
    pub spec: DisplaySpec,
}

impl Param {
    /// Label for an integer or boolean choice value, if this parameter is an enum.
    pub fn choice_label(&self, val: i64) -> Option<&str> {
        if self.choices.is_empty() {
            return None;
        }
        let base = if self.value_type == 2 { 0 } else { self.min.round() as i64 };
        let idx = (val - base).clamp(0, self.choices.len() as i64 - 1) as usize;
        self.choices.get(idx).map(String::as_str)
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ModelInfo {
    pub name: String,
    /// DSP cost of the mono (or only) version, and of the stereo one.
    pub load: f64,
    pub load_stereo: f64,
    pub params: Vec<Param>,
    pub cablink: Option<String>,
    /// Units the model is limited to (empty: every unit).
    pub devices: Vec<i64>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SymEntry {
    pub symbol: String,
    pub params: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PickModel {
    pub name: String,
    pub number: i64,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PickGroup {
    pub category: String,
    pub sub: String,
    pub colour: Rgb,
    pub models: Vec<PickModel>,
}

/// Where a catalog came from.
#[derive(Clone, Debug, PartialEq)]
pub enum CatalogSource {
    /// Read from an HX Edit installation's resource folder.
    HxEdit(PathBuf),
    /// Read from a pack made on a computer with HX Edit (the Android app's built-in list).
    Builtin { hx_edit_version: String },
}

pub struct Catalog {
    source: CatalogSource,
    pub sym: Vec<SymEntry>,
    sym_index: HashMap<String, usize>,
    models: HashMap<String, ModelInfo>,
    /// Base model id -> (category name, colour).
    category: HashMap<String, (String, Rgb)>,
    pub groups: Vec<PickGroup>,
    /// The NAM A2 Lite block's look (name, knobs), shown in place of the mono
    /// Xenomorph Fuzz when that optional firmware is turned on.
    nam_info: Option<ModelInfo>,
}

/// Knob ranges and displays for the NAM A2 Lite block, over the stock ones it reuses.
fn nam_param(p: &Param) -> Param {
    let mut q = p.clone();
    if let Some((_, n)) = crate::nam::KNOBS.iter().find(|(id, _)| *id == p.id) {
        q.name = (*n).to_owned();
    }
    q.spec = DisplaySpec::default();
    q.choices = Vec::new();
    match p.id.as_str() {
        "Gain" => q.display = "nam_input".into(),
        "Tone" | "OscLevel" | "OscTone" => q.display = "nam_band".into(),
        "Level" => q.display = "nam_output".into(),
        "Clipping" => q.choices = vec!["Normal".into(), "Inverted".into()],
        "MinFreq" => {
            q.display = "nam_slot".into();
            q.min = 0.0;
            q.max = 129.0;
        }
        "MaxFreq" => {
            q.display = "nam_midfreq".into();
            q.min = 150.0;
            q.max = 3000.0;
        }
        "WaveShape" => q.choices = vec!["0.7 (wide)".into(), "1.4".into(), "2.8 (narrow)".into()],
        "Sensitivity" => q.display = "nam_highcut".into(),
        _ => {}
    }
    // A new block starts flat: tone knobs at 5.0, High Cut off, Model on IR slot 1, Mid at 650 Hz.
    q.default = match p.id.as_str() {
        "Gain" | "Tone" | "Level" | "OscLevel" | "OscTone" => 0.5,
        "Sensitivity" => 1.0,
        "MinFreq" => 1.0,
        "MaxFreq" => 650.0,
        "Clipping" | "WaveShape" => 0.0,
        _ => p.default,
    };
    q
}

/// Where HX Edit usually keeps its resources.
pub fn find_dir(custom: Option<&str>) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(c) = custom {
        candidates.push(PathBuf::from(c));
        candidates.push(PathBuf::from(c).join("res"));
    }
    // Program Files wherever Windows put it (another drive, a localised name), then the usual places.
    for var in ["ProgramFiles(x86)", "ProgramFiles", "ProgramW6432"] {
        if let Some(pf) = std::env::var_os(var) {
            candidates.push(PathBuf::from(pf).join("Line6").join("HX Edit").join("res"));
        }
    }
    for base in ["C:\\Program Files (x86)\\Line6\\HX Edit", "C:\\Program Files\\Line6\\HX Edit"] {
        candidates.push(PathBuf::from(base).join("res"));
    }
    candidates.push(PathBuf::from("/Applications/HX Edit.app/Contents/Resources"));
    candidates.into_iter().find(|p| p.join("Helix.sym").is_file())
}

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/builtin_catalog.rs"));
}

/// The effects list built into ToneStrand: (HX Edit version it was made from, pack bytes). The newest folder in
/// `catalogs/` at build time; `None` when there was none.
pub fn builtin() -> Option<(&'static str, &'static [u8])> {
    embedded::BUILTIN
}

/// Which effects list to use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    Local,
    Builtin,
    None,
}

/// HX Edit's files do not record a version, but the list of models only grows, so more models means newer.
/// The local list wins a tie.
pub fn choose(local_symbols: Option<usize>, builtin_symbols: Option<usize>) -> Pick {
    match (local_symbols, builtin_symbols) {
        (None, None) => Pick::None,
        (Some(_), None) => Pick::Local,
        (None, Some(_)) => Pick::Builtin,
        (Some(l), Some(b)) => {
            if b > l {
                Pick::Builtin
            } else {
                Pick::Local
            }
        }
    }
}

/// The built-in list, or `None` (logged) when it is missing or unreadable.
fn load_builtin() -> Option<Catalog> {
    let (version, bytes) = builtin()?;
    match Catalog::from_pack(bytes) {
        Ok(mut c) => {
            c.source = CatalogSource::Builtin { hx_edit_version: version.to_owned() };
            Some(c)
        }
        Err(e) => {
            eprintln!("the built-in effects list could not be read: {e}");
            None
        }
    }
}

fn best_of(local: Option<Catalog>) -> Option<Catalog> {
    let builtin = load_builtin();
    match choose(local.as_ref().map(|c| c.sym.len()), builtin.as_ref().map(|c| c.sym.len())) {
        Pick::Local => {
            eprintln!("effects list: from HX Edit");
            local
        }
        Pick::Builtin => {
            eprintln!("effects list: built into ToneStrand");
            builtin
        }
        Pick::None => None,
    }
}

/// The effects list to use: HX Edit's files when found and not older than the built-in list, else the built-in one.
pub fn load_best(custom_hx_edit: Option<&str>) -> Option<Catalog> {
    best_of(find_dir(custom_hx_edit).and_then(|d| Catalog::load(&d).ok()))
}

/// A colour as red, green, blue; the app turns it into its own colour type (`theme::rgb`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Rgb(pub u8, pub u8, pub u8);

/// How much of one DSP the catalog's load figures add up to (an estimate).
/// Calibrated on the person's 126 HX Stomp presets (all of which run on the pedal):
/// without split/merge, the fullest adds up to 77.6, which reads 97% at 80.
pub const DSP_BUDGET: f64 = 80.0;

fn colour_from(text: &str) -> Rgb {
    let hex = text.trim_start_matches("0x").trim_start_matches('#');
    let n = u32::from_str_radix(hex, 16).unwrap_or(0x8899aa);
    Rgb((n >> 16) as u8, (n >> 8) as u8, n as u8)
}

/// Strip the variant ending the pedal's numbering adds to a model id.
fn base_id(symbol: &str) -> &str {
    for suffix in ["Mono", "Stereo"] {
        if let Some(b) = symbol.strip_suffix(suffix) {
            return b;
        }
    }
    symbol
}

fn num(v: Option<&Json>, default: f64) -> f64 {
    match v {
        Some(Json::Number(n)) => n.as_f64().unwrap_or(default),
        Some(Json::Bool(b)) => *b as u8 as f64,
        _ => default,
    }
}

/// Built-in enum choices for common Helix 3.80 display types (used as a fallback
/// and merged with `HelixControls.json` when loaded from disk).
pub fn builtin_choices(display: &str) -> Vec<String> {
    let items: &[&str] = match display {
        "sync_note" => &[
            "1/1",
            "1/2 Dotted",
            "1/2",
            "1/2 Triplet",
            "1/4 Dotted",
            "1/4",
            "1/4 Triplet",
            "1/8 Dotted",
            "1/8",
            "1/8 Triplet",
            "1/16 Dotted",
            "1/16",
            "1/16 Triplet",
            "1/32 Dotted",
            "1/32",
            "1/32 Triplet",
            "1/64 Dotted",
            "1/64",
            "1/64 Triplet",
        ],
        "delay_mod_mode" => &["Off", "Chorus", "Vibrato"],
        "chorus_mode" => &["Chorus", "Vibrato"],
        "tremolo_mode" => &["Tremolo", "Vibrato"],
        "off_on" => &["Off", "On"],
        "OFF_ON" => &["OFF", "ON"],
        "comp_mode" => &["Compress", "Limit"],
        "comp_ratio" => &["2:1", "3:1", "4:1", "6:1", "10:1", "20:1"],
        "comp_ratio_3band" => &["2:1", "3:1", "4:1", "6:1", "10:1", "20:1", "Inf:1"],
        "mic" => &[
            "57 Dynamic",
            "409 Dynamic",
            "421 Dynamic",
            "30 Dynamic",
            "20 Dynamic",
            "121 Ribbon",
            "160 Ribbon",
            "4038 Ribbon",
            "414 Cond",
            "84 Cond",
            "67 Cond",
            "87 Cond",
            "47 Cond",
            "112 Dynamic",
            "12 Dynamic",
            "7 Dynamic",
        ],
        "cabMICir" => &[
            "57 Dynamic",
            "421 Dynamic",
            "7 Dynamic",
            "906 Dynamic",
            "30 Dynamic",
            "121 Ribbon",
            "160 Ribbon",
            "4038 Ribbon",
            "84 Ribbon",
            "414 Cond",
            "47 Cond FET",
            "67 Cond",
        ],
        "cabMICirBass" => &[
            "57 Dynamic",
            "421 Dynamic",
            "7 Dynamic",
            "88 Dynamic",
            "52 Dynamic",
            "112 Dynamic",
            "20 Dynamic",
            "12 Dynamic",
            "121 Ribbon",
            "4038 Ribbon",
            "414 Cond",
            "47 Cond FET",
        ],
        "lfo_shape" => &["Sawtooth Up", "Sawtooth Down", "Triangle", "Sine", "Square", "Exp Sine"],
        "lfo_shape_dualphaser" => &["Sine", "Square"],
        "wave_shape" => &["Saw Up", "Saw Down", "Triangle", "Sine", "Square", "Inverse Sine", "Random"],
        "wave_shape_panner" | "wave_shape_xenofuzz" => &["Triangle", "Sine", "Square"],
        "wave_shape_pattern_trem" => &["Saw Up", "Saw Down", "Triangle", "Sine", "Square"],
        "wave_shape_throbber" => &["Ramp Up", "Ramp Down", "Triangle", "Square"],
        "wave_synth" => &["Square", "PWM", "Ramp"],
        "crisscross_wave_shape" => &["Sine", "Triangle"],
        "sweep_shape" => &["Triangle", "Sine", "Square", "Saw Up", "Saw Down", "Random"],
        "delay_mode" => &["Delay", "Echo"],
        "delay_source" => &["Mono", "Stereo", "Left", "Right"],
        "delay_heads" => &["1 - -", "- 2 -", "- - 3", "1 2 -", "1 - 3", "- 2 3"],
        "heads12" => &["1-2 Off", "HD1 On", "1-2 On", "Swelled"],
        "heads34" => &["3-4 Off", "HD3 On", "3-4 On", "Multi"],
        "bbd_size" => &["1024", "2048", "4096", "8192"],
        "delay_bit_depth" => &["6", "8", "10", "12", "16", "24", "32", "Off"],
        "crisscross_bitdepth" => &["6 Bits", "8 Bits", "10 Bits", "12 Bits", "16 Bits", "24 Bits", "32 Bits", "Off"],
        "sample_rate" => &["8 kHz", "11.025 kHz", "12 kHz", "16 kHz", "22.05 kHz", "24 kHz", "32 kHz", "Off"],
        "dyn_type" => &["Ducking", "Gating"],
        "Shimmer_Mode" => &["Luster", "Sheen"],
        "dynamicAmbience_room_size" => &["8 m", "10 m", "12 m"],
        "room_size" => &["10 m", "20 m", "30 m"],
        "nonlinear_shape" => &["Linear", "Logarithmic", "Inverse Log", "Triangle", "Sine", "Cosine", "Square", "Random"],
        "rise_time" => &["Short", "Medium", "Long"],
        "ramp_time" => &["Slow", "Medium", "Fast"],
        "reel_inchespresec" => &["7.5 ips", "15 ips", "30 ips"],
        "VitalDist_Clipping" => &["Opamp", "Asymmetric", "Symmetric"],
        "clipping" => &["Up", "Center", "Down"],
        "clipping_dist" => &["Overdrive", "Boost", "Distortion"],
        "clipping_dist_xenofuzz" => &["Soft", "Hard"],
        "clipping_sym" => &["Symmetric", "Asymmetric"],
        "diode" => &["None", "Germanium", "Silicon", "LED"],
        "dist_version" => &["V2", "V4"],
        "gain_mod" => &["Normal", "Higher"],
        "ts_shape" => &["Normal", "Thick", "Thicker"],
        "voltage" => &["9V", "18V"],
        "filter_range" | "lo_high" | "low_peak_high_peak" => &["Low", "High"],
        "lo_med_high" => &["Low", "Medium", "High"],
        "mode_pass" => &["Low Pass", "Band Pass", "High Pass"],
        "GSG_Bright" => &["Off", "Bright 1", "Bright 2"],
        "GSG_Jazz_Rock" => &["Jazz", "Rock"],
        "GermanXtra_Bright" => &["Off", "B1", "B2"],
        "GermanXtra_Class" => &["A", "AB"],
        "GermanXtra_Style" | "dt_tubeconfig" => &["Triode", "Pentode"],
        "Tight_Big" => &["Tight", "Big"],
        "Tight_Medium_Loose" => &["Tight", "Medium", "Loose"],
        "amp_midfreq" => &["220 Hz", "800 Hz", "3000 Hz"],
        "amp_voice" => &["1", "2", "3", "4", "5"],
        "bright" => &["Off", "1", "2"],
        "attack" => &["Cut", "Flat", "Boost"],
        "stage" => &["4", "8", "12", "16"],
        "phase" => &["0", "90", "180"],
        "polarity" => &["Normal", "Inverted"],
        "cv_polarity" => &["Normal", "Invert"],
        "cv_delay" => &["x1", "x2", "x4"],
        "control_select" => &["LFO", "Envelope", "Manual"],
        "detector" => &["Separate", "Link"],
        "down_up" => &["Down", "Up"],
        "up_down" => &["Up", "Down"],
        "up_updown" => &["Up", "Up/Down"],
        "even_odd" => &["Even", "Odd"],
        "odd_even" => &["Odd", "Even"],
        "fast_slow" => &["Slow", "Fast"],
        "out_in" => &["Out", "In"],
        "single_dual" => &["Single", "Dual"],
        "stereo_classic" => &["True", "Classic"],
        "stereo_mode" => &["Mono", "Stereo"],
        "panstereoimager_mode" => &["Mono In", "Stereo In"],
        "pan_discrete" | "sswitch" => &["Left", "Center", "Right"],
        "shift_mode" => &["Up", "Down", "Stereo"],
        "scale_voice" => {
            &["Major", "Minor", "Major Pentatonic", "Minor Pentatonic", "Harmonic Minor", "Melodic Minor", "Whole Tone", "Diminished"]
        }
        "key_voice" => &["A", "A#", "B", "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#"],
        "note_pitch" => &["C", "C sharp", "D", "D sharp", "E", "F", "F sharp", "G", "G sharp", "A", "A sharp", "B"],
        "synthharmony_interval1" => &["-Major 6th", "-Minor 6th", "-4th", "-Major 3rd", "-Minor 3rd", "Minor 3rd", "Major 3rd", "5th"],
        "synthharmony_interval2" => &["-1 Octave", "-5th", "-4th", "-2nd", "Unison", "4th", "5th", "1 Octave", "2 Octaves"],
        "pitch_shift" => &[
            "-9th", "-8th", "-7th", "-6th", "-5th", "-4th", "-3rd", "-2nd", "Unison", "+2nd", "+3rd", "+4th", "+5th", "+6th", "+7th",
            "+8th", "+9th",
        ],
        "feedback_type" => &[
            "-Octave",
            "Unison",
            "+Octave",
            "+Octave & 5th",
            "+2 Octaves",
            "+5th",
            "+Maj 3rd",
            "+Min 3rd",
            "+4th",
            "-5th",
            "-4th",
            "Random",
        ],
        "feedbacker_dryctrl" => &["Off", "On", "Always"],
        "feedbacker_reference" => &["Lowest", "Loudest"],
        "feedbacker_retrigger" => &["---", "Trigger"],
        "poly_tracking" => &["X Fast", "Fast", "Stable", "X Stable"],
        "polysustain_operation" | "stutterEdit_Operation" => &["MuteAll", "DryKll", "Normal"],
        "polytwelvestring_plucktype" => &["Normal", "FingrPick 1", "FingrPick 2"],
        "poly_acoustic_sim" => &["Standard", "Jumbo", "Enhanced", "Piezo"],
        "poly_chorus_hicut" => &["1 kHz", "2 kHz", "5 kHz", "10 kHz", "Off"],
        "poly_chorus_lowcut" => &["Off", "20 Hz", "50 Hz", "100 Hz", "150 Hz", "200 Hz", "250 Hz", "300 Hz", "400 Hz"],
        "poly_shift_slicecurve" => &[
            "StartSlow5",
            "StartSlow4",
            "StartSlow3",
            "StartSlow2",
            "StartSlow1",
            "Linear",
            "EndSlow1",
            "EndSlow2",
            "EndSlow3",
            "EndSlow4",
            "EndSlow5",
        ],
        "stutterEdit_Mode" => &["Speed", "Pitch"],
        "stutterEdit_Reverse" => &["Forward", "Reverse", "Fwd/Rev"],
        "vowel_start" | "vowel_end" => &["A", "E", "I", "O", "U"],
        "volume_curve" => &["Linear", "Logarithmic"],
        "preamp_sensitivity" => &["Mic", "Line"],
        "horizon_gatemode" => &["Bass", "Guitar"],
        "horizon_gaterange" => &["Authentic", "Extended"],
        "horizon_attack" => &["1", "2", "3", "4", "5", "6"],
        "ashville_direction" => &["Forward", "Reverse"],
        "ashville_voice" => &["Bass", "Mid"],
        "condition" => &["Stable", "Critical", "Hazard"],
        "lo_mid_freq" => &["250 Hz", "500 Hz", "1 kHz"],
        "hi_mid_freq" => &["750 Hz", "1.5 kHz", "3 kHz"],
        "input_pad" => &["-10 dB", "0 dB", "10 dB"],
        _ => &[],
    };
    items.iter().map(|s| (*s).to_owned()).collect()
}

/// Resolve a `displayType` entry in `HelixControls.json` (following `"alias"` links).
fn resolve_control(controls: &Json, display: &str) -> (Vec<String>, DisplaySpec) {
    let mut choices = builtin_choices(display);
    let mut spec = DisplaySpec::default();
    let Some(map) = controls.as_object() else {
        apply_fallback_spec(display, &mut spec);
        return (choices, spec);
    };
    let mut chain: Vec<&serde_json::Map<String, Json>> = Vec::new();
    let mut cur_name = display.to_owned();
    for _ in 0..8 {
        let lookup = if !map.contains_key(&cur_name) && cur_name.starts_with("CabMicIr_") && cur_name.ends_with("_Position") {
            "CabMicIr_Position"
        } else {
            cur_name.as_str()
        };
        let Some(obj) = map.get(lookup).and_then(Json::as_object) else { break };
        chain.push(obj);
        match obj.get("alias").and_then(Json::as_str) {
            Some(a) => cur_name = a.to_owned(),
            None => break,
        }
    }
    if chain.is_empty() {
        apply_fallback_spec(display, &mut spec);
        return (choices, spec);
    }
    // Apply from base alias to child override.
    for obj in chain.into_iter().rev() {
        if let Some(s) = obj.get("dspToDisplayScale").and_then(Json::as_f64) {
            spec.scale = s;
        }
        if let Some(o) = obj.get("dspToDisplayIntegerOffset").and_then(Json::as_i64) {
            spec.int_offset = o;
        }
        if let Some(mn) = obj.get("minimumValue").and_then(Json::as_f64) {
            spec.min_val = Some(mn);
        }
        if let Some(mx) = obj.get("maximumValue").and_then(Json::as_f64) {
            spec.max_val = Some(mx);
        }
        if let Some(fu) = obj.get("formatUnits").and_then(Json::as_str) {
            spec.format_units = Some(fu.to_owned());
        }
        if let Some(ct) = obj.get("controlType").and_then(Json::as_str) {
            spec.segmented = ct == "segmented";
        }
        match obj.get("format") {
            Some(Json::String(f)) => {
                spec.format = Some(f.clone());
            }
            Some(Json::Array(arr)) => {
                if arr.first().is_some_and(Json::is_string) {
                    let list: Vec<String> = arr.iter().filter_map(Json::as_str).map(str::to_owned).collect();
                    if !list.iter().all(|s| s == "-") {
                        choices = list;
                    }
                } else if arr.first().is_some_and(Json::is_object) {
                    spec.pieces = arr
                        .iter()
                        .filter_map(Json::as_object)
                        .map(|p| DisplayPiece {
                            lower: num(p.get("lowerBound"), -1e9),
                            upper: num(p.get("upperBound"), 1e9),
                            mult: num(p.get("unitsMultiplier"), 1.0),
                            format: p.get("format").and_then(Json::as_str).map(str::to_owned),
                            format_units: p.get("formatUnits").and_then(Json::as_str).map(str::to_owned),
                        })
                        .collect();
                }
            }
            _ => {}
        }
    }
    (choices, spec)
}

fn apply_fallback_spec(display: &str, spec: &mut DisplaySpec) {
    match display {
        "generic_knob" | "phasing" => {
            spec.scale = 10.0;
            spec.format = Some("%.1f".into());
        }
        "percent" | "percent_braced" | "percent_feedback" | "variax_strlevel" => {
            spec.scale = 100.0;
            spec.format_units = Some("%.0f %%".into());
        }
        "percent_discrete" => {
            spec.scale = 100.0;
            spec.format_units = Some("%d %%".into());
        }
        "volume" | "volume_eq" | "volume_old" => {
            spec.format_units = Some("%+.1f dB".into());
        }
        "cab_mic_distance" => {
            spec.format_units = Some("%.2f \"".into());
        }
        "time_sec" => {
            spec.format = Some("%.1f sec".into());
        }
        "amp_fac_type" | "matchstick_tone" | "integer1-5" | "synth_preset" | "preamp_fac" | "channel" | "integer_slider_1based" => {
            spec.int_offset = 1;
        }
        _ => {}
    }
}

fn build_param(p: &serde_json::Map<String, Json>, controls: &Json, stereo_variant: bool) -> Option<Param> {
    if !stereo_variant && p.get("stereo-only").and_then(Json::as_bool).unwrap_or(false) {
        return None;
    }
    let id = p.get("symbolicID")?.as_str()?.to_owned();
    let name = p.get("name").and_then(Json::as_str).unwrap_or("").to_owned();
    let value_type = num(p.get("valueType"), 1.0) as i64;
    let display = if stereo_variant {
        p.get("displayType_stereo").and_then(Json::as_str).or_else(|| p.get("displayType").and_then(Json::as_str)).unwrap_or("").to_owned()
    } else {
        p.get("displayType").and_then(Json::as_str).unwrap_or("").to_owned()
    };
    let min = num(p.get("min"), 0.0);
    let max = if stereo_variant { num(p.get("max_stereo").or_else(|| p.get("max")), 1.0) } else { num(p.get("max"), 1.0) };
    let default = if stereo_variant { num(p.get("default_stereo").or_else(|| p.get("default")), 0.0) } else { num(p.get("default"), 0.0) };
    let (mut choices, spec) = resolve_control(controls, &display);
    if choices.is_empty() && value_type == 0 && spec.int_offset != 0 && max >= min && (max - min) <= 32.0 {
        let lo = min.round() as i64;
        let hi = max.round() as i64;
        choices = (lo..=hi).map(|v| format!("{}", v + spec.int_offset)).collect();
    }
    Some(Param { id, name, value_type, display, min, max, default, choices, spec })
}

/// Find every object that looks like a model (an id and a parameter list).
fn collect_models(v: &Json, controls: &Json, out: &mut HashMap<String, ModelInfo>) {
    match v {
        Json::Array(items) => items.iter().for_each(|i| collect_models(i, controls, out)),
        Json::Object(o) => {
            if let (Some(Json::String(id)), Some(Json::Array(params))) = (o.get("symbolicID"), o.get("params")) {
                let mono_supported = o.get("mono").and_then(Json::as_bool).unwrap_or(true);
                let stereo_supported = o.get("stereo").and_then(Json::as_bool).unwrap_or(false);
                let mono_params: Vec<Param> =
                    params.iter().filter_map(|p| build_param(p.as_object()?, controls, !mono_supported && stereo_supported)).collect();
                let stereo_params: Vec<Param> = params.iter().filter_map(|p| build_param(p.as_object()?, controls, true)).collect();
                let base_info = ModelInfo {
                    name: o.get("name").and_then(Json::as_str).unwrap_or(id).to_owned(),
                    load: num(o.get("load"), 0.0),
                    load_stereo: num(o.get("load_stereo"), num(o.get("load"), 0.0)),
                    params: mono_params,
                    cablink: o.get("cablink").and_then(Json::as_str).map(str::to_owned),
                    devices: o
                        .get("devices")
                        .and_then(Json::as_array)
                        .map(|a| a.iter().filter_map(|d| d.get("id").and_then(Json::as_i64)).collect())
                        .unwrap_or_default(),
                };
                if stereo_supported {
                    let mut st_info = base_info.clone();
                    st_info.params = stereo_params;
                    out.insert(format!("{id}Stereo"), st_info);
                }
                out.insert(id.clone(), base_info);
            }
            for (_, child) in o {
                if child.is_array() || child.is_object() {
                    collect_models(child, controls, out);
                }
            }
        }
        _ => {}
    }
}

impl Catalog {
    pub fn load(dir: &Path) -> Result<Catalog, String> {
        let sym_text = std::fs::read_to_string(dir.join("Helix.sym")).map_err(|e| format!("Helix.sym: {e}"))?;
        let sym_json: Json = serde_json::from_str(&sym_text).map_err(|e| format!("Helix.sym: {e}"))?;
        let sym: Vec<SymEntry> = sym_json
            .as_array()
            .ok_or("Helix.sym is not a list")?
            .iter()
            .map(|e| SymEntry {
                symbol: e.get("symbol").and_then(Json::as_str).unwrap_or("").to_owned(),
                params: e
                    .get("parameters")
                    .and_then(Json::as_array)
                    .map(|a| a.iter().filter_map(|p| p.as_str().map(str::to_owned)).collect())
                    .unwrap_or_default(),
            })
            .collect();

        let controls: Json =
            std::fs::read_to_string(dir.join("HelixControls.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Json::Null);

        let mut models = HashMap::new();
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().and_then(|x| x.to_str()) == Some("models") {
                    if let Ok(t) = std::fs::read_to_string(&p) {
                        if let Ok(j) = serde_json::from_str::<Json>(&t) {
                            collect_models(&j, &controls, &mut models);
                        }
                    }
                }
            }
        }

        let mut cat = Catalog::assemble(CatalogSource::HxEdit(dir.to_owned()), sym, models, HashMap::new(), Vec::new());
        if let Ok(t) = std::fs::read_to_string(dir.join("HX_ModelCatalog.json")) {
            if let Ok(j) = serde_json::from_str::<Json>(&t) {
                cat.read_categories(&j);
            }
        }
        Ok(cat)
    }

    /// The one place a catalog is put together (from HX Edit's files or from a pack): the symbol index and the NAM block's look
    /// are worked out from the rest.
    fn assemble(
        source: CatalogSource,
        sym: Vec<SymEntry>,
        models: HashMap<String, ModelInfo>,
        category: HashMap<String, (String, Rgb)>,
        groups: Vec<PickGroup>,
    ) -> Catalog {
        let sym_index = sym.iter().enumerate().map(|(i, s)| (s.symbol.clone(), i)).collect();
        let nam_info = models.get(crate::nam::BASE_SYMBOL).map(|x| ModelInfo {
            name: crate::nam::NAME.to_owned(),
            load: DSP_BUDGET * crate::nam::DSP_SHARE,
            load_stereo: DSP_BUDGET * crate::nam::DSP_SHARE,
            params: x.params.iter().map(nam_param).collect(),
            cablink: None,
            devices: crate::nam::UNITS.to_vec(),
        });
        Catalog { source, sym, sym_index, models, category, groups, nam_info }
    }

    pub fn source(&self) -> &CatalogSource {
        &self.source
    }

    fn read_categories(&mut self, j: &Json) {
        let Some(cats) = j.get("categories").and_then(Json::as_array) else { return };
        for c in cats {
            let cname = c.get("name").and_then(Json::as_str).unwrap_or("").to_owned();
            if cname == "None" {
                continue;
            }
            let colour = colour_from(c.get("color").and_then(Json::as_str).unwrap_or("0x8899aa"));
            let mut subs: Vec<(String, &Vec<Json>)> = Vec::new();
            if let Some(m) = c.get("models").and_then(Json::as_array) {
                subs.push((String::new(), m));
            }
            if let Some(s) = c.get("subcategories").and_then(Json::as_array) {
                for sc in s {
                    if let Some(m) = sc.get("models").and_then(Json::as_array) {
                        subs.push((sc.get("name").and_then(Json::as_str).unwrap_or("").to_owned(), m));
                    }
                }
            }
            for (sub, list) in subs {
                let mut picks = Vec::new();
                for m in list {
                    let Some(id) = m.get("id").and_then(Json::as_str) else { continue };
                    self.category.entry(id.to_owned()).or_insert_with(|| (cname.clone(), colour));
                    // Entries that only point back at another group are not repeated.
                    if m.get("use_subcategory").is_some() {
                        continue;
                    }
                    let Some(number) = self.number_for(id, &sub) else { continue };
                    let name = m.get("name").and_then(Json::as_str).map(str::to_owned).unwrap_or_else(|| self.model_name(number));
                    picks.push(PickModel { name, number });
                }
                if !picks.is_empty() {
                    self.groups.push(PickGroup { category: cname.clone(), sub, colour, models: picks });
                }
            }
        }
    }

    /// The pedal's number for a catalog id in a sub-group (Mono, Stereo...).
    fn number_for(&self, id: &str, sub: &str) -> Option<i64> {
        let variants = [format!("{id}{sub}"), id.to_owned(), format!("{id}Mono"), format!("{id}Stereo")];
        variants.iter().find_map(|v| self.sym_index.get(v)).map(|i| *i as i64)
    }

    pub fn model_count(&self) -> usize {
        self.models.len()
    }

    #[cfg(test)]
    pub fn number_of_symbol(&self, symbol: &str) -> Option<i64> {
        self.sym_index.get(symbol).map(|i| *i as i64)
    }

    pub fn symbol(&self, number: i64) -> Option<&str> {
        self.sym.get(usize::try_from(number).ok()?).map(|s| s.symbol.as_str())
    }

    pub fn info_for_symbol(&self, symbol: &str) -> Option<&ModelInfo> {
        if crate::nam::is_nam_symbol(symbol) {
            if let Some(i) = &self.nam_info {
                return Some(i);
            }
        }
        self.models.get(symbol).or_else(|| self.models.get(base_id(symbol)))
    }

    /// The symbol and pedal number of the NAM A2 Lite block (the mono Xenomorph Fuzz).
    pub fn nam_number(&self) -> Option<i64> {
        self.sym_index.get(&format!("{}Mono", crate::nam::BASE_SYMBOL)).map(|&i| i as i64)
    }

    /// The model picker's groups, with the NAM A2 Lite block where its firmware lists it.
    pub fn picker_groups(&self) -> Vec<PickGroup> {
        let mut groups = self.groups.clone();
        // "Amp+Cab": the amps again, each placed with its matching cab (as HX Edit offers).
        if !groups.iter().any(|g| g.category.contains("Amp") && g.category.contains("Cab")) {
            let amps: Vec<PickGroup> = groups
                .iter()
                .filter(|g| g.category == "Amp")
                .map(|g| PickGroup {
                    category: "Amp+Cab".into(),
                    sub: g.sub.clone(),
                    colour: g.colour,
                    models: g.models.iter().filter(|m| self.paired_cab(m.number).is_some()).cloned().collect(),
                })
                .filter(|g| !g.models.is_empty())
                .collect();
            if let Some(at) = groups.iter().rposition(|g| g.category == "Amp").map(|i| i + 1) {
                for (k, g) in amps.into_iter().enumerate() {
                    groups.insert(at + k, g);
                }
            }
        }
        let Some(num) = self.nam_number().filter(|_| crate::nam::active()) else { return groups };
        if crate::nam::in_amp_menu() {
            // HX Stomp / XL: Amp → NAM, and the mono Xenomorph is gone from Distortion.
            for g in &mut groups {
                g.models.retain(|m| m.number != num);
            }
            let colour = groups.iter().find(|g| g.category == "Amp").map(|g| g.colour).unwrap_or(Rgb(0xff, 0x8a, 0x3d));
            let at = groups.iter().rposition(|g| g.category == "Amp").map(|i| i + 1).unwrap_or(groups.len());
            groups.insert(
                at,
                PickGroup {
                    category: "Amp".into(),
                    sub: "NAM".into(),
                    colour,
                    models: vec![PickModel { name: crate::nam::NAME.into(), number: num }],
                },
            );
        } else {
            // HX Effects: in the mono Xenomorph's place.
            for g in &mut groups {
                for m in &mut g.models {
                    if m.number == num {
                        m.name = crate::nam::NAME.into();
                    }
                }
            }
        }
        groups
    }

    pub fn info_for_hlx(&self, symbol: &str, stereo: bool) -> Option<&ModelInfo> {
        if stereo {
            let st = format!("{}Stereo", base_id(symbol));
            if let Some(i) = self.models.get(&st) {
                return Some(i);
            }
        }
        self.info_for_symbol(symbol)
    }

    pub fn info(&self, number: i64) -> Option<&ModelInfo> {
        self.info_for_symbol(self.symbol(number)?)
    }

    /// A model's display name, with its variant when there is one.
    pub fn model_name(&self, number: i64) -> String {
        let Some(sym) = self.symbol(number) else { return format!("Model {number}") };
        let variant = if sym.ends_with("Stereo") { " (stereo)" } else { "" };
        match self.info_for_symbol(sym) {
            Some(i) => format!("{}{variant}", crate::realnames::show(&i.name)),
            None => crate::hlx::pretty_model(sym),
        }
    }

    pub fn category_of_symbol(&self, symbol: &str) -> Option<(String, Rgb)> {
        if crate::nam::is_nam_symbol(symbol) && crate::nam::in_amp_menu() {
            if let Some(g) = self.groups.iter().find(|g| g.category == "Amp") {
                return Some(("Amp".into(), g.colour));
            }
        }
        self.category.get(base_id(symbol)).or_else(|| self.category.get(symbol)).cloned()
    }

    pub fn category(&self, number: i64) -> Option<(String, Rgb)> {
        self.category_of_symbol(self.symbol(number)?)
    }

    /// Name and description of parameter `index` of model `number`, in the pedal's order.
    pub fn param(&self, number: i64, index: usize) -> (String, Option<Param>) {
        let sym = self.sym.get(usize::try_from(number).unwrap_or(usize::MAX));
        let info = sym.and_then(|s| self.info_for_symbol(&s.symbol));
        let mut pid = sym.and_then(|s| s.params.get(index)).cloned();
        if pid.is_none() {
            // The pedal keeps one more value after the parameters `Helix.sym` lists: a single cab's Mic (HX Edit's `@mic`, a
            // choice of 16 mics) and a delay's Trails (`@trails`, checked on a Simple Delay by switching Trails on and reading
            // the preset back: the seventh value went from false to true). Name it instead of leaving "Setting N".
            if let (Some(s), Some(i)) = (sym, info) {
                if index == s.params.len() {
                    pid = ["@mic", "@trails"].into_iter().find(|id| i.params.iter().any(|p| p.id == *id)).map(str::to_owned);
                }
            }
        }
        let meta = match (&pid, info) {
            (Some(pid), Some(info)) => info.params.iter().find(|p| &p.id == pid).cloned(),
            _ => None,
        };
        let name =
            meta.as_ref().map(|m| m.name.clone()).filter(|n| !n.is_empty()).or(pid).unwrap_or_else(|| format!("Setting {}", index + 1));
        (name, meta)
    }

    /// Whether value `index` of model `number` sits after the parameters the pedal lets be set one by one (a single cab's Mic, a
    /// delay's Trails): those are changed by writing the document back.
    pub fn is_hidden_value(&self, number: i64, index: usize) -> bool {
        self.sym.get(usize::try_from(number).unwrap_or(usize::MAX)).is_some_and(|s| index >= s.params.len())
    }

    /// The matching cab for an amp, by the pedal's number.
    pub fn paired_cab(&self, number: i64) -> Option<i64> {
        let link = self.info(number)?.cablink.clone()?;
        self.number_for(&link, "")
    }
}

/// Format a number with one of `HelixControls.json`'s printf patterns.
fn c_printf(pattern: &str, val: f64) -> String {
    let clean = pattern.replace("%%", "\x01");
    if !clean.contains('%') {
        return clean.replace('\x01', "%");
    }
    type Format = fn(f64) -> String;
    let specs: &[(&str, Format)] = &[
        ("%+.0f", |x| format!("{x:+.0}")),
        ("%+.1f", |x| format!("{x:+.1}")),
        ("%+.2f", |x| format!("{x:+.2}")),
        ("%.0f", |x| format!("{x:.0}")),
        ("%.1f", |x| format!("{x:.1}")),
        ("%.2f", |x| format!("{x:.2}")),
        ("%.3f", |x| format!("{x:.3}")),
        ("%f", |x| format!("{x:.1}")),
        ("%+d", |x| format!("{:+}", x.round() as i64)),
        ("%03d", |x| format!("{:03}", x.round() as i64)),
        ("%d", |x| format!("{}", x.round() as i64)),
    ];
    for (tok, f) in specs {
        if let Some(pos) = clean.find(tok) {
            let mut out = String::new();
            out.push_str(&clean[..pos]);
            out.push_str(&f(val));
            out.push_str(&clean[pos + tok.len()..]);
            return out.replace('\x01', "%");
        }
    }
    clean.replace('\x01', "%")
}

/// A number as shown (or typed): "Left 30" is -30, "1.2 kHz" is 1.2 with a ×1000 unit,
/// "Center" is 0. Returns the bare number and the unit's multiplier, if it says one.
fn shown_number(s: &str) -> Option<(f64, Option<f64>)> {
    let l = s.trim().to_lowercase();
    if matches!(l.as_str(), "center" | "centre" | "c") {
        return Some((0.0, None));
    }
    let chars: Vec<char> = l.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next_digit = chars.get(i + 1).is_some_and(|n| n.is_ascii_digit() || *n == '.');
        if c.is_ascii_digit() || (c == '.' && next_digit) || ((c == '-' || c == '+') && next_digit) {
            break;
        }
        i += 1;
    }
    if i >= chars.len() {
        return None;
    }
    let mut j = i + 1;
    while j < chars.len() && (chars[j].is_ascii_digit() || chars[j] == '.') {
        j += 1;
    }
    let mut n: f64 = chars[i..j].iter().collect::<String>().parse().ok()?;
    let rest: String = chars[j..].iter().collect::<String>().trim().to_owned();
    if l.starts_with('l') {
        n = -n.abs();
    }
    let unit = if rest.starts_with("khz") || rest == "k" {
        Some(1000.0)
    } else if rest.starts_with("hz") || rest.starts_with("ms") {
        Some(1.0)
    } else if rest == "s" || rest == "sec" {
        Some(1000.0)
    } else {
        None
    };
    Some((n, unit))
}

/// The value between `lo` and `hi` that shows as close as possible to what the person typed
/// ("12", "-3.5 dB", "1.2k", "Left 20", "Off"), searched over the shown values themselves.
pub fn parse_typed(meta: Option<&Param>, typed: &str, lo: f64, hi: f64) -> Option<f64> {
    let typed = typed.trim();
    if typed.is_empty() {
        return None;
    }
    if meta.is_none() || hi <= lo {
        return shown_number(typed).map(|(n, u)| (n * u.unwrap_or(1.0)).clamp(lo.min(hi), hi.max(lo)));
    }
    const N: usize = 4000;
    let raw_at = |k: usize| lo + (hi - lo) * k as f64 / N as f64;
    let want = shown_number(typed);
    let word = typed.to_lowercase();
    let errs: Vec<f64> = (0..=N)
        .map(|k| {
            let shown = format_value(meta, raw_at(k));
            match (&want, shown_number(&shown)) {
                (Some((n, Some(u))), Some((dn, du))) => (n * u - dn * du.unwrap_or(1.0)).abs(),
                (Some((n, None)), Some((dn, du))) => (n - dn).abs().min((n - dn * du.unwrap_or(1.0)).abs()),
                (None, _) if shown.to_lowercase().starts_with(&word) => 0.0,
                _ => f64::INFINITY,
            }
        })
        .collect();
    let (best, min) = errs.iter().enumerate().fold((0, f64::INFINITY), |a, (k, e)| if *e < a.1 { (k, *e) } else { a });
    if !min.is_finite() {
        return None;
    }
    // Several values show the same; take the middle of that run.
    let (mut a, mut b) = (best, best);
    while a > 0 && errs[a - 1] <= min + 1e-9 {
        a -= 1;
    }
    while b < N && errs[b + 1] <= min + 1e-9 {
        b += 1;
    }
    Some(raw_at((a + b) / 2))
}

/// How to show a value, from the parameter's display type (`HelixControls.json` + fallbacks).
pub fn format_value(meta: Option<&Param>, v: f64) -> String {
    let Some(m) = meta else { return format!("{v:.2}") };
    if let Some(lbl) = m.choice_label(v.round() as i64) {
        return lbl.to_owned();
    }
    let spec = &m.spec;
    let dv = match (spec.min_val, spec.max_val) {
        (Some(lo_v), Some(hi_v)) if m.max > m.min => lo_v + ((v - m.min) / (m.max - m.min)) * (hi_v - lo_v),
        _ => v * spec.scale + (spec.int_offset as f64),
    };
    if !spec.pieces.is_empty() {
        let piece = spec
            .pieces
            .iter()
            // From each range's lower bound up to (not including) its upper one, as HX Edit
            // shows them: a cab mic's Position reads "Center" at 0 and "Edge" at 10.0.
            .find(|p| dv >= p.lower && dv < p.upper)
            .or_else(|| spec.pieces.iter().rev().find(|p| dv >= p.lower && dv <= p.upper))
            .or_else(|| if dv < spec.pieces[0].lower { spec.pieces.first() } else { spec.pieces.last() });
        if let Some(p) = piece {
            let shown = dv * p.mult;
            let pat = p.format_units.as_deref().or(p.format.as_deref()).unwrap_or("%.1f");
            return c_printf(pat, shown);
        }
    }
    if let Some(pat) = spec.format_units.as_deref().or(spec.format.as_deref()) {
        return c_printf(pat, dv);
    }
    match m.display.as_str() {
        // The NAM A2 Lite block (optional firmware).
        "nam_input" => format!("{:+.1} dB", (v - 0.5) * 40.0),
        "nam_band" => format!("{:+.1} dB", (v - 0.5) * 24.0),
        "nam_output" => format!("{:.1}", v * 10.0),
        "nam_slot" => {
            let slot = v.trunc() as i64;
            if (1..=128).contains(&slot) {
                match crate::nam::ir_name(slot) {
                    Some(n) => format!("{slot} · {n}"),
                    None => format!("IR slot {slot}"),
                }
            } else {
                "<EMPTY>".into()
            }
        }
        "nam_midfreq" => {
            if v >= 1000.0 {
                format!("{:.2} kHz", v / 1000.0)
            } else {
                format!("{v:.0} Hz")
            }
        }
        "nam_highcut" if v >= 0.995 => "Off".into(),
        "nam_highcut" => format!("{:.1}", v * 10.0),
        "generic_knob" | "raw_knob" if m.max <= 1.0 && m.min >= 0.0 => format!("{:.1}", v * 10.0),
        "percent" | "percent_braced" if m.max <= 1.0 => format!("{:.0}%", v * 100.0),
        "volume" | "volume_eq" => format!("{v:+.1} dB"),
        "pan" | "split_balance" => {
            if (v - 0.5).abs() < 0.005 {
                "Center".into()
            } else if v < 0.5 {
                format!("Left {:.0}", (0.5 - v) * 200.0)
            } else {
                format!("Right {:.0}", (v - 0.5) * 200.0)
            }
        }
        d if d.contains("low_cut") && v <= 20.0 => "Off".into(),
        d if d.contains("high_cut") && v >= 20000.1 => "Off".into(),
        d if d.contains("freq") || d.contains("cut") => {
            if v >= 1000.0 {
                format!("{:.1} kHz", v / 1000.0)
            } else {
                format!("{v:.0} Hz")
            }
        }
        d if d.contains("time_ms") || d == "comp_attack" || d == "comp_release" => {
            let ms = if m.max <= 10.0 { v * 1000.0 } else { v };
            if ms < 10.0 {
                format!("{ms:.1} ms")
            } else if ms >= 1000.0 {
                format!("{:.3} s", ms / 1000.0)
            } else {
                format!("{ms:.0} ms")
            }
        }
        _ if m.value_type == 0 => format!("{:.0}", dv),
        _ => format!("{dv:.2}"),
    }
}

/// The first number in `text` (sign, decimal point or decimal comma allowed) and what follows it.
fn first_number(text: &str) -> Option<(f64, String)> {
    let cs: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        let starts = c.is_ascii_digit()
            || ((c == '-' || c == '+' || c == '.' || c == ',' || c == '\u{2212}')
                && cs.get(i + 1).is_some_and(|n| {
                    n.is_ascii_digit() || ((*n == '.' || *n == ',') && cs.get(i + 2).is_some_and(|d| d.is_ascii_digit()))
                }));
        if starts {
            let mut s = String::new();
            let mut j = i;
            let mut dot = false;
            while j < cs.len() {
                let d = cs[j];
                if d.is_ascii_digit() {
                    s.push(d);
                } else if (j == i) && (d == '-' || d == '+' || d == '\u{2212}') {
                    s.push(if d == '+' { '+' } else { '-' });
                } else if (d == '.' || d == ',') && !dot {
                    dot = true;
                    s.push('.');
                } else {
                    break;
                }
                j += 1;
            }
            if let Ok(v) = s.parse::<f64>() {
                return Some((v, cs[j..].iter().collect::<String>().trim().to_owned()));
            }
        }
        i += 1;
    }
    None
}

/// The raw value behind a displayed-by-the-spec value `dv` (the inverse of `format_value`'s outer mapping).
fn raw_from_dv(m: &Param, dv: f64) -> f64 {
    let spec = &m.spec;
    match (spec.min_val, spec.max_val) {
        (Some(lo_v), Some(hi_v)) if m.max > m.min => {
            if (hi_v - lo_v).abs() < f64::EPSILON {
                m.min
            } else {
                m.min + (dv - lo_v) / (hi_v - lo_v) * (m.max - m.min)
            }
        }
        _ => {
            if spec.scale == 0.0 {
                dv
            } else {
                (dv - spec.int_offset as f64) / spec.scale
            }
        }
    }
}

/// What a person typed for a knob (in the units `format_value` shows) as the raw value, or `None` when it is not a value.
/// The inverse of [`format_value`].
pub fn parse_shown(meta: Option<&Param>, text: &str) -> Option<f64> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let Some(m) = meta else { return first_number(text).map(|(v, _)| v) };
    let clamp = |v: f64| if m.max > m.min { v.clamp(m.min, m.max) } else { v };
    let lower = text.to_lowercase();
    if !m.choices.is_empty() {
        let base = if m.value_type == 2 { 0 } else { m.min.round() as i64 };
        if let Some(i) = m.choices.iter().position(|c| c.trim().to_lowercase() == lower) {
            return Some((base + i as i64) as f64);
        }
    }
    let spec = &m.spec;
    // A piece shown as a word ("Center"): the one whose label at its lower bound matches.
    for p in &spec.pieces {
        let pat = p.format_units.as_deref().or(p.format.as_deref()).unwrap_or("%.1f");
        if c_printf(pat, p.lower * p.mult).trim().to_lowercase() == lower {
            return Some(clamp(raw_from_dv(m, p.lower)));
        }
    }
    let Some((n, rest)) = first_number(text) else {
        // Words only: the few labels the display rules print ("Off", "Center").
        let d = m.display.as_str();
        return match lower.as_str() {
            "off" if d == "nam_highcut" => Some(1.0),
            "off" if d.contains("low_cut") => Some(m.min),
            "off" if d.contains("high_cut") => Some(m.max),
            "center" if d == "pan" || d == "split_balance" => Some(0.5),
            _ => None,
        };
    };
    let unit = rest.to_lowercase();
    if !spec.pieces.is_empty() {
        let shown = |p: &DisplayPiece| (p.lower * p.mult, p.upper * p.mult);
        // Ranges in different units overlap (500 ms, 1.5 s): the typed unit picks the piece, else the first.
        let words_of = |p: &DisplayPiece| {
            let pat = p.format_units.as_deref().or(p.format.as_deref()).unwrap_or("").replace("%%", "\x01");
            let start = pat.find('%').unwrap_or(0);
            let end = pat.rfind(['f', 'd']).map_or(pat.len(), |i| i + 1);
            let tidy = |s: &str| s.trim().replace('\x01', "%").to_lowercase();
            (tidy(&pat[..start]), tidy(pat.get(end..).unwrap_or("")))
        };
        let in_range = |p: &&DisplayPiece| {
            let (a, b) = shown(p);
            n >= a.min(b) && n <= a.max(b)
        };
        let by_unit = |p: &&DisplayPiece| {
            let (pre, suf) = words_of(p);
            (!pre.is_empty() && lower.contains(&pre)) || (!suf.is_empty() && unit.starts_with(&suf))
        };
        let inside = spec.pieces.iter().filter(in_range).find(by_unit).or_else(|| spec.pieces.iter().find(in_range));
        let piece = inside.or_else(|| {
            spec.pieces.iter().min_by(|x, y| {
                let d = |p: &DisplayPiece| {
                    let (a, b) = shown(p);
                    if n < a.min(b) {
                        a.min(b) - n
                    } else {
                        n - a.max(b)
                    }
                };
                d(x).total_cmp(&d(y))
            })
        })?;
        let dv = if piece.mult == 0.0 { n } else { n / piece.mult };
        return Some(clamp(raw_from_dv(m, dv)));
    }
    if spec.format_units.is_some() || spec.format.is_some() {
        return Some(clamp(raw_from_dv(m, n)));
    }
    let khz = unit.starts_with("khz") || unit.starts_with('k');
    let v = match m.display.as_str() {
        "nam_input" => n / 40.0 + 0.5,
        "nam_band" => n / 24.0 + 0.5,
        "nam_output" => n / 10.0,
        "nam_highcut" if lower == "off" => 1.0,
        "nam_highcut" => n / 10.0,
        "nam_midfreq" => {
            if khz {
                n * 1000.0
            } else {
                n
            }
        }
        "generic_knob" | "raw_knob" if m.max <= 1.0 && m.min >= 0.0 => n / 10.0,
        "percent" | "percent_braced" if m.max <= 1.0 => n / 100.0,
        "volume" | "volume_eq" => n,
        "pan" | "split_balance" => {
            if lower == "center" {
                0.5
            } else if lower.starts_with("left") {
                0.5 - n.abs() / 200.0
            } else if lower.starts_with("right") {
                0.5 + n.abs() / 200.0
            } else {
                n
            }
        }
        d if d.contains("low_cut") && lower == "off" => m.min,
        d if d.contains("high_cut") && lower == "off" => m.max,
        d if d.contains("freq") || d.contains("cut") => {
            if khz {
                n * 1000.0
            } else {
                n
            }
        }
        d if d.contains("time_ms") || d == "comp_attack" || d == "comp_release" => {
            let ms = if unit.starts_with("ms") || !unit.starts_with('s') { n } else { n * 1000.0 };
            if m.max <= 10.0 {
                ms / 1000.0
            } else {
                ms
            }
        }
        _ => return Some(clamp(raw_from_dv(m, n))),
    };
    Some(clamp(v))
}

impl Catalog {
    /// DSP cost of a model, in the catalog's own units.
    pub fn dsp_cost(&self, number: i64) -> f64 {
        let Some(sym) = self.symbol(number) else { return 0.0 };
        // Split and merge are routing: every pedal preset keeps both even when the path
        // isn't split, and counting them (the merge alone is listed at ~11) put presets
        // that run fine on the pedal over 100%.
        if sym.starts_with("HD2_AppDSPFlow") {
            return 0.0;
        }
        match self.info_for_symbol(sym) {
            Some(i) if sym.ends_with("Stereo") => i.load_stereo,
            Some(i) => i.load,
            None => 0.0,
        }
    }

    /// Whether a .hlx model can run on a unit: effects limited to some units,
    /// and HX Effects, which has no amps, preamps or cabs.
    pub fn allowed_on(&self, model: &str, device: i64) -> bool {
        let Some(info) = self.info_for_symbol(model) else { return true };
        if !info.devices.is_empty() && !info.devices.contains(&device) {
            return false;
        }
        if device == 2162693 {
            let cat = self.category_of_symbol(model).map(|c| c.0).unwrap_or_default();
            let amp_or_cab = matches!(cat.as_str(), "Amp" | "Amp+Cab" | "Preamp" | "Cab");
            if amp_or_cab || model.starts_with("HD2_Amp") || model.starts_with("HD2_Cab") || model.starts_with("HD2_Preamp") {
                return false;
            }
        }
        true
    }

    /// The model's parameters in the pedal's own order (impulse data excluded).
    pub fn wire_params(&self, number: i64) -> Vec<String> {
        self.sym
            .get(usize::try_from(number).unwrap_or(usize::MAX))
            .map(|s| s.params.iter().filter(|p| p.as_str() != "IrData").cloned().collect())
            .unwrap_or_default()
    }

    fn param_meta(&self, number: i64, id: &str) -> Option<Param> {
        self.info(number)?.params.iter().find(|p| p.id == id).cloned()
    }

    /// Category name as HX Edit's catalog gives it ("Distortion", "Amp", "Cab"...).
    pub fn category_name(&self, number: i64) -> String {
        self.category(number).map(|c| c.0).unwrap_or_default()
    }

    /// The model id a .hlx file uses: the symbol without its Mono/Stereo ending.
    pub fn hlx_model(&self, number: i64) -> Option<(String, bool)> {
        let sym = self.symbol(number)?;
        Some((base_id(sym).to_owned(), sym.ends_with("Stereo")))
    }

    /// The pedal's number for a .hlx `@model`, honouring `@stereo`.
    pub fn number_for_hlx(&self, model: &str, stereo: bool) -> Option<i64> {
        let (first, second) = if stereo { ("Stereo", "Mono") } else { ("Mono", "Stereo") };
        [format!("{model}{first}"), model.to_owned(), format!("{model}{second}")]
            .iter()
            .find_map(|s| self.sym_index.get(s))
            .map(|i| *i as i64)
    }

    /// Each parameter with its type (0 whole number, 1 decimal, 2 on/off) and
    /// default, in the pedal's order.
    pub fn default_values(&self, number: i64) -> Vec<(String, i64, f64)> {
        self.wire_params(number)
            .into_iter()
            .map(|id| {
                let m = self.param_meta(number, &id);
                let vt = m.as_ref().map(|m| m.value_type).unwrap_or(1);
                let d = m.as_ref().map(|m| m.default).unwrap_or(0.0);
                (id, vt, d)
            })
            .collect()
    }

    /// The "kind" number the pedal keeps with a block (field 9), by category.
    pub fn kind_code(&self, number: i64) -> i64 {
        let Some(sym) = self.symbol(number) else { return 1 };
        let base = base_id(sym);
        let fixed: &[(&str, i64)] = &[
            ("HD2_SendMono1", 25),
            ("HD2_SendMono2", 26),
            ("HD2_FXLoopMono2", 10),
            ("HD2_FXLoopMono1", 9),
            ("HD2_FXLoopStereo1_2", 13),
            ("HD2_SendStereo1_2", 29),
            ("HD2_ReturnMono1", 2),
            ("HD2_ReturnMono2", 3),
            ("HD2_ReturnStereo1_2", 6),
            ("HD2_ImpulseResponse2048", 20),
        ];
        if let Some((_, k)) = fixed.iter().find(|(s, _)| *s == sym || *s == base) {
            return *k;
        }
        if sym.starts_with("HD2_Cab") {
            return if sym.starts_with("HD2_CabMicIr") { 31 } else { 15 };
        }
        match self.category_name(number).as_str() {
            "Delay" | "Reverb" => 8,
            "IR" => 19,
            "Amp" | "Amp+Cab" | "Preamp" => 17,
            "Looper" => 22,
            _ => 1,
        }
    }

    /// The kind number for a block carrying an amp (or a cab) together with a cab.
    pub fn paired_kind(&self, first: i64, cab: i64) -> i64 {
        let mic = |n: i64| self.symbol(n).is_some_and(|s| s.starts_with("HD2_CabMicIr"));
        let first_is_cab = self.symbol(first).is_some_and(|s| s.starts_with("HD2_Cab"));
        match (first_is_cab, mic(first), mic(cab)) {
            (false, _, true) => 33,
            (false, _, false) => 18,
            (true, true, true) => 32,
            _ => 16,
        }
    }
}

#[cfg(test)]
mod mic_tests {
    use super::*;

    /// A single cab's value after the five that `Helix.sym` lists is its Mic (a choice of 16), not an unnamed "Setting 6".
    #[test]
    fn a_single_cabs_sixth_value_is_the_mic() {
        let Some(cat) = find_dir(None).and_then(|p| Catalog::load(&p).ok()) else { return }; // needs HX Edit's files
        let Some(n) = cat.number_of_symbol("HD2_Cab1x12FieldCoil") else { return };
        assert_eq!(cat.param(n, 4).0, "Level");
        let (name, meta) = cat.param(n, 5);
        assert_eq!(name, "Mic");
        let m = meta.expect("the Mic has a description");
        assert_eq!((m.min, m.max), (0.0, 15.0));
        assert_eq!(cat.param(n, 6).0, "Setting 7", "nothing after the Mic is named");
    }
}

#[cfg(test)]
mod trails_tests {
    use super::*;

    /// A mono Simple Delay keeps its Trails as a seventh value, after the six `Helix.sym` lists.
    #[test]
    fn a_delays_seventh_value_is_trails() {
        let Some(cat) = find_dir(None).and_then(|p| Catalog::load(&p).ok()) else { return }; // needs HX Edit's files
        let Some(n) = cat.number_of_symbol("HD2_DelaySimpleDelayMono") else { return };
        assert_eq!(cat.param(n, 5).0, "Tempo Sync");
        let (name, meta) = cat.param(n, 6);
        assert_eq!(name, "Trails");
        assert_eq!(meta.map(|m| m.value_type), Some(2), "an on/off switch");
    }
}

#[cfg(test)]
mod trails_everywhere_tests {
    use super::*;

    /// Every model HX Edit gives a Trails (delays, reverbs, loops) names the value after its listed parameters "Trails", as an
    /// on/off switch, and every cab with a Mic names it "Mic". (The pedal was only seen storing it for a mono Simple Delay.)
    #[test]
    fn every_model_with_trails_or_a_mic_names_its_extra_value() {
        let Some(cat) = find_dir(None).and_then(|p| Catalog::load(&p).ok()) else { return }; // needs HX Edit's files
        let (mut trails, mut mics) = (0, 0);
        for n in 0..cat.sym.len() as i64 {
            let Some(info) = cat.symbol(n).and_then(|s| cat.info_for_symbol(s)) else { continue };
            let extra = cat.sym[n as usize].params.len();
            let (name, meta) = cat.param(n, extra);
            if info.params.iter().any(|p| p.id == "@mic") {
                assert_eq!(name, "Mic", "{}", cat.symbol(n).unwrap_or(""));
                mics += 1;
            } else if info.params.iter().any(|p| p.id == "@trails") {
                assert_eq!(name, "Trails", "{}", cat.symbol(n).unwrap_or(""));
                assert_eq!(meta.map(|m| m.value_type), Some(2), "{}", cat.symbol(n).unwrap_or(""));
                trails += 1;
            }
        }
        assert!(trails > 50 && mics > 20, "{trails} models with Trails, {mics} with a Mic");
    }
}

#[cfg(test)]
mod colour_tests {
    use super::*;

    #[test]
    fn colour_from_reads_hex() {
        assert_eq!(colour_from("0x8899aa"), Rgb(0x88, 0x99, 0xaa));
        assert_eq!(colour_from("nonsense"), colour_from("0x8899aa"));
    }
}

#[cfg(test)]
mod builtin_tests {
    use super::*;

    #[test]
    fn choose_prefers_newer_builtin() {
        assert_eq!(choose(Some(900), Some(950)), Pick::Builtin);
        assert_eq!(choose(Some(950), Some(950)), Pick::Local);
        assert_eq!(choose(Some(990), Some(950)), Pick::Local);
    }

    #[test]
    fn choose_falls_back_when_folder_missing() {
        assert_eq!(choose(None, Some(950)), Pick::Builtin);
        assert_eq!(choose(Some(900), None), Pick::Local);
        assert_eq!(choose(None, None), Pick::None);
    }

    #[test]
    fn builtin_catalog_loads() {
        let (version, bytes) = builtin().expect("catalogs/ has at least one version");
        assert!(!version.is_empty());
        let cat = Catalog::from_pack(bytes).unwrap();
        assert!(cat.model_count() > 100);
    }

    #[test]
    fn load_best_with_no_local_folder_uses_builtin_and_names_its_version() {
        let cat = best_of(None).expect("built-in list");
        let (version, _) = builtin().unwrap();
        assert_eq!(cat.source(), &CatalogSource::Builtin { hx_edit_version: version.to_owned() });
    }
}

#[cfg(test)]
mod parse_shown_tests {
    use super::*;

    fn param(display: &str, min: f64, max: f64, spec: DisplaySpec) -> Param {
        Param { id: "x".into(), name: "X".into(), value_type: 1, display: display.into(), min, max, default: min, choices: vec![], spec }
    }

    fn round_trips(m: &Param) {
        for v in [m.min, (m.min + m.max) / 2.0, m.max] {
            let shown = format_value(Some(m), v);
            let back = parse_shown(Some(m), &shown).unwrap_or_else(|| panic!("{shown:?} did not parse"));
            // Within what the display rounds to: re-formatting gives the same text.
            assert_eq!(format_value(Some(m), back), shown, "{shown:?}: {v} -> {back}");
        }
    }

    #[test]
    fn typed_values_are_in_the_shown_units() {
        let knob = param("generic_knob", 0.0, 1.0, DisplaySpec::default());
        assert_eq!(parse_shown(Some(&knob), "5"), Some(0.5));
        assert_eq!(parse_shown(Some(&knob), "2,5"), Some(0.25));
        assert_eq!(parse_shown(Some(&knob), "garbage"), None);
        assert_eq!(parse_shown(Some(&knob), ""), None);
        assert_eq!(parse_shown(None, "3.25"), Some(3.25));
        round_trips(&knob);

        let linear = param(
            "x",
            0.0,
            1.0,
            DisplaySpec { min_val: Some(-12.0), max_val: Some(12.0), format_units: Some("%+.1f dB".into()), ..Default::default() },
        );
        assert_eq!(parse_shown(Some(&linear), "-12 dB"), Some(0.0));
        round_trips(&linear);

        let scaled = param("x", 0.0, 100.0, DisplaySpec { scale: 0.5, int_offset: 10, format: Some("%.1f".into()), ..Default::default() });
        assert_eq!(parse_shown(Some(&scaled), "35"), Some(50.0));
        round_trips(&scaled);

        let pieces = param(
            "x",
            0.0,
            1.0,
            DisplaySpec {
                min_val: Some(0.0),
                max_val: Some(1000.0),
                pieces: vec![
                    DisplayPiece { lower: 0.0, upper: 1.0, mult: 1000.0, format: None, format_units: Some("%.0f ms".into()) },
                    DisplayPiece { lower: 1.0, upper: 1000.0, mult: 0.001, format: None, format_units: Some("%.2f s".into()) },
                ],
                ..Default::default()
            },
        );
        round_trips(&pieces);

        let word = param(
            "x",
            0.0,
            1.0,
            DisplaySpec {
                min_val: Some(0.0),
                max_val: Some(10.0),
                pieces: vec![
                    DisplayPiece { lower: 0.0, upper: 0.0, mult: 1.0, format: None, format_units: Some("Center".into()) },
                    DisplayPiece { lower: 0.0, upper: 10.0, mult: 1.0, format: Some("%.1f".into()), format_units: None },
                ],
                ..Default::default()
            },
        );
        assert_eq!(parse_shown(Some(&word), "center"), Some(0.0));
        assert_eq!(parse_shown(Some(&word), "5"), Some(0.5));
    }

    #[test]
    fn choice_labels_parse_to_their_value() {
        let mut m = param("x", 2.0, 4.0, DisplaySpec::default());
        m.value_type = 0;
        m.choices = vec!["Low".into(), "Mid".into(), "High".into()];
        assert_eq!(parse_shown(Some(&m), "mid"), Some(3.0));
        assert_eq!(parse_shown(Some(&m), " HIGH "), Some(4.0));
        round_trips(&m);
    }

    /// Across the built-in list, what `format_value` shows parses back to something that shows the same.
    #[test]
    fn every_builtin_param_round_trips() {
        let (_, bytes) = builtin().expect("built-in list");
        let cat = Catalog::from_pack(bytes).unwrap();
        let (mut n, mut offset, mut bad) = (0, 0, Vec::new());
        for info in cat.models.values() {
            for m in &info.params {
                n += 1;
                if m.spec.int_offset != 0 {
                    offset += 1;
                }
                for v in [m.min, (m.min + m.max) / 2.0, m.max] {
                    let shown = format_value(Some(m), v);
                    match parse_shown(Some(m), &shown) {
                        Some(b) if format_value(Some(m), b) == shown => {}
                        other => bad.push(format!("{}.{} {v}: {shown:?} -> {other:?}", info.name, m.id)),
                    }
                }
            }
        }
        eprintln!("params {n}, with int_offset != 0: {offset}, not round-tripping: {}", bad.len());
        for b in bad.iter().take(25) {
            eprintln!("  {b}");
        }
        assert!(bad.len() <= 5, "too many mismatches ({} of {n})", bad.len());
    }
}
