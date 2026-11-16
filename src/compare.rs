//! Comparing two presets: what is placed where, on or off, and every knob that differs.
//! Both kinds (a .hlx file and the pedal's own document) are read into the same plain
//! summary first, named and formatted as the editor shows them, so a file can be compared
//! with the preset that is in a slot.

use std::collections::BTreeMap;

use serde_json::Value as Json;

use crate::catalog::{format_value, Catalog};
use crate::devpreset::{DevicePreset, Kind};
use crate::hlx;

/// Where a block sits: path (from 0), lane (0 = main, 1 = parallel) and place (from 0).
pub type Place = (i64, i64, i64);

#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub model: String,
    pub on: Option<bool>,
    /// (knob name, shown value), in the pedal's order; a paired cab's knobs start with "Cab ".
    pub knobs: Vec<(String, String)>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Summary {
    pub blocks: BTreeMap<Place, Block>,
    /// Per path: the places the split and the merge sit before, when the path is split.
    pub splits: BTreeMap<i64, (i64, i64)>,
    pub tempo: Option<f64>,
    pub snapshots: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    Added,
    Removed,
    Changed,
    Note,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub change: Change,
    pub text: String,
}

fn place_text(p: Place) -> String {
    format!("Path {} {}{}", p.0 + 1, if p.1 == 0 { "A" } else { "B" }, p.2 + 1)
}

fn knobs_of(cat: &Catalog, model: i64, prefix: &str, values: impl Iterator<Item = f64>) -> Vec<(String, String)> {
    values
        .enumerate()
        .map(|(i, v)| {
            let (name, meta) = cat.param(model, i);
            let name = if name.is_empty() { format!("Setting {}", i + 1) } else { name };
            (format!("{prefix}{name}"), format_value(meta.as_ref(), v))
        })
        .collect()
}

/// A .hlx file as a summary.
pub fn from_hlx(doc: &Json, cat: &Catalog) -> Summary {
    let mut out = Summary { tempo: hlx::tempo(doc), ..Default::default() };
    for b in hlx::blocks(doc).iter().filter(|b| b.key.starts_with("block")) {
        let d: i64 = b.dsp.trim_start_matches("dsp").parse().unwrap_or(0);
        let obj = hlx::block(doc, b).cloned().unwrap_or_default();
        let stereo = obj.get("@stereo").and_then(Json::as_bool).unwrap_or(false);
        let Some(number) = cat.number_for_hlx(&b.model, stereo) else {
            out.blocks.insert((d, b.path, b.position), Block { model: hlx::pretty_model(&b.model), on: None, knobs: Vec::new() });
            continue;
        };
        let value_of = |o: &serde_json::Map<String, Json>, id: &str| match o.get(id) {
            Some(Json::Bool(x)) => *x as u8 as f64,
            Some(Json::Number(n)) => n.as_f64().unwrap_or(0.0),
            _ => 0.0,
        };
        let ids = cat.default_values(number);
        let mut model = cat.model_name(number);
        let mut knobs = knobs_of(cat, number, "", ids.iter().map(|(id, _, _)| value_of(&obj, id)));
        // An amp's cab (or a dual cab's second one) is its own section in the file.
        let cab = obj
            .get("@cab")
            .and_then(Json::as_str)
            .and_then(|k| doc.pointer(&format!("/data/tone/{}/{k}", b.dsp)))
            .and_then(Json::as_object);
        if let Some(c) = cab {
            if let Some(cn) = c.get("@model").and_then(Json::as_str).and_then(|m| cat.number_for_hlx(m, false)) {
                model = format!("{model} + {}", cat.model_name(cn));
                let cids = cat.default_values(cn);
                knobs.extend(knobs_of(cat, cn, "Cab ", cids.iter().map(|(id, _, _)| value_of(c, id))));
            }
        }
        let on = obj.get("@enabled").and_then(Json::as_bool);
        out.blocks.insert((d, b.path, b.position), Block { model, on, knobs });
    }
    for dsp in hlx::dsps(doc) {
        let d: i64 = dsp.trim_start_matches("dsp").parse().unwrap_or(0);
        if hlx::split_active(doc, &dsp) {
            if let Some((_, sp, jp)) = hlx::split_of(doc, &dsp) {
                out.splits.insert(d, (sp, jp));
            }
        }
    }
    out.snapshots = crate::snapshots::list(doc).into_iter().map(|(_, n)| n).collect();
    out
}

