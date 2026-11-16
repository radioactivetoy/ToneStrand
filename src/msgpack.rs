//! A small MessagePack codec that remembers how each value was written.
//!
//! The pedal's preset documents carry a table of byte offsets into themselves,
//! so a document that is read and written back unchanged must come out byte for
//! byte the same. Each value therefore keeps the format byte it arrived with and
//! is written back the same way whenever it still fits.

/// Format marker for values made here rather than decoded: pick the shortest form.
pub const AUTO: u8 = 0xc1;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Nil,
    Bool(bool),
    Int(i64, u8),
    F32(f32),
    F64(f64),
    Str(String, u8),
    Bin(Vec<u8>, u8),
    Array(Vec<Value>, u8),
    Map(Vec<(Value, Value)>, u8),
    Ext(i8, Vec<u8>, u8),
}

impl Value {
    pub fn int(n: i64) -> Value {
        Value::Int(n, AUTO)
    }
    pub fn str(s: &str) -> Value {
        Value::Str(s.to_owned(), AUTO)
    }
    pub fn map(pairs: Vec<(i64, Value)>) -> Value {
        Value::Map(pairs.into_iter().map(|(k, v)| (Value::int(k), v)).collect(), AUTO)
    }
    pub fn empty_map() -> Value {
        Value::Map(Vec::new(), AUTO)
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(n, _) => Some(*n),
            _ => None,
        }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Int(n, _) => Some(*n as f64),
            Value::F32(f) => Some(*f as f64),
            Value::F64(f) => Some(*f),
            Value::Bool(b) => Some(*b as u8 as f64),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s, _) => Some(s),
            _ => None,
        }
    }
    pub fn as_bin(&self) -> Option<&[u8]> {
        match self {
            Value::Bin(b, _) => Some(b),
            Value::Str(s, _) => Some(s.as_bytes()),
            _ => None,
        }
    }
    pub fn as_array(&self) -> Option<&Vec<Value>> {
        match self {
            Value::Array(a, _) => Some(a),
            _ => None,
        }
    }
    pub fn as_array_mut(&mut self) -> Option<&mut Vec<Value>> {
        match self {
            Value::Array(a, _) => Some(a),
            _ => None,
        }
    }

    /// The value under an integer key of a map.
    pub fn get(&self, key: i64) -> Option<&Value> {
        match self {
            Value::Map(pairs, _) => pairs.iter().find(|(k, _)| k.as_i64() == Some(key)).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn get_mut(&mut self, key: i64) -> Option<&mut Value> {
        match self {
            Value::Map(pairs, _) => pairs.iter_mut().find(|(k, _)| k.as_i64() == Some(key)).map(|(_, v)| v),
            _ => None,
        }
    }
    /// Follow a chain of integer keys.
    pub fn at(&self, keys: &[i64]) -> Option<&Value> {
        let mut v = self;
        for k in keys {
            v = v.get(*k)?;
        }
        Some(v)
    }
    pub fn at_mut(&mut self, keys: &[i64]) -> Option<&mut Value> {
        let mut v = self;
        for k in keys {
            v = v.get_mut(*k)?;
        }
        Some(v)
    }

    /// A short, readable rendering for logs and the raw view.
    pub fn summary(&self) -> String {
        match self {
            Value::Nil => "nil".into(),
            Value::Bool(b) => b.to_string(),
            Value::Int(n, _) => n.to_string(),
            Value::F32(f) => format!("{f:.4}"),
            Value::F64(f) => format!("{f:.4}"),
            Value::Str(s, _) => format!("\"{s}\""),
            Value::Bin(b, _) => format!("<{} bytes>", b.len()),
            Value::Array(a, _) => format!("[{} items]", a.len()),
            Value::Map(m, _) => format!("{{{} fields}}", m.len()),
            Value::Ext(t, b, _) => format!("ext {t} <{} bytes>", b.len()),
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        write(self, &mut out);
        out
    }

    /// The same value in the same format: like `==`, but floats compare by their bits, so a
    /// NaN equals itself and 0.0 differs from -0.0 (as their encodings do). Much cheaper
    /// than comparing encodings.
    pub fn same_as(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::F32(a), Value::F32(b)) => a.to_bits() == b.to_bits(),
            (Value::F64(a), Value::F64(b)) => a.to_bits() == b.to_bits(),
            (Value::Array(a, f), Value::Array(b, g)) => f == g && a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.same_as(y)),
            (Value::Map(a, f), Value::Map(b, g)) => {
                f == g && a.len() == b.len() && a.iter().zip(b).all(|((k1, v1), (k2, v2))| k1.same_as(k2) && v1.same_as(v2))
            }
            _ => self == other,
        }
    }
}

