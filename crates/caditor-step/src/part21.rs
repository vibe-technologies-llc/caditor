use std::borrow::Cow;

const MAX_NESTING: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Name<'a> {
    Borrowed(&'a str),
    Owned(Box<Box<str>>),
}

impl<'a> Name<'a> {
    fn uppercase(word: &'a str) -> Self {
        if word.bytes().any(|byte| byte.is_ascii_lowercase()) {
            Self::Owned(Box::new(word.to_ascii_uppercase().into_boxed_str()))
        } else {
            Self::Borrowed(word)
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Borrowed(word) => word,
            Self::Owned(word) => word,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Parameter<'a> {
    Integer(i64),
    Real(f64),
    Text(&'a str),
    Enumeration(Name<'a>),
    Reference(u64),
    List(Box<[Parameter<'a>]>),
    Typed(Box<(Name<'a>, Parameter<'a>)>),
    Omitted,
    Derived,
    Binary,
}

impl<'a> Parameter<'a> {
    pub fn real(&self) -> Option<f64> {
        match self {
            Self::Real(value) => Some(*value),
            Self::Integer(value) => Some(*value as f64),
            Self::Typed(typed) => typed.1.real(),
            _ => None,
        }
    }

    pub fn integer(&self) -> Option<i64> {
        match self {
            Self::Integer(value) => Some(*value),
            Self::Typed(typed) => typed.1.integer(),
            _ => None,
        }
    }

    pub fn reference(&self) -> Option<u64> {
        match self {
            Self::Reference(id) => Some(*id),
            _ => None,
        }
    }

    pub fn list(&self) -> Option<&[Parameter<'a>]> {
        match self {
            Self::List(items) => Some(items),
            _ => None,
        }
    }

    pub fn text(&self) -> Option<Cow<'a, str>> {
        match self {
            Self::Text(quoted) => Some(unquote(quoted)),
            _ => None,
        }
    }

    pub fn logical(&self) -> Option<bool> {
        match self.enumeration()? {
            "T" => Some(true),
            "F" => Some(false),
            _ => None,
        }
    }

    pub fn enumeration(&self) -> Option<&str> {
        match self {
            Self::Enumeration(value) => Some(value.as_str()),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Record<'a> {
    pub name: Name<'a>,
    pub parameters: Box<[Parameter<'a>]>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Instance<'a> {
    Simple(Record<'a>),
    Complex(Box<[Record<'a>]>),
}

impl<'a> Instance<'a> {
    pub fn record(&self, name: &str) -> Option<&Record<'a>> {
        match self {
            Self::Simple(record) => (record.name.as_str() == name).then_some(record),
            Self::Complex(records) => records.iter().find(|record| record.name.as_str() == name),
        }
    }

    #[cfg(test)]
    pub fn simple(&self) -> Option<&Record<'a>> {
        match self {
            Self::Simple(record) => Some(record),
            Self::Complex(_) => None,
        }
    }

    #[cfg(test)]
    pub fn names(&self) -> Vec<&str> {
        match self {
            Self::Simple(record) => vec![record.name.as_str()],
            Self::Complex(records) => records.iter().map(|record| record.name.as_str()).collect(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Exchange<'a> {
    pub header: Vec<Record<'a>>,
    data: Vec<(u64, Instance<'a>)>,
    pub unreadable: Vec<usize>,
    pub repeated: Vec<u64>,
    pub header_damaged: bool,
    pub trailer_missing: bool,
}

impl<'a> Exchange<'a> {
    pub fn instance(&self, id: u64) -> Option<&Instance<'a>> {
        let index = self
            .data
            .binary_search_by_key(&id, |(known, _)| *known)
            .ok()?;
        self.data.get(index).map(|(_, instance)| instance)
    }

    pub fn instances(&self) -> impl Iterator<Item = (u64, &Instance<'a>)> {
        self.data.iter().map(|(id, instance)| (*id, instance))
    }

    #[cfg(test)]
    pub fn ids(&self) -> Vec<u64> {
        self.data.iter().map(|(id, _)| *id).collect()
    }

    fn index(&mut self, entries: Vec<(u64, Instance<'a>)>) {
        if entries
            .windows(2)
            .all(|pair| matches!(pair, [(first, _), (second, _)] if first < second))
        {
            self.data = entries;
            return;
        }
        let mut numbered: Vec<(u64, usize, Instance<'a>)> = entries
            .into_iter()
            .enumerate()
            .map(|(position, (id, instance))| (id, position, instance))
            .collect();
        numbered.sort_unstable_by_key(|(id, position, _)| (*id, *position));
        let mut repeats = Vec::new();
        self.data = Vec::with_capacity(numbered.len());
        for (id, position, instance) in numbered {
            if self.data.last().is_some_and(|(last, _)| *last == id) {
                repeats.push((position, id));
            } else {
                self.data.push((id, instance));
            }
        }
        repeats.sort_unstable();
        self.repeated = repeats.into_iter().map(|(_, id)| id).collect();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SyntaxError {
    NotStep,
    Damaged { line: usize },
}

#[derive(Debug, Clone, PartialEq)]
enum Token<'a> {
    Keyword(Name<'a>),
    Reference(u64),
    Integer(i64),
    Real(f64),
    Text(&'a str),
    Enumeration(Name<'a>),
    Binary,
    Open,
    Close,
    Comma,
    Semicolon,
    Equals,
    Dollar,
    Star,
}

impl Token<'_> {
    fn is_keyword(&self, word: &str) -> bool {
        matches!(self, Self::Keyword(name) if name.as_str() == word)
    }
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

    fn next(&mut self) -> Result<Option<Token<'a>>, SyntaxError> {
        let token = self.token();
        self.after_semicolon = matches!(token, Ok(Some(Token::Semicolon)));
        token
    }

    fn token(&mut self) -> Result<Option<Token<'a>>, SyntaxError> {
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
                Token::Reference(digits.parse().map_err(|_| self.damaged())?)
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
                Token::Enumeration(Name::uppercase(name))
            }
            b'+' | b'-' | b'0'..=b'9' => self.number()?,
            byte if byte.is_ascii_alphabetic() || byte == b'!' || byte == b'_' => {
                let word = self.take_while(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'!')
                });
                Token::Keyword(Name::uppercase(word))
            }
            _ => return Err(self.damaged()),
        };
        Ok(Some(token))
    }

    fn single(&mut self, token: Token<'a>) -> Token<'a> {
        self.bump();
        token
    }

    fn take_while(&mut self, keep: impl Fn(u8) -> bool) -> &'a str {
        let start = self.position;
        while self.peek_byte().is_some_and(&keep) {
            self.bump();
        }
        self.text.get(start..self.position).unwrap_or_default()
    }

    fn number(&mut self) -> Result<Token<'a>, SyntaxError> {
        let text = self.take_while(|byte| {
            byte.is_ascii_digit() || matches!(byte, b'+' | b'-' | b'.' | b'E' | b'e')
        });
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

    fn text(&mut self) -> Result<&'a str, SyntaxError> {
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
        Ok(self.text.get(start..end).unwrap_or_default())
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

enum Statement<'a> {
    Instance(u64, Instance<'a>),
    End,
}

struct Parser<'a> {
    lexer: Lexer<'a>,
    lookahead: Option<Token<'a>>,
}

impl<'a> Parser<'a> {
    fn peek(&mut self) -> Result<Option<&Token<'a>>, SyntaxError> {
        if self.lookahead.is_none() {
            self.lookahead = self.lexer.next()?;
        }
        Ok(self.lookahead.as_ref())
    }

    fn take(&mut self) -> Result<Token<'a>, SyntaxError> {
        match self.lookahead.take() {
            Some(token) => Ok(token),
            None => self.lexer.next()?.ok_or(self.lexer.damaged()),
        }
    }

    fn expect(&mut self, expected: &Token<'_>) -> Result<(), SyntaxError> {
        if self.take()? == *expected {
            Ok(())
        } else {
            Err(self.lexer.damaged())
        }
    }

    fn keyword(&mut self) -> Result<Name<'a>, SyntaxError> {
        match self.take()? {
            Token::Keyword(word) => Ok(word),
            _ => Err(self.lexer.damaged()),
        }
    }

    fn record(&mut self, name: Name<'a>) -> Result<Record<'a>, SyntaxError> {
        self.expect(&Token::Open)?;
        let parameters = self.parameters(0)?.into_boxed_slice();
        Ok(Record { name, parameters })
    }

    fn parameters(&mut self, depth: usize) -> Result<Vec<Parameter<'a>>, SyntaxError> {
        if depth > MAX_NESTING {
            return Err(self.lexer.damaged());
        }
        let mut parameters = Vec::new();
        if self.peek()? == Some(&Token::Close) {
            self.take()?;
            return Ok(parameters);
        }
        loop {
            parameters.push(self.parameter(depth)?);
            match self.take()? {
                Token::Comma => {}
                Token::Close => return Ok(parameters),
                _ => return Err(self.lexer.damaged()),
            }
        }
    }

    fn parameter(&mut self, depth: usize) -> Result<Parameter<'a>, SyntaxError> {
        Ok(match self.take()? {
            Token::Integer(value) => Parameter::Integer(value),
            Token::Real(value) => Parameter::Real(value),
            Token::Text(text) => Parameter::Text(text),
            Token::Enumeration(value) => Parameter::Enumeration(value),
            Token::Reference(id) => Parameter::Reference(id),
            Token::Binary => Parameter::Binary,
            Token::Dollar => Parameter::Omitted,
            Token::Star => Parameter::Derived,
            Token::Open => Parameter::List(self.parameters(depth + 1)?.into_boxed_slice()),
            Token::Keyword(name) => {
                self.expect(&Token::Open)?;
                let mut inner = self.parameters(depth + 1)?;
                let value = if inner.len() == 1 {
                    inner.pop().unwrap_or(Parameter::Omitted)
                } else {
                    Parameter::List(inner.into_boxed_slice())
                };
                Parameter::Typed(Box::new((name, value)))
            }
            _ => return Err(self.lexer.damaged()),
        })
    }

    fn statement(&mut self) -> Result<Statement<'a>, SyntaxError> {
        match self.take()? {
            Token::Reference(id) => {
                self.expect(&Token::Equals)?;
                let instance = self.instance()?;
                self.expect(&Token::Semicolon)?;
                Ok(Statement::Instance(id, instance))
            }
            token if token.is_keyword("ENDSEC") => {
                self.expect(&Token::Semicolon)?;
                Ok(Statement::End)
            }
            _ => Err(self.lexer.damaged()),
        }
    }

    fn header(&mut self, exchange: &mut Exchange<'a>) -> Result<(), SyntaxError> {
        loop {
            let name = self.keyword()?;
            if name.as_str() == "ENDSEC" {
                return self.expect(&Token::Semicolon);
            }
            let record = self.record(name)?;
            self.expect(&Token::Semicolon)?;
            exchange.header.push(record);
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

    fn instance(&mut self) -> Result<Instance<'a>, SyntaxError> {
        match self.take()? {
            Token::Keyword(name) => Ok(Instance::Simple(self.record(name)?)),
            Token::Open => {
                let mut records = Vec::new();
                loop {
                    match self.take()? {
                        Token::Keyword(name) => records.push(self.record(name)?),
                        Token::Close => break,
                        _ => return Err(self.lexer.damaged()),
                    }
                }
                records.sort_by(|a, b| a.name.as_str().cmp(b.name.as_str()));
                Ok(Instance::Complex(records.into_boxed_slice()))
            }
            _ => Err(self.lexer.damaged()),
        }
    }
}

pub(crate) fn parse(text: &str) -> Result<Exchange<'_>, SyntaxError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut parser = Parser {
        lexer: Lexer::new(text),
        lookahead: None,
    };
    match parser.take() {
        Ok(token) if token.is_keyword("ISO-10303-21") => {}
        _ => return Err(SyntaxError::NotStep),
    }
    parser.expect(&Token::Semicolon)?;
    let mut exchange = Exchange::default();
    let mut entries = Vec::new();
    loop {
        if parser.peek()?.is_none() {
            exchange.trailer_missing = true;
            exchange.index(entries);
            return Ok(exchange);
        }
        match parser.keyword()?.as_str() {
            "HEADER" => {
                parser.expect(&Token::Semicolon)?;
                let section = parser.lexer.mark();
                if parser.header(&mut exchange).is_err() {
                    exchange.header_damaged = true;
                    exchange.header.clear();
                    parser.lookahead = None;
                    parser.lexer.reset(section);
                    parser.lexer.skip_section()?;
                }
            }
            "DATA" => {
                if parser.peek()? == Some(&Token::Open) {
                    parser.take()?;
                    parser.parameters(0)?;
                }
                parser.expect(&Token::Semicolon)?;
                loop {
                    let line = parser.next_line();
                    match parser.statement() {
                        Ok(Statement::Instance(id, instance)) => entries.push((id, instance)),
                        Ok(Statement::End) => break,
                        Err(_) => {
                            exchange.unreadable.push(line);
                            parser.recover()?;
                        }
                    }
                }
            }
            "ANCHOR" | "REFERENCE" | "SIGNATURE" => {
                parser.expect(&Token::Semicolon)?;
                if parser.lookahead.is_some() {
                    return Err(parser.lexer.damaged());
                }
                parser.lexer.skip_section()?;
            }
            "END-ISO-10303-21" => {
                exchange.index(entries);
                return Ok(exchange);
            }
            _ => return Err(parser.lexer.damaged()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_lists_and_typed_values_are_read() {
        let text = "ISO-10303-21;\nHEADER;\nFILE_NAME('a''b',/* note */'2026');\nENDSEC;\nDATA;\n\
                    #1=CARTESIAN_POINT('',(1.,-2.5E-1,3));\n\
                    #20 = ( LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.MILLI.,.METRE.) );\n\
                    #3=UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE(1.E-07),#20,'d','c');\n\
                    #4=ORIENTED_EDGE('',*,*,#5,.T.);\nENDSEC;\nEND-ISO-10303-21;\n";
        let exchange = parse(text).unwrap();
        assert_eq!(exchange.header[0].parameters[0].text().unwrap(), "a'b");
        let point = exchange.instance(1).unwrap().simple().unwrap();
        assert_eq!(point.name.as_str(), "CARTESIAN_POINT");
        assert_eq!(
            point.parameters[1],
            Parameter::List(Box::new([
                Parameter::Real(1.0),
                Parameter::Real(-0.25),
                Parameter::Integer(3)
            ]))
        );
        assert_eq!(
            exchange.instance(20).unwrap().names(),
            ["LENGTH_UNIT", "NAMED_UNIT", "SI_UNIT"]
        );
        let unit = exchange.instance(20).unwrap().record("SI_UNIT").unwrap();
        assert_eq!(unit.parameters[0].enumeration(), Some("MILLI"));
        let measure = exchange.instance(3).unwrap().simple().unwrap();
        assert_eq!(measure.parameters[0].real(), Some(1e-7));
        let edge = exchange.instance(4).unwrap().simple().unwrap();
        assert_eq!(edge.parameters[1], Parameter::Derived);
        assert_eq!(edge.parameters[4].logical(), Some(true));
    }

    #[test]
    fn names_and_plain_text_borrow_from_the_file_and_the_rest_is_decoded() {
        let text = "ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\n\
                    #1=cartesian_point('plain',(0.,0.,0.));\n\
                    #2=PRODUCT('it''s','K\\X2\\00FC\\X0\\hler','two\nlines',(.t.,.F.));\n\
                    ENDSEC;\nEND-ISO-10303-21;\n";

        let exchange = parse(text).unwrap();
        let point = exchange.instance(1).unwrap().simple().unwrap();
        let product = exchange.instance(2).unwrap().simple().unwrap();
        let flags = product.parameters[3].list().unwrap();

        assert_eq!(point.name, Name::Owned(Box::new("CARTESIAN_POINT".into())));
        assert_eq!(point.parameters[0].text(), Some(Cow::Borrowed("plain")));
        assert_eq!(product.name, Name::Borrowed("PRODUCT"));
        assert_eq!(product.parameters[0].text().unwrap(), "it's");
        assert_eq!(product.parameters[1].text().unwrap(), "Kühler");
        assert_eq!(product.parameters[2].text().unwrap(), "twolines");
        assert_eq!(flags[0].logical(), Some(true));
        assert_eq!(flags[1], Parameter::Enumeration(Name::Borrowed("F")));
    }

    #[test]
    fn numbers_ending_in_a_point_or_with_a_bare_exponent_are_read() {
        let text = "ISO-10303-21;\nDATA;\n#1=A((1.,-2.E1,3.e-1,-.,+.5,7,99999999999999999999));\n\
                    ENDSEC;\nEND-ISO-10303-21;\n";

        let exchange = parse(text).unwrap();
        let values: Vec<Option<f64>> = exchange.instance(1).unwrap().simple().unwrap().parameters
            [0]
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

        assert_eq!(exchange.ids(), [2, 4]);
        assert_eq!(exchange.unreadable, [5, 7]);
        assert_eq!(exchange.repeated, [2]);
        let x = |id: u64| {
            exchange.instance(id).unwrap().simple().unwrap().parameters[1]
                .list()
                .unwrap()[0]
                .real()
        };
        assert_eq!(x(2), Some(0.0));
        assert_eq!(x(4), Some(99_999_999_999_999_999_999.0));
    }

    #[test]
    fn entities_out_of_order_are_found_and_repeats_are_listed_in_file_order() {
        let text = "ISO-10303-21;\nDATA;\n#9=A(1);#3=A(2);#9=A(3);#5=A(4);#3=A(5);#9=A(6);\n\
                    ENDSEC;\nEND-ISO-10303-21;\n";

        let exchange = parse(text).unwrap();
        let first =
            |id: u64| exchange.instance(id).unwrap().simple().unwrap().parameters[0].integer();

        assert_eq!(exchange.ids(), [3, 5, 9]);
        assert_eq!(exchange.repeated, [9, 3, 9]);
        assert_eq!([first(3), first(5), first(9)], [Some(2), Some(4), Some(1)]);
        assert_eq!(exchange.instance(4), None);
    }

    #[test]
    fn damage_is_located_and_other_files_are_refused() {
        assert_eq!(parse("solid cube\nfacet"), Err(SyntaxError::NotStep));
        let unterminated =
            parse("ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\n#1=LINE('',#2,#3;\nENDSEC;");
        assert_eq!(unterminated.unwrap().unreadable, [5]);
        assert_eq!(
            parse("ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\n#1=LINE('',#2,'#3);\n"),
            Err(SyntaxError::Damaged { line: 6 })
        );
        let deep = format!(
            "ISO-10303-21;DATA;#1=A({}{});ENDSEC;END-ISO-10303-21;",
            "(".repeat(200),
            ")".repeat(200)
        );
        let nested = parse(&deep).unwrap();
        assert!(nested.ids().is_empty());
        assert_eq!(nested.unreadable, [1]);
    }

    #[test]
    fn a_damaged_header_is_skipped_and_a_missing_trailer_accepted() {
        let broken = "ISO-10303-21;\nHEADER;\nFILE_NAME('a;b','2026'\nFILE_SCHEMA(('x'));\nENDSEC;\n\
                      DATA;\n#1=A(1);\nENDSEC;\nEND-ISO-10303-21;\n";
        let exchange = parse(broken).unwrap();
        assert!(exchange.header_damaged && exchange.header.is_empty());
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
