//! A careful self-test of every pedal operation the app uses, for the hidden
//! Tests page. It backs up first, works only on one preset slot and one IR slot
//! the person picks, checks each step by reading back, and always puts both
//! slots (and the loaded preset) back the way they were, checking that too.

use std::path::PathBuf;

use crate::device::{sanitize, Event};
use crate::devpreset::DevicePreset;
use crate::proto::slot_label;
use crate::session::{Session, IR_AUDIO_BYTES};

#[derive(Clone, Debug)]
pub struct Plan {
    pub setlist: i64,
    /// The preset slot to test writes on (never the loaded one).
    pub preset_slot: i64,
    /// The User IR slot to test writes on (from 0).
    pub ir_slot: i64,
    pub folder: PathBuf,
    /// Write, rename and restore the preset slot.
    pub preset_writes: bool,
    /// Change one knob of the loaded preset to the value it already has, and reload it.
    pub live_edit: bool,
    /// Write, rename and restore the IR slot.
    pub ir_writes: bool,
}

const TEST_NAME: &str = "RS SELFTEST";
const TEST_NAME_2: &str = "RS SELFTEST 2";
const TEST_IR_NAME: &str = "RS SELFTEST IR";
const TEST_IR_NAME_2: &str = "RS SELFTEST IR 2";

struct Run<'a> {
    send: &'a dyn Fn(Event),
    failures: usize,
    /// The link to the pedal dropped: nothing more is sent (more writes can leave the pedal stuck).
    lost: bool,
}

impl Run<'_> {
    fn step(&mut self, name: &str, r: Result<String, String>) -> bool {
        match r {
            Ok(detail) => {
                (self.send)(Event::TestStep { name: name.into(), ok: Some(true), detail });
                true
            }
            Err(detail) => {
                self.failures += 1;
                if link_lost(&detail) {
                    self.lost = true;
                }
                (self.send)(Event::TestStep { name: name.into(), ok: Some(false), detail });
                false
            }
        }
    }
    fn info(&self, name: &str, detail: impl Into<String>) {
        (self.send)(Event::TestStep { name: name.into(), ok: None, detail: detail.into() });
    }
    fn busy(&self, what: &str) {
        (self.send)(Event::Busy(format!("Self-test: {what}...")));
    }
}

/// Whether an error means the connection itself is gone (not just a refused step).
fn link_lost(e: &str) -> bool {
    let e = e.to_lowercase();
    ["lost usb", "lost the box", "usb contact", "closed the link", "forcibly closed", "half way", "power-cycle", "disconnected", "no reply"].iter().any(|w| e.contains(w))
}

fn canon(b: &[u8]) -> Result<Vec<u8>, String> {
    DevicePreset::parse(b).map(|d| d.encode())
}

