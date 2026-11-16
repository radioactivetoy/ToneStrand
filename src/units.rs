//! Moving a .hlx preset from one unit to another (HX Stomp, Helix...), with a
//! check of everything that would not fit. Nothing changes when it does not fit.

use serde_json::{json, Value};

use crate::catalog::Catalog;
use crate::hlx;
use crate::snapshots::{self, Source};

pub fn unit_name(device: i64) -> &'static str {
    snapshots::UNITS.iter().find(|u| u.0 == device).map(|u| u.1).unwrap_or("Helix")
}

/// Convert `doc` to `target`. Ok carries what was adjusted on the way (may be
/// empty); Err lists why the preset cannot run on that unit (the preset is
/// left exactly as it was).
pub fn convert(doc: &mut Value, target: i64, cat: Option<&Catalog>) -> Result<Vec<String>, Vec<String>> {
    let from = snapshots::family(doc);
    let to = snapshots::family_of_device(target);
    let to_name = unit_name(target);
    if snapshots::device(doc) == target {
        return Ok(Vec::new());
    }
    let mut work = doc.clone();
    let mut errors = Vec::new();
    let mut changes = Vec::new();
    let name_of = |m: &str| cat.and_then(|c| c.info_for_symbol(m).map(|i| i.name.clone())).unwrap_or_else(|| hlx::pretty_model(m));

    // Paths: a unit with one path cannot carry blocks on a second one.
    let blocks = hlx::blocks(&work);
    for extra in to.paths..2 {
        let dsp = format!("dsp{extra}");
        let on_it = blocks.iter().filter(|b| b.dsp == dsp).count();
        if on_it > 0 {
            errors.push(format!("It uses Path {} ({on_it} blocks); the {to_name} has only one path.", extra + 1));
        } else if work.pointer(&format!("/data/tone/{dsp}")).is_some() {
            if let Some(t) = work.pointer_mut("/data/tone").and_then(Value::as_object_mut) {
                t.remove(&dsp);
                // What the file keeps about that path elsewhere goes with it.
                if let Some(g) = t.get_mut("global").and_then(Value::as_object_mut) {
                    g.remove(&format!("@topology{extra}"));
                }
                if let Some(c) = t.get_mut("controller").and_then(Value::as_object_mut) {
                    c.remove(&dsp);
                }
                for (_, snap) in t.iter_mut().filter(|(k, _)| k.starts_with("snapshot")) {
                    if let Some(b) = snap.get_mut("blocks").and_then(Value::as_object_mut) {
                        b.remove(&dsp);
                    }
                }
            }
        }
    }

    // Block count per path: each unit has a hard limit (8 on HX Stomp/XL, 9 on HX Effects, 16 on Helix).
    for dsp in hlx::dsps(&work) {
        let count = hlx::block_count(&work, &dsp);
        if count > to.max_blocks {
            let pnum = dsp.trim_start_matches("dsp").parse::<usize>().unwrap_or(0) + 1;
            errors.push(format!("Path {pnum} has {count} blocks; the {to_name} holds at most {}.", to.max_blocks));
        }
    }

    // Effects that the target unit does not have.
    if let Some(c) = cat {
        for b in &blocks {
            if !c.allowed_on(&b.model, target) {
                errors.push(format!("{} is not available on the {to_name}.", name_of(&b.model)));
            }
        }
        // Processing: each path must fit.
        for dsp in hlx::dsps(&work) {
            let used: f64 = blocks
                .iter()
                .filter(|b| b.dsp == dsp)
                .filter_map(|b| {
                    let stereo = hlx::block(&work, b).and_then(|o| o.get("@stereo")).and_then(Value::as_bool).unwrap_or(false);
                    c.number_for_hlx(&b.model, stereo).map(|n| c.dsp_cost(n) / crate::editors::DSP_BUDGET * 100.0)
                })
                .sum();
            if used > 100.5 {
                errors.push(format!(
                    "Path {} needs about {used:.0}% of the processing; more than the {to_name} has.",
                    dsp.trim_start_matches("dsp").parse::<usize>().unwrap_or(0) + 1
                ));
            }
        }
    }

    // Footswitches for on/off.
    for (b, index, _) in snapshots::all_footswitches(&work) {
        if index > to.fs_count {
            errors.push(format!("{} is on footswitch {index}; the {to_name} has {}.", name_of(&b.model), to.fs_count));
        }
    }

    // Command Center triggers and commands.
    for (slot_key, cmd_id) in crate::controllers::active_commands(&work) {
        if let Some(n) = slot_key.strip_prefix("commandFS").and_then(|s| s.parse::<i64>().ok()) {
            if n > to.cc_switches {
                errors.push(format!("Command Center uses FS {n}; the {to_name} has {} footswitches.", to.cc_switches));
            }
        } else if let Some(n) = slot_key.strip_prefix("commandExpPedal").and_then(|s| s.parse::<i64>().ok()) {
            if n > to.exp_count {
                errors.push(format!("Command Center uses EXP {n}; the {to_name} takes {} expression pedals.", to.exp_count));
            }
        } else if slot_key == "commandExpSwitch" && !to.name.starts_with("Helix") {
            errors.push(format!("Command Center uses EXP Toe; the {to_name} does not have that switch."));
        }
        if to.name.starts_with("HX Stomp") && (cmd_id == 7 || cmd_id == 8) {
            errors.push(format!("Command Center uses Ext Amp, which is not available on the {to_name}."));
        }
    }

    // Knob controllers: the numbers differ per unit, so each is renumbered.
    let mut renumber = Vec::new();
    for b in hlx::blocks(&work) {
        let Some(knobs) = work.pointer(&format!("/data/tone/controller/{}/{}", b.dsp, b.key)).and_then(Value::as_object) else { continue };
        for (knob, c) in knobs {
            let Some(code) = c.get("@controller").and_then(Value::as_i64) else { continue };
            match Source::from_code(code, from) {
                Some(Source::Footswitch(n)) if n > to.fs_count => {
                    errors.push(format!("{} › {knob} is on footswitch {n}; the {to_name} has {}.", name_of(&b.model), to.fs_count))
                }
                Some(Source::Exp(n)) if n > to.exp_count => {
                    errors.push(format!("{} › {knob} is on EXP {n}; the {to_name} takes {}.", name_of(&b.model), to.exp_count))
                }
                Some(src) => renumber.push((b.dsp.clone(), b.key.clone(), knob.clone(), src.code(to))),
                None => changes.push(format!("{} › {knob}: a controller this app does not know was left as it is.", name_of(&b.model))),
            }
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    for (dsp, key, knob, code) in renumber {
        if let Some(c) = work.pointer_mut(&format!("/data/tone/controller/{dsp}/{key}/{knob}")).and_then(Value::as_object_mut) {
            c.insert("@controller".into(), json!(code));
        }
    }

    // Snapshots beyond what the unit has are switched off (kept in the file).
    for (i, name) in snapshots::list(&work) {
        if i >= to.snapshot_count {
            if let Some(s) = work.pointer_mut(&format!("/data/tone/snapshot{i}")).and_then(Value::as_object_mut) {
                s.insert("@valid".into(), json!(false));
            }
            changes.push(format!("Snapshot \"{name}\" was set aside: the {to_name} has {} snapshots.", to.snapshot_count));
        }
    }
    if let Some(d) = work.pointer_mut("/data").and_then(Value::as_object_mut) {
        d.insert("device".into(), json!(target));
    }
    // A unit with fewer places on the main lane (HX Effects 9, the others 8): blocks past its
    // last place move to free ones, in order.
    for dsp in hlx::dsps(&work) {
        let mut over: Vec<_> = hlx::blocks(&work)
            .into_iter()
            .filter(|b| b.dsp == dsp && b.path == 0 && b.key.starts_with("block") && b.position >= to.positions)
            .collect();
        over.sort_by_key(|b| b.position);
        for b in over {
            let taken: Vec<i64> = hlx::blocks(&work).iter().filter(|x| x.dsp == dsp && x.path == 0).map(|x| x.position).collect();
            if let Some(free) = (0..to.positions).find(|p| !taken.contains(p)) {
                hlx::move_to(&mut work, &b, 0, free);
                changes.push(format!("A block past the {to_name}'s last place moved to place {}.", free + 1));
            }
        }
    }
    changes.extend(convert_io(&mut work, from, to, to_name));
    if hlx::ensure_paths(&mut work, to.paths) {
        changes.push(format!("Path 2 was added (empty): the {to_name} has two paths."));
    }
    for dsp in hlx::dsps(&work) {
        hlx::fit_lanes(&mut work, &dsp, to.positions, to.max_blocks, to.shared_slots);
    }
    *doc = work;
    changes.insert(0, format!("Converted from {} to {to_name}.", from.name));
    Ok(changes)
}

/// Input and output of every path on the new unit: the routes keep their meaning (the numbers
/// differ per unit), and the models take the unit family's naming. Returns what to tell the person.
fn convert_io(work: &mut Value, from: snapshots::Family, to: snapshots::Family, to_name: &str) -> Vec<String> {
    let mut notes = Vec::new();
    for dsp in hlx::dsps(work) {
        let pnum = dsp.trim_start_matches("dsp").parse::<usize>().unwrap_or(0) + 1;
        for (sec, input) in [("inputA", true), ("inputB", true), ("outputA", false), ("outputB", false)] {
            let Some(obj) = work.pointer_mut(&format!("/data/tone/{dsp}/{sec}")).and_then(Value::as_object_mut) else { continue };
            let route_key = if input { "@input" } else { "@output" };
            if let Some(v) = obj.get(route_key).and_then(Value::as_i64) {
                let (nv, exact) = crate::routes::map_route(from, to, input, v);
                if nv != v {
                    obj.insert(route_key.into(), json!(nv));
                }
                if !exact {
                    let was = crate::routes::route_name(from, input, v).map(str::to_owned).unwrap_or(format!("route {v}"));
                    let now = crate::routes::route_name(to, input, nv).unwrap_or("the main one");
                    let what = if input { "input" } else { "output" };
                    notes.push(format!("Path {pnum} {what} ({sec}): \"{was}\" isn't on the {to_name}; set to \"{now}\"."));
                }
            }
            if from.paths != to.paths {
                if let Some(m) = obj.get("@model").and_then(Value::as_str).map(str::to_owned) {
                    let renamed = if to.paths == 1 {
                        match (input, sec.ends_with('B'), m.starts_with("HD2_AppDSPFlow")) {
                            (true, _, true) => Some("HelixStomp_AppDSPFlowInput"),
                            (false, false, true) => Some("HelixStomp_AppDSPFlowOutputMain"),
                            (false, true, true) => Some("HelixStomp_AppDSPFlowOutputSend"),
                            _ => None,
                        }
                    } else if m.starts_with("HelixStomp_AppDSPFlow") {
                        Some(match (input, dsp.as_str()) {
                            (true, "dsp0") => "HD2_AppDSPFlow1Input",
                            (true, _) => "HD2_AppDSPFlow2Input",
                            (false, _) => "HD2_AppDSPFlowOutput",
                        })
                    } else {
                        None
                    };
                    if let Some(r) = renamed {
                        obj.insert("@model".into(), json!(r));
                    }
                }
            }
        }
    }
    notes
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stomp_shares_eight_slots_across_parallel_lanes() {
        let mut doc = hlx::sample();
        let fam = snapshots::family(&doc);
        assert_eq!(fam.max_blocks, 8);
        assert!(fam.shared_slots);
        hlx::ensure_split(&mut doc, "dsp0", &|_| Vec::new());
        hlx::fit_lanes(&mut doc, "dsp0", fam.positions, fam.max_blocks, fam.shared_slots);
        // Sample Tone has 1 parallel block -> main lane shows 7 columns (7 + 1 = 8 total slots).
        let main_cols = hlx::main_cols_for(&doc, "dsp0", fam.positions, fam.max_blocks, fam.shared_slots);
        let (_, sp, jp) = hlx::split_of(&doc, "dsp0").unwrap();
        assert_eq!(main_cols + (jp - sp), 8);

        // Extending the parallel lane to hold a second effect reduces main_cols to 6 (6 + 2 = 8 total slots).
        let col = hlx::extend_lower(&mut doc, "dsp0", fam.positions, fam.max_blocks, fam.shared_slots).unwrap();
        hlx::place_model(
            &mut doc,
            "dsp0",
            1,
            col as i64,
            None,
            &hlx::NewModel { model: "HD2_DelaySimpleDelay", stereo: false, params: &[], cab: None },
        );
        hlx::fit_lanes(&mut doc, "dsp0", fam.positions, fam.max_blocks, fam.shared_slots);
        let main_cols2 = hlx::main_cols_for(&doc, "dsp0", fam.positions, fam.max_blocks, fam.shared_slots);
        let lower_slots2 = hlx::lower_slots_of(&doc, "dsp0");
        assert_eq!(lower_slots2, 2);
        assert_eq!(main_cols2, 6);
        assert_eq!(main_cols2 + lower_slots2, 8);

        // Moving the Merge location earlier on the primary path persists across fit_lanes/fit_split
        // without changing the number of parallel-lane slots.
        hlx::move_split_or_join(&mut doc, "dsp0", true, 3, fam.positions, fam.max_blocks, fam.shared_slots);
        hlx::fit_split(&mut doc, "dsp0");
        hlx::fit_lanes(&mut doc, "dsp0", fam.positions, fam.max_blocks, fam.shared_slots);
        let (_, _, jp_moved) = hlx::split_of(&doc, "dsp0").unwrap();
        assert_eq!(jp_moved, 3);
        assert_eq!(hlx::lower_slots_of(&doc, "dsp0"), 2);

        // The second path can be longer than the primary path (e.g. 5 parallel slots vs 3 primary slots).
        hlx::move_split_or_join(&mut doc, "dsp0", false, 1, fam.positions, fam.max_blocks, fam.shared_slots);
        for _ in 0..3 {
            let c = hlx::extend_lower(&mut doc, "dsp0", fam.positions, fam.max_blocks, fam.shared_slots).unwrap();
            hlx::place_model(
                &mut doc,
                "dsp0",
                1,
                c as i64,
                None,
                &hlx::NewModel { model: "HD2_DelaySimpleDelay", stereo: false, params: &[], cab: None },
            );
        }
        hlx::fit_lanes(&mut doc, "dsp0", fam.positions, fam.max_blocks, fam.shared_slots);
        let lower_slots5 = hlx::lower_slots_of(&doc, "dsp0");
        let main_cols3 = hlx::main_cols_for(&doc, "dsp0", fam.positions, fam.max_blocks, fam.shared_slots);
        assert_eq!(lower_slots5, 5);
        assert_eq!(main_cols3, 3);
        assert!(lower_slots5 > main_cols3);
    }

    #[test]
    fn helix_to_stomp_checks_and_renumbers() {
        // A Helix preset with a knob on footswitch 2 becomes an HX Stomp preset with it renumbered.
        let mut doc = hlx::blank_for("t", 2162689);
        hlx::place_model(
            &mut doc,
            "dsp0",
            0,
            0,
            None,
            &hlx::NewModel { model: "HD2_DistScream808", stereo: false, params: &[("Gain".into(), 1, 0.5)], cab: None },
        );
        let b = hlx::blocks(&doc).into_iter().next().unwrap();
        snapshots::assign(&mut doc, &b, "Gain", Some(Source::Footswitch(2)), 0.0, 1.0, None);
        assert_eq!(doc.pointer("/data/tone/controller/dsp0/block0/Gain/@controller").unwrap(), &json!(7));
        let changes = convert(&mut doc, 2162694, None).unwrap();
        assert!(!changes.is_empty());
        assert_eq!(doc.pointer("/data/tone/controller/dsp0/block0/Gain/@controller").unwrap(), &json!(4));
        assert!(doc.pointer("/data/tone/dsp1").is_none());

        // A block on Path 2 cannot go to a one-path unit: refused, nothing changed.
        let mut two = hlx::blank_for("t", 2162689);
        hlx::place_model(
            &mut two,
            "dsp1",
            0,
            0,
            None,
            &hlx::NewModel { model: "HD2_DistScream808", stereo: false, params: &[], cab: None },
        );
        let before = two.clone();
        assert!(convert(&mut two, 2162694, None).is_err());
        assert_eq!(two, before);
    }

    #[test]
    fn note_sync_and_mod_mode_use_musical_and_explanatory_labels() {
        let sync = crate::catalog::Param {
            id: "SyncSelect1".into(),
            name: "Note Sync".into(),
            value_type: 0,
            display: "sync_note".into(),
            min: 1.0,
            max: 19.0,
            default: 6.0,
            choices: crate::catalog::builtin_choices("sync_note"),
            spec: crate::catalog::DisplaySpec::default(),
        };
        assert_eq!(crate::catalog::format_value(Some(&sync), 1.0), "1/1");
        assert_eq!(crate::catalog::format_value(Some(&sync), 6.0), "1/4");
        assert_eq!(crate::catalog::format_value(Some(&sync), 8.0), "1/8 Dotted");
        assert_eq!(crate::catalog::format_value(Some(&sync), 9.0), "1/8");
        assert_eq!(crate::catalog::format_value(Some(&sync), 11.0), "1/16 Dotted");

        let mod_mode = crate::catalog::Param {
            id: "ModulationMode".into(),
            name: "Mod Mode".into(),
            value_type: 0,
            display: "delay_mod_mode".into(),
            min: 0.0,
            max: 2.0,
            default: 0.0,
            choices: crate::catalog::builtin_choices("delay_mod_mode"),
            spec: crate::catalog::DisplaySpec::default(),
        };
        assert_eq!(crate::catalog::format_value(Some(&mod_mode), 0.0), "Off");
        assert_eq!(crate::catalog::format_value(Some(&mod_mode), 1.0), "Chorus");
        assert_eq!(crate::catalog::format_value(Some(&mod_mode), 2.0), "Vibrato");
    }

    #[test]
    fn sample_preset_parameters_match_catalog_ranges() {
        let doc = hlx::sample();
        let Some(dir) = crate::catalog::find_dir(None) else { return };
        let Ok(cat) = crate::catalog::Catalog::load(&dir) else { return };
        for b in hlx::blocks(&doc) {
            let obj = hlx::block(&doc, &b).expect("block object");
            let stereo = obj.get("@stereo").and_then(Value::as_bool).unwrap_or(false);
            let info = cat.info_for_hlx(&b.model, stereo).unwrap_or_else(|| panic!("missing model {}", b.model));
            for p in &info.params {
                if p.id.starts_with('@') && !matches!(p.id.as_str(), "@mic" | "@trails") {
                    continue;
                }
                let v = obj.get(&p.id).unwrap_or_else(|| panic!("block {} ({}) missing param {}", b.key, b.model, p.id));
                if let Some(f) = v.as_f64() {
                    assert!(
                        f >= p.min - 1e-6 && f <= p.max + 1e-6,
                        "block {} ({}) param {} = {} outside {}..={}",
                        b.key,
                        b.model,
                        p.id,
                        f,
                        p.min,
                        p.max
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod split_tests {
    use super::*;

    fn nm(m: &'static str) -> hlx::NewModel<'static> {
        hlx::NewModel { model: m, stereo: false, params: &[], cab: None }
    }

    #[test]
    fn remove_split_keeps_blocks_within_the_units_positions() {
        let mut doc = hlx::blank_for("t", 2162689);
        let fam = snapshots::family(&doc);
        for p in 0..fam.positions {
            hlx::place_model(&mut doc, "dsp0", 0, p, None, &nm("HD2_DelaySimpleDelay"));
        }
        hlx::add_split_at(&mut doc, "dsp0", Some(2), &|_| Vec::new());
        hlx::place_model(&mut doc, "dsp0", 1, 2, None, &nm("HD2_DelaySimpleDelay"));
        // The main lane is full: refuse, change nothing.
        assert!(!hlx::remove_split(&mut doc, "dsp0", fam.positions));
        assert!(hlx::split_of(&doc, "dsp0").is_some());
        assert!(hlx::blocks(&doc).iter().any(|b| b.path == 1));
        // With a place free, the block folds back inside the unit's positions.
        let b = hlx::blocks(&doc).into_iter().find(|b| b.path == 0 && b.position == 5).unwrap();
        hlx::remove_block(&mut doc, &b);
        assert!(hlx::remove_split(&mut doc, "dsp0", fam.positions));
        assert!(hlx::blocks(&doc).iter().all(|b| b.path == 0 && b.position < fam.positions));
    }
}

#[cfg(test)]
mod io_tests {
    use super::*;

    #[test]
    fn stomp_to_helix_keeps_the_input_meaning_and_gives_path_two_its_io() {
        let mut doc = hlx::blank_for("t", 2162694);
        // As HX Edit writes an HX Stomp file: an empty dsp1, the second lane's input and output.
        doc["data"]["tone"]["dsp1"] = json!({});
        doc["data"]["tone"]["dsp0"]["inputA"]["@input"] = json!(4); // Return L/R
        doc["data"]["tone"]["dsp0"]["inputB"] = json!({ "@input": 0, "@model": "HelixStomp_AppDSPFlowInput" });
        let notes = convert(&mut doc, 2162689, None).unwrap();
        let tone = &doc["data"]["tone"];
        assert_eq!(tone["dsp0"]["inputA"]["@input"], json!(11), "Return L/R becomes Return 1/2");
        assert_eq!(tone["dsp0"]["inputA"]["@model"], json!("HD2_AppDSPFlow1Input"));
        assert_eq!(tone["dsp0"]["outputA"]["@model"], json!("HD2_AppDSPFlowOutput"));
        assert!(tone["dsp1"]["inputA"].is_object() && tone["dsp1"]["outputA"].is_object());
        assert!(tone["dsp1"]["inputB"].is_object(), "the second lane's input follows the file");
        assert!(notes.iter().any(|n| n.starts_with("Path 2 was added")));
    }

    #[test]
    fn helix_to_stomp_maps_routes_and_drops_path_two_leftovers() {
        let mut doc = hlx::blank_for("t", 2162689);
        doc["data"]["tone"]["dsp0"]["inputA"]["@input"] = json!(6); // Mic
        doc["data"]["tone"]["dsp0"]["outputA"]["@output"] = json!(6); // XLR
        doc["data"]["tone"]["controller"] = json!({ "dsp1": {} });
        doc["data"]["tone"]["snapshot0"]["blocks"] = json!({ "dsp1": {} });
        let notes = convert(&mut doc, 2162694, None).unwrap();
        let tone = &doc["data"]["tone"];
        assert_eq!(tone["dsp0"]["inputA"]["@input"], json!(1));
        assert_eq!(tone["dsp0"]["outputA"]["@output"], json!(1));
        assert_eq!(tone["dsp0"]["inputA"]["@model"], json!("HelixStomp_AppDSPFlowInput"));
        assert_eq!(tone["dsp0"]["outputA"]["@model"], json!("HelixStomp_AppDSPFlowOutputMain"));
        assert!(tone.get("dsp1").is_none());
        assert!(tone["global"].get("@topology1").is_none());
        assert!(tone["controller"].get("dsp1").is_none());
        assert!(tone["snapshot0"]["blocks"].get("dsp1").is_none());
        assert!(notes.iter().any(|n| n.contains("\"Mic\" isn't on the")));
        assert!(notes.iter().any(|n| n.contains("\"XLR\" isn't on the")));
    }
}

#[cfg(test)]
mod range_tests {
    use super::*;

    #[test]
    fn blocks_past_the_new_units_last_place_move_in() {
        let mut doc = hlx::blank_for("t", 2162693); // HX Effects: 9 places
        let nm = hlx::NewModel { model: "HD2_DelaySimpleDelay", stereo: false, params: &[], cab: None };
        for p in [0, 1, 8] {
            hlx::place_model(&mut doc, "dsp0", 0, p, None, &nm);
        }
        let notes = convert(&mut doc, 2162694, None).unwrap(); // HX Stomp: 8 places
        assert!(hlx::blocks(&doc).iter().all(|b| b.position < 8));
        assert!(notes.iter().any(|n| n.contains("moved to place 3")));
    }

    #[test]
    fn stomp_family_has_no_ext_amp() {
        let mut doc = hlx::blank_for("t", 2162689);
        doc["data"]["tone"]["controller"] = json!({});
        for target in [2162694, 2162699] {
            assert!(snapshots::family_of_device(target).name.starts_with("HX Stomp"));
        }
        assert!(!snapshots::family_of_device(2162693).name.starts_with("HX Stomp"));
    }
}