// ------------------------------------------------------------------ writing ---

fn write(v: &Value, out: &mut Vec<u8>) {
    match v {
        Value::Nil => out.push(0xc0),
        Value::Bool(b) => out.push(if *b { 0xc3 } else { 0xc2 }),
        Value::F32(f) => {
            out.push(0xca);
            out.extend_from_slice(&f.to_be_bytes());
        }
        Value::F64(f) => {
            out.push(0xcb);
            out.extend_from_slice(&f.to_be_bytes());
        }
        Value::Int(n, fmt) => write_int(*n, *fmt, out),
        Value::Str(s, fmt) => {
            let n = s.len();
            match *fmt {
                0xa0 if n < 32 => out.push(0xa0 | n as u8),
                0xd9 if n < 256 => {
                    out.push(0xd9);
                    out.push(n as u8)
                }
                0xda if n < 65536 => {
                    out.push(0xda);
                    out.extend_from_slice(&(n as u16).to_be_bytes())
                }
                0xdb => {
                    out.push(0xdb);
                    out.extend_from_slice(&(n as u32).to_be_bytes())
                }
                _ => {
                    if n < 32 {
                        out.push(0xa0 | n as u8)
                    } else if n < 256 {
                        out.push(0xd9);
                        out.push(n as u8)
                    } else if n < 65536 {
                        out.push(0xda);
                        out.extend_from_slice(&(n as u16).to_be_bytes())
                    } else {
                        out.push(0xdb);
                        out.extend_from_slice(&(n as u32).to_be_bytes())
                    }
                }
            }
            out.extend_from_slice(s.as_bytes());
        }
        Value::Bin(b, fmt) if matches!(*fmt, 0xa0 | 0xd9 | 0xda | 0xdb) => {
            // Bytes that arrived as a string go back as one.
            let n = b.len();
            match *fmt {
                0xa0 if n < 32 => out.push(0xa0 | n as u8),
                0xd9 if n < 256 => out.extend_from_slice(&[0xd9, n as u8]),
                0xda if n < 65536 => {
                    out.push(0xda);
                    out.extend_from_slice(&(n as u16).to_be_bytes())
                }
                _ => {
                    out.push(0xdb);
                    out.extend_from_slice(&(n as u32).to_be_bytes())
                }
            }
            out.extend_from_slice(b);
        }
        Value::Bin(b, fmt) => {
            let n = b.len();
            let tag = match *fmt {
                0xc4 if n < 256 => 0xc4,
                0xc5 if n < 65536 => 0xc5,
                0xc6 => 0xc6,
                _ if n < 256 => 0xc4,
                _ if n < 65536 => 0xc5,
                _ => 0xc6,
            };
            write_len(tag, 0xc4, n, out);
            out.extend_from_slice(b);
        }
        Value::Array(a, fmt) => {
            let n = a.len();
            if (*fmt == 0x90 || *fmt == AUTO) && n < 16 {
                out.push(0x90 | n as u8);
            } else if *fmt != 0xdd && n < 65536 {
                out.push(0xdc);
                out.extend_from_slice(&(n as u16).to_be_bytes());
            } else {
                out.push(0xdd);
                out.extend_from_slice(&(n as u32).to_be_bytes());
            }
            for item in a {
                write(item, out);
            }
        }
        Value::Map(m, fmt) => {
            let n = m.len();
            if (*fmt == 0x80 || *fmt == AUTO) && n < 16 {
                out.push(0x80 | n as u8);
            } else if *fmt != 0xdf && n < 65536 {
                out.push(0xde);
                out.extend_from_slice(&(n as u16).to_be_bytes());
            } else {
                out.push(0xdf);
                out.extend_from_slice(&(n as u32).to_be_bytes());
            }
            for (k, v) in m {
                write(k, out);
                write(v, out);
            }
        }
        Value::Ext(t, b, fmt) => {
            let n = b.len();
            let fixed = match n {
                1 => Some(0xd4),
                2 => Some(0xd5),
                4 => Some(0xd6),
                8 => Some(0xd7),
                16 => Some(0xd8),
                _ => None,
            };
            match fixed {
                Some(tag) if *fmt == tag || *fmt == AUTO => out.push(tag),
                _ => write_len(
                    if n < 256 {
                        0xc7
                    } else if n < 65536 {
                        0xc8
                    } else {
                        0xc9
                    },
                    0xc7,
                    n,
                    out,
                ),
            }
            out.push(*t as u8);
            out.extend_from_slice(b);
        }
    }
}