/// A pedal document as a summary.
pub fn from_device(p: &DevicePreset, cat: &Catalog) -> Summary {
    let mut out = Summary { tempo: p.tempo(), snapshots: p.snapshot_names(), ..Default::default() };
    let slots = p.slots();
    for s in &slots {
        if s.kind != Kind::Block {
            continue;
        }
        let Some(m) = s.model else { continue };
        // A path of `c` columns has 2c + 4 slots: lane A from 1, lane B from c + 3.
        let len = slots.iter().filter(|x| x.path == s.path).count();
        let c = (len.saturating_sub(4) / 2) as i64;
        let i = s.position as i64;
        let (lane, place) = if i > c + 2 { (1, i - (c + 3)) } else { (0, i - 1) };
        let mut model = cat.model_name(m);
        let mut knobs = knobs_of(cat, m, "", s.values.iter().map(|v| v.as_f64().unwrap_or(0.0)));
        if let Some(pm) = s.paired {
            model = format!("{model} + {}", cat.model_name(pm));
            let pv = p.paired_values(s.path, s.position);
            knobs.extend(knobs_of(cat, pm, "Cab ", pv.iter().map(|v| v.as_f64().unwrap_or(0.0))));
        }
        out.blocks.insert((s.path, lane, place), Block { model, on: s.enabled, knobs });
    }
    for path in slots.iter().map(|s| s.path).collect::<std::collections::BTreeSet<_>>() {
        if !p.split_active(path) {
            continue;
        }
        let of = |kind: Kind| slots.iter().find(|s| s.path == path && s.kind == kind).and_then(|s| p.attach(path, s.position));
        if let (Some(sp), Some(jp)) = (of(Kind::Split), of(Kind::Join)) {
            out.splits.insert(path, (sp - 1, jp - 1));
        }
    }
    out
}

