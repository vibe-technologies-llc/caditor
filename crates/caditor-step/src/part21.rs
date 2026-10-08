use std::{borrow::Cow, ops::Range};

const MAX_NESTING: usize = 64;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Span {
    start: u32,
    len: u32,
}

impl Span {
    fn of(range: Range<usize>) -> Option<Self> {
        Some(Self {
            start: u32::try_from(range.start).ok()?,
            len: u32::try_from(range.end.checked_sub(range.start)?).ok()?,
        })
    }

    fn range(self) -> Range<usize> {
        let start = self.start as usize;
        start..start + self.len as usize
    }

    fn packed(self) -> u64 {
        (u64::from(self.start) << 32) | u64::from(self.len)
    }

    fn unpacked(packed: u64) -> Self {
        Self {
            start: (packed >> 32) as u32,
            len: packed as u32,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Name {
    InText(Span),
    Uppercased(Span),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Value {
    Integer(i64),
    Real(f64),
    Text(Span),
    Enumeration(Name),
    Reference(u64),
    List(Span),
    Typed(u32),
    Omitted,
    Derived,
    Binary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Integer,
    Real,
    Text,
    Enumeration,
    UppercasedEnumeration,
    Reference,
    List,
    Typed,
    Omitted,
    Derived,
    Binary,
}

impl Value {
    fn encoded(self) -> (Kind, u64) {
        match self {
            Self::Integer(value) => (Kind::Integer, value.cast_unsigned()),
            Self::Real(value) => (Kind::Real, value.to_bits()),
            Self::Text(span) => (Kind::Text, span.packed()),
            Self::Enumeration(Name::InText(span)) => (Kind::Enumeration, span.packed()),
            Self::Enumeration(Name::Uppercased(span)) => {
                (Kind::UppercasedEnumeration, span.packed())
            }
            Self::Reference(id) => (Kind::Reference, id),
            Self::List(span) => (Kind::List, span.packed()),
            Self::Typed(index) => (Kind::Typed, u64::from(index)),
            Self::Omitted => (Kind::Omitted, 0),
            Self::Derived => (Kind::Derived, 0),
            Self::Binary => (Kind::Binary, 0),
        }
    }

    fn decoded(kind: Kind, payload: u64) -> Self {
        match kind {
            Kind::Integer => Self::Integer(payload.cast_signed()),
            Kind::Real => Self::Real(f64::from_bits(payload)),
            Kind::Text => Self::Text(Span::unpacked(payload)),
            Kind::Enumeration => Self::Enumeration(Name::InText(Span::unpacked(payload))),
            Kind::UppercasedEnumeration => {
                Self::Enumeration(Name::Uppercased(Span::unpacked(payload)))
            }
            Kind::Reference => Self::Reference(payload),
            Kind::List => Self::List(Span::unpacked(payload)),
            Kind::Typed => Self::Typed(payload as u32),
            Kind::Omitted => Self::Omitted,
            Kind::Derived => Self::Derived,
            Kind::Binary => Self::Binary,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Values {
    kinds: Vec<Kind>,
    payloads: Vec<u64>,
}

impl Values {
    fn len(&self) -> usize {
        self.kinds.len()
    }

    fn get(&self, index: usize) -> Option<Value> {
        Some(Value::decoded(
            *self.kinds.get(index)?,
            *self.payloads.get(index)?,
        ))
    }

    fn push(&mut self, value: Value) {
        let (kind, payload) = value.encoded();
        self.kinds.push(kind);
        self.payloads.push(payload);
    }

    fn truncate(&mut self, len: usize) {
        self.kinds.truncate(len);
        self.payloads.truncate(len);
    }

    fn shrink_to_fit(&mut self) {
        self.kinds.shrink_to_fit();
        self.payloads.shrink_to_fit();
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Typed {
    name: Name,
    value: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StoredRecord {
    name: Name,
    parameters: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Body {
    Simple(StoredRecord),
    Complex(Span),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Entry {
    id: u64,
    body: Body,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Exchange<'a> {
    text: &'a str,
    uppercased: String,
    values: Values,
    typed: Vec<Typed>,
    records: Vec<StoredRecord>,
    header: Span,
    data: Vec<Entry>,
    pub unreadable: Vec<usize>,
    pub repeated: Vec<u64>,
    pub header_damaged: bool,
    pub trailer_missing: bool,
    pub cut_short: bool,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Parameter<'a> {
    exchange: &'a Exchange<'a>,
    value: Value,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct List<'a> {
    exchange: &'a Exchange<'a>,
    span: Span,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Record<'a> {
    exchange: &'a Exchange<'a>,
    stored: &'a StoredRecord,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Instance<'a> {
    exchange: &'a Exchange<'a>,
    entry: &'a Entry,
}

impl<'a> Parameter<'a> {
    pub fn real(self) -> Option<f64> {
        match self.value {
            Value::Real(value) => Some(value),
            Value::Integer(value) => Some(value as f64),
            Value::Typed(_) => self.typed()?.1.real(),
            _ => None,
        }
    }

    pub fn integer(self) -> Option<i64> {
        match self.value {
            Value::Integer(value) => Some(value),
            Value::Typed(_) => self.typed()?.1.integer(),
            _ => None,
        }
    }

    pub fn reference(self) -> Option<u64> {
        match self.value {
            Value::Reference(id) => Some(id),
            _ => None,
        }
    }

    pub fn list(self) -> Option<List<'a>> {
        match self.value {
            Value::List(span) => Some(self.exchange.list(span)),
            _ => None,
        }
    }

    pub fn text(self) -> Option<Cow<'a, str>> {
        match self.value {
            Value::Text(span) => Some(unquote(self.exchange.text.get(span.range())?)),
            _ => None,
        }
    }

    pub fn logical(self) -> Option<bool> {
        match self.enumeration()? {
            "T" => Some(true),
            "F" => Some(false),
            _ => None,
        }
    }

    pub fn enumeration(self) -> Option<&'a str> {
        match self.value {
            Value::Enumeration(name) => Some(self.exchange.name(name)),
            _ => None,
        }
    }

    pub fn typed(self) -> Option<(&'a str, Parameter<'a>)> {
        let Value::Typed(index) = self.value else {
            return None;
        };
        let typed = self.exchange.typed.get(index as usize)?;
        Some((
            self.exchange.name(typed.name),
            Parameter {
                exchange: self.exchange,
                value: typed.value,
            },
        ))
    }

    #[cfg(test)]
    pub fn is_omitted(self) -> bool {
        self.value == Value::Omitted
    }

    #[cfg(test)]
    pub fn is_derived(self) -> bool {
        self.value == Value::Derived
    }
}

impl<'a> List<'a> {
    pub fn len(self) -> usize {
        self.span.len as usize
    }

    pub fn get(self, index: usize) -> Option<Parameter<'a>> {
        let index = (index < self.len()).then(|| self.span.start as usize + index)?;
        let value = self.exchange.values.get(index)?;
        Some(Parameter {
            exchange: self.exchange,
            value,
        })
    }

    pub fn first(self) -> Option<Parameter<'a>> {
        self.get(0)
    }

    pub fn iter(self) -> impl DoubleEndedIterator<Item = Parameter<'a>> + ExactSizeIterator {
        let exchange = self.exchange;
        self.span.range().map(move |index| Parameter {
            exchange,
            value: exchange.values.get(index).unwrap_or(Value::Omitted),
        })
    }
}

impl<'a> Record<'a> {
    pub fn name(self) -> &'a str {
        self.exchange.name(self.stored.name)
    }

    pub fn parameters(self) -> List<'a> {
        self.exchange.list(self.stored.parameters)
    }
}

impl<'a> Instance<'a> {
    pub fn records(self) -> impl Iterator<Item = Record<'a>> {
        let exchange = self.exchange;
        let (simple, complex) = match &self.entry.body {
            Body::Simple(stored) => (Some(stored), &[][..]),
            Body::Complex(span) => (None, exchange.records.get(span.range()).unwrap_or_default()),
        };
        simple
            .into_iter()
            .chain(complex)
            .map(move |stored| Record { exchange, stored })
    }

    pub fn record(self, name: &str) -> Option<Record<'a>> {
        self.records().find(|record| record.name() == name)
    }

    pub fn simple(self) -> Option<Record<'a>> {
        match &self.entry.body {
            Body::Simple(stored) => Some(Record {
                exchange: self.exchange,
                stored,
            }),
            Body::Complex(_) => None,
        }
    }

    #[cfg(test)]
    pub fn names(self) -> Vec<&'a str> {
        self.records().map(Record::name).collect()
    }
}

fn name_in<'s>(text: &'s str, uppercased: &'s str, name: Name) -> &'s str {
    match name {
        Name::InText(span) => text.get(span.range()),
        Name::Uppercased(span) => uppercased.get(span.range()),
    }
    .unwrap_or_default()
}

impl<'a> Exchange<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            ..Self::default()
        }
    }

    fn name(&self, name: Name) -> &str {
        name_in(self.text, &self.uppercased, name)
    }

    fn list<'e>(&'e self, span: Span) -> List<'e> {
        List {
            exchange: self,
            span: if span.range().end <= self.values.len() {
                span
            } else {
                Span::default()
            },
        }
    }

    pub fn header<'e>(&'e self) -> impl Iterator<Item = Record<'e>> {
        let exchange: &'e Exchange<'e> = self;
        exchange
            .records
            .get(exchange.header.range())
            .unwrap_or_default()
            .iter()
            .map(move |stored| Record { exchange, stored })
    }

    pub fn instance<'e>(&'e self, id: u64) -> Option<Instance<'e>> {
        let index = self.data.binary_search_by_key(&id, |entry| entry.id).ok()?;
        self.data.get(index).map(|entry| Instance {
            exchange: self,
            entry,
        })
    }

    pub fn instances<'e>(&'e self) -> impl Iterator<Item = (u64, Instance<'e>)> {
        let exchange: &'e Exchange<'e> = self;
        exchange
            .data
            .iter()
            .map(move |entry| (entry.id, Instance { exchange, entry }))
    }

    #[cfg(test)]
    pub fn ids(&self) -> Vec<u64> {
        self.data.iter().map(|entry| entry.id).collect()
    }

    fn finish(mut self, entries: Vec<Entry>) -> Self {
        self.index(entries);
        self.uppercased.shrink_to_fit();
        self.values.shrink_to_fit();
        self.typed.shrink_to_fit();
        self.records.shrink_to_fit();
        self
    }

    fn cut_short(mut self, entries: Vec<Entry>) -> Self {
        self.cut_short = true;
        self.trailer_missing = true;
        self.finish(entries)
    }

    fn index(&mut self, entries: Vec<Entry>) {
        if entries
            .windows(2)
            .all(|pair| matches!(pair, [first, second] if first.id < second.id))
        {
            self.data = entries;
            return;
        }
        let mut numbered: Vec<(usize, Entry)> = entries.into_iter().enumerate().collect();
        numbered.sort_unstable_by_key(|(position, entry)| (entry.id, *position));
        let mut repeats = Vec::new();
        self.data = Vec::with_capacity(numbered.len());
        for (position, entry) in numbered {
            if self.data.last().is_some_and(|last| last.id == entry.id) {
                repeats.push((position, entry.id));
            } else {
                self.data.push(entry);
            }
        }
        repeats.sort_unstable();
        self.repeated = repeats.into_iter().map(|(_, id)| id).collect();
    }

    fn mark(&self) -> Mark {
        Mark {
            uppercased: self.uppercased.len(),
            values: self.values.len(),
            typed: self.typed.len(),
            records: self.records.len(),
        }
    }

    fn restore(&mut self, mark: Mark) {
        self.uppercased.truncate(mark.uppercased);
        self.values.truncate(mark.values);
        self.typed.truncate(mark.typed);
        self.records.truncate(mark.records);
    }
}

