//! The dynamic [`Value`] that crosses language boundaries (service arguments and results, custom event payloads),
//! and its wire form: canonical CBOR (RFC 8949 §4.2.1, core deterministic encoding), the WIT `types.value`.
//!
//! Supported CBOR: integers (major 0 and 1, within `i64`), byte and text strings, arrays, maps with text keys,
//! `false` / `true` / `null`, and floats (encoded in the shortest of half / single / double precision that keeps the
//! value; NaN is `f97e00`). Tags, `undefined`, simple values and indefinite lengths are rejected, as are maps with
//! non-text or duplicate keys, keys out of canonical order and non-shortest arguments, so decoding and re-encoding
//! always returns the input bytes.
//!
//! Host handles (players) are the one-entry map `{"$h": n}` ([`Value::handle`]).

use std::fmt;

/// Deepest allowed nesting of lists and maps.
pub const MAX_DEPTH: usize = 32;
/// Largest encoded value accepted from a mod (1 MiB).
pub const MAX_BYTES: usize = 1 << 20;
/// The map key of a host handle.
pub const HANDLE_KEY: &str = "$h";

/// A dynamic value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// `null` (`option::none`).
    Null,
    /// A boolean.
    Bool(bool),
    /// An integer.
    Int(i64),
    /// A float.
    Float(f64),
    /// UTF-8 text.
    Text(String),
    /// Bytes.
    Bytes(Vec<u8>),
    /// A list.
    List(Vec<Value>),
    /// A map with text keys (records use kebab-case keys). Encoding sorts the entries.
    Map(Vec<(String, Value)>),
}

/// Why bytes are not a valid value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueError(pub String);

impl fmt::Display for ValueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ValueError {}

fn bad(msg: impl Into<String>) -> ValueError {
    ValueError(msg.into())
}

impl Value {
    /// Shorthand for a text value.
    pub fn text(s: &str) -> Value {
        Value::Text(s.to_string())
    }

    /// A host handle `{"$h": n}`.
    pub fn handle(n: u64) -> Value {
        Value::Map(vec![(HANDLE_KEY.to_string(), Value::Int(n as i64))])
    }

    /// The handle number when this is a host handle.
    pub fn as_handle(&self) -> Option<u64> {
        match self {
            Value::Map(m) if m.len() == 1 && m[0].0 == HANDLE_KEY => match m[0].1 {
                Value::Int(n) if n >= 0 => Some(n as u64),
                _ => None,
            },
            _ => None,
        }
    }

    /// Looks up a map entry.
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Map(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Canonical CBOR bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        write(&mut out, self);
        out
    }

    /// Decodes canonical CBOR (the whole input must be one value).
    pub fn decode(bytes: &[u8]) -> Result<Value, ValueError> {
        if bytes.len() > MAX_BYTES {
            return Err(bad(format!("value larger than {MAX_BYTES} bytes")));
        }
        let mut r = Reader { b: bytes, i: 0 };
        let v = r.value(0)?;
        if r.i != bytes.len() {
            return Err(bad("trailing bytes after the value"));
        }
        Ok(v)
    }
}

fn head(out: &mut Vec<u8>, major: u8, n: u64) {
    let m = major << 5;
    match n {
        0..=23 => out.push(m | n as u8),
        24..=0xff => out.extend([m | 24, n as u8]),
        0x100..=0xffff => {
            out.push(m | 25);
            out.extend((n as u16).to_be_bytes());
        }
        0x1_0000..=0xffff_ffff => {
            out.push(m | 26);
            out.extend((n as u32).to_be_bytes());
        }
        _ => {
            out.push(m | 27);
            out.extend(n.to_be_bytes());
        }
    }
}

