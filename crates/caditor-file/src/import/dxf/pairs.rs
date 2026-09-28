use crate::import::ImportError;

const BINARY_SENTINEL: &[u8] = b"AutoCAD Binary DXF\r\n\x1a\0";
const EXTENDED_CODE: u8 = 255;

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Value {
    Text(String),
    Real(f64),
    Integer(i64),
    Bytes,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Pair {
    pub code: i32,
    pub value: Value,
}

impl Pair {
    pub fn text(&self) -> &str {
        match &self.value {
            Value::Text(text) => text.as_str(),
            Value::Real(_) | Value::Integer(_) | Value::Bytes => "",
        }
    }

    pub fn real(&self) -> Option<f64> {
        let value = match &self.value {
            Value::Real(value) => *value,
            Value::Integer(value) => *value as f64,
            Value::Text(text) => text.trim().parse().ok()?,
            Value::Bytes => return None,
        };
        value.is_finite().then_some(value)
    }

    pub fn integer(&self) -> Option<i64> {
        match &self.value {
            Value::Integer(value) => Some(*value),
            Value::Real(value) => Some(*value as i64),
            Value::Text(text) => {
                let text = text.trim();
                text.parse()
                    .ok()
                    .or_else(|| text.parse::<f64>().ok().map(|value| value as i64))
            }
            Value::Bytes => None,
        }
    }
}

pub(super) fn read_pairs(bytes: &[u8]) -> Result<Vec<Pair>, ImportError> {
    match bytes.strip_prefix(BINARY_SENTINEL) {
        Some(rest) => Binary::new(rest).read(),
        None => read_text(bytes),
    }
}

fn read_text(bytes: &[u8]) -> Result<Vec<Pair>, ImportError> {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    let mut lines = bytes.split(|byte| *byte == b'\n').enumerate();
    let mut pairs = Vec::new();
    while let Some((index, code_line)) = lines.next() {
        let code_text = decode(code_line);
        let code_text = code_text.trim();
        if code_text.is_empty() {
            if lines
                .clone()
                .all(|(_, line)| decode(line).trim().is_empty())
            {
                break;
            }
            return Err(damaged(index));
        }
        let code: i32 = code_text.parse().map_err(|_| damaged(index))?;
        let Some((_, value_line)) = lines.next() else {
            return Err(damaged(index));
        };
        let value = decode(value_line);
        let value = value.strip_suffix('\r').unwrap_or(&value).to_owned();
        let is_end = code == 0 && value.trim().eq_ignore_ascii_case("EOF");
        pairs.push(Pair {
            code,
            value: Value::Text(value),
        });
        if is_end {
            break;
        }
    }
    Ok(pairs)
}

fn decode(line: &[u8]) -> String {
    match std::str::from_utf8(line) {
        Ok(text) => text.to_owned(),
        Err(_) => line.iter().map(|byte| char::from(*byte)).collect(),
    }
}

fn damaged(index: usize) -> ImportError {
    ImportError::DamagedAt(index + 1)
}

struct Binary<'a> {
    bytes: &'a [u8],
    position: usize,
    wide_codes: bool,
}

impl<'a> Binary<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        let wide_codes = bytes.get(1) == Some(&0);
        Self {
            bytes,
            position: 0,
            wide_codes,
        }
    }

    fn read(mut self) -> Result<Vec<Pair>, ImportError> {
        let mut pairs = Vec::new();
        while self.position < self.bytes.len() {
            let code = self.code()?;
            let value = self.value(code)?;
            let is_end = code == 0 && matches!(&value, Value::Text(text) if text == "EOF");
            pairs.push(Pair { code, value });
            if is_end {
                break;
            }
        }
        Ok(pairs)
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], ImportError> {
        let end = self.position.checked_add(count).ok_or(Self::damaged())?;
        let taken = self.bytes.get(self.position..end).ok_or(Self::damaged())?;
        self.position = end;
        Ok(taken)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], ImportError> {
        self.take(N)?.try_into().map_err(|_| Self::damaged())
    }

    fn damaged() -> ImportError {
        ImportError::Damaged
    }

    fn code(&mut self) -> Result<i32, ImportError> {
        if self.wide_codes {
            return Ok(i32::from(i16::from_le_bytes(self.array()?)));
        }
        let [byte] = self.array()?;
        if byte == EXTENDED_CODE {
            Ok(i32::from(i16::from_le_bytes(self.array()?)))
        } else {
            Ok(i32::from(byte))
        }
    }

    fn value(&mut self, code: i32) -> Result<Value, ImportError> {
        Ok(match value_kind(code) {
            Kind::Text => {
                let length = self
                    .bytes
                    .get(self.position..)
                    .and_then(|rest| rest.iter().position(|byte| *byte == 0))
                    .ok_or(Self::damaged())?;
                let text = decode(self.take(length)?);
                self.take(1)?;
                Value::Text(text)
            }
            Kind::Real => Value::Real(f64::from_le_bytes(self.array()?)),
            Kind::Short => Value::Integer(i64::from(i16::from_le_bytes(self.array()?))),
            Kind::Int => Value::Integer(i64::from(i32::from_le_bytes(self.array()?))),
            Kind::Long => Value::Integer(i64::from_le_bytes(self.array()?)),
            Kind::Bool => {
                let [byte] = self.array()?;
                Value::Integer(i64::from(byte))
            }
            Kind::Bytes => {
                let [length] = self.array()?;
                self.take(usize::from(length))?;
                Value::Bytes
            }
        })
    }
}

enum Kind {
    Text,
    Real,
    Short,
    Int,
    Long,
    Bool,
    Bytes,
}

fn value_kind(code: i32) -> Kind {
    match code {
        10..=59 | 110..=149 | 210..=239 | 460..=469 | 1010..=1059 => Kind::Real,
        60..=79 | 170..=179 | 270..=289 | 370..=389 | 400..=409 | 1060..=1070 => Kind::Short,
        90..=99 | 420..=429 | 440..=459 | 1071 => Kind::Int,
        160..=169 => Kind::Long,
        290..=299 => Kind::Bool,
        310..=319 | 1004 => Kind::Bytes,
        _ => Kind::Text,
    }
}