#[derive(Debug, Clone, Copy)]
struct Mark {
    uppercased: usize,
    values: usize,
    typed: usize,
    records: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SyntaxError {
    NotStep,
    TooLarge,
    Damaged { line: usize },
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Token {
    Keyword(Span),
    Reference(u64),
    Integer(i64),
    Real(f64),
    Text(Span),
    Enumeration(Span),
    Binary,
    Open,
    Close,
    Comma,
    Semicolon,
    Equals,
    Dollar,
    Star,
}

struct Lexer<'a> {
    text: &'a str,
    bytes: &'a [u8],
    position: usize,
    line: usize,
    after_semicolon: bool,
}

impl<'a> Lexer<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            bytes: text.as_bytes(),
            position: 0,
            line: 1,
            after_semicolon: false,
        }
    }

    fn is_keyword(&self, token: Token, word: &str) -> bool {
        matches!(token, Token::Keyword(span) if self.word(span).eq_ignore_ascii_case(word))
    }

    fn word(&self, span: Span) -> &'a str {
        self.text.get(span.range()).unwrap_or_default()
    }

    fn span(&self, range: Range<usize>) -> Result<Span, SyntaxError> {
        Span::of(range).ok_or(SyntaxError::TooLarge)
    }

    fn skip_statement(&mut self) -> Result<(), SyntaxError> {
        let mut quoted = false;
        while let Some(byte) = self.bump() {
            if quoted {
                quoted = byte != b'\'';
                continue;
            }
            match byte {
                b'\'' => quoted = true,
                b'/' if self.peek_byte() == Some(b'*') => {
                    self.bump();
                    while let Some(inner) = self.bump() {
                        if inner == b'*' && self.peek_byte() == Some(b'/') {
                            self.bump();
                            break;
                        }
                    }
                }
                b';' => return Ok(()),
                _ => {}
            }
        }
        Err(self.damaged())
    }

    fn damaged(&self) -> SyntaxError {
        SyntaxError::Damaged { line: self.line }
    }

    fn mark(&self) -> (usize, usize, bool) {
        (self.position, self.line, self.after_semicolon)
    }

    fn reset(&mut self, (position, line, after_semicolon): (usize, usize, bool)) {
        self.position = position;
        self.line = line;
        self.after_semicolon = after_semicolon;
    }

    fn skip_section(&mut self) -> Result<(), SyntaxError> {
        let mut quoted = false;
        while let Some(&byte) = self.bytes.get(self.position) {
            if byte == b'\n' {
                self.line += 1;
            }
            if quoted {
                quoted = byte != b'\'';
                self.position += 1;
                continue;
            }
            match byte {
                b'\'' => quoted = true,
                b'/' if self.bytes.get(self.position + 1) == Some(&b'*') => {
                    self.position += 2;
                    while self.bytes.get(self.position).is_some()
                        && self.bytes.get(self.position..self.position + 2) != Some(b"*/")
                    {
                        if self.bytes.get(self.position) == Some(&b'\n') {
                            self.line += 1;
                        }
                        self.position += 1;
                    }
                }
                b'E' if self.bytes.get(self.position..self.position + 6) == Some(b"ENDSEC") => {
                    let mut after = self.position + 6;
                    while self
                        .bytes
                        .get(after)
                        .is_some_and(|byte| byte.is_ascii_whitespace())
                    {
                        after += 1;
                    }
                    if self.bytes.get(after) == Some(&b';') {
                        self.position = after + 1;
                        return Ok(());
                    }
                }
                _ => {}
            }
            self.position += 1;
        }
        Err(self.damaged())
    }

    fn peek_byte(&self) -> Option<u8> {
        self.bytes.get(self.position).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let byte = self.peek_byte()?;
        self.position += 1;
        if byte == b'\n' {
            self.line += 1;
        }
        Some(byte)
    }

    fn skip_space(&mut self) -> Result<(), SyntaxError> {
        loop {
            match self.peek_byte() {
                Some(byte) if byte.is_ascii_whitespace() => {
                    self.bump();
                }
                Some(b'/') if self.bytes.get(self.position + 1) == Some(&b'*') => {
                    self.position += 2;
                    loop {
                        match self.bump() {
                            Some(b'*') if self.peek_byte() == Some(b'/') => {
                                self.bump();
                                break;
                            }
                            Some(_) => {}
                            None => return Err(self.damaged()),
                        }
                    }
                }
                _ => return Ok(()),
            }
        }
    }

    fn next(&mut self) -> Result<Option<Token>, SyntaxError> {
        let token = self.token();
        self.after_semicolon = matches!(token, Ok(Some(Token::Semicolon)));
        token
    }

    fn token(&mut self) -> Result<Option<Token>, SyntaxError> {
        self.skip_space()?;
        let Some(byte) = self.peek_byte() else {
            return Ok(None);
        };
        let token = match byte {
            b'(' => self.single(Token::Open),
            b')' => self.single(Token::Close),
            b',' => self.single(Token::Comma),
            b';' => self.single(Token::Semicolon),
            b'=' => self.single(Token::Equals),
            b'$' => self.single(Token::Dollar),
            b'*' => self.single(Token::Star),
            b'#' => {
                self.bump();
                let digits = self.take_while(|byte| byte.is_ascii_digit());
                Token::Reference(self.slice(digits).parse().map_err(|_| self.damaged())?)
            }
            b'\'' => Token::Text(self.text()?),
            b'"' => {
                self.bump();
                self.take_while(|byte| byte != b'"');
                if self.bump() != Some(b'"') {
                    return Err(self.damaged());
                }
                Token::Binary
            }
            b'.' => {
                self.bump();
                let name = self.take_while(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
                if self.bump() != Some(b'.') {
                    return Err(self.damaged());
                }
                Token::Enumeration(self.span(name)?)
            }
            b'+' | b'-' | b'0'..=b'9' => self.number()?,
            byte if byte.is_ascii_alphabetic() || byte == b'!' || byte == b'_' => {
                let word = self.take_while(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'!')
                });
                Token::Keyword(self.span(word)?)
            }
            _ => return Err(self.damaged()),
        };
        Ok(Some(token))
    }

    fn single(&mut self, token: Token) -> Token {
        self.bump();
        token
    }

    fn slice(&self, range: Range<usize>) -> &'a str {
        self.text.get(range).unwrap_or_default()
    }

    fn take_while(&mut self, keep: impl Fn(u8) -> bool) -> Range<usize> {
        let start = self.position;
        while self.peek_byte().is_some_and(&keep) {
            self.bump();
        }
        start..self.position
    }

    fn number(&mut self) -> Result<Token, SyntaxError> {
        let range = self.take_while(|byte| {
            byte.is_ascii_digit() || matches!(byte, b'+' | b'-' | b'.' | b'E' | b'e')
        });
        let text = self.slice(range);
        if text.contains(['.', 'E', 'e']) {
            text.parse()
                .or_else(|_| {
                    let normalized = if text.ends_with('.') {
                        format!("{text}0")
                    } else {
                        text.replace(".E", ".0E").replace(".e", ".0e")
                    };
                    normalized.parse()
                })
                .map(Token::Real)
                .map_err(|_| self.damaged())
        } else {
            match text.parse() {
                Ok(integer) => Ok(Token::Integer(integer)),
                Err(_) => text.parse().map(Token::Real).map_err(|_| self.damaged()),
            }
        }
    }

    fn text(&mut self) -> Result<Span, SyntaxError> {
        self.bump();
        let start = self.position;
        loop {
            match self.bump() {
                Some(b'\'') if self.peek_byte() == Some(b'\'') => {
                    self.bump();
                }
                Some(b'\'') => break,
                Some(_) => {}
                None => return Err(self.damaged()),
            }
        }
        let end = self.position.saturating_sub(1);
        self.span(start..end)
    }
}