/// The IEEE half-precision bits of `v` when the conversion is exact.
fn to_half(v: f64) -> Option<u16> {
    let f = v as f32;
    if f as f64 != v {
        return None;
    }
    let bits = f.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xff) as i32;
    let man = bits & 0x7f_ffff;
    if exp == 0xff {
        // Infinity (NaN is handled by the caller).
        return Some(sign | 0x7c00);
    }
    if exp == 0 && man == 0 {
        return Some(sign);
    }
    let e = exp - 127;
    if (-14..=15).contains(&e) {
        // Normal half: 10 mantissa bits.
        if man & 0x1fff != 0 {
            return None;
        }
        return Some(sign | (((e + 15) as u16) << 10) | (man >> 13) as u16);
    }
    if (-24..-14).contains(&e) {
        // Subnormal half: value = m * 2^-24 with m < 1024.
        let full = man | 0x80_0000;
        let shift = (-14 - e) + 13;
        if full & ((1 << shift) - 1) != 0 {
            return None;
        }
        return Some(sign | (full >> shift) as u16);
    }
    None
}

fn from_half(h: u16) -> f64 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = ((h >> 10) & 0x1f) as i32;
    let man = (h & 0x3ff) as f64;
    match exp {
        0 => sign * man * 2f64.powi(-24),
        0x1f if man == 0.0 => sign * f64::INFINITY,
        0x1f => f64::NAN,
        _ => sign * (1.0 + man / 1024.0) * 2f64.powi(exp - 15),
    }
}

fn write_float(out: &mut Vec<u8>, v: f64) {
    if v.is_nan() {
        out.extend([0xf9, 0x7e, 0x00]);
    } else if let Some(h) = to_half(v) {
        out.push(0xf9);
        out.extend(h.to_be_bytes());
    } else if (v as f32) as f64 == v {
        out.push(0xfa);
        out.extend((v as f32).to_bits().to_be_bytes());
    } else {
        out.push(0xfb);
        out.extend(v.to_bits().to_be_bytes());
    }
}

fn write(out: &mut Vec<u8>, v: &Value) {
    match v {
        Value::Null => out.push(0xf6),
        Value::Bool(b) => out.push(if *b { 0xf5 } else { 0xf4 }),
        Value::Int(n) if *n >= 0 => head(out, 0, *n as u64),
        Value::Int(n) => head(out, 1, !(*n) as u64),
        Value::Float(f) => write_float(out, *f),
        Value::Text(s) => {
            head(out, 3, s.len() as u64);
            out.extend(s.as_bytes());
        }
        Value::Bytes(b) => {
            head(out, 2, b.len() as u64);
            out.extend(b);
        }
        Value::List(items) => {
            head(out, 4, items.len() as u64);
            for i in items {
                write(out, i);
            }
        }
        Value::Map(entries) => {
            let mut enc: Vec<(Vec<u8>, &Value)> = entries
                .iter()
                .map(|(k, v)| {
                    let mut kb = Vec::new();
                    write(&mut kb, &Value::Text(k.clone()));
                    (kb, v)
                })
                .collect();
            enc.sort_by(|a, b| a.0.cmp(&b.0));
            enc.dedup_by(|a, b| a.0 == b.0);
            head(out, 5, enc.len() as u64);
            for (k, v) in enc {
                out.extend(k);
                write(out, v);
            }
        }
    }
}

struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], ValueError> {
        if self.b.len() - self.i < n {
            return Err(bad("truncated value"));
        }
        let s = &self.b[self.i..self.i + n];
        self.i += n;
        Ok(s)
    }

    fn arg(&mut self, info: u8) -> Result<u64, ValueError> {
        let (n, min) = match info {
            0..=23 => return Ok(info as u64),
            24 => (self.take(1)?[0] as u64, 24),
            25 => (u16::from_be_bytes(self.take(2)?.try_into().expect("2 bytes")) as u64, 0x100),
            26 => (u32::from_be_bytes(self.take(4)?.try_into().expect("4 bytes")) as u64, 0x1_0000),
            27 => (u64::from_be_bytes(self.take(8)?.try_into().expect("8 bytes")), 0x1_0000_0000),
            _ => return Err(bad("indefinite lengths and reserved arguments are not allowed")),
        };
        if n < min {
            return Err(bad("argument not in its shortest form"));
        }
        Ok(n)
    }

    fn len(&mut self, info: u8) -> Result<usize, ValueError> {
        let n = self.arg(info)?;
        if n > (self.b.len() - self.i) as u64 {
            return Err(bad("length larger than the input"));
        }
        Ok(n as usize)
    }

    fn text(&mut self, info: u8) -> Result<String, ValueError> {
        let n = self.len(info)?;
        String::from_utf8(self.take(n)?.to_vec()).map_err(|_| bad("text is not UTF-8"))
    }

    fn value(&mut self, depth: usize) -> Result<Value, ValueError> {
        if depth > MAX_DEPTH {
            return Err(bad(format!("nested deeper than {MAX_DEPTH}")));
        }
        let ib = self.take(1)?[0];
        let (major, info) = (ib >> 5, ib & 0x1f);
        match major {
            0 => {
                let n = self.arg(info)?;
                i64::try_from(n).map(Value::Int).map_err(|_| bad("integer out of range"))
            }
            1 => {
                let n = self.arg(info)?;
                i64::try_from(n).map(|n| Value::Int(!n)).map_err(|_| bad("integer out of range"))
            }
            2 => {
                let n = self.len(info)?;
                Ok(Value::Bytes(self.take(n)?.to_vec()))
            }
            3 => Ok(Value::Text(self.text(info)?)),
            4 => {
                let n = self.len(info)?;
                (0..n).map(|_| self.value(depth + 1)).collect::<Result<_, _>>().map(Value::List)
            }
            5 => {
                let n = self.len(info)?;
                let mut out = Vec::with_capacity(n);
                let mut last: Option<&[u8]> = None;
                for _ in 0..n {
                    let start = self.i;
                    let kb = self.take(1)?[0];
                    if kb >> 5 != 3 {
                        return Err(bad("map keys must be text"));
                    }
                    let key = self.text(kb & 0x1f)?;
                    let enc = &self.b[start..self.i];
                    if last.is_some_and(|l| l >= enc) {
                        return Err(bad("map keys duplicated or not in canonical order"));
                    }
                    last = Some(enc);
                    out.push((key, self.value(depth + 1)?));
                }
                Ok(Value::Map(out))
            }
            6 => Err(bad("tags are not allowed")),
            _ => match info {
                20 => Ok(Value::Bool(false)),
                21 => Ok(Value::Bool(true)),
                22 => Ok(Value::Null),
                25 => {
                    let h = u16::from_be_bytes(self.take(2)?.try_into().expect("2 bytes"));
                    let f = from_half(h);
                    if f.is_nan() && h != 0x7e00 {
                        return Err(bad("NaN must be encoded as f97e00"));
                    }
                    Ok(Value::Float(f))
                }
                26 => {
                    let f = f32::from_bits(u32::from_be_bytes(self.take(4)?.try_into().expect("4 bytes"))) as f64;
                    self.float_canonical(f, 5)
                }
                27 => {
                    let f = f64::from_bits(u64::from_be_bytes(self.take(8)?.try_into().expect("8 bytes")));
                    self.float_canonical(f, 9)
                }
                _ => Err(bad("unsupported simple value")),
            },
        }
    }

    fn float_canonical(&self, f: f64, len: usize) -> Result<Value, ValueError> {
        let mut enc = Vec::new();
        write_float(&mut enc, f);
        if enc.len() != len {
            return Err(bad("float not in its shortest form"));
        }
        Ok(Value::Float(f))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    fn rt(v: Value, h: &str) {
        assert_eq!(v.encode(), hex(h), "{v:?}");
        assert_eq!(Value::decode(&hex(h)).unwrap(), v, "{h}");
    }

    #[test]
    fn rfc8949_examples() {
        rt(Value::Int(0), "00");
        rt(Value::Int(23), "17");
        rt(Value::Int(24), "1818");
        rt(Value::Int(1000), "1903e8");
        rt(Value::Int(1_000_000), "1a000f4240");
        rt(Value::Int(1_000_000_000_000), "1b000000e8d4a51000");
        rt(Value::Int(-1), "20");
        rt(Value::Int(-1000), "3903e7");
        rt(Value::Int(i64::MIN), "3b7fffffffffffffff");
        rt(Value::Float(0.0), "f90000");
        rt(Value::Float(-0.0), "f98000");
        rt(Value::Float(1.5), "f93e00");
        rt(Value::Float(65504.0), "f97bff");
        rt(Value::Float(100000.0), "fa47c35000");
        rt(Value::Float(3.4028234663852886e38), "fa7f7fffff");
        rt(Value::Float(1.1), "fb3ff199999999999a");
        rt(Value::Float(5.960464477539063e-8), "f90001");
        rt(Value::Float(0.00006103515625), "f90400");
        rt(Value::Float(-4.0), "f9c400");
        rt(Value::Float(f64::INFINITY), "f97c00");
        rt(Value::Float(f64::NEG_INFINITY), "f9fc00");
        rt(Value::Bool(false), "f4");
        rt(Value::Bool(true), "f5");
        rt(Value::Null, "f6");
        rt(Value::Bytes(vec![1, 2, 3, 4]), "4401020304");
        rt(Value::text("IETF"), "6449455446");
        rt(Value::text("ü"), "62c3bc");
        rt(Value::List(vec![Value::Int(1), Value::List(vec![Value::Int(2)])]), "82018102");
        rt(Value::Map(vec![("a".into(), Value::Int(1)), ("b".into(), Value::List(vec![]))]), "a2616101616280");
        assert_eq!(Value::Float(f64::NAN).encode(), hex("f97e00"));
        assert!(matches!(Value::decode(&hex("f97e00")).unwrap(), Value::Float(f) if f.is_nan()));
    }

    #[test]
    fn half_precision_edges() {
        // Subnormal halves that need more than 10 bits, too small, too large, and an inexact f32 mantissa.
        for v in [3.0 * 2f64.powi(-25), 2f64.powi(-25), 65536.0, 1.0 + 2f64.powi(-11)] {
            assert_ne!(Value::Float(v).encode()[0], 0xf9, "{v}");
            assert_eq!(Value::decode(&Value::Float(v).encode()).unwrap(), Value::Float(v));
        }
        assert_eq!(from_half(0x3c00), 1.0);
        assert!(from_half(0x7e01).is_nan());
    }

    #[test]
    fn maps_sort_and_dedup() {
        let m =
            Value::Map(vec![("bb".into(), Value::Int(1)), ("c".into(), Value::Int(2)), ("c".into(), Value::Int(3))]);
        assert_eq!(m.encode(), hex("a261630262626201"));
        assert_eq!(m.get("bb"), Some(&Value::Int(1)));
        assert_eq!(m.get("zz"), None);
        assert_eq!(Value::Null.get("x"), None);
    }

    #[test]
    fn handles() {
        let h = Value::handle(7);
        assert_eq!(h.as_handle(), Some(7));
        assert_eq!(h.encode(), hex("a162246807"));
        assert_eq!(Value::Map(vec![("$h".into(), Value::Int(-1))]).as_handle(), None);
        assert_eq!(Value::Map(vec![("$h".into(), Value::Null)]).as_handle(), None);
        assert_eq!(Value::Map(vec![("x".into(), Value::Int(1))]).as_handle(), None);
        assert_eq!(Value::Int(1).as_handle(), None);
    }

    #[test]
    fn rejects_non_canonical_and_unsupported() {
        let cases = [
            ("", "truncated"),
            ("1817", "shortest"),
            ("190017", "shortest"),
            ("1a00000017", "shortest"),
            ("1b0000000000000017", "shortest"),
            ("1bffffffffffffffff", "out of range"),
            ("3bffffffffffffffff", "out of range"),
            ("1f", "indefinite"),
            ("5f", "indefinite"),
            ("45010203", "length larger"),
            ("62ff00", "UTF-8"),
            ("a2616201616101", "canonical order"),
            ("a2616101616101", "canonical order"),
            ("a10101", "keys must be text"),
            ("c000", "tags"),
            ("f7", "simple"),
            ("f97e01", "NaN"),
            ("fa3fc00000", "shortest"),
            ("fb3ff8000000000000", "shortest"),
            ("0000", "trailing"),
            ("19", "truncated"),
        ];
        for (h, want) in cases {
            let e = Value::decode(&hex(h)).unwrap_err();
            assert!(e.to_string().contains(want), "{h}: {e}");
        }
        let mut deep = vec![0x81; MAX_DEPTH + 1];
        deep.push(0x00);
        assert!(Value::decode(&deep).unwrap_err().0.contains("nested"));
        assert!(Value::decode(&vec![0; MAX_BYTES + 1]).unwrap_err().0.contains("larger"));
    }
}
