//! Presets out of HX Edit's files: a device backup (.hxb) and a setlist export (.hls).
//! Layouts of HX Edit's backup (.hxb) and setlist (.hls) files, as found in real files.
//!
//! .hxb: "AF6L", u32 version 1, u64 index offset, u64 entry count, u32 device...;
//! each 36-byte index entry: a 4-character tag stored backwards, u64 offset,
//! u64 stored size, u32 compressed flag, u64 size. Tags: "SLnn" setlists (zlib JSON
//! `{data: {meta: {name}, presets: [...]}}`), "SLNM" setlist names, "Innn" IRs.
//! .hls: JSON `{schema: "L6Setlist", encoded_data: base64 zlib, compression: {type, decompressed_size, crc32}, meta: {name}}`.

use std::io::Read;

use serde_json::Value;

const CEILING: u64 = 64 * 1024 * 1024;

/// One preset taken out: its slot in the setlist, name and .hlx document.
pub struct Preset {
    pub slot: usize,
    pub name: String,
    pub hlx: Value,
}

pub struct Setlist {
    pub name: String,
    pub presets: Vec<Preset>,
    pub skipped: usize,
}

/// One User IR from a backup: its slot (from 0), name and the .wav file as stored.
pub struct Ir {
    pub slot: usize,
    pub name: String,
    pub wav: Vec<u8>,
}

pub struct Contents {
    pub setlists: Vec<Setlist>,
    /// The User IRs (each entry "Innn" is a whole .wav, named in its INFO/INAM chunk).
    pub irs: Vec<Ir>,
}

/// The name a .wav carries in its LIST/INFO/INAM chunk.
fn wav_name(w: &[u8]) -> Option<String> {
    let at = w.windows(4).position(|x| x == b"INAM")?;
    let len = u32::from_le_bytes(w.get(at + 4..at + 8)?.try_into().ok()?) as usize;
    let s = w.get(at + 8..at + 8 + len)?;
    Some(String::from_utf8_lossy(s).trim_end_matches('\0').trim().to_owned())
}

fn unzlib(b: &[u8], expect: Option<u64>) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(b).take(CEILING + 1).read_to_end(&mut out).map_err(|e| format!("couldn't unpack ({e})"))?;
    if out.len() as u64 > CEILING {
        return Err("it unpacks to more than 64 MB".into());
    }
    if let Some(n) = expect.filter(|n| *n as usize != out.len()) {
        return Err(format!("it unpacked to {} bytes but says {n}: the file is damaged", out.len()));
    }
    Ok(out)
}

/// A setlist's presets as .hlx documents (the backup keeps each without the outer wrapper).
fn setlist_from(v: &Value, fallback: &str) -> Result<Setlist, String> {
    let data = v.get("data").filter(|d| d.get("presets").is_some()).unwrap_or(v);
    let presets = data.get("presets").and_then(Value::as_array).ok_or("it has no list of presets")?;
    let name = data.pointer("/meta/name").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or(fallback).to_owned();
    let mut out = Vec::new();
    let mut skipped = 0;
    for (slot, e) in presets.iter().enumerate() {
        let Some(obj) = e.as_object() else {
            skipped += 1;
            continue;
        };
        if obj.is_empty() {
            continue; // an empty slot
        }
        let (Some(tone), Some(pname)) = (obj.get("tone"), e.pointer("/meta/name").and_then(Value::as_str)) else {
            skipped += 1;
            continue;
        };
        // A backup entry becomes `data`, wrapped as an L6Preset.
        let mut data = obj.clone();
        data.remove("schema");
        data.insert("tone".into(), tone.clone());
        data.insert("schema".into(), Value::Null);
        let hlx = serde_json::json!({ "data": Value::Object(data), "meta": Value::Null, "schema": "L6Preset", "version": 6 });
        out.push(Preset { slot, name: pname.to_owned(), hlx });
    }
    Ok(Setlist { name, presets: out, skipped })
}

fn u32le(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}
fn u64le(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap_or([0; 8]))
}

