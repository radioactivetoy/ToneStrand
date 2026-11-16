#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod window;
mod boxes;
mod catalog;
mod chain;
mod controllers;
mod editors;
mod library;
mod device;
mod devpreset;
mod hlx;
mod msgpack;
mod proto;
mod realnames;
mod nam;
mod irs;
mod selftest;
mod modelinfo;
mod icons;
mod folders;
mod backup;
mod tonelib;
mod session;
mod snapshots;
mod theme;
mod units;
mod wire;

use eframe::egui;

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let screenshot = args
        .iter()
        .position(|a| a == "--screenshot")
        .and_then(|i| args.get(i + 1))
        .map(std::path::PathBuf::from);

    if let Some(i) = args.iter().position(|a| a == "--probe") {
        probe(args.get(i + 1).cloned());
        return Ok(());
    }
    if let Some(i) = args.iter().position(|a| a == "--nam2ir") {
        // Convert NAM captures to User IR .wav files: --nam2ir <out folder> <file.nam>...
        let out = std::path::PathBuf::from(args.get(i + 1).cloned().unwrap_or_else(|| ".".into()));
        for f in &args[(i + 2).min(args.len())..] {
            match nam::convert_file(std::path::Path::new(f), &out, nam::Options::default()) {
                Ok(l) => println!("{l}"),
                Err(e) => println!("{f}: {e}"),
            }
        }
        return Ok(());
    }
    if let Some(i) = args.iter().position(|a| a == "--read-backup") {
        // Read-only: list what a .hxb or .hls holds.
        for f in &args[i + 1..] {
            let b = std::fs::read(f).expect("read");
            let r = if f.to_ascii_lowercase().ends_with(".hls") { backup::read_hls(&b).map(|s| (vec![s], 0)) } else { backup::read_hxb(&b).map(|c| (c.setlists, c.irs.len())) };
            match r {
                Ok((sets, irs)) => {
                    println!("{f}: {} setlists, {irs} IRs", sets.len());
                    for s in &sets {
                        let first: Vec<String> = s.presets.iter().take(4).map(|p| format!("{} {} ({} blocks)", proto::slot_label(p.slot), p.name, hlx::blocks(&p.hlx).len())).collect();
                        println!("  {}: {} presets, {} skipped; {}", s.name, s.presets.len(), s.skipped, first.join(", "));
                    }
                }
                Err(e) => println!("{f}: {e}"),
            }
        }
        return Ok(());
    }
    if args.iter().any(|a| a == "--fmt-check") {
        // Read-only: how a dual cab's knobs read at their edges.
        let cat = catalog::find_dir(None).and_then(|p| catalog::Catalog::load(&p).ok()).expect("HX Edit's files");
        let n = cat.number_for_hlx("HD2_CabMicIr_1x12USDeluxeWithPan", false).expect("model");
        for (k, (id, _, _)) in cat.default_values(n).into_iter().enumerate() {
            let (name, meta) = cat.param(n, k);
            let (lo, hi) = meta.as_ref().map(|m| (m.min, m.max)).unwrap_or((0.0, 1.0));
            let mid = (lo + hi) / 2.0;
            let seg = meta.as_ref().is_some_and(|m| m.spec.segmented);
            println!(
                "{id} ({name}): min {lo} -> {}, mid -> {}, max {hi} -> {}{}",
                catalog::format_value(meta.as_ref(), lo),
                catalog::format_value(meta.as_ref(), mid),
                catalog::format_value(meta.as_ref(), hi),
                if seg { "  [two choices]" } else { "" }
            );
        }
        return Ok(());
    }
    if let Some(i) = args.iter().position(|a| a == "--find-blocks") {
        // Read-only: print every block (and what rides with it) whose model contains a text, from .hxb backups.
        let needle = args.get(i + 1).cloned().unwrap_or_default();
        let mut shown = 0;
        for f in &args[i + 2..] {
            let Ok(c) = std::fs::read(f).map_err(|e| e.to_string()).and_then(|b| backup::read_hxb(&b)) else { continue };
            for s in &c.setlists {
                for p in &s.presets {
                    for dsp in hlx::dsps(&p.hlx) {
                        let Some(sec) = p.hlx.pointer(&format!("/data/tone/{dsp}")).and_then(serde_json::Value::as_object) else { continue };
                        for (k, v) in sec {
                            let m = v.get("@model").and_then(serde_json::Value::as_str).unwrap_or("");
                            if m.contains(&needle) && shown < 6 {
                                shown += 1;
                                println!("== {} / {} {dsp}.{k}: {v}", p.name, s.name);
                                if let Some(c) = v.get("@cab").and_then(serde_json::Value::as_str) {
                                    println!("   rides with {c}: {}", sec.get(c).cloned().unwrap_or_default());
                                }
                            }
                        }
                    }
                }
            }
        }
        println!("{shown} shown");
        return Ok(());
    }
    if let Some(i) = args.iter().position(|a| a == "--attach-check") {
        // Read-only: move each path's split and merge in memory and read the positions back.
        for f in &args[i + 1..] {
            let Ok(b) = std::fs::read(f) else { continue };
            let Ok(mut d) = devpreset::DevicePreset::parse(&b) else { continue };
            let slots = d.slots();
            for s in slots.iter().filter(|s| matches!(s.kind, devpreset::Kind::Split | devpreset::Kind::Join)) {
                let before = d.attach(s.path, s.position);
                d.set_attach(s.path, s.position, 3);
                let after = d.attach(s.path, s.position);
                println!("{:?} path {} pos {}: {:?} -> {:?}", s.kind, s.path, s.position, before, after);
            }
        }
        return Ok(());
    }
    if let Some(i) = args.iter().position(|a| a == "--dsp-report") {
        // Read-only: the DSP estimate of pedal presets, path by path, with each block's load.
        let cat = catalog::find_dir(None).and_then(|p| catalog::Catalog::load(&p).ok()).expect("HX Edit's files");
        for f in &args[i + 1..] {
            let Ok(b) = std::fs::read(f) else { continue };
            let Ok(d) = devpreset::DevicePreset::parse(&b) else { continue };
            let slots = d.slots();
            for path in [0i64, 1] {
                let mut total = 0.0;
                let mut parts = Vec::new();
                for s in slots.iter().filter(|s| s.path == path) {
                    for m in [s.model, s.paired].into_iter().flatten() {
                        let c = cat.dsp_cost(m);
                        total += c;
                        parts.push(format!("{}={:.2}", cat.symbol(m).unwrap_or("?"), c));
                    }
                }
                if !parts.is_empty() {
                    println!("{:.2}\t{}\tpath {path}\t{}", total, std::path::Path::new(f).file_name().and_then(|x| x.to_str()).unwrap_or("?"), parts.join(" "));
                }
            }
        }
        return Ok(());
    }
    if let Some(i) = args.iter().position(|a| a == "--ctl-check") {
        // Read-only: read each preset's controllers and write them back unchanged; it must stay identical.
        let (mut same, mut differ) = (0, 0);
        for f in &args[i + 1..] {
            let b = std::fs::read(f).expect("read");
            let d = devpreset::DevicePreset::parse(&b).expect("parses");
            let list = d.controllers();
            let mut d2 = d.clone();
            d2.set_controllers(&list);
            if d2.encode() == d.encode() {
                same += 1;
            } else {
                differ += 1;
                println!("differs: {f} ({} assignments)", list.len());
            }
        }
        println!("round trip: {same} identical, {differ} different");
        return Ok(());
    }
    if let Some(i) = args.iter().position(|a| a == "--dump-doc") {
        // Read-only: print a pedal preset document's structure (to learn its layout).
        let b = std::fs::read(args.get(i + 1).expect("file")).expect("read");
        let d = devpreset::DevicePreset::parse(&b).expect("parses");
        fn show(v: &msgpack::Value) -> String {
            use msgpack::Value as V;
            match v {
                V::Nil => "nil".into(),
                V::Bool(b) => b.to_string(),
                V::Int(n, _) => n.to_string(),
                V::F32(f) => format!("{f}f"),
                V::F64(f) => format!("{f}d"),
                V::Str(s, _) => format!("{:?}", s.trim_end_matches('\0')),
                V::Bin(b, _) => format!("bin({})", b.len()),
                V::Array(a, _) => format!("[{}]", a.iter().map(show).collect::<Vec<_>>().join(", ")),
                V::Map(m, _) => format!("{{{}}}", m.iter().map(|(k, v)| format!("{}: {}", show(k), show(v))).collect::<Vec<_>>().join(", ")),
                #[allow(unreachable_patterns)]
                _ => "ext".into(),
            }
        }
        if let msgpack::Value::Map(m, _) = &d.tone {
            for (k, v) in m {
                println!("== {} ==\n{}\n", show(k), show(v));
            }
        }
        return Ok(());
    }
    if let Some(i) = args.iter().position(|a| a == "--hlx-check") {
        // Offline check of the .hlx -> pedal conversion: --hlx-check <template.hxpreset> [file.hlx]
        let cat = catalog::find_dir(None).and_then(|p| catalog::Catalog::load(&p).ok()).expect("HX Edit's files");
        let template = std::fs::read(args.get(i + 1).expect("template")).expect("read template");
        let template = devpreset::DevicePreset::parse(&template).expect("template parses");
        let json = match args.get(i + 2) {
            Some(p) => serde_json::from_str(&std::fs::read_to_string(p).expect("read hlx")).expect("hlx json"),
            None => hlx::sample(),
        };
        match devpreset::DevicePreset::from_hlx(&template, &json, &cat) {
            Ok((d, notes)) => {
                let bytes = d.encode();
                let again = devpreset::DevicePreset::parse(&bytes).map(|x| x.encode() == bytes);
                println!("converted: {} bytes, parses back identical: {again:?}", bytes.len());
                for s in d.slots() {
                    let name = s.model.map(|m| cat.model_name(m)).unwrap_or_default();
                    let vals: Vec<String> = s.values.iter().take(4).enumerate().map(|(k, v)| {
                        let (n, meta) = s.model.map(|m| cat.param(m, k)).unwrap_or_default();
                        format!("{n}={}", catalog::format_value(meta.as_ref(), v.as_f64().unwrap_or(0.0)))
                    }).collect();
                    let attach = d.attach(s.path, s.position).map(|a| format!(" attach={a}")).unwrap_or_default();
                    let snaps: String = (0..d.snapshot_names().len())
                        .map(|k| match d.snapshot_enabled(k, s.wire_block().max(0) as usize) { Some(true) => '1', Some(false) => '0', None => '-' })
                        .collect();
                    println!("  #{} {:?} {name} on={:?} snaps={snaps} paired={:?}{attach} {}", s.global, s.kind, s.enabled, s.paired.map(|p| cat.model_name(p)), vals.join(", "));
                }
                for c in d.controllers() {
                    println!("  controller {} kind {} slot {} knob {} cab {} min {:?} max {:?} cc {} snaps {:?}", c.controller, c.kind, c.wire, c.index, c.cab, c.min, c.max, c.cc, c.snaps);
                }
                for w in 0..20 {
                    if let Some(fs) = d.block_footswitch(w) {
                        println!("  slot {w} on/off is on footswitch {fs}");
                    }
                }
                for n in notes {
                    println!("note: {n}");
                }
            }
            Err(e) => println!("refused: {e}"),
        }
        return Ok(());
    }
    if args.iter().any(|a| a == "--list-models") {
        // Print every effect in the picker with its real-gear name, if known.
        if let Some(cat) = catalog::find_dir(None).and_then(|p| catalog::Catalog::load(&p).ok()) {
            for g in &cat.groups {
                for m in &g.models {
                    println!("{}\t{}\t{}\t{}", g.category, g.sub, m.name, realnames::real(&m.name).unwrap_or_else(|| "-".into()));
                }
            }
        }
        return Ok(());
    }
    if args.iter().any(|a| a == "--find-boxes") {
        // Look for Helrig Boxes for 6 seconds and print what answers.
        let (tx, rx) = std::sync::mpsc::channel();
        let _d = boxes::start_discovery(tx);
        let until = std::time::Instant::now() + std::time::Duration::from_secs(6);
        while let Ok(b) = rx.recv_timeout(until.saturating_duration_since(std::time::Instant::now())) {
            let status = boxes::relay_status(&b.host)
                .map(|s| format!("relay {} pedal {:?} in use {}", s.relay_only, s.pedal, s.client))
                .unwrap_or_else(|e| e);
            println!("{} at {} (id {}): {status}", b.name, b.host, b.id);
        }
        return Ok(());
    }

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("ToneStrand")
        .with_app_id("tonestrand")
        .with_inner_size([1320.0, 820.0])
        .with_min_inner_size([960.0, 600.0]);
    if let Some(icon) = theme::window_icon() {
        viewport = viewport.with_icon(std::sync::Arc::new(icon));
    }
    // Filling the screen happens once the window is up (see App::update): asking while it is
    // created leaves it flagged maximized but at its first size on some Windows setups.
    if screenshot.is_some() {
        // Open quietly behind other windows; it keeps drawing there.
        viewport = viewport.with_active(false);
        #[cfg(windows)]
        {
            viewport = viewport.with_window_level(egui::WindowLevel::AlwaysOnBottom);
        }
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    let result = eframe::run_native(
        "ToneStrand",
        options,
        Box::new(move |cc| Ok(Box::new(window::App::new(cc, screenshot)))),
    );
    // Background helpers (network search, a pedal link mid-answer) must not
    // keep the program alive once its window is closed.
    if result.is_ok() {
        std::process::exit(0);
    }
    result
}

