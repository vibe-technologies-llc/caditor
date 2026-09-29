use std::{iter::Enumerate, slice::Split};

use crate::import::{ImportError, dxf::code_page::CodePage};

const BINARY_SENTINEL: &[u8] = b"AutoCAD Binary DXF\r\n\x1a\0";
const EXTENDED_CODE: u8 = 255;
const CODE_PAGE_VARIABLE: &[u8] = b"$DWGCODEPAGE";

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
    let page = declared_code_page(tokens(bytes));
    tokens(bytes)
        .map(|token| {
            let Token { code, value } = token?;
            Ok(Pair {
                code,
                value: match value {
                    Raw::Text(text) => Value::Text(page.decode(text).into_owned()),
                    Raw::Real(value) => Value::Real(value),
                    Raw::Integer(value) => Value::Integer(value),
                    Raw::Bytes => Value::Bytes,
                },
            })
        })
        .collect()
}

fn declared_code_page<'a>(
    tokens: impl Iterator<Item = Result<Token<'a>, ImportError>>,
) -> CodePage {
    let mut opening_section = false;
    let mut naming_page = false;
    for token in tokens {
        let Ok(Token { code, value }) = token else {
            break;
        };
        let text = match value {
            Raw::Text(text) => text.trim_ascii(),
            Raw::Real(_) | Raw::Integer(_) | Raw::Bytes => &[],
        };
        match code {
            0 if text.eq_ignore_ascii_case(b"ENDSEC") => break,
            0 => opening_section = text.eq_ignore_ascii_case(b"SECTION"),
            2 if opening_section && !text.eq_ignore_ascii_case(b"HEADER") => break,
            9 => naming_page = text.eq_ignore_ascii_case(CODE_PAGE_VARIABLE),
            3 if naming_page => {
                return std::str::from_utf8(text)
                    .ok()
                    .and_then(CodePage::named)
                    .unwrap_or(CodePage::DEFAULT);
            }
            _ => {}
        }
    }
    CodePage::DEFAULT
}

struct Token<'a> {
    code: i32,
    value: Raw<'a>,
}

enum Raw<'a> {
    Text(&'a [u8]),
    Real(f64),
    Integer(i64),
    Bytes,
}

fn tokens(bytes: &[u8]) -> Box<dyn Iterator<Item = Result<Token<'_>, ImportError>> + '_> {
    match bytes.strip_prefix(BINARY_SENTINEL) {
        Some(rest) => Box::new(Binary::new(rest)),
        None => Box::new(Text::new(bytes)),
    }
}

type Lines<'a> = Enumerate<Split<'a, u8, fn(&u8) -> bool>>;

struct Text<'a> {
    lines: Lines<'a>,
    finished: bool,
}

impl<'a> Text<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
        let newline: fn(&u8) -> bool = |byte| *byte == b'\n';
        Self {
            lines: bytes.split(newline).enumerate(),
            finished: false,
        }
    }

    fn next_token(&mut self) -> Option<Result<Token<'a>, ImportError>> {
        let (index, code_line) = self.lines.next()?;
        let code_text = code_line.trim_ascii();
        if code_text.is_empty() {
            if self
                .lines
                .clone()
                .all(|(_, line)| line.trim_ascii().is_empty())
            {
                return None;
            }
            return Some(Err(damaged(index)));
        }
        let Some(code) = std::str::from_utf8(code_text)
            .ok()
            .and_then(|text| text.parse::<i32>().ok())
        else {
            return Some(Err(damaged(index)));
        };
        let Some((_, value_line)) = self.lines.next() else {
            return Some(Err(damaged(index)));
        };
        let value = value_line.strip_suffix(b"\r").unwrap_or(value_line);
        self.finished = code == 0 && value.trim_ascii().eq_ignore_ascii_case(b"EOF");
        Some(Ok(Token {
            code,
            value: Raw::Text(value),
        }))
    }
}

impl<'a> Iterator for Text<'a> {
    type Item = Result<Token<'a>, ImportError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        let token = self.next_token();
        if !matches!(token, Some(Ok(_))) {
            self.finished = true;
        }
        token
    }
}

fn damaged(index: usize) -> ImportError {
    ImportError::DamagedAt(index + 1)
}

struct Binary<'a> {
    bytes: &'a [u8],
    position: usize,
    wide_codes: bool,
    finished: bool,
}

impl<'a> Binary<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        let wide_codes = bytes.get(1) == Some(&0);
        Self {
            bytes,
            position: 0,
            wide_codes,
            finished: false,
        }
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

    fn value(&mut self, code: i32) -> Result<Raw<'a>, ImportError> {
        Ok(match value_kind(code) {
            Kind::Text => {
                let length = self
                    .bytes
                    .get(self.position..)
                    .and_then(|rest| rest.iter().position(|byte| *byte == 0))
                    .ok_or(Self::damaged())?;
                let text = self.take(length)?;
                self.take(1)?;
                Raw::Text(text)
            }
            Kind::Real => Raw::Real(f64::from_le_bytes(self.array()?)),
            Kind::Short => Raw::Integer(i64::from(i16::from_le_bytes(self.array()?))),
            Kind::Int => Raw::Integer(i64::from(i32::from_le_bytes(self.array()?))),
            Kind::Long => Raw::Integer(i64::from_le_bytes(self.array()?)),
            Kind::Bool => {
                let [byte] = self.array()?;
                Raw::Integer(i64::from(byte))
            }
            Kind::Bytes => {
                let [length] = self.array()?;
                self.take(usize::from(length))?;
                Raw::Bytes
            }
        })
    }

    fn next_token(&mut self) -> Result<Token<'a>, ImportError> {
        let code = self.code()?;
        let value = self.value(code)?;
        self.finished = code == 0 && matches!(value, Raw::Text(b"EOF"));
        Ok(Token { code, value })
    }
}

impl<'a> Iterator for Binary<'a> {
    type Item = Result<Token<'a>, ImportError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished || self.position >= self.bytes.len() {
            return None;
        }
        let token = self.next_token();
        if token.is_err() {
            self.finished = true;
        }
        Some(token)
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
