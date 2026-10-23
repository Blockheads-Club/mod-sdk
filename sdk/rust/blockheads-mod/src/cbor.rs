//! Canonical CBOR values (RFC 8949 section 4.2.1) for service arguments, results and custom event payloads: the
//! `value` type of the `blockheads:api` WIT. Records are maps with kebab-case text keys, variants one-entry maps,
//! host handles `{"$h": n}`.

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
    /// A map with text keys.
    Map(Vec<(String, Value)>),
}

impl From<&str> for Value {
    fn from(s: &str) -> Value {
        Value::Text(s.to_string())
    }
}

impl From<String> for Value {
    fn from(s: String) -> Value {
        Value::Text(s)
    }
}

impl From<i64> for Value {
    fn from(n: i64) -> Value {
        Value::Int(n)
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Value {
        Value::Bool(b)
    }
}

impl Value {
    /// The entry `key` of a map.
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Map(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// The text, if this is text.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Text(s) => Some(s),
            _ => None,
        }
    }

    /// The integer, if this is an integer.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(n) => Some(*n),
            _ => None,
        }
    }

    /// The boolean, if this is a boolean.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Canonical CBOR bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        write(&mut out, self);
        out
    }

    /// Reads one value; the whole input must be used.
    pub fn decode(bytes: &[u8]) -> Result<Value, String> {
        let mut r = Reader { b: bytes, i: 0 };
        let v = r.value(0)?;
        if r.i != bytes.len() {
            return Err("trailing bytes after the value".into());
        }
        Ok(v)
    }
}

/// The canonical CBOR array of `args`: what `services::call` and `services::call_remote` take.
pub fn args(items: &[Value]) -> Vec<u8> {
    Value::List(items.to_vec()).encode()
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

/// The half-precision bits of `v` when the conversion is exact.
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
        return Some(sign | 0x7c00);
    }
    if exp == 0 && man == 0 {
        return Some(sign);
    }
    let e = exp - 127;
    if (-14..=15).contains(&e) {
        if man & 0x1fff != 0 {
            return None;
        }
        return Some(sign | (((e + 15) as u16) << 10) | (man >> 13) as u16);
    }
    if (-24..-14).contains(&e) {
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

fn write(out: &mut Vec<u8>, v: &Value) {
    match v {
        Value::Null => out.push(0xf6),
        Value::Bool(b) => out.push(if *b { 0xf5 } else { 0xf4 }),
        Value::Int(n) if *n >= 0 => head(out, 0, *n as u64),
        Value::Int(n) => head(out, 1, !(*n) as u64),
        Value::Float(f) => {
            if f.is_nan() {
                out.extend([0xf9, 0x7e, 0x00]);
            } else if let Some(h) = to_half(*f) {
                out.push(0xf9);
                out.extend(h.to_be_bytes());
            } else if (*f as f32) as f64 == *f {
                out.push(0xfa);
                out.extend((*f as f32).to_bits().to_be_bytes());
            } else {
                out.push(0xfb);
                out.extend(f.to_bits().to_be_bytes());
            }
        }
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
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        if self.b.len() - self.i < n {
            return Err("truncated value".into());
        }
        let s = &self.b[self.i..self.i + n];
        self.i += n;
        Ok(s)
    }

    fn arg(&mut self, info: u8) -> Result<u64, String> {
        Ok(match info {
            0..=23 => u64::from(info),
            24 => u64::from(self.take(1)?[0]),
            25 => u64::from(u16::from_be_bytes(self.take(2)?.try_into().expect("2 bytes"))),
            26 => u64::from(u32::from_be_bytes(self.take(4)?.try_into().expect("4 bytes"))),
            27 => u64::from_be_bytes(self.take(8)?.try_into().expect("8 bytes")),
            _ => return Err("indefinite lengths are not allowed".into()),
        })
    }

    fn len(&mut self, info: u8) -> Result<usize, String> {
        let n = self.arg(info)?;
        if n > (self.b.len() - self.i) as u64 {
            return Err("length larger than the input".into());
        }
        Ok(n as usize)
    }

    fn text(&mut self, info: u8) -> Result<String, String> {
        let n = self.len(info)?;
        String::from_utf8(self.take(n)?.to_vec()).map_err(|_| "text is not UTF-8".to_string())
    }

    fn value(&mut self, depth: usize) -> Result<Value, String> {
        if depth > 32 {
            return Err("nested too deeply".into());
        }
        let ib = self.take(1)?[0];
        let (major, info) = (ib >> 5, ib & 0x1f);
        match major {
            0 => i64::try_from(self.arg(info)?).map(Value::Int).map_err(|_| "integer out of range".to_string()),
            1 => i64::try_from(self.arg(info)?).map(|n| Value::Int(!n)).map_err(|_| "integer out of range".to_string()),
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
                for _ in 0..n {
                    let kb = self.take(1)?[0];
                    if kb >> 5 != 3 {
                        return Err("map keys must be text".into());
                    }
                    let key = self.text(kb & 0x1f)?;
                    out.push((key, self.value(depth + 1)?));
                }
                Ok(Value::Map(out))
            }
            6 => Err("tags are not allowed".into()),
            _ => match info {
                20 => Ok(Value::Bool(false)),
                21 => Ok(Value::Bool(true)),
                22 => Ok(Value::Null),
                25 => Ok(Value::Float(from_half(u16::from_be_bytes(self.take(2)?.try_into().expect("2 bytes"))))),
                26 => Ok(Value::Float(f64::from(f32::from_bits(u32::from_be_bytes(
                    self.take(4)?.try_into().expect("4 bytes"),
                ))))),
                27 => Ok(Value::Float(f64::from_bits(u64::from_be_bytes(self.take(8)?.try_into().expect("8 bytes"))))),
                _ => Err("unsupported simple value".into()),
            },
        }
    }
}
