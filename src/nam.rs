//! Optional support for the community "NAM A2 Lite" firmware block on the
//! HX Stomp, HX Stomp XL and HX Effects (github.com/nocaseguy/hx-stomp-nam-lite),
//! and a converter that turns a `.nam` capture into the User IR `.wav` that
//! block loads.
//!
//! The block takes the place of the stock "Xenomorph Fuzz" (mono) model, so
//! presets store it as that model; this module only changes what the editor
//! shows (its name, knob names and values) when the person turns it on.
//!
//! The converter is our own code written from the published file layout:
//! a 2,048-sample, 48 kHz, mono 32-bit float WAV whose samples are
//!   0.5, -0.5, 3.0, 2^e, 1871/1024, level, loudness, 0,
//! then the capture's 1,871 weights in the order the block loads them, then
//! 72 words of its silence state (each / 16), all divided by 2^e so every
//! sample stays within HX Edit's +-15.8 limit; the rest are 0.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

use serde_json::Value;

static ENABLED: AtomicBool = AtomicBool::new(false);
static UNIT: AtomicI64 = AtomicI64::new(2162694);

/// The stock model the NAM block replaces (its mono version).
pub const BASE_SYMBOL: &str = "HD2_DistXenomorphFuzz";
pub const NAME: &str = "NAM A2 Lite";
/// The units the firmware exists for: HX Stomp, HX Stomp XL, HX Effects.
pub const UNITS: [i64; 3] = [2162694, 2162699, 2162693];
const HX_EFFECTS: i64 = 2162693;
/// About how much of the DSP one block uses on an HX Stomp (README: ~23%).
pub const DSP_SHARE: f64 = 0.23;
/// Most NAM blocks a preset can run at once.
pub const MAX_BLOCKS: usize = 3;

pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}
pub fn set_unit(unit: i64) {
    UNIT.store(unit, Ordering::Relaxed);
}
/// The unit the app works for now (the connected pedal, or the one picked).
pub fn unit() -> i64 {
    UNIT.load(Ordering::Relaxed)
}
/// Whether the NAM block is shown now: turned on, and the unit can run it.
pub fn active() -> bool {
    enabled() && UNITS.contains(&UNIT.load(Ordering::Relaxed))
}
/// On the HX Stomp and XL it is listed under Amp -> NAM; on the HX Effects in Distortion's place.
pub fn in_amp_menu() -> bool {
    UNIT.load(Ordering::Relaxed) != HX_EFFECTS
}

static IR_NAMES: std::sync::Mutex<Vec<(i64, String)>> = std::sync::Mutex::new(Vec::new());

/// Remember the pedal's IR names (slot from 0), so the Model knob can show them.
pub fn set_ir_names(list: &[(i64, String)]) {
    if let Ok(mut g) = IR_NAMES.lock() {
        *g = list.to_vec();
    }
}

static NAM_SLOTS: std::sync::Mutex<Vec<(i64, bool)>> = std::sync::Mutex::new(Vec::new());

/// Remember which pedal slots (from 0) hold NAM captures and which plain IRs.
pub fn set_slot_kinds(kinds: Vec<(i64, bool)>) {
    if let Ok(mut g) = NAM_SLOTS.lock() {
        *g = kinds;
    }
}

/// Whether the slot (numbered from 1) holds a NAM capture: None when not known yet.
pub fn slot_is_capture(slot: i64) -> Option<bool> {
    NAM_SLOTS.lock().ok()?.iter().find(|(s, _)| *s == slot - 1).map(|(_, n)| *n)
}

/// The pedal's filled slots (numbered from 1) with their names.
pub fn ir_names() -> Vec<(i64, String)> {
    IR_NAMES.lock().map(|g| g.iter().map(|(s, n)| (s + 1, n.clone())).collect()).unwrap_or_default()
}