/// Read a device backup (.hxb).
pub fn read_hxb(b: &[u8]) -> Result<Contents, String> {
    if b.len() < 48 {
        return Err("it's too short to be a Helix backup".into());
    }
    if &b[..4] != b"AF6L" {
        return Err("it isn't a Helix backup (no AF6L mark)".into());
    }
    let version = u32le(b, 4);
    if version != 1 {
        return Err(format!("it's backup format {version}; only format 1 is known"));
    }
    let index = u64le(b, 8) as usize;
    let count = u64le(b, 16) as usize;
    if !(1..=4096).contains(&count) || index < 48 || index + 36 * count > b.len() {
        return Err("its index is damaged or cut short".into());
    }
    struct Entry {
        tag: String,
        offset: usize,
        stored: usize,
        compressed: bool,
        size: u64,
    }
    let mut entries = Vec::new();
    for i in 0..count {
        let o = index + 36 * i;
        let tag: String = (0..4).rev().map(|k| b[o + k] as char).collect();
        if !tag.chars().all(|c| (' '..='~').contains(&c)) {
            return Err(format!("index entry {i} is unreadable"));
        }
        if i == 0 {
            continue; // the index's own header
        }
        let (offset, stored, compressed, size) = (u64le(b, o + 4) as usize, u64le(b, o + 12) as usize, u32le(b, o + 20) != 0, u64le(b, o + 24));
        if offset < 48 || offset + stored > b.len() || size > CEILING {
            return Err(format!("entry {tag} points outside the file"));
        }
        entries.push(Entry { tag, offset, stored, compressed, size });
    }
    let chunk = |e: &Entry| -> Result<Vec<u8>, String> {
        let raw = &b[e.offset..e.offset + e.stored];
        if e.compressed {
            unzlib(raw, Some(e.size)).map_err(|err| format!("{}: {err}", e.tag))
        } else {
            Ok(raw.to_vec())
        }
    };
    let names: Vec<String> = match entries.iter().find(|e| e.tag == "SLNM") {
        Some(e) => String::from_utf8_lossy(&chunk(e)?).split('\0').filter(|s| !s.is_empty()).map(str::to_owned).collect(),
        None => Vec::new(),
    };
    let num = |tag: &str, prefix: &str, digits: usize| -> Option<usize> {
        tag.strip_prefix(prefix).filter(|r| r.len() == digits && r.chars().all(|c| c.is_ascii_hexdigit())).and_then(|r| usize::from_str_radix(r, 16).ok())
    };
    let mut sl: Vec<(usize, &Entry)> = entries.iter().filter_map(|e| num(&e.tag, "SL", 2).map(|n| (n, e))).collect();
    sl.sort_by_key(|x| x.0);
    let mut setlists = Vec::new();
    for (n, e) in sl {
        let json: Value = serde_json::from_slice(&chunk(e)?).map_err(|err| format!("{}: not readable ({err})", e.tag))?;
        let fallback = names.get(n).cloned().unwrap_or_else(|| format!("Setlist {}", n + 1));
        let mut s = setlist_from(&json, &fallback).map_err(|err| format!("{}: {err}", e.tag))?;
        if let Some(name) = names.get(n) {
            s.name = name.clone();
        }
        setlists.push(s);
    }
    let mut irs = Vec::new();
    for e in &entries {
        let Some(slot) = num(&e.tag, "I", 3) else { continue };
        let wav = chunk(e)?;
        if wav.get(..4) != Some(b"RIFF") {
            continue;
        }
        let name = wav_name(&wav).filter(|n| !n.is_empty()).unwrap_or_else(|| format!("IR {}", slot + 1));
        irs.push(Ir { slot, name, wav });
    }
    irs.sort_by_key(|i| i.slot);
    Ok(Contents { setlists, irs })
}

fn base64(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' | b'\n' | b'\r' | b' ' => continue,
            _ => return None,
        } as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// Read a setlist export (.hls).
