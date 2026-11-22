//! The command-line tools: read-only checks and helpers run as `tonestrand --<tool> ...`.
//! Anything else opens the window. `tonestrand --help` lists them.

use crate::msgpack::Value;
use crate::{backup, boxes, catalog, devpreset, hlx, msgpack, nam, proto, realnames, session, wire};

/// The command-line tools (everything else opens the window).
pub const CLI_FLAGS: [&str; 26] = [
    "--export-catalog",
    "--export-focus",
    "--to-hlx",
    "--cc-live",
    "--put-slot",
    "--cc-snap",
    "--send-hlx",
    "--dump-loaded",
    "--box-proxy",
    "--cc-raw",
    "--op-raw",
    "--cc-reload",
    "--commands",
    "--cc-compare",
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

/// The effects list for the tools that need it.
fn tool_catalog() -> Option<catalog::Catalog> {
    let cat = catalog::load_best(boxes::Settings::load().hx_edit.as_deref());
    if cat.is_none() {
        eprintln!("no effects list found; install HX Edit or set its folder in Settings");
    }
    cat
}

fn read_preset(f: &str) -> Option<devpreset::DevicePreset> {
    let b = need(std::fs::read(f), f)?;
    need(devpreset::DevicePreset::parse(&b), f)
}

/// A session to the first USB pedal, or to a paired ToneStrand Box at `host`.
fn open_session(host: Option<&str>) -> Option<session::Session> {
    let w: Box<dyn wire::Wire> = if let Some(host) = host {
        let settings = boxes::Settings::load();
        // A box behind a forwarder ("192.168.1.5:8080") uses the pairing saved for its address.
        let bare = host.rsplit_once(':').filter(|(h, _)| !h.contains(':')).map_or(host, |(h, _)| h);
        // TONESTRAND_BOX_PAIRING=<address> names the pairing to use when the address is a forwarder's.
        let named = std::env::var("TONESTRAND_BOX_PAIRING").unwrap_or_default();
        let token = [host, bare, named.as_str()].iter().filter(|h| !h.is_empty()).find_map(|h| {
            settings.tokens.get(*h).cloned().or_else(|| settings.saved.iter().find(|b| b.host == *h).and_then(|b| settings.token_for(b)))
        });
        let Some(token) = token else {
            eprintln!("no saved pairing for {host}");
            return None;
        };
        Box::new(need(wire::BoxWire::open(host, &token, "box"), "the box")?)
    } else {
        let pedals = wire::list_usb_pedals();
        let Some(p) = pedals.first() else {
            eprintln!("no USB pedal found (name a ToneStrand Box address as the last argument to use one)");
            return None;
        };
        Box::new(need(wire::UsbWire::open(p.pid), "the pedal")?)
    };
    need(session::Session::open(w), "the session")
}

/// A pedal document from a file, or from a slot of the pedal ("42B").
fn load_source(src: &str, host: Option<&str>) -> Option<devpreset::DevicePreset> {
    if src.to_ascii_lowercase().ends_with(".hxpreset") {
        return read_preset(src);
    }
    let Some(slot) = crate::cmdtool::slot_index(src) else {
        eprintln!("{src}: not a .hxpreset file or a slot like 42B");
        return None;
    };
    let mut s = open_session(host)?;
    let blob = need(s.fetch_slot(0, slot as i64), "reading the slot")?;
    let Some(blob) = blob else {
        eprintln!("slot {src} is empty");
        return None;
    };
    need(devpreset::DevicePreset::parse(&blob), "the slot's document")
}

/// `--commands <file|slot> [box address]`
fn commands_tool(rest: &[String]) {
    let Some(src) = rest.first() else {
        eprintln!("usage: --commands <file.hxpreset | slot like 42B> [box address]");
        return;
    };
    let Some(p) = load_source(src, rest.get(1).map(String::as_str)) else { return };
    for l in crate::cmdtool::describe(&p, &crate::cmdtool::unit_slots()) {
        println!("{l}");
    }
}

/// `--cc-compare <file|slot> <file.hlx> [box address]`
fn cc_compare_tool(rest: &[String]) {
    let (Some(src), Some(hlx_path)) = (rest.first(), rest.get(1)) else {
        eprintln!("usage: --cc-compare <file.hxpreset | slot like 42B> <file.hlx> [box address]");
        return;
    };
    let Some(text) = need(std::fs::read_to_string(hlx_path), hlx_path) else { return };
    let Some(json) = need(hlx::parse_text(&text), hlx_path) else { return };
    let Some(p) = load_source(src, rest.get(2).map(String::as_str)) else { return };
    let slots = crate::cmdtool::unit_slots();
    let dev = crate::cmdtool::device_commands(&p);
    let hlx_cmds = crate::cmdtool::hlx_commands(&json);
    println!("pedal: {} command(s) set in {} slots; .hlx: {} command(s)", dev.len(), p.command_count(), hlx_cmds.len());
    for l in crate::cmdtool::compare(&dev, &hlx_cmds, &slots) {
        println!("{l}");
    }
    let (d, h) = (crate::cmdtool::device_overrides(&p), crate::cmdtool::hlx_overrides(&json));
    if !d.is_empty() || !h.is_empty() {
        println!("per-snapshot command entries");
        for (n, what, v) in d {
            println!("  pedal: snapshot {} {what}: {v}", n + 1);
        }
        for (n, key, v) in h {
            println!("  .hlx:  snapshot {n} {key}: {v}");
        }
    }
}

/// `--cc-raw <box address> <slot> <id> [args]`: change one Command Center slot of the loaded preset in the
/// pedal's edit buffer (nothing is saved to a slot; `--cc-reload` or loading the preset drops it).
fn cc_raw_tool(rest: &[String]) {
    let (Some(host), Some(slot), Some(id)) = (rest.first(), rest.get(1), rest.get(2)) else {
        eprintln!("usage: --cc-raw <box address> <slot 0-12> <command id> [arg1 .. arg6]");
        return;
    };
    let (Some(slot), Some(id)) = (need(slot.parse::<usize>(), "slot"), need(id.parse::<i64>(), "command id")) else { return };
    let Some(args) = rest[3..].iter().map(|a| need(a.parse::<i64>(), a)).collect::<Option<Vec<i64>>>() else { return };
    let Some(mut s) = open_session(Some(host)) else { return };
    let Some(blob) = need(s.read_current(), "reading the loaded preset") else { return };
    let Some(mut doc) = need(devpreset::DevicePreset::parse(&blob), "the loaded preset") else { return };
    if !doc.set_command(slot, id, &args) {
        eprintln!("the loaded preset has no Command Center slot {slot}");
        return;
    }
    let Some(bytes) = need(doc.encode_checked(), "the document") else { return };
    if need(s.write_edit_buffer(bytes), "writing the edit buffer").is_none() {
        return;
    }
    let Some(back) = need(s.read_current(), "reading it back").and_then(|b| need(devpreset::DevicePreset::parse(&b), "it")) else { return };
    println!("written to the pedal's edit buffer (not saved). The loaded preset now has:");
    for l in crate::cmdtool::describe(&back, &crate::cmdtool::unit_slots()) {
        println!("{l}");
    }
}

/// `--cc-snap <box address> <snapshot 1-based> <slot 0-12> [arg1 .. arg6]`: WRITES one command's values in one
/// snapshot of the loaded preset into the pedal's edit buffer (not saved), as the app's snapshot selector does.
fn cc_snap_tool(rest: &[String]) {
    let (Some(host), Some(snap), Some(slot)) = (rest.first(), rest.get(1), rest.get(2)) else {
        eprintln!("usage: --cc-snap <box address> <snapshot 1-based> <slot 0-12> [arg1 .. arg6]");
        return;
    };
    let (Some(snap), Some(slot)) = (need(snap.parse::<usize>(), "snapshot"), need(slot.parse::<usize>(), "slot")) else { return };
    let Some(args) = rest[3..].iter().map(|a| need(a.parse::<i64>(), a)).collect::<Option<Vec<i64>>>() else { return };
    let Some(mut s) = open_session(Some(host)) else { return };
    let Some(blob) = need(s.read_current(), "reading the loaded preset") else { return };
    let Some(mut doc) = need(devpreset::DevicePreset::parse(&blob), "the loaded preset") else { return };
    if !doc.set_command_values(snap.saturating_sub(1), slot, &args) {
        eprintln!("the loaded preset has no command in slot {slot}");
        return;
    }
    let Some(bytes) = need(doc.encode_checked(), "the document") else { return };
    if need(s.write_edit_buffer(bytes), "writing the edit buffer").is_none() {
        return;
    }
    let Some(back) = need(s.read_current(), "reading it back").and_then(|b| need(devpreset::DevicePreset::parse(&b), "it")) else { return };
    println!("written to the pedal's edit buffer (not saved). The loaded preset now has:");
    for l in crate::cmdtool::describe(&back, &crate::cmdtool::unit_slots()) {
        println!("{l}");
    }
}

/// `--put-slot <box address> <slot like 42B> <file.hxpreset> <name>`: WRITES a saved pedal document back to a
/// slot (to undo a test). Reads the slot back and says whether it came back the same.
fn put_slot_tool(rest: &[String]) {
    let (Some(host), Some(label), Some(file), Some(name)) = (rest.first(), rest.get(1), rest.get(2), rest.get(3)) else {
        eprintln!("usage: --put-slot <box address> <slot like 42B> <file.hxpreset> <name>");
        return;
    };
    let Some(slot) = crate::cmdtool::slot_index(label) else {
        eprintln!("{label}: not a slot like 42B");
        return;
    };
    let Some(bytes) = need(std::fs::read(file), file) else { return };
    if need(devpreset::DevicePreset::parse(&bytes), file).is_none() {
        return;
    }
    let Some(mut s) = open_session(Some(host)) else { return };
    let Some(sent) = need(s.write_slot(0, slot as i64, name, bytes), "writing the slot") else { return };
    let back = need(s.fetch_slot(0, slot as i64), "reading it back").flatten();
    println!("wrote {file} to {label}; read back {}", if back.as_deref() == Some(sent.as_slice()) { "the same" } else { "DIFFERENT" });
}

/// `--cc-live <box address> type <slot> <id> [args]`, `value <snapshot 1-based> <slot> <arg1..>` or
/// `momentary <slot> <0|1>`: a Command Center edit on the loaded preset the way the app makes it (the pedal's own
/// requests, then a check, falling back to writing the edit buffer). Says which way it went. Edit buffer only.
fn cc_live_tool(rest: &[String]) {
    let (Some(host), Some(kind)) = (rest.first(), rest.get(1)) else {
        eprintln!("usage: --cc-live <box address> type <slot> <id> [args] | value <snapshot> <slot> <args> | momentary <slot> <0|1>");
        return;
    };
    let nums: Option<Vec<i64>> = rest[2..].iter().map(|a| need(a.parse::<i64>(), a)).collect();
    let Some(n) = nums else { return };
    let change = match (kind.as_str(), n.as_slice()) {
        ("type", [slot, id, args @ ..]) => crate::device::CcChange::Type { slot: *slot as usize, id: *id, args: args.to_vec() },
        ("value", [snap, slot, args @ ..]) => {
            crate::device::CcChange::Values { snapshot: (*snap as usize).saturating_sub(1), slot: *slot as usize, args: args.to_vec() }
        }
        ("momentary", [slot, m]) => crate::device::CcChange::Momentary { slot: *slot as usize, momentary: *m != 0 },
        _ => {
            eprintln!("not understood; see the usage");
            return;
        }
    };
    let Some(mut s) = open_session(Some(host)) else { return };
    match crate::device::change_command_center(&mut s, &|_| {}, change) {
        Ok(true) => println!("done with the pedal's own requests"),
        Ok(false) => println!("done by writing the edit buffer (the pedal's own requests were not enough)"),
        Err(e) => {
            eprintln!("failed: {e}");
            return;
        }
    }
    let Some(back) = need(s.read_current(), "reading it back").and_then(|b| need(devpreset::DevicePreset::parse(&b), "it")) else { return };
    for l in crate::cmdtool::describe(&back, &crate::cmdtool::unit_slots()) {
        println!("{l}");
    }
}

/// `--to-hlx <file.hxpreset> <out.hlx> [device number of the unit to make it for]`: a pedal preset as a `.hlx`,
/// optionally made for another unit, with the list of what was left out or changed. Writes only the `.hlx`.
fn to_hlx_tool(rest: &[String]) {
    let (Some(src), Some(out)) = (rest.first(), rest.get(1)) else {
        eprintln!("usage: --to-hlx <file.hxpreset> <out.hlx> [device number, e.g. 2162689 for a Helix Floor]");
        return;
    };
    let Some(cat) = tool_catalog() else { return };
    let Some(preset) = read_preset(src) else { return };
    let from = preset.guess_device();
    let name = std::path::Path::new(src).file_stem().and_then(|s| s.to_str()).unwrap_or("Preset").to_owned();
    let Some((mut json, mut lines)) = need(preset.to_hlx(&cat, from, &name), "converting") else { return };
    if let Some(unit) = rest.get(2).and_then(|u| u.parse::<i64>().ok()).filter(|u| *u != from) {
        match crate::units::convert(&mut json, unit, Some(&cat)) {
            Ok(changes) => lines.extend(changes),
            Err(why) => {
                eprintln!("doesn't fit that unit:");
                for w in why {
                    eprintln!("  - {w}");
                }
                return;
            }
        }
    }
    let Some(text) = need(serde_json::to_string_pretty(&json), "writing") else { return };
    if need(std::fs::write(out, text), out).is_none() {
        return;
    }
    println!("wrote {out} (made from a {} preset)", crate::units::unit_name(from));
    for l in lines {
        println!("  - {l}");
    }
}

/// `--dump-loaded <box address> <file.hxpreset>`: save the loaded preset's edit buffer (what the pedal
/// holds right now, unsaved changes included) to a file. Reads only.
fn dump_loaded_tool(rest: &[String]) {
    let (Some(host), Some(file)) = (rest.first(), rest.get(1)) else {
        eprintln!("usage: --dump-loaded <box address> <file.hxpreset>");
        return;
    };
    let Some(mut s) = open_session(Some(host)) else { return };
    let Some(blob) = need(s.read_current(), "reading the loaded preset") else { return };
    if let Some(dir) = std::path::Path::new(file).parent().filter(|d| !d.as_os_str().is_empty()) {
        let _ = std::fs::create_dir_all(dir);
    }
    match std::fs::write(file, &blob) {
        Ok(()) => println!("saved the loaded preset ({} bytes) to {file}", blob.len()),
        Err(e) => eprintln!("can't write {file}: {e}"),
    }
}

/// `--send-hlx <box address> <slot like 42B> <file.hlx> <backup folder>`: WRITES a `.hlx` preset to a slot,
/// as the app does (the slot's own document is the template), after saving the slot's current document into
/// the folder. Reads the slot back and says whether it came back the same.
fn send_hlx_tool(rest: &[String]) {
    let (Some(host), Some(label), Some(file), Some(folder)) = (rest.first(), rest.get(1), rest.get(2), rest.get(3)) else {
        eprintln!("usage: --send-hlx <box address> <slot like 42B> <file.hlx> <backup folder>");
        return;
    };
    let Some(slot) = crate::cmdtool::slot_index(label) else {
        eprintln!("{label}: not a slot like 42B");
        return;
    };
    let Some(cat) = tool_catalog() else { return };
    let Some(text) = need(std::fs::read_to_string(file), file) else { return };
    let Some(json) = need(serde_json::from_str::<serde_json::Value>(&text), file) else { return };
    let name = json.pointer("/data/meta/name").and_then(serde_json::Value::as_str).unwrap_or("Preset").to_owned();
    let Some(mut s) = open_session(Some(host)) else { return };
    let Some(Some(old)) = need(s.fetch_slot(0, slot as i64), "reading the slot") else {
        eprintln!("slot {label} is empty or can't be read; nothing written");
        return;
    };
    let _ = std::fs::create_dir_all(folder);
    let backup = std::path::Path::new(folder).join(format!("{label} before.hxpreset"));
    if need(std::fs::write(&backup, &old), "saving the backup").is_none() {
        return;
    }
    println!("backed up {label} to {}", backup.display());
    let Some(template) = need(devpreset::DevicePreset::parse(&old), "the slot's document") else { return };
    let Some((doc, notes)) = need(devpreset::DevicePreset::from_hlx(&template, &json, &cat), "converting the .hlx") else { return };
    for n in &notes {
        println!("note: {n}");
    }
    let Some(bytes) = need(doc.encode_checked(), "encoding") else { return };
    let Some(sent) = need(s.write_slot(0, slot as i64, &name, bytes), "writing the slot") else { return };
    let back = need(s.fetch_slot(0, slot as i64), "reading it back").flatten();
    println!("wrote \"{name}\" to {label}; read back {}", if back.as_deref() == Some(sent.as_slice()) { "the same" } else { "DIFFERENT" });
}

/// `--cc-reload <box address>`: load the loaded preset again from flash.
fn cc_reload_tool(rest: &[String]) {
    let Some(host) = rest.first() else {
        eprintln!("usage: --cc-reload <box address>");
        return;
    };
    let Some(mut s) = open_session(Some(host)) else { return };
    let Some((sl, idx, name)) = need(s.preset_info(), "the loaded preset") else { return };
    let other = if idx > 0 { idx - 1 } else { idx + 1 };
    if need(s.select_preset(sl.max(0), other), "loading another preset").is_none()
        || need(s.select_preset(sl.max(0), idx), "loading it again").is_none()
    {
        return;
    }
    println!("reloaded \"{name}\" from flash");
}

/// `--box-proxy <listen address:port> <box address[:port]> [folder for documents]`
fn box_proxy_tool(rest: &[String]) {
    let (Some(listen), Some(target)) = (rest.first(), rest.get(1)) else {
        eprintln!("usage: --box-proxy <listen address:port, e.g. 0.0.0.0:8080> <box address[:port]> [folder for documents]");
        return;
    };
    let target = if target.rsplit_once(':').is_some_and(|(_, p)| p.chars().all(|c| c.is_ascii_digit())) && !target.contains("::") {
        target.clone()
    } else {
        format!("{target}:80")
    };
    let docs = rest.get(2).map(std::path::PathBuf::from);
    let log = docs.as_ref().map(|d| d.join("box-proxy.log"));
    if let Err(e) = crate::boxproxy::run(listen, &target, docs, log) {
        eprintln!("{e}");
    }
}

/// Convert an HX Edit resource folder into `<out>/<version>/catalog.json.gz` and `meta.json`.
/// Refuses if that version's folder already exists.
pub fn export_catalog(res: &std::path::Path, out: &std::path::Path, version: &str) -> Result<std::path::PathBuf, String> {
    if !catalog::version::is_version(version) {
        return Err(format!("\"{version}\" is not a version name (numbers and dots, like 3.82)"));
    }
    let dir = out.join(version);
    if dir.exists() {
        return Err(format!("{} already exists; nothing was changed", dir.display()));
    }
    let cat = catalog::Catalog::load(res)?;
    let pack = cat.to_pack()?;
    let mut sources = Vec::new();
    for e in std::fs::read_dir(res).map_err(|e| format!("{}: {e}", res.display()))?.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let wanted = matches!(name.as_str(), "Helix.sym" | "HelixControls.json" | "HX_ModelCatalog.json") || name.ends_with(".models");
        if wanted {
            if let Ok(m) = e.metadata() {
                sources.push((name, m.len()));
            }
        }
    }
    sources.sort();
    let created = crate::support::stamp(std::time::SystemTime::now()).chars().take(10).collect();
    let meta = catalog::PackMeta { format: catalog::PACK_FORMAT, hx_edit_version: version.to_owned(), created, sources };
    let mut meta_json = serde_json::to_string_pretty(&meta).map_err(|e| e.to_string())?;
    meta_json.push('\n');
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    // `create_dir` (not `_all`) fails if the folder appeared meanwhile: an existing version is never written into.
    std::fs::create_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let written = crate::files::write_atomic(&dir.join("catalog.json.gz"), &pack)
        .map_err(|e| format!("catalog.json.gz: {e}"))
        .and_then(|_| crate::files::write_atomic(&dir.join("meta.json"), meta_json.as_bytes()).map_err(|e| format!("meta.json: {e}")));
    if let Err(e) = written {
        // Leave no half-made version behind (it would block the next try).
        let _ = std::fs::remove_dir_all(&dir);
        return Err(e);
    }
    Ok(dir)
}