/// Length prefix for the bin/ext families: tag, then 1, 2 or 4 bytes.
fn write_len(tag: u8, base: u8, n: usize, out: &mut Vec<u8>) {
    out.push(tag);
    match tag - base {
        0 => out.push(n as u8),
        1 => out.extend_from_slice(&(n as u16).to_be_bytes()),
        _ => out.extend_from_slice(&(n as u32).to_be_bytes()),
    }
}

fn fits(n: i64, fmt: u8) -> bool {
    match fmt {
        0x00 => (0..=0x7f).contains(&n),
        0xe0 => (-32..0).contains(&n),
        0xcc => (0..=0xff).contains(&n),
        0xcd => (0..=0xffff).contains(&n),
        0xce => (0..=0xffff_ffff).contains(&n),
        0xcf => n >= 0,
        0xd0 => (i8::MIN as i64..=i8::MAX as i64).contains(&n),
        0xd1 => (i16::MIN as i64..=i16::MAX as i64).contains(&n),
        0xd2 => (i32::MIN as i64..=i32::MAX as i64).contains(&n),
        0xd3 => true,
        _ => false,
    }
}

fn write_int(n: i64, fmt: u8, out: &mut Vec<u8>) {
    let fmt = if fits(n, fmt) {
        fmt
    } else if n >= 0 {
        match n {
            0..=0x7f => 0x00,
            0x80..=0xff => 0xcc,
            0x100..=0xffff => 0xcd,
            0x1_0000..=0xffff_ffff => 0xce,
            _ => 0xcf,
        }
    } else if n >= -32 {
        0xe0
    } else if n >= i8::MIN as i64 {
        0xd0
    } else if n >= i16::MIN as i64 {
        0xd1
    } else if n >= i32::MIN as i64 {
        0xd2
    } else {
        0xd3
    };
    match fmt {
        0x00 => out.push(n as u8),
        0xe0 => out.push(n as i8 as u8),
        0xcc => out.extend_from_slice(&[0xcc, n as u8]),
        0xcd => {
            out.push(0xcd);
            out.extend_from_slice(&(n as u16).to_be_bytes())
        }
        0xce => {
            out.push(0xce);
            out.extend_from_slice(&(n as u32).to_be_bytes())
        }
        0xcf => {
            out.push(0xcf);
            out.extend_from_slice(&(n as u64).to_be_bytes())
        }
        0xd0 => out.extend_from_slice(&[0xd0, n as i8 as u8]),
        0xd1 => {
            out.push(0xd1);
            out.extend_from_slice(&(n as i16).to_be_bytes())
        }
        0xd2 => {
            out.push(0xd2);
            out.extend_from_slice(&(n as i32).to_be_bytes())
        }
        _ => {
            out.push(0xd3);
            out.extend_from_slice(&n.to_be_bytes())
        }
    }
}