pub fn read_hls(b: &[u8]) -> Result<Setlist, String> {
    let v: Value = serde_json::from_slice(b).map_err(|e| format!("it isn't readable ({e})"))?;
    if v.get("encryption").is_some() {
        return Err("it's an encrypted (premium) Line 6 setlist; only HX Edit can open those".into());
    }
    if v.get("schema").and_then(Value::as_str) != Some("L6Setlist") {
        return Err("it isn't a Helix setlist file".into());
    }
    let comp = v.get("compression");
    if comp.and_then(|c| c.get("type")).and_then(Value::as_str) != Some("zlib") {
        return Err("its contents aren't packed the way this app reads".into());
    }
    let data = v.get("encoded_data").and_then(Value::as_str).and_then(base64).ok_or("its contents aren't readable")?;
    let size = comp.and_then(|c| c.get("decompressed_size")).and_then(Value::as_u64);
    let raw = unzlib(&data, size)?;
    let json: Value = serde_json::from_slice(&raw).map_err(|e| format!("its contents aren't readable ({e})"))?;
    let name = v.pointer("/meta/name").and_then(Value::as_str).unwrap_or("Setlist");
    setlist_from(&json, name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zlib(b: &[u8]) -> Vec<u8> {
        let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(b).unwrap();
        e.finish().unwrap()
    }

    fn setlist_json() -> Vec<u8> {
        serde_json::json!({"data": {"meta": {"name": "GIG"}, "presets": [
            {"meta": {"name": "Clean"}, "device": 2162694, "tone": {"dsp0": {}}},
            {},
            {"meta": {"name": "Lead"}, "device": 2162694, "tone": {"dsp0": {}}}
        ]}})
        .to_string()
        .into_bytes()
    }

    #[test]
    fn reads_a_backup() {
        let sl = zlib(&setlist_json());
        let names = b"GIG\0".to_vec();
        let mut f = vec![0u8; 48];
        f[..4].copy_from_slice(b"AF6L");
        f[4..8].copy_from_slice(&1u32.to_le_bytes());
        let sl_at = f.len();
        f.extend(&sl);
        let nm_at = f.len();
        f.extend(&names);
        let index = f.len();
        let entry = |tag: &str, off: usize, stored: usize, comp: bool, size: usize| {
            let mut e = vec![0u8; 36];
            for (k, c) in tag.bytes().rev().enumerate() {
                e[k] = c;
            }
            e[4..12].copy_from_slice(&(off as u64).to_le_bytes());
            e[12..20].copy_from_slice(&(stored as u64).to_le_bytes());
            e[20..24].copy_from_slice(&(comp as u32).to_le_bytes());
            e[24..32].copy_from_slice(&(size as u64).to_le_bytes());
            e
        };
        f.extend(entry("IDXH", 0, 0, false, 0));
        f.extend(entry("SL00", sl_at, sl.len(), true, setlist_json().len()));
        f.extend(entry("SLNM", nm_at, names.len(), false, names.len()));
        f[8..16].copy_from_slice(&(index as u64).to_le_bytes());
        f[16..24].copy_from_slice(&3u64.to_le_bytes());
        let c = read_hxb(&f).unwrap();
        assert_eq!(c.setlists.len(), 1);
        assert_eq!(c.setlists[0].name, "GIG");
        let p: Vec<(usize, &str)> = c.setlists[0].presets.iter().map(|p| (p.slot, p.name.as_str())).collect();
        assert_eq!(p, vec![(0, "Clean"), (2, "Lead")]);
        assert_eq!(c.setlists[0].presets[0].hlx.pointer("/data/device").and_then(Value::as_i64), Some(2162694));
        assert_eq!(c.setlists[0].presets[0].hlx.get("schema").and_then(Value::as_str), Some("L6Preset"));
    }

    #[test]
    fn reads_a_setlist_file() {
        let raw = setlist_json();
        let enc: String = {
            const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            let z = zlib(&raw);
            let mut s = String::new();
            for ch in z.chunks(3) {
                let n = (ch[0] as u32) << 16 | (*ch.get(1).unwrap_or(&0) as u32) << 8 | *ch.get(2).unwrap_or(&0) as u32;
                for k in 0..4 {
                    if k <= ch.len() {
                        s.push(T[((n >> (18 - 6 * k)) & 63) as usize] as char);
                    } else {
                        s.push('=');
                    }
                }
            }
            s
        };
        let file = serde_json::json!({"schema": "L6Setlist", "encoded_data": enc, "compression": {"type": "zlib", "decompressed_size": raw.len()}, "meta": {"name": "GIG"}});
        let s = read_hls(file.to_string().as_bytes()).unwrap();
        assert_eq!(s.presets.len(), 2);
        assert_eq!(s.presets[1].name, "Lead");
    }
}