/// Make `focus/<version>/` (the Focus mode tones, `focus.json.gz` and `meta.json`) from the folder Helix Stadium Native keeps them in.
/// Refuses to touch a version that exists. `cat` is the effects list the parameter positions come from.
pub fn export_focus(
    src: &std::path::Path,
    out: &std::path::Path,
    version: &str,
    cat: &catalog::Catalog,
) -> Result<std::path::PathBuf, String> {
    use tonestrand_core::focus::{FocusMeta, FocusSet, PACK_FORMAT};
    if !catalog::version::is_version(version) {
        return Err(format!("\"{version}\" is not a version name (numbers and dots, like 1.4.1)"));
    }
    let dir = out.join(version);
    if dir.exists() {
        return Err(format!("{} already exists; nothing was changed", dir.display()));
    }
    let (set, files_read) = FocusSet::resolve_folder(src, cat)?;
    if set.models.is_empty() {
        return Err(format!("{} holds no Focus mode tones the effects list knows", src.display()));
    }
    let pack = set.to_pack()?;
    let catalog_name = match cat.source() {
        catalog::CatalogSource::HxEdit(p) => format!("HX Edit files in {}", p.display()),
        catalog::CatalogSource::Builtin { hx_edit_version } => format!("built-in list from HX Edit {hx_edit_version}"),
    };
    let created = crate::support::stamp(std::time::SystemTime::now()).chars().take(10).collect();
    let r = &set.report;
    let meta = FocusMeta {
        format: PACK_FORMAT,
        stadium_version: version.to_owned(),
        created,
        catalog: catalog_name,
        files_read,
        mapped: r.mapped,
        stadium_only: r.stadium_only,
        not_in_catalog: r.unmapped.len(),
        params_left_out: r.skipped_params.len(),
    };
    let mut meta_json = serde_json::to_string_pretty(&meta).map_err(|e| e.to_string())?;
    meta_json.push('\n');
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    std::fs::create_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let written = crate::files::write_atomic(&dir.join("focus.json.gz"), &pack)
        .map_err(|e| format!("focus.json.gz: {e}"))
        .and_then(|_| crate::files::write_atomic(&dir.join("meta.json"), meta_json.as_bytes()).map_err(|e| format!("meta.json: {e}")));
    if let Err(e) = written {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(e);
    }
    Ok(dir)
}