/// The name of the IR in a User IR slot (numbered from 1, as the Model knob shows it).
pub fn ir_name(slot: i64) -> Option<String> {
    IR_NAMES.lock().ok()?.iter().find(|(s, _)| *s == slot - 1).map(|(_, n)| n.clone())
}

/// Whether a model symbol is the NAM block (the mono Xenomorph Fuzz), while active.
pub fn is_nam_symbol(symbol: &str) -> bool {
    if !active() || symbol.ends_with("Stereo") {
        return false;
    }
    symbol.strip_suffix("Mono").unwrap_or(symbol) == BASE_SYMBOL
}

/// The NAM block's knob names, by the stock parameter id they take the place of.
pub const KNOBS: [(&str, &str); 10] = [
    ("Gain", "Input"),
    ("Tone", "Treble"),
    ("Level", "Output"),
    ("Clipping", "Polarity"),
    ("OscLevel", "Bass"),
    ("OscTone", "Mid"),
    ("MinFreq", "Capture"),
    ("MaxFreq", "Mid Freq"),
    ("WaveShape", "Q"),
    ("Sensitivity", "High Cut"),
];

// ------------------------------------------------------------ converter ---

const C: usize = 3;
const IR_SAMPLES: usize = 2048;
const N_WEIGHTS: usize = 1871;
const HEAD_TAPS: usize = 16;
const IR_LIMIT: f64 = 15.8489;
const LOUD_REF: f64 = -18.0;
/// The firmware's own constants, for the report only.
pub const STOMP_DBU: f64 = 11.5;
pub const LOUD_TARGET: f64 = -18.0;

fn kernels() -> Vec<usize> {
    let mut k = vec![6; 14];
    k.extend([15, 15]);
    k.extend([6; 7]);
    k
}
fn dilations() -> Vec<i64> {
    let base = [1, 3, 7, 17, 41, 101, 239];
    let mut d = Vec::new();
    d.extend(base);
    d.extend(base);
    d.extend([1, 13]);
    d.extend(base);
    d
}

/// What a conversion found and wrote.
#[derive(Debug, Clone)]
pub struct Converted {
    pub samples: Vec<f32>,
    pub scale_exp: i32,
    pub input_level_dbu: Option<f64>,
    pub loudness: Option<f64>,
    pub loudness_used: bool,
    pub warnings: Vec<String>,
}

/// Options for a conversion.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// Store the capture's input level so the block trims its input to match.
    pub use_level: bool,
    /// Store the capture's loudness so captures play at about the same volume.
    pub use_loudness: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { use_level: true, use_loudness: true }
    }
}

fn finite(v: Option<&Value>) -> Option<f64> {
    match v {
        Some(Value::Number(n)) => n.as_f64().filter(|x| x.is_finite()),
        _ => None,
    }
}

/// Pick the 3-channel A2 Lite WaveNet out of a `.nam` (or a SlimmableContainer of several).
fn pick_model(doc: &Value) -> Option<&Value> {
    let subs: Vec<&Value> = if doc.get("architecture").and_then(Value::as_str) == Some("SlimmableContainer") {
        doc.pointer("/config/submodels")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|s| s.get("model")).collect())
            .unwrap_or_default()
    } else {
        vec![doc]
    };
    subs.into_iter().find(|m| {
        m.get("architecture").and_then(Value::as_str) == Some("WaveNet")
            && m.pointer("/config/layers/0/channels").and_then(Value::as_u64) == Some(C as u64)
    })
}

