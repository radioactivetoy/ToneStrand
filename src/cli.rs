//! The command-line tools: read-only checks and helpers run as `tonestrand --<tool> ...`.
//! Anything else opens the window. `tonestrand --help` lists them.

use crate::{backup, boxes, catalog, devpreset, hlx, msgpack, nam, proto, realnames, session, wire};

/// The command-line tools (everything else opens the window).
pub const CLI_FLAGS: [&str; 12] = [
    "--probe",
    "--nam2ir",
    "--read-backup",
    "--fmt-check",
    "--find-blocks",
    "--attach-check",
    "--dsp-report",
    "--ctl-check",
    "--dump-doc",
    "--hlx-check",
    "--list-models",
    "--find-boxes",
];

/// Release builds are windowed programs on Windows, with no console: borrow the console of
/// the shell that started us so the tools' output shows up there.
fn attach_console() {
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        extern "system" {
            fn AttachConsole(process_id: u32) -> i32;
        }
        const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
        // Fails harmlessly when there already is a console (debug builds) or no parent one.
        unsafe {
            AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}

/// A value or, for the tools, a printed reason and None.
fn need<T, E: std::fmt::Display>(r: Result<T, E>, what: &str) -> Option<T> {
    match r {
        Ok(v) => Some(v),
        Err(e) => {
            eprintln!("{what}: {e}");
            None
        }
    }
}

/// HX Edit's catalog for the tools that need it.
fn tool_catalog() -> Option<catalog::Catalog> {
    let Some(dir) = catalog::find_dir(boxes::Settings::load().hx_edit.as_deref()) else {
        eprintln!("HX Edit's files were not found; install HX Edit or set its folder in Settings");
        return None;
    };
    need(catalog::Catalog::load(&dir), "HX Edit's files")
}

fn read_preset(f: &str) -> Option<devpreset::DevicePreset> {
    let b = need(std::fs::read(f), f)?;
    need(devpreset::DevicePreset::parse(&b), f)
}

const HELP: &str = "\
ToneStrand command-line tools (all read-only unless noted):

  --probe [box address]            connect to the first USB pedal (or a paired box), list presets and IRs
  --nam2ir <out folder> <file.nam>...  convert NAM captures to User IR .wav files (writes files)
  --read-backup <file.hxb|.hls>...  list what an HX Edit backup or setlist holds
  --find-blocks <text> <file.hxb>...  print blocks whose model contains <text>
  --attach-check <file.hxpreset>...  move each split/merge in memory and read it back
  --dsp-report <file.hxpreset>...   DSP estimate per path, block by block
  --ctl-check <file.hxpreset>...    controllers must round-trip unchanged
  --dump-doc <file.hxpreset>        print a pedal preset document's structure
  --hlx-check <template.hxpreset> [file.hlx]  try the .hlx -> pedal conversion offline
  --fmt-check                       how a dual cab's knobs read at their edges
  --list-models                     every effect in the picker, with real-gear names
  --find-boxes                      look for Helrig Boxes for 6 seconds
  --screenshot <file.png>           open the window, save a picture of it and quit
";

/// Run the tool named in `args`, if any. Returns true when one ran (the program then exits).
pub fn run(args: &[String]) -> bool {
    let wants_help = args.iter().skip(1).any(|a| a == "--help" || a == "-h" || a == "/?");
    if !wants_help && !args.iter().any(|a| CLI_FLAGS.contains(&a.as_str())) {
        return false;
    }
    attach_console();
    if wants_help {
        print!("{HELP}");
        return true;
    }
    if let Some(i) = args.iter().position(|a| a == "--probe") {
        probe(args.get(i + 1).cloned());
        return true;
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
        return true;
    }
    if let Some(i) = args.iter().position(|a| a == "--read-backup") {
        // Read-only: list what a .hxb or .hls holds.
        for f in &args[i + 1..] {
            let Some(b) = need(std::fs::read(f), f) else { continue };
            let r = if f.to_ascii_lowercase().ends_with(".hls") {
                backup::read_hls(&b).map(|s| (vec![s], 0))
            } else {
                backup::read_hxb(&b).map(|c| (c.setlists, c.irs.len()))
            };
            match r {
                Ok((sets, irs)) => {
                    println!("{f}: {} setlists, {irs} IRs", sets.len());
                    for s in &sets {
                        let first: Vec<String> = s
                            .presets
                            .iter()
                            .take(4)
                            .map(|p| format!("{} {} ({} blocks)", proto::slot_label(p.slot), p.name, hlx::blocks(&p.hlx).len()))
                            .collect();
                        println!("  {}: {} presets, {} skipped; {}", s.name, s.presets.len(), s.skipped, first.join(", "));
                    }
                }
                Err(e) => println!("{f}: {e}"),
            }
        }
        return true;
    }
    if args.iter().any(|a| a == "--fmt-check") {
        // Read-only: how a dual cab's knobs read at their edges.
        let Some(cat) = tool_catalog() else { return true };
        let Some(n) = cat.number_for_hlx("HD2_CabMicIr_1x12USDeluxeWithPan", false) else {
            eprintln!("this HX Edit has no HD2_CabMicIr_1x12USDeluxeWithPan");
            return true;
        };
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
        return true;
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
                        let Some(sec) = p.hlx.pointer(&format!("/data/tone/{dsp}")).and_then(serde_json::Value::as_object) else {
                            continue;
                        };
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
        return true;
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
        return true;
    }
    if let Some(i) = args.iter().position(|a| a == "--dsp-report") {
        // Read-only: the DSP estimate of pedal presets, path by path, with each block's load.
        let Some(cat) = tool_catalog() else { return true };
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
                    println!(
                        "{:.2}\t{}\tpath {path}\t{}",
                        total,
                        std::path::Path::new(f).file_name().and_then(|x| x.to_str()).unwrap_or("?"),
                        parts.join(" ")
                    );
                }
            }
        }
        return true;
    }
    if let Some(i) = args.iter().position(|a| a == "--ctl-check") {
        // Read-only: read each preset's controllers and write them back unchanged; it must stay identical.
        let (mut same, mut differ) = (0, 0);
        for f in &args[i + 1..] {
            let Some(d) = read_preset(f) else { continue };
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
        return true;
    }
    if let Some(i) = args.iter().position(|a| a == "--dump-doc") {
        // Read-only: print a pedal preset document's structure (to learn its layout).
        let Some(f) = args.get(i + 1) else {
            eprintln!("usage: --dump-doc <file.hxpreset>");
            return true;
        };
        let Some(d) = read_preset(f) else { return true };
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
                V::Map(m, _) => {
                    format!("{{{}}}", m.iter().map(|(k, v)| format!("{}: {}", show(k), show(v))).collect::<Vec<_>>().join(", "))
                }
                #[allow(unreachable_patterns)]
                _ => "ext".into(),
            }
        }
        if let msgpack::Value::Map(m, _) = &d.tone {
            for (k, v) in m {
                println!("== {} ==\n{}\n", show(k), show(v));
            }
        }
        return true;
    }
    if let Some(i) = args.iter().position(|a| a == "--hlx-check") {
        // Offline check of the .hlx -> pedal conversion: --hlx-check <template.hxpreset> [file.hlx]
        let Some(cat) = tool_catalog() else { return true };
        let Some(f) = args.get(i + 1) else {
            eprintln!("usage: --hlx-check <template.hxpreset> [file.hlx]");
            return true;
        };
        let Some(template) = read_preset(f) else { return true };
        let json = match args.get(i + 2) {
            Some(p) => {
                let Some(text) = need(std::fs::read_to_string(p), p) else { return true };
                let Some(j) = need(serde_json::from_str(&text), p) else { return true };
                j
            }
            None => hlx::sample(),
        };
        match devpreset::DevicePreset::from_hlx(&template, &json, &cat) {
            Ok((d, notes)) => {
                let bytes = d.encode();
                let again = devpreset::DevicePreset::parse(&bytes).map(|x| x.encode() == bytes);
                println!("converted: {} bytes, parses back identical: {again:?}", bytes.len());
                for s in d.slots() {
                    let name = s.model.map(|m| cat.model_name(m)).unwrap_or_default();
                    let vals: Vec<String> = s
                        .values
                        .iter()
                        .take(4)
                        .enumerate()
                        .map(|(k, v)| {
                            let (n, meta) = s.model.map(|m| cat.param(m, k)).unwrap_or_default();
                            format!("{n}={}", catalog::format_value(meta.as_ref(), v.as_f64().unwrap_or(0.0)))
                        })
                        .collect();
                    let attach = d.attach(s.path, s.position).map(|a| format!(" attach={a}")).unwrap_or_default();
                    let snaps: String = (0..d.snapshot_names().len())
                        .map(|k| match d.snapshot_enabled(k, s.wire_block().max(0) as usize) {
                            Some(true) => '1',
                            Some(false) => '0',
                            None => '-',
                        })
                        .collect();
                    println!(
                        "  #{} {:?} {name} on={:?} snaps={snaps} paired={:?}{attach} {}",
                        s.global,
                        s.kind,
                        s.enabled,
                        s.paired.map(|p| cat.model_name(p)),
                        vals.join(", ")
                    );
                }
                for c in d.controllers() {
                    println!(
                        "  controller {} kind {} slot {} knob {} cab {} min {:?} max {:?} cc {} snaps {:?}",
                        c.controller, c.kind, c.wire, c.index, c.cab, c.min, c.max, c.cc, c.snaps
                    );
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
        return true;
    }
    if args.iter().any(|a| a == "--list-models") {
        // Print every effect in the picker with its real-gear name, if known.
        if let Some(cat) = tool_catalog() {
            for g in &cat.groups {
                for m in &g.models {
                    println!("{}\t{}\t{}\t{}", g.category, g.sub, m.name, realnames::real(&m.name).unwrap_or_else(|| "-".into()));
                }
            }
        }
        return true;
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
        return true;
    }
    true
}

/// `--probe`: connect to the first USB pedal, list its setlists and presets,
/// print them and disconnect. Read-only; nothing on the pedal changes.
fn probe(host: Option<String>) {
    let w: Box<dyn wire::Wire> = if let Some(host) = host {
        // The pairing ToneStrand saved for that box (pair it in the app first).
        let settings = boxes::Settings::load();
        let Some(token) = settings
            .tokens
            .get(&host)
            .cloned()
            .or_else(|| settings.saved.iter().find(|b| b.host == host).and_then(|b| settings.token_for(b)))
        else {
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
                    let total: f64 = d
                        .slots()
                        .iter()
                        .map(|s| s.model.map(|m| cat.dsp_cost(m)).unwrap_or(0.0) + s.paired.map(|m| cat.dsp_cost(m)).unwrap_or(0.0))
                        .sum();
                    println!("dsp load total {total:.2}");
                    for s in d.slots() {
                        let name = s.model.map(|m| cat.model_name(m)).unwrap_or_default();
                        let params: Vec<String> = s
                            .values
                            .iter()
                            .enumerate()
                            .take(4)
                            .map(|(i, v)| {
                                let (n, meta) = s.model.map(|m| cat.param(m, i)).unwrap_or_default();
                                format!("{n}={}", catalog::format_value(meta.as_ref(), v.as_f64().unwrap_or(0.0)))
                            })
                            .collect();
                        println!(
                            "  #{} path {} {:?} {name} on={:?} paired={:?} {}",
                            s.global,
                            s.path,
                            s.kind,
                            s.enabled,
                            s.paired.map(|p| cat.model_name(p)),
                            params.join(", ")
                        );
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