pub(crate) fn unquote(quoted: &str) -> Cow<'_, str> {
    if !quoted.contains(['\'', '\\', '\n', '\r']) {
        return Cow::Borrowed(quoted);
    }
    let joined: String = quoted
        .replace("''", "'")
        .chars()
        .filter(|character| !matches!(character, '\n' | '\r'))
        .collect();
    Cow::Owned(decode_text(&joined))
}

pub(crate) fn decode_text(raw: &str) -> String {
    let mut out = String::new();
    let mut rest = raw;
    while let Some(index) = rest.find('\\') {
        out.push_str(rest.get(..index).unwrap_or_default());
        rest = rest.get(index..).unwrap_or_default();
        if let Some(after) = rest.strip_prefix("\\\\") {
            out.push('\\');
            rest = after;
        } else if let Some(after) = rest.strip_prefix("\\X2\\") {
            let end = after.find("\\X0\\").unwrap_or(after.len());
            let hex = after.get(..end).unwrap_or_default();
            let units: Vec<u16> = hex
                .as_bytes()
                .chunks(4)
                .filter_map(|chunk| u16::from_str_radix(std::str::from_utf8(chunk).ok()?, 16).ok())
                .collect();
            out.push_str(&String::from_utf16_lossy(&units));
            rest = after.get(end + 4..).unwrap_or_default();
        } else if let Some(after) = rest.strip_prefix("\\X4\\") {
            let end = after.find("\\X0\\").unwrap_or(after.len());
            let hex = after.get(..end).unwrap_or_default();
            out.extend(hex.as_bytes().chunks(8).filter_map(|chunk| {
                char::from_u32(u32::from_str_radix(std::str::from_utf8(chunk).ok()?, 16).ok()?)
            }));
            rest = after.get(end + 4..).unwrap_or_default();
        } else if let Some(after) = rest.strip_prefix("\\X\\") {
            let code = after
                .get(..2)
                .filter(|hex| hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
                .and_then(|hex| u8::from_str_radix(hex, 16).ok());
            out.push(code.map_or('?', char::from));
            rest = match code {
                Some(_) => after.get(2..).unwrap_or_default(),
                None => after,
            };
        } else if let Some(after) = rest.strip_prefix("\\S\\") {
            let mut characters = after.chars();
            if let Some(character) = characters.next() {
                out.push(char::from_u32(u32::from(character) + 128).unwrap_or('?'));
            }
            rest = characters.as_str();
        } else if let Some(after) = rest.strip_prefix("\\P") {
            let mut characters = after.chars();
            characters.next();
            let page = characters.as_str();
            rest = page.strip_prefix('\\').unwrap_or(page);
        } else {
            out.push('\\');
            rest = rest.get(1..).unwrap_or_default();
        }
    }
    out.push_str(rest);
    out
}

enum Statement {
    Instance(Entry),
    End,
}

struct Parser<'a> {
    lexer: Lexer<'a>,
    lookahead: Option<Token>,
    exchange: Exchange<'a>,
    pending: Vec<Value>,
}