/// `--probe`: connect to the first USB pedal, list its setlists and presets,
/// print them and disconnect. Read-only; nothing on the pedal changes.
fn probe(host: Option<String>) {
    let w: Box<dyn wire::Wire> = if let Some(host) = host {
        // The pairing ToneStrand saved for that box (pair it in the app first).
        let settings = boxes::Settings::load();
        let Some(token) = settings.tokens.get(&host).cloned().or_else(|| settings.saved.iter().find(|b| b.host == host).and_then(|b| settings.token_for(b))) else {
            return println!("no saved pairing for {host}");
        };
        match wire::BoxWire::open(&host, &token, "box") {
            Ok(w) => Box::new(w),
            Err(e) => return println!("box open failed: {e}"),
        }
    } else {
        let pedals = wire::list_usb_pedals();
        println!("USB pedals: {pedals:?}");
        let Some(p) = pedals.first() else { return };
        match wire::UsbWire::open(p.pid) {
            Ok(w) => Box::new(w),
            Err(e) => return println!("open failed: {e}"),
        }
    };
    let mut s = match session::Session::open(w) {
        Ok(s) => s,
        Err(e) => return println!("session failed: {e}"),
    };
    println!("connected: {}", s.describe());
    match s.preset_info() {
        Ok(i) => println!("current: {i:?}"),
        Err(e) => println!("preset_info: {e}"),
    }
    match s.setlists() {
        Ok(l) => println!("setlists: {l:?}"),
        Err(e) => println!("setlists: {e}"),
    }
    match s.presets(0) {
        Ok(l) => println!("presets ({}): {:?}", l.len(), &l[..l.len().min(12)]),
        Err(e) => println!("presets: {e}"),
    }
    // Read-only: one stored slot and the IR list, the two reads that were timing out.
    let slot = s.preset_info().map(|i| i.1.max(0)).unwrap_or(0);
    match s.fetch_slot(0, slot) {
        Ok(Some(b)) => println!("fetch slot {slot}: {} bytes", b.len()),
        Ok(None) => println!("fetch slot {slot}: empty"),
        Err(e) => println!("fetch slot {slot}: {e}"),
    }
    match s.list_irs() {
        Ok(l) => println!("irs ({}): {:?}", l.len(), &l[..l.len().min(8)]),
        Err(e) => println!("irs: {e}"),
    }
    match s.read_current() {
        Ok(b) => match devpreset::DevicePreset::parse(&b) {
            Ok(d) => {
                let same = d.encode() == b;
                println!("current preset document: {} bytes, round trip identical: {same}, slots {}", b.len(), d.slots().len());
                if let Some(cat) = catalog::find_dir(None).and_then(|p| catalog::Catalog::load(&p).ok()) {
                    println!("catalog: {} models, {} picker groups", cat.sym.len(), cat.groups.len());
                    let total: f64 = d.slots().iter().map(|s| s.model.map(|m| cat.dsp_cost(m)).unwrap_or(0.0) + s.paired.map(|m| cat.dsp_cost(m)).unwrap_or(0.0)).sum();
                    println!("dsp load total {total:.2}");
                    for s in d.slots() {
                        let name = s.model.map(|m| cat.model_name(m)).unwrap_or_default();
                        let params: Vec<String> = s.values.iter().enumerate().take(4).map(|(i, v)| { let (n, meta) = s.model.map(|m| cat.param(m, i)).unwrap_or_default(); format!("{n}={}", catalog::format_value(meta.as_ref(), v.as_f64().unwrap_or(0.0))) }).collect();
                        println!("  #{} path {} {:?} {name} on={:?} paired={:?} {}", s.global, s.path, s.kind, s.enabled, s.paired.map(|p| cat.model_name(p)), params.join(", "));
                    }
                }
            }
            Err(e) => println!("document: {e}"),
        },
        Err(e) => println!("read_current: {e}"),
    }
    // The app's order: a live read, then idling like the window's worker does, then the reads.
    for round in 0..2 {
        // Long enough for the pedal's "still there?" messages to need answering.
        let until = std::time::Instant::now() + std::time::Duration::from_secs(8);
        while std::time::Instant::now() < until {
            let _ = s.idle(std::time::Duration::from_millis(30));
            std::thread::sleep(std::time::Duration::from_millis(40));
        }
        let t = std::time::Instant::now();
        match s.fetch_slot(0, slot) {
            Ok(b) => println!("after idle {round}: fetch slot {slot}: {:?} bytes in {:?}", b.map(|x| x.len()), t.elapsed()),
            Err(e) => println!("after idle {round}: fetch slot {slot}: {e} after {:?}", t.elapsed()),
        }
        let t = std::time::Instant::now();
        match s.list_irs() {
            Ok(l) => println!("after idle {round}: irs {} in {:?}", l.len(), t.elapsed()),
            Err(e) => println!("after idle {round}: irs: {e} after {:?}", t.elapsed()),
        }
        let _ = s.read_current();
    }
    let _ = s.idle(std::time::Duration::from_millis(300));
}