const HELP: &str = "\
ToneStrand command-line tools. The first group only reads; the second WRITES to the pedal and is for testing.

Read and convert (nothing is written to a pedal):
  --probe [box address]            connect to the first USB pedal (or a paired box), list presets and IRs
  --nam2ir <out folder> <file.nam>...  convert NAM captures to User IR .wav files (writes files on this computer)
  --read-backup <file.hxb|.hls>...  list what an HX Edit backup or setlist holds
  --find-blocks <text> <file.hxb>...  print blocks whose model contains <text>
  --attach-check <file.hxpreset>...  move each split/merge in memory and read it back
  --dsp-report <file.hxpreset>...   DSP estimate per path, block by block
  --ctl-check <file.hxpreset>...    controllers must round-trip unchanged
  --dump-doc <file.hxpreset>        print a pedal preset document's structure
  --to-hlx <file.hxpreset> <out.hlx> [device number]  a pedal preset as a .hlx, optionally made for another unit (writes the .hlx only)
  --commands <file.hxpreset|slot like 42B> [box address]  print a preset's Command Center as the pedal stores it
  --cc-compare <file.hxpreset|slot> <file.hlx> [box address]  line an HX Edit export up with the pedal's Command Center
  --dump-loaded <box address> <file.hxpreset>  save the loaded preset (edit buffer, unsaved changes included) to a file
  --box-proxy <listen address:port> <box address> [folder for documents]  log what another editor sends the pedal through the box
  --hlx-check <template.hxpreset> [file.hlx]  try the .hlx -> pedal conversion offline
  --fmt-check                       how a dual cab's knobs read at their edges
  --list-models                     every effect in the picker, with real-gear names
  --export-catalog <hx edit res folder> <catalogs folder> <version>  convert HX Edit's effects list into catalogs/<version>/ (writes files on this computer)
  --export-focus <focus-view-xy-defaults folder> <focus folder> <version>  convert Helix Stadium Native's Focus mode tones into focus/<version>/ (writes files on this computer)
  --find-boxes                      look for ToneStrand Boxes for 6 seconds
  --screenshot <file.png>           open the window, save a picture of it and quit