impl<'a> Parser<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            lexer: Lexer::new(text),
            lookahead: None,
            exchange: Exchange::new(text),
            pending: Vec::new(),
        }
    }

    fn peek(&mut self) -> Result<Option<Token>, SyntaxError> {
        if self.lookahead.is_none() {
            self.lookahead = self.lexer.next()?;
        }
        Ok(self.lookahead)
    }

    fn take(&mut self) -> Result<Token, SyntaxError> {
        match self.lookahead.take() {
            Some(token) => Ok(token),
            None => self.lexer.next()?.ok_or(self.lexer.damaged()),
        }
    }

    fn expect(&mut self, expected: Token) -> Result<(), SyntaxError> {
        if self.take()? == expected {
            Ok(())
        } else {
            Err(self.lexer.damaged())
        }
    }

    fn keyword(&mut self) -> Result<Span, SyntaxError> {
        match self.take()? {
            Token::Keyword(word) => Ok(word),
            _ => Err(self.lexer.damaged()),
        }
    }

    fn name(&mut self, span: Span) -> Result<Name, SyntaxError> {
        let word = self.lexer.word(span);
        if !word.bytes().any(|byte| byte.is_ascii_lowercase()) {
            return Ok(Name::InText(span));
        }
        let uppercased = &mut self.exchange.uppercased;
        let start = uppercased.len();
        uppercased.extend(word.chars().map(|character| character.to_ascii_uppercase()));
        let span = self.lexer.span(start..uppercased.len())?;
        Ok(Name::Uppercased(span))
    }

    fn record(&mut self, name: Span) -> Result<StoredRecord, SyntaxError> {
        self.expect(Token::Open)?;
        let name = self.name(name)?;
        let parameters = self.parameters(0)?;
        Ok(StoredRecord { name, parameters })
    }

    fn parameters(&mut self, depth: usize) -> Result<Span, SyntaxError> {
        if depth > MAX_NESTING {
            return Err(self.lexer.damaged());
        }
        let base = self.pending.len();
        if self.peek()? == Some(Token::Close) {
            self.take()?;
            return self.close_list(base);
        }
        loop {
            let value = self.parameter(depth)?;
            self.pending.push(value);
            match self.take()? {
                Token::Comma => {}
                Token::Close => return self.close_list(base),
                _ => return Err(self.lexer.damaged()),
            }
        }
    }

    fn close_list(&mut self, base: usize) -> Result<Span, SyntaxError> {
        let values = &mut self.exchange.values;
        let start = values.len();
        for value in self.pending.drain(base..) {
            values.push(value);
        }
        self.lexer.span(start..values.len())
    }

    fn parameter(&mut self, depth: usize) -> Result<Value, SyntaxError> {
        Ok(match self.take()? {
            Token::Integer(value) => Value::Integer(value),
            Token::Real(value) => Value::Real(value),
            Token::Text(text) => Value::Text(text),
            Token::Enumeration(value) => Value::Enumeration(self.name(value)?),
            Token::Reference(id) => Value::Reference(id),
            Token::Binary => Value::Binary,
            Token::Dollar => Value::Omitted,
            Token::Star => Value::Derived,
            Token::Open => Value::List(self.parameters(depth + 1)?),
            Token::Keyword(name) => {
                self.expect(Token::Open)?;
                let name = self.name(name)?;
                let inner = self.parameters(depth + 1)?;
                let only = (inner.len == 1)
                    .then(|| self.exchange.values.get(inner.start as usize))
                    .flatten();
                let value = match only {
                    Some(only) => {
                        self.exchange.values.truncate(inner.start as usize);
                        only
                    }
                    None => Value::List(inner),
                };
                let index =
                    u32::try_from(self.exchange.typed.len()).map_err(|_| SyntaxError::TooLarge)?;
                self.exchange.typed.push(Typed { name, value });
                Value::Typed(index)
            }
            _ => return Err(self.lexer.damaged()),
        })
    }

    fn records_since(&self, start: usize) -> Result<Span, SyntaxError> {
        self.lexer.span(start..self.exchange.records.len())
    }

    fn statement(&mut self) -> Result<Statement, SyntaxError> {
        match self.take()? {
            Token::Reference(id) => {
                self.expect(Token::Equals)?;
                let body = self.instance()?;
                self.expect(Token::Semicolon)?;
                Ok(Statement::Instance(Entry { id, body }))
            }
            token if self.lexer.is_keyword(token, "ENDSEC") => {
                self.expect(Token::Semicolon)?;
                Ok(Statement::End)
            }
            _ => Err(self.lexer.damaged()),
        }
    }

    fn header(&mut self) -> Result<(), SyntaxError> {
        let start = self.exchange.records.len();
        loop {
            let name = self.keyword()?;
            if self.lexer.word(name).eq_ignore_ascii_case("ENDSEC") {
                self.exchange.header = self.records_since(start)?;
                return self.expect(Token::Semicolon);
            }
            let record = self.record(name)?;
            self.expect(Token::Semicolon)?;
            self.exchange.records.push(record);
        }
    }

    fn next_line(&mut self) -> usize {
        let _ = self.peek();
        self.lexer.line
    }

    fn recover(&mut self) -> Result<(), SyntaxError> {
        match self.lookahead.take() {
            Some(Token::Semicolon) => Ok(()),
            Some(_) => self.lexer.skip_statement(),
            None if self.lexer.after_semicolon => Ok(()),
            None => self.lexer.skip_statement(),
        }
    }

    fn instance(&mut self) -> Result<Body, SyntaxError> {
        let start = self.exchange.records.len();
        match self.take()? {
            Token::Keyword(name) => Ok(Body::Simple(self.record(name)?)),
            Token::Open => {
                loop {
                    match self.take()? {
                        Token::Keyword(name) => {
                            let record = self.record(name)?;
                            self.exchange.records.push(record);
                        }
                        Token::Close => break,
                        _ => return Err(self.lexer.damaged()),
                    }
                }
                let Exchange {
                    text,
                    uppercased,
                    records,
                    ..
                } = &mut self.exchange;
                if let Some(parts) = records.get_mut(start..) {
                    parts.sort_by(|a, b| {
                        name_in(text, uppercased, a.name).cmp(name_in(text, uppercased, b.name))
                    });
                }
                Ok(Body::Complex(self.records_since(start)?))
            }
            _ => Err(self.lexer.damaged()),
        }
    }

    fn data(&mut self, entries: &mut Vec<Entry>) -> Result<bool, SyntaxError> {
        if self.peek()? == Some(Token::Open) {
            self.take()?;
            let mark = self.exchange.mark();
            self.parameters(0)?;
            self.exchange.restore(mark);
        }
        self.expect(Token::Semicolon)?;
        loop {
            let line = self.next_line();
            let ended = matches!(self.peek(), Ok(None));
            if ended {
                return Ok(false);
            }
            let mark = self.exchange.mark();
            match self.statement() {
                Ok(Statement::Instance(entry)) => entries.push(entry),
                Ok(Statement::End) => return Ok(true),
                Err(_) => {
                    self.exchange.restore(mark);
                    self.pending.clear();
                    self.exchange.unreadable.push(line);
                    if self.recover().is_err() {
                        return Ok(false);
                    }
                }
            }
        }
    }
}