// ------------------------------------------------------------------ reading ---

pub struct Decoder<'a> {
    buf: &'a [u8],
    pos: usize,
    /// How many arrays/maps the value being read sits inside.
    depth: usize,
}

/// Real documents nest a handful of levels; a damaged file nesting deeper than this is
/// refused instead of overflowing the stack.
const MAX_DEPTH: usize = 64;

#[derive(Debug)]
pub struct DecodeError(pub &'static str);

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl<'a> Decoder<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Decoder { buf, pos: 0, depth: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    /// Every value in the buffer, one after another.
    pub fn decode_all(buf: &[u8]) -> Result<Vec<Value>, DecodeError> {
        let mut d = Decoder::new(buf);
        let mut out = Vec::new();
        while d.remaining() > 0 {
            out.push(d.value()?);
        }
        Ok(out)
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        if self.pos + n > self.buf.len() {
            return Err(DecodeError("truncated"));
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, DecodeError> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Result<u32, DecodeError> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn u64(&mut self) -> Result<u64, DecodeError> {
        let b = self.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        Ok(u64::from_be_bytes(a))
    }

    fn string(&mut self, n: usize, fmt: u8) -> Result<Value, DecodeError> {
        let b = self.take(n)?;
        // Old-style "raw" strings can hold any bytes; keep those as bytes
        // that remember they were written as a string.
        Ok(match std::str::from_utf8(b) {
            Ok(s) => Value::Str(s.to_owned(), fmt),
            Err(_) => Value::Bin(b.to_vec(), fmt),
        })
    }
    fn enter(&mut self) -> Result<(), DecodeError> {
        if self.depth >= MAX_DEPTH {
            return Err(DecodeError("nested too deeply"));
        }
        self.depth += 1;
        Ok(())
    }
    fn array(&mut self, n: usize, fmt: u8) -> Result<Value, DecodeError> {
        self.enter()?;
        let mut items = Vec::with_capacity(n.min(4096));
        for _ in 0..n {
            items.push(self.value()?);
        }
        self.depth -= 1;
        Ok(Value::Array(items, fmt))
    }
    fn map(&mut self, n: usize, fmt: u8) -> Result<Value, DecodeError> {
        self.enter()?;
        let mut pairs = Vec::with_capacity(n.min(4096));
        for _ in 0..n {
            let k = self.value()?;
            let v = self.value()?;
            pairs.push((k, v));
        }
        self.depth -= 1;
        Ok(Value::Map(pairs, fmt))
    }
    fn ext(&mut self, n: usize, fmt: u8) -> Result<Value, DecodeError> {
        let t = self.u8()? as i8;
        Ok(Value::Ext(t, self.take(n)?.to_vec(), fmt))
    }

    pub fn value(&mut self) -> Result<Value, DecodeError> {
        let b = self.u8()?;
        Ok(match b {
            0x00..=0x7f => Value::Int(b as i64, 0x00),
            0x80..=0x8f => return self.map((b & 0x0f) as usize, 0x80),
            0x90..=0x9f => return self.array((b & 0x0f) as usize, 0x90),
            0xa0..=0xbf => return self.string((b & 0x1f) as usize, 0xa0),
            0xc0 => Value::Nil,
            0xc2 => Value::Bool(false),
            0xc3 => Value::Bool(true),
            0xc4 => {
                let n = self.u8()? as usize;
                Value::Bin(self.take(n)?.to_vec(), 0xc4)
            }
            0xc5 => {
                let n = self.u16()? as usize;
                Value::Bin(self.take(n)?.to_vec(), 0xc5)
            }
            0xc6 => {
                let n = self.u32()? as usize;
                Value::Bin(self.take(n)?.to_vec(), 0xc6)
            }
            0xc7 => {
                let n = self.u8()? as usize;
                return self.ext(n, 0xc7);
            }
            0xc8 => {
                let n = self.u16()? as usize;
                return self.ext(n, 0xc8);
            }
            0xc9 => {
                let n = self.u32()? as usize;
                return self.ext(n, 0xc9);
            }
            0xca => Value::F32(f32::from_bits(self.u32()?)),
            0xcb => Value::F64(f64::from_bits(self.u64()?)),
            0xcc => Value::Int(self.u8()? as i64, 0xcc),
            0xcd => Value::Int(self.u16()? as i64, 0xcd),
            0xce => Value::Int(self.u32()? as i64, 0xce),
            0xcf => Value::Int(self.u64()? as i64, 0xcf),
            0xd0 => Value::Int(self.u8()? as i8 as i64, 0xd0),
            0xd1 => Value::Int(self.u16()? as i16 as i64, 0xd1),
            0xd2 => Value::Int(self.u32()? as i32 as i64, 0xd2),
            0xd3 => Value::Int(self.u64()? as i64, 0xd3),
            0xd4 => return self.ext(1, 0xd4),
            0xd5 => return self.ext(2, 0xd5),
            0xd6 => return self.ext(4, 0xd6),
            0xd7 => return self.ext(8, 0xd7),
            0xd8 => return self.ext(16, 0xd8),
            0xd9 => {
                let n = self.u8()? as usize;
                return self.string(n, 0xd9);
            }
            0xda => {
                let n = self.u16()? as usize;
                return self.string(n, 0xda);
            }
            0xdb => {
                let n = self.u32()? as usize;
                return self.string(n, 0xdb);
            }
            0xdc => {
                let n = self.u16()? as usize;
                return self.array(n, 0xdc);
            }
            0xdd => {
                let n = self.u32()? as usize;
                return self.array(n, 0xdd);
            }
            0xde => {
                let n = self.u16()? as usize;
                return self.map(n, 0xde);
            }
            0xdf => {
                let n = self.u32()? as usize;
                return self.map(n, 0xdf);
            }
            0xe0..=0xff => Value::Int(b as i8 as i64, 0xe0),
            0xc1 => return Err(DecodeError("reserved byte 0xc1")),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_a_request() {
        // {102: 1009, 100: 20, 101: {107: 0, 108: 12}} as the pedal's editors send it.
        let raw = [0x83, 0x66, 0xcd, 0x03, 0xf1, 0x64, 0x14, 0x65, 0x82, 0x6b, 0x00, 0x6c, 0x0c];
        let v = Decoder::new(&raw).value().unwrap();
        assert_eq!(v.get(102).unwrap().as_i64(), Some(1009));
        assert_eq!(v.encode(), raw);
    }

    #[test]
    fn refuses_deep_nesting_instead_of_crashing() {
        let mut raw = vec![0x91u8; 100_000];
        raw.push(0x00);
        assert!(Decoder::decode_all(&raw).is_err());
        // Ordinary nesting still reads.
        let mut ok = vec![0x91u8; 10];
        ok.push(0x00);
        assert_eq!(Decoder::decode_all(&ok).unwrap()[0].encode(), ok);
    }

    #[test]
    fn same_as_compares_floats_by_bits() {
        let nan = Value::Array(vec![Value::F32(f32::NAN)], AUTO);
        assert!(nan.same_as(&nan.clone()));
        assert!(!Value::F32(0.0).same_as(&Value::F32(-0.0)));
        let a = Value::map(vec![(1, Value::F32(0.5))]);
        let mut b = a.clone();
        assert!(a.same_as(&b));
        *b.get_mut(1).unwrap() = Value::F32(0.6);
        assert!(!a.same_as(&b));
    }

    #[test]
    fn keeps_wide_formats() {
        let raw = [0xcd, 0x00, 0x05, 0xc5, 0x00, 0x01, 0xaa];
        let vals = Decoder::decode_all(&raw).unwrap();
        let mut out = Vec::new();
        for v in &vals {
            out.extend(v.encode());
        }
        assert_eq!(out, raw);
    }
}
