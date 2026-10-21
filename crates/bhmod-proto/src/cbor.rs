//! The CBOR subset used by 0xBC bodies, with deterministic encoding and strict decoding.
//!
//! Supported: unsigned integers (major 0), byte strings (2), text strings (3), arrays (4), maps (5), `false`,
//! `true`, `null` (7). Not supported: negative integers, tags, floats, `undefined`, indefinite lengths.
//!
//! Encoding follows RFC 8949 §4.2.1 (core deterministic encoding): shortest argument form, definite lengths, map
//! keys sorted by the bytewise order of their encodings. The decoder accepts only that form (so decode -> encode
//! returns the input bytes), rejects duplicate keys and nesting deeper than [`MAX_DEPTH`].

use crate::Error;

/// Deepest allowed nesting of arrays and maps.
pub const MAX_DEPTH: usize = 16;

/// A CBOR value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// Unsigned integer.
    Uint(u64),
    /// Byte string.
    Bytes(Vec<u8>),
    /// UTF-8 text string.
    Text(String),
    /// Array.
    Array(Vec<Value>),
    /// Map; [`encode`] sorts the entries.
    Map(Vec<(Value, Value)>),
    /// `false` / `true`.
    Bool(bool),
    /// `null`.
    Null,
}

impl Value {
    /// Shorthand for a text value.
    pub fn text(s: &str) -> Value {
        Value::Text(s.to_string())
    }
}