pub(crate) fn parse(text: &str) -> Result<Exchange<'_>, SyntaxError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    if u32::try_from(text.len()).is_err() {
        return Err(SyntaxError::TooLarge);
    }
    let mut parser = Parser::new(text);
    match parser.take() {
        Ok(token) if parser.lexer.is_keyword(token, "ISO-10303-21") => {}
        _ => return Err(SyntaxError::NotStep),
    }
    parser.expect(Token::Semicolon)?;
    let mut entries = Vec::new();
    loop {
        if parser.peek()?.is_none() {
            parser.exchange.trailer_missing = true;
            return Ok(parser.exchange.finish(entries));
        }
        let section = parser.keyword()?;
        let word = parser.lexer.word(section);
        if word.eq_ignore_ascii_case("HEADER") {
            parser.expect(Token::Semicolon)?;
            let section = parser.lexer.mark();
            let mark = parser.exchange.mark();
            if parser.header().is_err() {
                parser.exchange.restore(mark);
                parser.pending.clear();
                parser.exchange.header = Span::default();
                parser.exchange.header_damaged = true;
                parser.lookahead = None;
                parser.lexer.reset(section);
                parser.lexer.skip_section()?;
            }
        } else if word.eq_ignore_ascii_case("DATA") {
            if !parser.data(&mut entries)? {
                return Ok(parser.exchange.cut_short(entries));
            }
        } else if ["ANCHOR", "REFERENCE", "SIGNATURE"]
            .iter()
            .any(|known| word.eq_ignore_ascii_case(known))
        {
            parser.expect(Token::Semicolon)?;
            if parser.lookahead.is_some() {
                return Err(parser.lexer.damaged());
            }
            parser.lexer.skip_section()?;
        } else if word.eq_ignore_ascii_case("END-ISO-10303-21") {
            return Ok(parser.exchange.finish(entries));
        } else {
            return Err(parser.lexer.damaged());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parameter<'a>(instance: Instance<'a>, index: usize) -> Parameter<'a> {
        instance.simple().unwrap().parameters().get(index).unwrap()
    }

    #[test]
    fn values_take_nine_bytes_and_a_simple_entity_thirty_two() {
        assert_eq!(size_of::<Kind>() + size_of::<u64>(), 9);
        assert_eq!(size_of::<Entry>(), 32);
    }

    #[test]
    fn records_lists_and_typed_values_are_read() {
        let text = "ISO-10303-21;\nHEADER;\nFILE_NAME('a''b',/* note */'2026');\nENDSEC;\nDATA;\n\
                    #1=CARTESIAN_POINT('',(1.,-2.5E-1,3));\n\
                    #20 = ( LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.MILLI.,.METRE.) );\n\
                    #3=UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE(1.E-07),#20,'d','c');\n\
                    #4=ORIENTED_EDGE('',*,*,#5,.T.);\n\
                    #5=A(B((1,2)),C(),D(E(F(7))));\nENDSEC;\nEND-ISO-10303-21;\n";
        let exchange = parse(text).unwrap();
        let header: Vec<Record<'_>> = exchange.header().collect();
        let point = exchange.instance(1).unwrap().simple().unwrap();
        let coordinates = point.parameters().get(1).unwrap().list().unwrap();
        let unit = exchange.instance(20).unwrap().record("SI_UNIT").unwrap();
        let measure = parameter(exchange.instance(3).unwrap(), 0);
        let edge = exchange.instance(4).unwrap();
        let nested = exchange.instance(5).unwrap();
        let (b, pair) = parameter(nested, 0).typed().unwrap();
        let (c, empty) = parameter(nested, 1).typed().unwrap();
        let (d, inner) = parameter(nested, 2).typed().unwrap();

        assert_eq!(header.len(), 1);
        assert_eq!(header[0].name(), "FILE_NAME");
        assert_eq!(
            header[0].parameters().first().unwrap().text().unwrap(),
            "a'b"
        );
        assert_eq!(point.name(), "CARTESIAN_POINT");
        assert_eq!(
            coordinates.iter().map(Parameter::real).collect::<Vec<_>>(),
            [Some(1.0), Some(-0.25), Some(3.0)]
        );
        assert_eq!(coordinates.get(2).unwrap().integer(), Some(3));
        assert_eq!(
            exchange.instance(20).unwrap().names(),
            ["LENGTH_UNIT", "NAMED_UNIT", "SI_UNIT"]
        );
        assert!(exchange.instance(20).unwrap().simple().is_none());
        assert!(
            exchange
                .instance(20)
                .unwrap()
                .record("NAMED_UNIT")
                .unwrap()
                .parameters()
                .first()
                .unwrap()
                .is_derived()
        );
        assert_eq!(
            unit.parameters().first().unwrap().enumeration(),
            Some("MILLI")
        );
        assert_eq!(measure.real(), Some(1e-7));
        assert_eq!(measure.typed().unwrap().0, "LENGTH_MEASURE");
        assert!(parameter(edge, 1).is_derived());
        assert_eq!(parameter(edge, 4).logical(), Some(true));
        assert_eq!(b, "B");
        assert_eq!(
            pair.list()
                .unwrap()
                .iter()
                .map(Parameter::integer)
                .collect::<Vec<_>>(),
            [Some(1), Some(2)]
        );
        assert_eq!(c, "C");
        assert_eq!(empty.list().unwrap().len(), 0);
        assert_eq!(d, "D");
        assert_eq!(inner.integer(), Some(7));
        assert_eq!(inner.typed().unwrap().1.typed().unwrap().0, "F");
    }

    #[test]
    fn names_and_plain_text_borrow_from_the_file_and_the_rest_is_decoded() {
        let text = "ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\n\
                    #1=cartesian_point('plain',(0.,0.,0.));\n\
                    #2=PRODUCT('it''s','K\\X2\\00FC\\X0\\hler','two\nlines',(.t.,.F.),$);\n\
                    ENDSEC;\nEND-ISO-10303-21;\n";

        let exchange = parse(text).unwrap();
        let point = exchange.instance(1).unwrap().simple().unwrap();
        let product = exchange.instance(2).unwrap().simple().unwrap();
        let fields = product.parameters();
        let flags = fields.get(3).unwrap().list().unwrap();

        assert_eq!(point.name(), "CARTESIAN_POINT");
        assert_eq!(exchange.uppercased, "CARTESIAN_POINTT");
        assert_eq!(
            point.parameters().first().unwrap().text(),
            Some(Cow::Borrowed("plain"))
        );
        assert_eq!(product.name(), "PRODUCT");
        assert_eq!(fields.get(0).unwrap().text().unwrap(), "it's");
        assert_eq!(fields.get(1).unwrap().text().unwrap(), "Kühler");
        assert_eq!(fields.get(2).unwrap().text().unwrap(), "twolines");
        assert!(fields.get(4).unwrap().is_omitted());
        assert_eq!(flags.get(0).unwrap().logical(), Some(true));
        assert_eq!(flags.get(1).unwrap().enumeration(), Some("F"));
    }

    #[test]
    fn numbers_ending_in_a_point_or_with_a_bare_exponent_are_read() {
        let text = "ISO-10303-21;\nDATA;\n#1=A((1.,-2.E1,3.e-1,-.,+.5,7,99999999999999999999));\n\
                    ENDSEC;\nEND-ISO-10303-21;\n";

        let exchange = parse(text).unwrap();
        let values: Vec<Option<f64>> = parameter(exchange.instance(1).unwrap(), 0)
            .list()
            .unwrap()
            .iter()
            .map(Parameter::real)
            .collect();

        assert_eq!(
            values,
            [
                Some(1.0),
                Some(-20.0),
                Some(0.3),
                Some(-0.0),
                Some(0.5),
                Some(7.0),
                Some(1e20)
            ]
        );
    }

    #[test]
    fn encoded_text_is_decoded() {
        assert_eq!(decode_text("K\\X2\\00FC\\X0\\hler"), "Kühler");
        assert_eq!(decode_text("\\X2\\D83DDE42\\X0\\"), "🙂");
        assert_eq!(decode_text("\\X4\\0001F642\\X0\\"), "🙂");
        assert_eq!(decode_text("caf\\X\\E9"), "café");
        assert_eq!(decode_text("\\S\\i"), "é");
        assert_eq!(decode_text("a\\\\b"), "a\\b");
        assert_eq!(decode_text("a\\PA\\b"), "ab");
        assert_eq!(decode_text("a\\P€\\b"), "ab");
        assert_eq!(decode_text("\\X\\€ rest"), "?€ rest");
    }

    #[test]
    fn unreadable_and_repeated_entries_are_left_out_and_counted() {
        let text = "ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\n\
                    #1=LINE('',#2,#3;\n\
                    #2=CARTESIAN_POINT('',(0.,0.,0.));\n\
                    #3=CARTESIAN_POINT('a;b',(1.,0.,0.)) @ ;\n\
                    #4=CARTESIAN_POINT('',(99999999999999999999,0.,0.));\n\
                    #2=CARTESIAN_POINT('',(5.,0.,0.));\n\
                    ENDSEC;\nEND-ISO-10303-21;\n";

        let exchange = parse(text).unwrap();
        let x = |id: u64| {
            parameter(exchange.instance(id).unwrap(), 1)
                .list()
                .unwrap()
                .first()
                .unwrap()
                .real()
        };

        assert_eq!(exchange.ids(), [2, 4]);
        assert_eq!(exchange.unreadable, [5, 7]);
        assert_eq!(exchange.repeated, [2]);
        assert_eq!(x(2), Some(0.0));
        assert_eq!(x(4), Some(99_999_999_999_999_999_999.0));
        assert_eq!(exchange.values.len(), 3 * 5);
    }

    #[test]
    fn entities_out_of_order_are_found_and_repeats_are_listed_in_file_order() {
        let text = "ISO-10303-21;\nDATA;\n#9=A(1);#3=A(2);#9=A(3);#5=A(4);#3=A(5);#9=A(6);\n\
                    ENDSEC;\nEND-ISO-10303-21;\n";

        let exchange = parse(text).unwrap();
        let first = |id: u64| parameter(exchange.instance(id).unwrap(), 0).integer();

        assert_eq!(exchange.ids(), [3, 5, 9]);
        assert_eq!(exchange.repeated, [9, 3, 9]);
        assert_eq!([first(3), first(5), first(9)], [Some(2), Some(4), Some(1)]);
        assert!(exchange.instance(4).is_none());
    }

    #[test]
    fn damage_is_located_and_other_files_are_refused() {
        assert_eq!(parse("solid cube\nfacet"), Err(SyntaxError::NotStep));
        let unterminated =
            parse("ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\n#1=LINE('',#2,#3;\nENDSEC;");
        assert_eq!(unterminated.unwrap().unreadable, [5]);
        let mid_entity =
            parse("ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\n#1=A(1);\n#2=LINE('',#2,'#3);\n")
                .unwrap();
        assert!(mid_entity.cut_short && mid_entity.trailer_missing);
        assert_eq!(mid_entity.ids(), [1]);
        assert_eq!(mid_entity.unreadable, [6]);
        let without_endsec =
            parse("ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\n#1=A(1);\n#2=A(2);\n").unwrap();
        assert!(without_endsec.cut_short);
        assert_eq!(without_endsec.ids(), [1, 2]);
        assert!(without_endsec.unreadable.is_empty());
        let mid_record = parse("ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\n#1=A(1);\n#2=A(").unwrap();
        assert!(mid_record.cut_short);
        assert_eq!(mid_record.ids(), [1]);
        let deep = format!(
            "ISO-10303-21;DATA;#1=A({}{});ENDSEC;END-ISO-10303-21;",
            "(".repeat(200),
            ")".repeat(200)
        );
        let nested = parse(&deep).unwrap();
        assert!(nested.ids().is_empty());
        assert_eq!(nested.unreadable, [1]);
        assert!(nested.values.len() == 0 && nested.records.is_empty());
    }

    #[test]
    fn a_damaged_header_is_skipped_and_a_missing_trailer_accepted() {
        let broken = "ISO-10303-21;\nHEADER;\nFILE_NAME('a;b','2026'\nFILE_SCHEMA(('x'));\nENDSEC;\n\
                      DATA;\n#1=A(1);\nENDSEC;\nEND-ISO-10303-21;\n";
        let exchange = parse(broken).unwrap();
        assert!(exchange.header_damaged && exchange.header().next().is_none());
        assert_eq!(exchange.ids(), [1]);
        assert!(!exchange.trailer_missing);

        let swallowed =
            "ISO-10303-21;\nHEADER;\nFILE_NAME('a'\nENDSEC;\nDATA;\n#1=A(1);\nENDSEC;\n";
        let exchange = parse(swallowed).unwrap();
        assert!(exchange.header_damaged && exchange.trailer_missing);
        assert_eq!(exchange.ids(), [1]);

        let intact =
            "ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\n#1=A(1);\nENDSEC;\nEND-ISO-10303-21;\n";
        let exchange = parse(intact).unwrap();
        assert!(!exchange.header_damaged && !exchange.trailer_missing);
    }

    #[test]
    fn edition_three_sections_and_named_data_sections_are_read_past() {
        let text = "ISO-10303-21;\nHEADER;\nENDSEC;\nANCHOR;\n<top>=#1;\n<'odd;ENDSEC;'>=#2;\n\
                    ENDSEC;\nREFERENCE;\n#20=<http://example.invalid/part.stp#frame>;\n\
                    /* ENDSEC; inside a comment */\nENDSEC ;\nDATA('first',());\n\
                    #1=CARTESIAN_POINT('',(0.,0.,0.));\nENDSEC;\nDATA('second',());\n\
                    #2=CARTESIAN_POINT('',(1.,0.,0.));\nENDSEC;\nSIGNATURE;\nabc\nENDSEC;\n\
                    END-ISO-10303-21;\n";
        let exchange = parse(text).unwrap();
        assert_eq!(exchange.ids().len(), 2);
        assert_eq!(
            parse("ISO-10303-21;\nANCHOR;\n<top>=#1;\n"),
            Err(SyntaxError::Damaged { line: 4 })
        );
    }
}