/// Why a WaveNet config is not the A2 Lite shape the block computes (None when it is).
fn shape_problem(cfg: &Value) -> Option<String> {
    let layers = cfg.get("layers").and_then(Value::as_array);
    let Some(layers) = layers.filter(|l| l.len() == 1) else { return Some("it needs exactly one layer array".into()) };
    if !cfg.get("head").is_none_or(Value::is_null) || !cfg.get("condition_dsp").is_none_or(Value::is_null) {
        return Some("it has a post-stack head or a condition DSP".into());
    }
    if !cfg.get("head_scale").is_some_and(Value::is_number) {
        return Some("it has no head scale".into());
    }
    let la = &layers[0];
    let num = |k: &str| la.get(k).and_then(Value::as_u64);
    if num("input_size") != Some(1) || num("condition_size") != Some(1) {
        return Some("input and condition size must be 1".into());
    }
    if num("channels") != Some(C as u64) || num("bottleneck") != Some(C as u64) {
        return Some("it must have 3 channels".into());
    }
    let ks: Option<Vec<u64>> = la.get("kernel_sizes").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).collect());
    let ds: Option<Vec<i64>> = la.get("dilations").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_i64).collect());
    if ks != Some(kernels().iter().map(|&k| k as u64).collect()) || ds != Some(dilations()) {
        return Some("its kernel sizes or dilations differ from A2's".into());
    }
    let acts_ok = la.get("activation").and_then(Value::as_array).is_some_and(|a| {
        a.len() == kernels().len()
            && a.iter().all(|x| {
                x.get("type").and_then(Value::as_str) == Some("LeakyReLU")
                    && (x.get("negative_slope").and_then(Value::as_f64).unwrap_or(0.0) - 0.01).abs() < 1e-7
            })
    });
    if !acts_ok {
        return Some("every layer must use LeakyReLU 0.01".into());
    }
    if la.get("gated").and_then(Value::as_bool) == Some(true)
        || la.get("gating_mode").and_then(Value::as_array).is_some_and(|a| a.iter().any(|g| g.as_str().is_some_and(|s| s != "none")))
    {
        return Some("it uses gating".into());
    }
    if !la.get("layer1x1").and_then(|l| l.get("active")).and_then(Value::as_bool).unwrap_or(false) {
        return Some("it needs layer1x1 on".into());
    }
    let h = la.get("head");
    let head_ok = h.and_then(|h| h.get("out_channels")).and_then(Value::as_u64) == Some(1)
        && h.and_then(|h| h.get("kernel_size")).and_then(Value::as_u64) == Some(HEAD_TAPS as u64)
        && h.and_then(|h| h.get("bias")).and_then(Value::as_bool).unwrap_or(false);
    if !head_ok {
        return Some("it needs a 16-tap head with a bias".into());
    }
    for k in ["conv_pre_film", "conv_post_film", "input_mixin_pre_film", "input_mixin_post_film", "activation_pre_film", "activation_post_film", "layer1x1_post_film", "head1x1_post_film"] {
        if let Some(v) = la.get(k) {
            let off = v.is_null() || v.as_bool() == Some(false) || (v.is_object() && !v.get("active").and_then(Value::as_bool).unwrap_or(false));
            if !off {
                return Some("it uses FiLM conditioning".into());
            }
        }
    }
    None
}

/// The weights, reordered the way the block loads them, plus pieces the silence state needs.
struct Layout {
    rech: Vec<f64>,
    head: Vec<f64>, // bias, then [tap][channel]
    scale: f64,
    blocks: Vec<Vec<f64>>,
}

fn layout(w: &[f64]) -> Layout {
    let mut o = C;
    let mut blocks = Vec::new();
    for k in kernels() {
        let conv = o;
        o += C * C * k;
        let bias = o;
        o += C;
        let mixin = o;
        o += C;
        let w1 = o;
        o += C * C;
        let b1 = o;
        o += C;
        let mut blk = Vec::new();
        for i in 0..C {
            blk.push(w[bias + i]);
            blk.push(w[mixin + i]);
        }
        for t in 0..k {
            for j in 0..C {
                for i in 0..C {
                    blk.push(w[conv + (i * C + j) * k + t]);
                }
            }
        }
        for i in 0..C {
            blk.push(w[b1 + i]);
            for j in 0..C {
                blk.push(w[w1 + i * C + j]);
            }
        }
        blocks.push(blk);
    }
    let mut head = vec![w[o + C * HEAD_TAPS]];
    for t in 0..HEAD_TAPS {
        for j in 0..C {
            head.push(w[o + j * HEAD_TAPS + t]);
        }
    }
    Layout { rech: w[..C].to_vec(), head, scale: w[o + C * HEAD_TAPS + 1], blocks }
}