/// A made-up IR to write: an impulse with a short decaying tail (deterministic).
fn test_ir() -> Vec<u8> {
    let mut seed: u32 = 0x1234_5678;
    let mut out = Vec::with_capacity(IR_AUDIO_BYTES);
    for i in 0..IR_AUDIO_BYTES / 4 {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let noise = (seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0;
        let v = if i == 0 { 0.9 } else if i == 2047 { 0.0 } else { noise * 0.3 * (-(i as f32) / 180.0).exp() };
        out.extend(v.to_le_bytes());
    }
    out
}

fn same_names(a: &[String], b: &[String]) -> Result<String, String> {
    let diffs: Vec<String> = a
        .iter()
        .zip(b.iter())
        .enumerate()
        .filter(|(_, (x, y))| x != y)
        .map(|(i, (x, y))| format!("{}: \"{x}\" -> \"{y}\"", slot_label(i)))
        .collect();
    if a.len() != b.len() {
        return Err(format!("the list had {} names, now {}", a.len(), b.len()));
    }
    if diffs.is_empty() {
        Ok(format!("all {} names as before", a.len()))
    } else {
        Err(diffs.join("; "))
    }
}

pub fn run(s: &mut Session, plan: Plan, send: &dyn Fn(Event)) {
    let mut r = Run { send, failures: 0, lost: false };
    let sl = plan.setlist;
    let ps = plan.preset_slot;
    let label = slot_label(ps.max(0) as usize);
    r.info("Plan", format!(
        "Setlist {}, preset slot {label}, IR slot {}. Writes: presets {}, live edit {}, IRs {}. Backups in {}.",
        sl + 1, plan.ir_slot + 1, yes(plan.preset_writes), yes(plan.live_edit), yes(plan.ir_writes), plan.folder.display()
    ));

    // ---------------------------------------------------------------- backup ---
    r.busy("backing up");
    if let Err(e) = std::fs::create_dir_all(&plan.folder) {
        r.step("Backup folder", Err(format!("can't make it: {e}")));
        return finish(&mut r);
    }
    let start = match s.preset_info() {
        Ok(v) => v,
        Err(e) => {
            r.step("Read the loaded preset", Err(e.to_string()));
            return finish(&mut r);
        }
    };
    r.step("Read the loaded preset", Ok(format!("{} \"{}\" (setlist {})", slot_label(start.1.max(0) as usize), start.2, start.0 + 1)));
    if start.0 == sl && start.1 == ps && (plan.preset_writes) {
        r.step("Safety", Err(format!("{label} is the loaded preset; pick another slot so the pedal's sound isn't touched")));
        return finish(&mut r);
    }
    let names = match s.presets(sl) {
        Ok(n) => n,
        Err(e) => {
            r.step("Read the preset list", Err(e.to_string()));
            return finish(&mut r);
        }
    };
    r.step("Read the preset list", Ok(format!("{} slots", names.len())));
    // Back up the whole setlist, so anything can be put back by hand too.
    let mut saved = 0;
    let mut backup_ok = true;
    for (i, n) in names.iter().enumerate() {
        r.busy(&format!("backing up {}/{}", i + 1, names.len()));
        match s.fetch_slot(sl, i as i64) {
            Ok(Some(blob)) => {
                let file = plan.folder.join(format!("{} {}.hxpreset", slot_label(i), sanitize(n)));
                if crate::files::write_atomic(&file, &blob).is_ok() {
                    saved += 1;
                } else {
                    backup_ok = false;
                }
            }
            Ok(None) => {}
            Err(e) => {
                backup_ok = false;
                r.info("Backup", format!("{} could not be read: {e}", slot_label(i)));
            }
        }
    }
    if !r.step("Back up the setlist", if backup_ok { Ok(format!("{saved} presets saved")) } else { Err(format!("only {saved} saved; stopping before any write")) }) {
        return finish(&mut r);
    }
    // The test slot itself, read twice: reads must be stable.
    let original = match (s.fetch_slot(sl, ps), s.fetch_slot(sl, ps)) {
        (Ok(a), Ok(b)) => {
            r.step("Read the test slot twice", if a == b { Ok(match &a { Some(x) => format!("{} bytes, both reads the same", x.len()), None => "empty, both reads".into() }) } else { Err("the two reads differ".into()) });
            a
        }
        (Err(e), _) | (_, Err(e)) => {
            r.step("Read the test slot", Err(e.to_string()));
            return finish(&mut r);
        }
    };
    let original_name = names.get(ps as usize).cloned().unwrap_or_default();
    let mut plan = plan;
    if let Some(b) = &original {
        let ok = r.step("Test slot document parses", canon(b).map(|_| "it reads as a pedal preset, so it can be written back".into()));
        if !ok && plan.preset_writes {
            plan.preset_writes = false;
            r.info("Preset writes", "skipped: the slot's preset couldn't be written back exactly; pick another slot");
        }
    }
    // IRs.
    let irs_before = match s.list_irs() {
        Ok(l) => {
            let _ = std::fs::write(plan.folder.join("ir-list.txt"), l.iter().map(|(s, n)| format!("{:03} {n}\n", s + 1)).collect::<String>());
            r.step("Read the IR list", Ok(format!("{} slots in use", l.len())));
            Some(l)
        }
        Err(e) => {
            r.step("Read the IR list", Err(e.to_string()));
            None
        }
    };
    let ir_original: Option<(String, Vec<u8>)> = match irs_before.as_ref().and_then(|l| l.iter().find(|(s, _)| *s == plan.ir_slot).cloned()) {
        Some((_, name)) => match (s.ir_audio(plan.ir_slot), s.ir_audio(plan.ir_slot)) {
            (Ok(a), Ok(b)) => {
                let _ = std::fs::write(plan.folder.join(crate::irs::file_name(plan.ir_slot, &name)), crate::irs::audio_to_wav(&a));
                r.step("Back up the test IR (read twice)", if a == b { Ok(format!("\"{name}\" saved, both reads the same")) } else { Err("the two reads differ".into()) });
                Some((name, a))
            }
            (Err(e), _) | (_, Err(e)) => {
                r.step("Back up the test IR", Err(e.to_string()));
                if plan.ir_writes {
                    r.info("IR tests", "skipped: its IR couldn't be backed up");
                }
                None
            }
        },
        None => {
            r.info("Test IR slot", "empty");
            None
        }
    };
    // Read another IR, to test downloads even when the test slot is empty.
    if let Some((slot, name)) = irs_before.as_ref().and_then(|l| l.iter().find(|(s, _)| *s != plan.ir_slot).cloned()) {
        r.busy("downloading an IR");
        r.step("Download another IR", s.ir_audio(slot).map(|a| format!("slot {} \"{name}\": {} bytes{}", slot + 1, a.len(), if crate::irs::is_nam_capture(&a) { ", a NAM capture" } else { "" })).map_err(|e| e.to_string()));
    }

    // ------------------------------------------------------- live edit (no-op) ---
    if plan.live_edit {
        r.busy("live edit");
        let res = (|| -> Result<String, String> {
            let before = s.read_current().map_err(|e| e.to_string())?;
            let doc = DevicePreset::parse(&before)?;
            let slot = doc.slots().into_iter().find(|x| x.model.is_some() && !x.values.is_empty()).ok_or("the loaded preset has no block with knobs")?;
            s.set_param(slot.wire_block(), 0, 0, slot.values[0].clone()).map_err(|e| e.to_string())?;
            let after = s.read_current().map_err(|e| e.to_string())?;
            let same = canon(&after)? == canon(&before)?;
            if same {
                Ok(format!("knob 1 of block {} set to its own value; the preset reads back unchanged", slot.wire_block()))
            } else {
                Err("the preset reads back different after setting a knob to its own value".into())
            }
        })();
        r.step("Live edit (no change)", res);
        // Put the edit buffer back exactly as stored.
        r.busy("reloading the loaded preset");
        r.step("Reload the loaded preset", s.select_preset(start.0, start.1).map(|_| "edit buffer discarded".into()).map_err(|e| e.to_string()));
        // Let the pedal finish loading before anything is written.
        let _ = s.idle(std::time::Duration::from_millis(1500));
    }
    if r.lost {
        return stop_lost(&mut r, &plan);
    }

    // ---------------------------------------------------------- preset writes ---
    if plan.preset_writes {
        let mut touched = false;
        let res = (|| -> Result<(), String> {
            r.busy("writing the test preset");
            // The test document: the loaded preset, under a test name.
            let doc = s.read_current().map_err(|e| e.to_string())?;
            let expect = canon(&doc)?;
            touched = true;
            let ok = r.step("Write a test preset", s.write_slot(sl, ps, TEST_NAME, doc.clone()).map(|_| format!("the loaded preset, as \"{TEST_NAME}\", into {label}")).map_err(|e| e.to_string()));
            if !ok {
                return Err("write failed".into());
            }
            let back = s.fetch_slot(sl, ps).map_err(|e| e.to_string())?.ok_or("the slot reads back empty")?;
            if !r.step("Verify it reads back the same", if canon(&back)? == expect { Ok(format!("{} bytes match", back.len())) } else { Err("the document read back differs from what was written".into()) }) {
                return Err("verify failed".into());
            }
            let n = s.presets(sl).map_err(|e| e.to_string())?;
            r.step("Verify its name", if n.get(ps as usize).map(String::as_str) == Some(TEST_NAME) { Ok(format!("\"{TEST_NAME}\"")) } else { Err(format!("the list shows \"{}\"", n.get(ps as usize).cloned().unwrap_or_default())) });
            r.busy("renaming");
            r.step("Rename", s.rename_slot(sl, ps, TEST_NAME_2).map(|_| String::new()).map_err(|e| e.to_string()));
            let n = s.presets(sl).map_err(|e| e.to_string())?;
            r.step("Verify the new name", if n.get(ps as usize).map(String::as_str) == Some(TEST_NAME_2) { Ok(format!("\"{TEST_NAME_2}\"")) } else { Err(format!("the list shows \"{}\"", n.get(ps as usize).cloned().unwrap_or_default())) });
            Ok(())
        })();
        if r.lost {
            return stop_lost(&mut r, &plan);
        }
        if let Err(e) = res {
            r.info("Preset tests", format!("stopped ({e}); restoring now"));
        }
        if touched {
            restore_preset(s, &mut r, &plan, &original, &original_name);
        }
        if r.lost {
            return stop_lost(&mut r, &plan);
        }
    }

    // -------------------------------------------------------------- IR writes ---
    let ir_backup_ok = irs_before.is_some() && (ir_original.is_some() || !irs_before.as_ref().is_some_and(|l| l.iter().any(|(x, _)| *x == plan.ir_slot)));
    if plan.ir_writes && ir_backup_ok {
        let mut touched = false;
        let res = (|| -> Result<(), String> {
            let audio = test_ir();
            r.busy("writing the test IR");
            touched = true;
            if !r.step("Upload a test IR", s.import_ir(plan.ir_slot, TEST_IR_NAME, &audio).map(|_| format!("into IR slot {}", plan.ir_slot + 1)).map_err(|e| e.to_string())) {
                return Err("upload failed".into());
            }
            let l = s.list_irs().map_err(|e| e.to_string())?;
            r.step("Verify the IR's name", match l.iter().find(|(x, _)| *x == plan.ir_slot) { Some((_, n)) if n == TEST_IR_NAME => Ok(format!("\"{n}\"")), Some((_, n)) => Err(format!("the list shows \"{n}\"")), None => Err("the slot shows empty".into()) });
            let back = s.ir_audio(plan.ir_slot).map_err(|e| e.to_string())?;
            if !r.step("Verify the IR downloads the same", if back == audio { Ok("8,192 bytes match".into()) } else { Err(format!("{} of 2,048 samples differ", back.chunks(4).zip(audio.chunks(4)).filter(|(a, b)| a != b).count())) }) {
                return Err("verify failed".into());
            }
            r.busy("renaming the IR");
            r.step("Rename the IR", s.rename_ir(plan.ir_slot, TEST_IR_NAME_2).map(|_| String::new()).map_err(|e| e.to_string()));
            let l = s.list_irs().map_err(|e| e.to_string())?;
            r.step("Verify the IR's new name", match l.iter().find(|(x, _)| *x == plan.ir_slot) { Some((_, n)) if n == TEST_IR_NAME_2 => Ok(format!("\"{n}\"")), Some((_, n)) => Err(format!("the list shows \"{n}\"")), None => Err("the slot shows empty".into()) });
            Ok(())
        })();
        if r.lost {
            return stop_lost(&mut r, &plan);
        }
        if let Err(e) = res {
            r.info("IR tests", format!("stopped ({e}); restoring now"));
        }
        if touched {
            restore_ir(s, &mut r, &plan, &ir_original, irs_before.as_deref().unwrap_or(&[]));
        }
        if r.lost {
            return stop_lost(&mut r, &plan);
        }
    } else if plan.ir_writes {
        r.info("IR tests", "skipped: the IR slot couldn't be backed up first");
    }

    // ------------------------------------------------------------ final state ---
    r.busy("checking the final state");
    match s.presets(sl) {
        Ok(after) => {
            r.step("Final: preset names", same_names(&names, &after));
        }
        Err(e) => {
            r.step("Final: preset names", Err(e.to_string()));
        }
    }
    match s.preset_info() {
        Ok(now) if now.0 == start.0 && now.1 == start.1 => {
            r.step("Final: loaded preset", Ok(format!("{} as at the start", slot_label(now.1.max(0) as usize))));
        }
        Ok(_) => {
            let back = s.select_preset(start.0, start.1);
            r.step("Final: loaded preset", back.map(|_| "it had changed; the starting preset is loaded again".into()).map_err(|e| e.to_string()));
        }
        Err(e) => {
            r.step("Final: loaded preset", Err(e.to_string()));
        }
    }
    finish(&mut r);
}

/// The connection dropped part way: send nothing more (more transfers into a pedal that's
/// mid-transfer is what leaves it stuck), and say how to recover.
fn stop_lost(r: &mut Run, plan: &Plan) {
    r.info(
        "Stopped",
        format!(
            "the connection to the pedal dropped, so nothing more was sent. If the pedal shows it is transferring, \
             switch it off and on, connect again, and check the test slots: the backups are in {}.",
            plan.folder.display()
        ),
    );
    finish(r);
}

fn restore_preset(s: &mut Session, r: &mut Run, plan: &Plan, original: &Option<Vec<u8>>, name: &str) {
    let (sl, ps) = (plan.setlist, plan.preset_slot);
    r.busy("restoring the test slot");
    match original {
        Some(doc) => {
            let ok = r.step("Restore the test slot", s.write_slot(sl, ps, name, doc.clone()).map(|_| format!("\"{name}\" written back")).map_err(|e| e.to_string()));
            if ok {
                let res = s.fetch_slot(sl, ps).map_err(|e| e.to_string()).and_then(|b| {
                    let b = b.ok_or("the slot reads back empty")?;
                    if canon(&b)? == canon(doc)? {
                        Ok("the document matches the backup".to_owned())
                    } else {
                        Err("the document differs from the backup; restore it by hand from the backup folder".to_owned())
                    }
                });
                r.step("Verify the restore", res);
            }
        }
        None => {
            let ok = r.step("Restore the test slot", s.clear_slot(sl, ps).map(|_| "emptied again (it was empty)".into()).map_err(|e| e.to_string()));
            if ok {
                r.step("Verify the restore", match s.fetch_slot(sl, ps) {
                    Ok(None) => Ok("it reads back empty".into()),
                    Ok(Some(_)) => Err("it still holds a preset".into()),
                    Err(e) => Err(e.to_string()),
                });
            }
        }
    }
}

fn restore_ir(s: &mut Session, r: &mut Run, plan: &Plan, original: &Option<(String, Vec<u8>)>, before: &[(i64, String)]) {
    r.busy("restoring the IR slot");
    match original {
        Some((name, audio)) => {
            if r.step("Restore the IR slot", s.import_ir(plan.ir_slot, name, audio).map(|_| format!("\"{name}\" written back")).map_err(|e| e.to_string())) {
                r.step("Verify the IR restore", match s.ir_audio(plan.ir_slot) {
                    Ok(b) if &b == audio => Ok("the audio matches the backup".into()),
                    Ok(_) => Err("the audio differs from the backup; restore it by hand from the backup folder".into()),
                    Err(e) => Err(e.to_string()),
                });
            }
        }
        None => {
            if r.step("Restore the IR slot", s.delete_ir(plan.ir_slot).map(|_| "emptied again (it was empty)".into()).map_err(|e| e.to_string())) {
                // checked with the whole list below
            }
        }
    }
    match s.list_irs() {
        Ok(after) => {
            let res = if after == before { Ok(format!("{} slots, names as before", after.len())) } else { Err(format!("the IR list differs: {} slots before, {} now", before.len(), after.len())) };
            r.step("Final: IR list", res);
        }
        Err(e) => {
            r.step("Final: IR list", Err(e.to_string()));
        }
    }
}

fn finish(r: &mut Run) {
    (r.send)(Event::TestDone { failures: r.failures });
}

fn yes(b: bool) -> &'static str {
    if b {
        "yes"
    } else {
        "no"
    }
}