/// Encodes a value deterministically.
pub fn encode(v: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    write(&mut out, v);
    out
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

fn write(out: &mut Vec<u8>, v: &Value) {
    match v {
        Value::Uint(n) => head(out, 0, *n),
        Value::Bytes(b) => {
            head(out, 2, b.len() as u64);
            out.extend(b);
        }
        Value::Text(s) => {
            head(out, 3, s.len() as u64);
            out.extend(s.as_bytes());
        }
        Value::Array(items) => {
            head(out, 4, items.len() as u64);
            items.iter().for_each(|i| write(out, i));
        }
        Value::Map(entries) => {
            let mut enc: Vec<(Vec<u8>, Vec<u8>)> = entries.iter().map(|(k, v)| (encode(k), encode(v))).collect();
            enc.sort();
            head(out, 5, enc.len() as u64);
            for (k, v) in enc {
                out.extend(k);
                out.extend(v);
            }
        }
        Value::Bool(false) => out.push(0xf4),
        Value::Bool(true) => out.push(0xf5),
        Value::Null => out.push(0xf6),
    }
}

/// Decodes exactly one value occupying all of `bytes`.
pub fn decode(bytes: &[u8]) -> Result<Value, Error> {
    let mut r = Reader { bytes, pos: 0 };
    let v = r.value(0)?;
    if r.pos != bytes.len() {
        return Err(Error::Cbor("trailing bytes"));
    }
    Ok(v)
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: u64) -> Result<&[u8], Error> {
        let n = usize::try_from(n).map_err(|_| Error::Cbor("truncated"))?;
        let end = self.pos.checked_add(n).filter(|e| *e <= self.bytes.len()).ok_or(Error::Cbor("truncated"))?;
        let s = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    fn arg(&mut self, info: u8) -> Result<u64, Error> {
        let (n, min) = match info {
            0..=23 => return Ok(u64::from(info)),
            24 => (u64::from(self.take(1)?[0]), 24),
            25 => (u64::from(u16::from_be_bytes(self.take(2)?.try_into().expect("2 bytes"))), 0x100),
            26 => (u64::from(u32::from_be_bytes(self.take(4)?.try_into().expect("4 bytes"))), 0x1_0000),
            27 => (u64::from_be_bytes(self.take(8)?.try_into().expect("8 bytes")), 0x1_0000_0000),
            _ => return Err(Error::Cbor("indefinite length or reserved argument")),
        };
        if n < min {
            return Err(Error::Cbor("non-shortest integer encoding"));
        }
        Ok(n)
    }

    fn value(&mut self, depth: usize) -> Result<Value, Error> {
        if depth > MAX_DEPTH {
            return Err(Error::Cbor("nesting too deep"));
        }
        let ib = self.take(1)?[0];
        let (major, info) = (ib >> 5, ib & 0x1f);
        if major == 7 {
            return match info {
                20 => Ok(Value::Bool(false)),
                21 => Ok(Value::Bool(true)),
                22 => Ok(Value::Null),
                _ => Err(Error::Cbor("unsupported simple value or float")),
            };
        }
        let n = self.arg(info)?;
        match major {
            0 => Ok(Value::Uint(n)),
            2 => Ok(Value::Bytes(self.take(n)?.to_vec())),
            3 => {
                let s = std::str::from_utf8(self.take(n)?).map_err(|_| Error::Cbor("invalid UTF-8 text"))?;
                Ok(Value::Text(s.to_string()))
            }
            4 => {
                let mut items = Vec::new();
                for _ in 0..n {
                    items.push(self.value(depth + 1)?);
                }
                Ok(Value::Array(items))
            }
            5 => {
                let mut entries = Vec::new();
                let mut last: Option<Vec<u8>> = None;
                for _ in 0..n {
                    let start = self.pos;
                    let k = self.value(depth + 1)?;
                    let kb = self.bytes[start..self.pos].to_vec();
                    if last.as_ref().is_some_and(|l| *l >= kb) {
                        return Err(Error::Cbor("map keys not in canonical order or duplicated"));
                    }
                    last = Some(kb);
                    entries.push((k, self.value(depth + 1)?));
                }
                Ok(Value::Map(entries))
            }
            _ => Err(Error::Cbor("unsupported major type (negative integer or tag)")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    #[test]
    fn integers_use_shortest_form() {
        for (n, h) in [
            (0, "00"),
            (23, "17"),
            (24, "1818"),
            (255, "18ff"),
            (256, "190100"),
            (65535, "19ffff"),
            (65536, "1a00010000"),
            (u64::from(u32::MAX), "1affffffff"),
            (1 << 32, "1b0000000100000000"),
        ] {
            let e = encode(&Value::Uint(n));
            assert_eq!(hex(&e), h);
            assert_eq!(decode(&e).unwrap(), Value::Uint(n));
        }
    }

    #[test]
    fn round_trip_all_kinds() {
        let v = Value::Map(vec![
            (Value::text("b"), Value::Array(vec![Value::Bool(true), Value::Bool(false), Value::Null])),
            (Value::text("a"), Value::Bytes(vec![1, 2, 3])),
            (Value::text("aa"), Value::Map(vec![])),
        ]);
        let e = encode(&v);
        // Keys sorted by encoding: "a" (61 61) < "b" (61 62) < "aa" (62 61 61).
        assert_eq!(hex(&e), "a3616143010203616283f5f4f6626161a0");
        let sorted = Value::Map(vec![
            (Value::text("a"), Value::Bytes(vec![1, 2, 3])),
            (Value::text("b"), Value::Array(vec![Value::Bool(true), Value::Bool(false), Value::Null])),
            (Value::text("aa"), Value::Map(vec![])),
        ]);
        assert_eq!(decode(&e).unwrap(), sorted);
    }

    #[test]
    fn rejects_non_canonical_and_unsupported() {
        for (h, why) in [
            ("1817", "non-shortest"),
            ("190017", "non-shortest"),
            ("1a000000ff", "non-shortest"),
            ("1b00000000ffffffff", "non-shortest"),
            ("1f", "indefinite"),
            ("5f", "indefinite"),
            ("20", "major type"),
            ("c0", "major type"),
            ("f7", "simple"),
            ("fb0000000000000000", "simple"),
            ("62ff00", "UTF-8"),
            ("a2616100616100", "canonical order"),
            ("a2616200616100", "canonical order"),
            ("0000", "trailing"),
            ("", "truncated"),
            ("19", "truncated"),
            ("43", "truncated"),
            ("82", "truncated"),
            ("a1", "truncated"),
            ("5b7fffffffffffffff", "truncated"),
        ] {
            let bytes: Vec<u8> =
                (0..h.len()).step_by(2).map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap()).collect();
            let err = decode(&bytes).unwrap_err().to_string();
            assert!(err.contains(why), "{h}: {err}");
        }
    }

    #[test]
    fn depth_limit() {
        let mut v = Value::Null;
        for _ in 0..=MAX_DEPTH {
            v = Value::Array(vec![v]);
        }
        assert!(decode(&encode(&v)).unwrap_err().to_string().contains("deep"));
        let Value::Array(inner) = v else { unreachable!() };
        assert!(decode(&encode(&inner[0])).is_ok());
    }

    #[test]
    fn take_overflow() {
        let mut r = Reader { bytes: &[0], pos: 1 };
        assert!(r.take(u64::MAX).is_err());
    }
}