/// What the block's rings hold after silence, computed in 32-bit float in the
/// same order as the block, so a change of capture doesn't pop: the head's
/// input (3 words) and each layer's input (23 x 3).
fn silence(w: &[f64]) -> (Vec<f32>, Vec<Vec<f32>>) {
    let wf: Vec<f64> = w.iter().map(|&v| v as f32 as f64).collect();
    let l = layout(&wf);
    let blocks: Vec<Vec<f32>> = l.blocks.iter().map(|b| b.iter().map(|&v| v as f32).collect()).collect();
    let mut x: Vec<f32> = l.rech.iter().map(|&r| r as f32 * 0.0).collect();
    let mut hs = vec![0.0f32; C];
    let mut xs = Vec::new();
    for (k, blk) in kernels().into_iter().zip(blocks.iter()) {
        xs.push(x.clone());
        let mut a = Vec::with_capacity(C);
        for i in 0..C {
            let mut v = (blk[2 * i] + blk[2 * i + 1] * 0.0) + 0.0;
            for t in 0..k {
                for j in 0..C {
                    v += blk[2 * C + (t * C + j) * C + i] * x[j];
                }
            }
            let leak = v * 0.01f32;
            a.push(if leak > v { leak } else { v });
        }
        for i in 0..C {
            hs[i] += a[i];
        }
        let o = 2 * C + k * C * C;
        for i in 0..C {
            let mut u = blk[o + 4 * i];
            for j in 0..C {
                u += a[j] * blk[o + 4 * i + 1 + j];
            }
            x[i] += u;
        }
    }
    (hs, xs)
}

fn level_word(dbu: Option<f64>) -> Result<f64, String> {
    let Some(dbu) = dbu else { return Ok(0.0) };
    let g = 10f64.powf(-dbu / 20.0) as f32 as f64;
    if !(g > 0.0 && g < IR_LIMIT) {
        return Err(format!("its input level ({dbu} dBu) is out of range"));
    }
    Ok(g)
}

fn loud_word(db: Option<f64>) -> f64 {
    let Some(db) = db else { return 0.0 };
    let g = 10f64.powf((LOUD_REF - db) / 20.0) as f32 as f64;
    if g > 0.0 && g < IR_LIMIT {
        g
    } else {
        0.0
    }
}

