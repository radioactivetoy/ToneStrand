//! User IRs as files: read a .wav into the pedal's IR format (2,048 samples of
//! 32-bit float at 48 kHz, 8,192 bytes) and write one back out as a .wav.
//! A .nam capture is turned into the NAM A2 Lite firmware's IR on the way.

use std::path::Path;

pub const SAMPLES: usize = 2048;
pub const RATE: u32 = 48_000;

/// What reading a file gave: the IR bytes, a name for the slot, and notes for the person.
pub struct Loaded {
    pub audio: Vec<u8>,
    pub name: String,
    pub notes: Vec<String>,
}

fn tag(b: &[u8], at: usize) -> &[u8] {
    b.get(at..at + 4).unwrap_or(&[])
}
fn u16le(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}
fn u32le(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// A name the pedal can show: printable ASCII, at most 26 characters.
pub fn clean_name(s: &str) -> String {
    let t: String = s.chars().map(|c| if (' '..='~').contains(&c) { c } else { ' ' }).collect();
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    t.chars().take(crate::session::IR_NAME_MAX).collect::<String>().trim().to_owned()
}

/// Read a .wav into the pedal's IR: mixed to mono, resampled to 48 kHz when it isn't, cut or padded to 2,048 samples.
pub fn wav_to_audio(b: &[u8]) -> Result<(Vec<u8>, Vec<String>), String> {
    if b.len() < 12 || tag(b, 0) != b"RIFF" || tag(b, 8) != b"WAVE" {
        return Err("it isn't a WAV file".into());
    }
    let mut fmt: Option<(u16, u16, u32, u16, u16)> = None; // format, channels, rate, align, bits
    let mut data: Option<(usize, usize)> = None;
    let mut at = 12;
    while at + 8 <= b.len() {
        let id = tag(b, at);
        let size = u32le(b, at + 4) as usize;
        let body = at + 8;
        if id == b"fmt " && size >= 16 && body + 16 <= b.len() {
            let mut f = u16le(b, body);
            if f == 0xfffe && size >= 40 && body + 26 <= b.len() {
                f = u16le(b, body + 24); // the sub-format of an extensible WAV
            }
            fmt = Some((f, u16le(b, body + 2), u32le(b, body + 4), u16le(b, body + 12), u16le(b, body + 14)));
        } else if id == b"data" {
            data = Some((body, size.min(b.len().saturating_sub(body))));
        }
        at = body + size + (size & 1);
    }
    let (format, channels, rate, align, bits) = fmt.ok_or("it has no format chunk")?;
    let (start, size) = data.filter(|d| d.1 > 0).ok_or("it has no audio in it")?;
    let float = format == 3;
    if format != 1 && !float {
        return Err(format!("it's a compressed WAV (format {format}); save it as PCM or 32-bit float"));
    }
    if channels == 0 || !(1000..=768_000).contains(&rate) {
        return Err("its channels or sample rate can't be read".into());
    }
    let ok_bits = if float { bits == 32 || bits == 64 } else { matches!(bits, 8 | 16 | 24 | 32) };
    if !ok_bits {
        return Err(format!("its {bits}-bit samples aren't a depth this app reads"));
    }
    let width = (bits / 8) as usize;
    let frame = (align as usize).max(width * channels as usize);
    let frames = size / frame;
    if frames == 0 {
        return Err("it has no audio in it".into());
    }
    let sample = |i: usize, c: usize| -> f64 {
        let p = start + i * frame + c * width;
        match (float, bits) {
            (true, 32) => f32::from_le_bytes([b[p], b[p + 1], b[p + 2], b[p + 3]]) as f64,
            (true, _) => f64::from_le_bytes(b[p..p + 8].try_into().unwrap_or([0; 8])),
            (false, 8) => (b[p] as f64 - 128.0 + 0.5) / 127.5,
            (false, 16) => (i16::from_le_bytes([b[p], b[p + 1]]) as f64 + 0.5) / 32767.5,
            (false, 24) => ((((b[p + 2] as i8 as i32) << 16) | ((b[p + 1] as i32) << 8) | b[p] as i32) as f64 + 0.5) / 8_388_607.5,
            (false, _) => (i32::from_le_bytes([b[p], b[p + 1], b[p + 2], b[p + 3]]) as f64 + 0.5) / 2_147_483_647.5,
        }
    };
    let mut notes = Vec::new();
    let mut out = vec![0f32; SAMPLES];
    // A 48 kHz mono float file goes over exactly as it is (the NAM firmware's files rely on this).
    if float && bits == 32 && channels == 1 && rate == RATE {
        for (i, o) in out.iter_mut().enumerate().take(frames.min(SAMPLES)) {
            *o = sample(i, 0) as f32;
        }
    } else {
        let mono: Vec<f64> = (0..frames).map(|i| (0..channels as usize).map(|c| sample(i, c)).sum::<f64>() / channels as f64).collect();
        if channels > 1 {
            notes.push(format!("{channels} channels mixed to mono"));
        }
        if rate == RATE {
            for (o, v) in out.iter_mut().zip(mono.iter()) {
                *o = *v as f32;
            }
        } else {
            notes.push(format!("resampled from {rate} Hz to 48 kHz"));
            resample(&mono, rate, &mut out);
        }
    }
    if (frames as u64 * RATE as u64 / rate as u64) > SAMPLES as u64 {
        notes.push("cut to 2,048 samples (about 43 ms)".into());
    }
    let mut bytes = Vec::with_capacity(SAMPLES * 4);
    for s in out {
        bytes.extend(s.to_le_bytes());
    }
    Ok((bytes, notes))
}

/// Windowed-sinc resampling to 48 kHz (Blackman window, 16 zero crossings).
fn resample(x: &[f64], rate: u32, out: &mut [f32]) {
    let step = rate as f64 / RATE as f64;
    let cut = (RATE as f64 / rate as f64).min(1.0);
    let half = 16.0 / cut;
    let n = out.len().min((RATE as f64 * x.len() as f64 / rate as f64) as usize);
    for (t, o) in out.iter_mut().enumerate().take(n) {
        let centre = t as f64 * step;
        let lo = (centre - half).ceil().max(0.0) as usize;
        let hi = ((centre + half).floor() as usize).min(x.len().saturating_sub(1));
        let mut acc = 0.0;
        for (k, &xk) in x.iter().enumerate().take(hi + 1).skip(lo) {
            let d = centre - k as f64;
            let w = d / half;
            let win = 0.42 + 0.5 * (std::f64::consts::PI * w).cos() + 0.08 * (2.0 * std::f64::consts::PI * w).cos();
            let a = std::f64::consts::PI * cut * d;
            let sinc = if a == 0.0 { 1.0 } else { a.sin() / a };
            acc += xk * cut * sinc * win;
        }
        *o = acc as f32;
    }
}

/// The pedal's IR bytes as a 48 kHz mono 32-bit float .wav.
pub fn audio_to_wav(audio: &[u8]) -> Vec<u8> {
    let samples: Vec<f32> = audio.chunks_exact(4).map(|w| f32::from_le_bytes([w[0], w[1], w[2], w[3]])).collect();
    crate::nam::wav_bytes(&samples)
}

/// Read a .wav or a .nam capture, ready to send to a slot.
pub fn load_file(path: &Path, nam_opts: crate::nam::Options) -> Result<Loaded, String> {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("IR");
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if ext == "nam" {
        let text = std::fs::read_to_string(path).map_err(|e| format!("can't read it ({e})"))?;
        let c = crate::nam::convert(&text, nam_opts)?;
        let mut audio = Vec::with_capacity(SAMPLES * 4);
        for s in &c.samples {
            audio.extend(s.to_le_bytes());
        }
        let mut notes = vec!["NAM capture converted for the NAM A2 Lite block".to_owned()];
        notes.extend(c.warnings);
        return Ok(Loaded { audio, name: clean_name(stem), notes });
    }
    let bytes = std::fs::read(path).map_err(|e| format!("can't read it ({e})"))?;
    let (audio, notes) = wav_to_audio(&bytes)?;
    Ok(Loaded { audio, name: clean_name(stem), notes })
}

/// Whether IR bytes hold a NAM A2 Lite capture (its header: 0.5, -0.5, 3.0).
pub fn is_nam_capture(audio: &[u8]) -> bool {
    let f = |i: usize| audio.get(i * 4..i * 4 + 4).map(|w| f32::from_le_bytes([w[0], w[1], w[2], w[3]]));
    f(0) == Some(0.5) && f(1) == Some(-0.5) && f(2) == Some(3.0)
}

/// A file name for a downloaded IR: "005 Name.wav".
pub fn file_name(slot: i64, name: &str) -> String {
    let n = crate::device::sanitize(if name.is_empty() { "IR" } else { name });
    format!("{:03} {n}.wav", slot + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_wav_round_trips_exactly() {
        let samples: Vec<f32> = (0..SAMPLES).map(|i| ((i as f32) * 0.01).sin() * 0.5).collect();
        let wav = crate::nam::wav_bytes(&samples);
        let (audio, notes) = wav_to_audio(&wav).unwrap();
        assert!(notes.is_empty());
        assert_eq!(audio.len(), 8192);
        assert_eq!(audio_to_wav(&audio), wav);
    }

    #[test]
    fn pcm16_44k_stereo_is_converted() {
        let frames = 3000usize;
        let mut data = Vec::new();
        for i in 0..frames {
            let v = (((i as f64) * 0.05).sin() * 12000.0) as i16;
            data.extend(v.to_le_bytes());
            data.extend(v.to_le_bytes());
        }
        let mut w = Vec::new();
        w.extend(b"RIFF");
        w.extend((36 + data.len() as u32).to_le_bytes());
        w.extend(b"WAVEfmt ");
        w.extend(16u32.to_le_bytes());
        w.extend(1u16.to_le_bytes());
        w.extend(2u16.to_le_bytes());
        w.extend(44100u32.to_le_bytes());
        w.extend((44100u32 * 4).to_le_bytes());
        w.extend(4u16.to_le_bytes());
        w.extend(16u16.to_le_bytes());
        w.extend(b"data");
        w.extend((data.len() as u32).to_le_bytes());
        w.extend(&data);
        let (audio, notes) = wav_to_audio(&w).unwrap();
        assert_eq!(audio.len(), 8192);
        assert!(notes.iter().any(|n| n.contains("mono")));
        assert!(notes.iter().any(|n| n.contains("resampled")));
    }

    #[test]
    fn nam_header_is_recognised() {
        let mut s = vec![0f32; SAMPLES];
        s[0] = 0.5;
        s[1] = -0.5;
        s[2] = 3.0;
        let bytes: Vec<u8> = s.iter().flat_map(|v| v.to_le_bytes()).collect();
        assert!(is_nam_capture(&bytes));
    }
}