/// What differs, from `a` to `b`: blocks added, removed or changed, and the rest.
pub fn diff(a: &Summary, b: &Summary) -> Vec<Line> {
    let mut lines = Vec::new();
    let line = |change, text: String| Line { change, text };
    let places: std::collections::BTreeSet<Place> = a.blocks.keys().chain(b.blocks.keys()).copied().collect();
    for p in places {
        match (a.blocks.get(&p), b.blocks.get(&p)) {
            (Some(x), None) => lines.push(line(Change::Removed, format!("{}: {} is not in the other", place_text(p), x.model))),
            (None, Some(y)) => lines.push(line(Change::Added, format!("{}: {} is only in the other", place_text(p), y.model))),
            (Some(x), Some(y)) => {
                if x.model != y.model {
                    lines.push(line(Change::Changed, format!("{}: {} → {}", place_text(p), x.model, y.model)));
                    continue;
                }
                if x.on != y.on {
                    let word = |o: Option<bool>| match o {
                        Some(true) => "on",
                        Some(false) => "off",
                        None => "?",
                    };
                    lines.push(line(Change::Changed, format!("{}: {} is {} → {}", place_text(p), x.model, word(x.on), word(y.on))));
                }
                for ((name, v1), (_, v2)) in x.knobs.iter().zip(&y.knobs).filter(|((_, v1), (_, v2))| v1 != v2) {
                    lines.push(line(Change::Changed, format!("{}: {} · {name} {v1} → {v2}", place_text(p), x.model)));
                }
            }
            (None, None) => {}
        }
    }
    let paths: std::collections::BTreeSet<i64> = a.splits.keys().chain(b.splits.keys()).copied().collect();
    for d in paths {
        match (a.splits.get(&d), b.splits.get(&d)) {
            (Some(x), Some(y)) if x != y => lines.push(line(
                Change::Changed,
                format!(
                    "Path {}: split before {}, merge before {} → split before {}, merge before {}",
                    d + 1,
                    x.0 + 1,
                    x.1 + 1,
                    y.0 + 1,
                    y.1 + 1
                ),
            )),
            (Some(_), None) => lines.push(line(Change::Removed, format!("Path {} is split here, not in the other", d + 1))),
            (None, Some(_)) => lines.push(line(Change::Added, format!("Path {} is split only in the other", d + 1))),
            _ => {}
        }
    }
    if let (Some(x), Some(y)) = (a.tempo, b.tempo) {
        if (x - y).abs() > 0.05 {
            lines.push(line(Change::Changed, format!("Tempo {x:.0} → {y:.0} BPM")));
        }
    }
    if a.snapshots != b.snapshots {
        lines.push(line(Change::Note, format!("Snapshots: {} → {}", a.snapshots.join(", "), b.snapshots.join(", "))));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(model: &str, on: bool, knobs: &[(&str, &str)]) -> Block {
        Block { model: model.into(), on: Some(on), knobs: knobs.iter().map(|(n, v)| ((*n).into(), (*v).into())).collect() }
    }

    #[test]
    fn identical_presets_have_no_differences() {
        let mut a = Summary::default();
        a.blocks.insert((0, 0, 0), block("Minotaur", true, &[("Gain", "3.5")]));
        assert!(diff(&a, &a.clone()).is_empty());
    }

    #[test]
    fn differences_are_listed_by_place() {
        let mut a = Summary { tempo: Some(120.0), ..Default::default() };
        a.blocks.insert((0, 0, 0), block("Minotaur", true, &[("Gain", "3.5"), ("Tone", "7.1")]));
        a.blocks.insert((0, 0, 2), block("Digital", true, &[]));
        a.blocks.insert((0, 1, 1), block("Rotary", true, &[]));
        a.splits.insert(0, (1, 2));
        let mut b = Summary { tempo: Some(96.0), ..Default::default() };
        b.blocks.insert((0, 0, 0), block("Minotaur", false, &[("Gain", "4.0"), ("Tone", "7.1")]));
        b.blocks.insert((0, 0, 2), block("Simple Delay", true, &[]));
        b.blocks.insert((0, 0, 4), block("Plate", true, &[]));
        b.splits.insert(0, (1, 3));
        let text: Vec<String> = diff(&a, &b).into_iter().map(|l| l.text).collect();
        assert!(text.contains(&"Path 1 A1: Minotaur is on → off".to_owned()), "{text:?}");
        assert!(text.contains(&"Path 1 A1: Minotaur · Gain 3.5 → 4.0".to_owned()));
        assert!(text.contains(&"Path 1 A3: Digital → Simple Delay".to_owned()));
        assert!(text.contains(&"Path 1 A5: Plate is only in the other".to_owned()));
        assert!(text.contains(&"Path 1 B2: Rotary is not in the other".to_owned()));
        assert!(text.iter().any(|t| t.starts_with("Path 1: split before 2, merge before 3 → split before 2, merge before 4")));
        assert!(text.contains(&"Tempo 120 → 96 BPM".to_owned()));
        assert!(!text.iter().any(|t| t.contains("Tone")), "an unchanged knob is not listed");
    }
}

#[cfg(test)]
mod fixture_tests {
    use super::*;

    /// A .hlx and the same preset converted onto a real pedal document should compare as the
    /// same effects in the same places (`HX_FIXTURE` = a pedal document, `HLX_FIXTURE` = the .hlx).
    #[test]
    fn a_converted_preset_matches_its_source() {
        let (Ok(tpl), Ok(file), Some(dir)) = (std::env::var("HX_FIXTURE"), std::env::var("HLX_FIXTURE"), crate::catalog::find_dir(None))
        else {
            return;
        };
        let cat = Catalog::load(&dir).unwrap();
        let template = DevicePreset::parse(&std::fs::read(tpl).unwrap()).unwrap();
        let json = hlx::parse_text(&std::fs::read_to_string(file).unwrap()).unwrap();
        let (converted, _) = DevicePreset::from_hlx(&template, &json, &cat).unwrap();
        let lines = diff(&from_hlx(&json, &cat), &from_device(&converted, &cat));
        for l in &lines {
            println!("{:?} {}", l.change, l.text);
        }
        assert!(lines.iter().all(|l| l.change != Change::Added && l.change != Change::Removed), "same blocks in the same places");
    }
}