/// Turn the text of a `.nam` file into the block's User IR samples.
pub fn convert(text: &str, opts: Options) -> Result<Converted, String> {
    let doc: Value = serde_json::from_str(text).map_err(|e| format!("not a readable .nam file ({e})"))?;
    let model = pick_model(&doc).ok_or("it has no 3-channel A2 Lite model (NAM A2 Lite needs the 3-channel \"A2 Lite\" size)")?;
    for x in [&doc, model] {
        if let Some(rate) = finite(x.get("sample_rate")) {
            if rate != 48000.0 {
                return Err(format!("it was trained at {rate} Hz; the pedal runs at 48 kHz and does not resample"));
            }
        }
    }
    if let Some(why) = model.get("config").and_then(shape_problem) {
        return Err(format!("not an A2 Lite capture the block can run: {why}"));
    }
    let w: Vec<f64> = model.get("weights").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_f64).collect()).unwrap_or_default();
    if w.len() != N_WEIGHTS {
        return Err(format!("it has {} weights; A2 Lite has {N_WEIGHTS}", w.len()));
    }
    let meta = doc.get("metadata");
    let dbu = if opts.use_level { finite(meta.and_then(|m| m.get("input_level_dbu"))) } else { None };
    let loud = if opts.use_loudness { finite(meta.and_then(|m| m.get("loudness"))) } else { None };

    let l = layout(&w);
    let (hs, xs) = silence(&w);
    let mut stream: Vec<f64> = Vec::with_capacity(N_WEIGHTS + 72);
    stream.extend(&l.rech);
    stream.extend(&l.head);
    stream.push(l.scale);
    for b in &l.blocks {
        stream.extend(b);
    }
    for v in hs.iter().chain(xs.iter().flatten()) {
        stream.push(*v as f64 / 16.0);
    }
    let peak = stream.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    let mut e = 0i32;
    while peak / 2f64.powi(e) > 15.5 {
        e += 1;
    }
    let div = 2f64.powi(e);
    let lw = loud_word(loud);
    let mut out: Vec<f64> = vec![0.5, -0.5, C as f64, div, N_WEIGHTS as f64 / 1024.0, level_word(dbu)?, lw, 0.0];
    out.extend(stream.iter().map(|v| v / div));
    if out.len() >= IR_SAMPLES || out.iter().any(|v| v.abs() >= IR_LIMIT) {
        return Err("its values don't fit in a User IR".into());
    }
    let mut samples: Vec<f32> = out.iter().map(|&v| v as f32).collect();
    samples.resize(IR_SAMPLES, 0.0);
    let mut warnings = Vec::new();
    if opts.use_loudness && loud.is_some() && lw == 0.0 {
        warnings.push("too quiet to normalise; it plays as it comes out".into());
    }
    Ok(Converted { samples, scale_exp: e, input_level_dbu: dbu, loudness: loud, loudness_used: lw > 0.0, warnings })
}