Lab tools (WRITE to the pedal; use a spare preset and keep a backup):
  --cc-raw <box address> <slot 0-12> <id> [arg1 .. arg6]  one Command Center slot of the loaded preset (edit buffer only)
  --cc-live <box address> type <slot> <id> [args] | value <snapshot> <slot> <args> | momentary <slot> <0|1>  an edit the way the app makes it (the pedal's own requests; edit buffer only)
  --cc-snap <box address> <snapshot 1-based> <slot 0-12> [arg1 .. arg6]  one command's values in one snapshot (edit buffer only)
  --cc-reload <box address>         reload the loaded preset from flash, dropping edit-buffer changes
  --op-raw <box address> <op> <key=value>...  one raw request to the pedal, printing the reply (lab: to find out how the pedal wants something asked)
  --send-hlx <box address> <slot like 42B> <file.hlx> <backup folder>  a .hlx to a slot, after saving the slot's document
  --put-slot <box address> <slot like 42B> <file.hxpreset> <name>  a saved pedal document to a slot (undo a test)
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
    if let Some(i) = args.iter().position(|a| a == "--export-catalog") {
        let rest = &args[i + 1..];
        let (Some(res), Some(out), Some(version)) = (rest.first(), rest.get(1), rest.get(2)) else {
            eprintln!("usage: --export-catalog <hx edit res folder> <catalogs folder> <hx edit version, e.g. 3.82>");
            std::process::exit(1);
        };
        match export_catalog(std::path::Path::new(res), std::path::Path::new(out), version) {
            Ok(dir) => println!("wrote {}", dir.display()),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        return true;
    }
    if let Some(i) = args.iter().position(|a| a == "--export-focus") {
        let rest = &args[i + 1..];
        let (Some(src), Some(out), Some(version)) = (rest.first(), rest.get(1), rest.get(2)) else {
            eprintln!("usage: --export-focus <focus-view-xy-defaults folder> <focus folder> <Helix Stadium Native version, e.g. 1.4.1>");
            std::process::exit(1);
        };
        let Some(cat) = catalog::load_best(boxes::Settings::load().hx_edit.as_deref()) else {
            eprintln!("no effects list found (HX Edit's files or the built-in list)");
            std::process::exit(1);
        };
        match export_focus(std::path::Path::new(src), std::path::Path::new(out), version, &cat) {
            Ok(dir) => println!("wrote {}", dir.display()),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        return true;
    }
    if let Some(i) = args.iter().position(|a| a == "--box-proxy") {
        box_proxy_tool(&args[i + 1..]);
        return true;
    }
    if let Some(i) = args.iter().position(|a| a == "--cc-raw") {
        cc_raw_tool(&args[i + 1..]);
        return true;
    }
    if let Some(i) = args.iter().position(|a| a == "--send-hlx") {
        send_hlx_tool(&args[i + 1..]);
        return true;
    }
    if let Some(i) = args.iter().position(|a| a == "--cc-snap") {
        cc_snap_tool(&args[i + 1..]);
        return true;
    }
    if let Some(i) = args.iter().position(|a| a == "--put-slot") {
        put_slot_tool(&args[i + 1..]);
        return true;
    }
    if let Some(i) = args.iter().position(|a| a == "--cc-live") {
        cc_live_tool(&args[i + 1..]);
        return true;
    }
    if let Some(i) = args.iter().position(|a| a == "--to-hlx") {
        to_hlx_tool(&args[i + 1..]);
        return true;
    }
    if let Some(i) = args.iter().position(|a| a == "--dump-loaded") {
        dump_loaded_tool(&args[i + 1..]);
        return true;
    }
    if let Some(i) = args.iter().position(|a| a == "--op-raw") {
        op_raw_tool(&args[i + 1..]);
        return true;
    }
    if let Some(i) = args.iter().position(|a| a == "--cc-reload") {
        cc_reload_tool(&args[i + 1..]);
        return true;
    }
    if let Some(i) = args.iter().position(|a| a == "--commands") {
        commands_tool(&args[i + 1..]);
        return true;
    }
    if let Some(i) = args.iter().position(|a| a == "--cc-compare") {
        cc_compare_tool(&args[i + 1..]);
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
        // Look for ToneStrand Boxes for 6 seconds and print what answers.
        let (tx, rx) = std::sync::mpsc::channel();
        let _d = boxes::start_discovery(tx.clone());
        boxes::lookup_local_names(tx);
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
                if let Some(cat) = catalog::load_best(None) {
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

/// `--op-raw <box address> <op> <key=value>...`: LAB tool, sends one request to the pedal as given and prints the reply, to find out
/// how the pedal wants something asked (it changes the edit buffer only if the request does). Values: true, false, a whole number, or a
/// number ending in `f` for a 32-bit float (`0.5f`). Keys are numbers.
fn op_raw_tool(rest: &[String]) {
    let (Some(host), Some(op)) = (rest.first(), rest.get(1)) else {
        eprintln!("usage: --op-raw <box address> <op> <key=value>...   e.g. 30 98=8 29=true 26=0 28=6 119=false");
        return;
    };
    let Some(op) = need(op.parse::<i64>(), "op") else { return };
    let mut pairs = Vec::new();
    for kv in &rest[2..] {
        let Some((k, v)) = kv.split_once('=') else {
            eprintln!("{kv}: write key=value");
            return;
        };
        let Some(k) = need(k.parse::<i64>(), k) else { return };
        let value = match v {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            _ if v.ends_with('f') => match v.trim_end_matches('f').parse::<f32>() {
                Ok(f) => Value::F32(f),
                Err(_) => {
                    eprintln!("{v}: not a number");
                    return;
                }
            },
            _ => match v.parse::<i64>() {
                Ok(n) => Value::int(n),
                Err(_) => {
                    eprintln!("{v}: not a number");
                    return;
                }
            },
        };
        pairs.push((k, value));
    }
    let Some(mut s) = open_session(Some(host)) else { return };
    match s.request(op, Value::map(pairs)) {
        Ok((status, result)) => println!("status {status}, result {}", result.summary()),
        Err(e) => println!("error: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{read_pack_meta, Catalog, PACK_FORMAT};

    #[test]
    fn export_focus_writes_pack_and_meta_and_refuses_overwrite() {
        use tonestrand_core::focus::{FocusMeta, FocusSet};
        let res = tonestrand_core::catalog::test_support::mini_hx_edit_dir();
        let cat = Catalog::load(&res).unwrap();
        let src = res.join("focus-view-xy-defaults");
        std::fs::create_dir_all(src.join("amp")).unwrap();
        std::fs::write(
            src.join("amp").join("TS_TestAmp.json"),
            r#"{"modelName": "Test Amp", "params": {"TS_Drive": {"id": 1, "behavior": "interpolate"}}, "regions": {"origin": {"name": "Home", "TS_Drive": 0.5}, "x_min_y_min": {"name": "A", "TS_Drive": 0.1}, "x_min_y_max": {"name": "B", "TS_Drive": 0.2}, "x_max_y_min": {"name": "C", "TS_Drive": 0.3}, "x_max_y_max": {"name": "D", "TS_Drive": 0.4}}}"#,
        )
        .unwrap();
        let out = std::env::temp_dir().join(format!("tonestrand-export-focus-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        let dir = export_focus(&src, &out, "9.9.9", &cat).unwrap();
        let meta: FocusMeta = serde_json::from_str(&std::fs::read_to_string(dir.join("meta.json")).unwrap()).unwrap();
        assert_eq!((meta.stadium_version.as_str(), meta.files_read, meta.mapped), ("9.9.9", 1, 1));
        assert!(FocusSet::from_pack(&std::fs::read(dir.join("focus.json.gz")).unwrap()).unwrap().model_for("TS_TestAmp").is_some());
        assert!(export_focus(&src, &out, "9.9.9", &cat).is_err(), "an existing version is not touched");
        assert!(export_focus(&src, &out, "v9", &cat).is_err());
        assert!(export_focus(&out.join("missing"), &out, "9.9.8", &cat).is_err());
        assert!(!out.join("9.9.8").exists());
        let _ = std::fs::remove_dir_all(&out);
    }

    #[test]
    fn export_catalog_writes_pack_and_meta_and_refuses_overwrite() {
        let res = tonestrand_core::catalog::test_support::mini_hx_edit_dir();
        let out = std::env::temp_dir().join(format!("tonestrand-export-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        let dir = export_catalog(&res, &out, "9.99").unwrap();
        let meta = read_pack_meta(&std::fs::read_to_string(dir.join("meta.json")).unwrap()).unwrap();
        assert_eq!(meta.hx_edit_version, "9.99");
        assert_eq!(meta.format, PACK_FORMAT);
        assert!(meta.sources.iter().any(|(n, _)| n == "Helix.sym"));
        assert!(Catalog::from_pack(&std::fs::read(dir.join("catalog.json.gz")).unwrap()).is_ok());
        assert!(std::fs::read_to_string(dir.join("meta.json")).unwrap().ends_with('\n'));
        assert!(export_catalog(&res, &out, "9.99").is_err());
        for bad in ["../x", "3*8", "3.8 ", "v3"] {
            assert!(export_catalog(&res, &out, bad).is_err(), "{bad:?}");
        }
        // A failed export (no HX Edit files) leaves no folder behind.
        assert!(export_catalog(&out.join("missing"), &out, "9.98").is_err());
        assert!(!out.join("9.98").exists());
        let _ = std::fs::remove_dir_all(&out);
    }
}