/// A 48 kHz mono 32-bit float WAV of the samples.
pub fn wav_bytes(samples: &[f32]) -> Vec<u8> {
    let rate: u32 = 48000;
    let data_len = (samples.len() * 4) as u32;
    let mut b = Vec::with_capacity(44 + data_len as usize);
    b.extend(b"RIFF");
    b.extend((4 + 8 + 16 + 8 + data_len).to_le_bytes());
    b.extend(b"WAVE");
    b.extend(b"fmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(3u16.to_le_bytes()); // IEEE float
    b.extend(1u16.to_le_bytes()); // mono
    b.extend(rate.to_le_bytes());
    b.extend((rate * 4).to_le_bytes());
    b.extend(4u16.to_le_bytes());
    b.extend(32u16.to_le_bytes());
    b.extend(b"data");
    b.extend(data_len.to_le_bytes());
    for s in samples {
        b.extend(s.to_le_bytes());
    }
    b
}

/// Convert one `.nam` file and write `<name> NAM.wav` into `out_dir`. Returns a one-line report.
pub fn convert_file(src: &Path, out_dir: &Path, opts: Options) -> Result<String, String> {
    let text = std::fs::read_to_string(src).map_err(|e| format!("can't read it ({e})"))?;
    let c = convert(&text, opts)?;
    let stem = src.file_stem().and_then(|s| s.to_str()).unwrap_or("capture");
    let out = out_dir.join(format!("{stem} NAM.wav"));
    std::fs::write(&out, wav_bytes(&c.samples)).map_err(|e| format!("can't write {} ({e})", out.display()))?;
    let mut notes = Vec::new();
    match c.input_level_dbu {
        Some(d) => notes.push(format!("captured at {d:.1} dBu (input trimmed {:+.1} dB)", STOMP_DBU - d)),
        None => notes.push("no input level".into()),
    }
    match (c.loudness, c.loudness_used) {
        (Some(l), true) => notes.push(format!("normalised {:+.1} dB", LOUD_TARGET - l)),
        _ => notes.push("not normalised".into()),
    }
    notes.extend(c.warnings);
    let name_len = stem.chars().count() + 4;
    if name_len > 16 {
        notes.push("the pedal shows only the first 16 characters of the name".into());
    }
    Ok(format!("{} -> {}  ({})", src.file_name().and_then(|s| s.to_str()).unwrap_or("?"), out.file_name().and_then(|s| s.to_str()).unwrap_or("?"), notes.join(", ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A made-up A2 Lite capture with the right shape.
    fn fake_nam(level: Option<f64>, loud: Option<f64>) -> String {
        let acts: Vec<Value> = (0..23).map(|_| serde_json::json!({"type": "LeakyReLU", "negative_slope": 0.01})).collect();
        let weights: Vec<f64> = (0..N_WEIGHTS).map(|i| ((i * 37 % 101) as f64 - 50.0) / 170.0).collect();
        let mut meta = serde_json::Map::new();
        if let Some(l) = level {
            meta.insert("input_level_dbu".into(), serde_json::json!(l));
        }
        if let Some(l) = loud {
            meta.insert("loudness".into(), serde_json::json!(l));
        }
        serde_json::json!({
            "architecture": "WaveNet",
            "sample_rate": 48000,
            "metadata": meta,
            "config": {
                "head_scale": 0.02,
                "layers": [{
                    "input_size": 1, "condition_size": 1, "channels": 3, "bottleneck": 3,
                    "kernel_sizes": kernels(), "dilations": dilations(), "activation": acts,
                    "layer1x1": {"active": true, "groups": 1},
                    "head": {"out_channels": 1, "kernel_size": 16, "bias": true}
                }]
            },
            "weights": weights
        })
        .to_string()
    }

    #[test]
    fn converts_a_capture() {
        let c = convert(&fake_nam(Some(12.2), Some(-20.0)), Options::default()).unwrap();
        assert_eq!(c.samples.len(), IR_SAMPLES);
        assert_eq!(&c.samples[..3], &[0.5, -0.5, 3.0]);
        assert!((c.samples[4] - 1871.0 / 1024.0).abs() < 1e-6);
        assert!(c.samples[5] > 0.0 && c.samples[6] > 0.0);
        assert_eq!(*c.samples.last().unwrap(), 0.0);
        assert!(c.samples.iter().all(|v| (v.abs() as f64) < IR_LIMIT));
        // 8 header words + 1871 weights + 72 silence words, then zeros.
        assert!(c.samples[8 + N_WEIGHTS + 72..].iter().all(|&v| v == 0.0));
        let wav = wav_bytes(&c.samples);
        assert_eq!(wav.len(), 44 + IR_SAMPLES * 4);
        assert_eq!(&wav[..4], b"RIFF");
    }

    #[test]
    fn options_leave_level_and_loudness_out() {
        let c = convert(&fake_nam(Some(12.2), Some(-20.0)), Options { use_level: false, use_loudness: false }).unwrap();
        assert_eq!(c.samples[5], 0.0);
        assert_eq!(c.samples[6], 0.0);
    }

    /// The real capture in tests/data/ converts to exactly the bytes the NAM A2 Lite project's own
    /// nam2ir.py writes (references made with it), with and without level/loudness.
    #[test]
    fn matches_nam2ir_py_byte_for_byte() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("data");
        let Ok(text) = std::fs::read_to_string(dir.join("MES DRECF RD Gain2 BAL O808 CAB.nam")) else { return };
        for (file, opts) in [
            ("reference-nam2ir.wav", Options::default()),
            ("reference-nam2ir-no-level-no-loudness.wav", Options { use_level: false, use_loudness: false }),
        ] {
            let Ok(want) = std::fs::read(dir.join(file)) else { continue };
            let got = wav_bytes(&convert(&text, opts).unwrap().samples);
            let differing = got.iter().zip(want.iter()).filter(|(a, b)| a != b).count();
            assert!(got.len() == want.len() && differing == 0, "{file}: {differing} bytes differ");
        }
    }

    #[test]
    fn refuses_other_rates_and_shapes() {
        let t = fake_nam(None, None).replace("\"sample_rate\":48000", "\"sample_rate\":44100");
        assert!(convert(&t, Options::default()).is_err());
        let t = fake_nam(None, None).replace("\"channels\":3", "\"channels\":8");
        assert!(convert(&t, Options::default()).is_err());
    }
}
