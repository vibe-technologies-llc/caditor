use std::collections::BTreeMap;

const MAX_NESTING: usize = 64;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Parameter {
    Integer(i64),
    Real(f64),
    Text(String),
    Enumeration(String),
    Reference(u64),
    List(Vec<Parameter>),
    Typed(String, Box<Parameter>),
    Omitted,
    Derived,
    Binary,
}

impl Parameter {
    pub fn real(&self) -> Option<f64> {
        match self {
            Self::Real(value) => Some(*value),
            Self::Integer(value) => Some(*value as f64),
            Self::Typed(_, inner) => inner.real(),
            _ => None,
        }
    }

    pub fn integer(&self) -> Option<i64> {
        match self {
            Self::Integer(value) => Some(*value),
            Self::Typed(_, inner) => inner.integer(),
            _ => None,
        }
    }

    pub fn reference(&self) -> Option<u64> {
        match self {
            Self::Reference(id) => Some(*id),
            _ => None,
        }
    }

    pub fn list(&self) -> Option<&[Parameter]> {
        match self {
            Self::List(items) => Some(items),
            _ => None,
        }
    }

    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
            _ => None,
        }
    }

    pub fn logical(&self) -> Option<bool> {
        match self {
            Self::Enumeration(value) => match value.as_str() {
                "T" => Some(true),
                "F" => Some(false),
                _ => None,
            },
            _ => None,
        }
    }

    pub fn enumeration(&self) -> Option<&str> {
        match self {
            Self::Enumeration(value) => Some(value),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Record {
    pub name: String,
    pub parameters: Vec<Parameter>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Instance {
    Simple(Record),
    Complex(Vec<Record>),
}

impl Instance {
    pub fn record(&self, name: &str) -> Option<&Record> {
        match self {
            Self::Simple(record) => (record.name == name).then_some(record),
            Self::Complex(records) => records.iter().find(|record| record.name == name),
        }
    }

    #[cfg(test)]
    pub fn simple(&self) -> Option<&Record> {
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
pub(crate) struct Exchange {
    pub header: Vec<Record>,
    pub data: BTreeMap<u64, Instance>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SyntaxError {
    NotStep,
    Damaged { line: usize },
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Keyword(String),
    Reference(u64),
    Integer(i64),
    Real(f64),
    Text(String),
    Enumeration(String),
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
    bytes: &'a [u8],
    position: usize,
    line: usize,
}

impl<'a> Lexer<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            bytes: text.as_bytes(),
            position: 0,
            line: 1,
        }
    }

    fn damaged(&self) -> SyntaxError {
        SyntaxError::Damaged { line: self.line }
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
                Token::Enumeration(name.to_ascii_uppercase())
            }
            b'+' | b'-' | b'0'..=b'9' => self.number()?,
            byte if byte.is_ascii_alphabetic() || byte == b'!' || byte == b'_' => {
                let word = self.take_while(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'!')
                });
                Token::Keyword(word.to_ascii_uppercase())
            }
            _ => return Err(self.damaged()),
        };
        Ok(Some(token))
    }

    fn single(&mut self, token: Token) -> Token {
        self.bump();
        token
    }

    fn take_while(&mut self, keep: impl Fn(u8) -> bool) -> String {
        let start = self.position;
        while self.peek_byte().is_some_and(&keep) {
            self.bump();
        }
        String::from_utf8_lossy(self.bytes.get(start..self.position).unwrap_or_default())
            .into_owned()
    }

    fn number(&mut self) -> Result<Token, SyntaxError> {
        let text = self.take_while(|byte| {
            byte.is_ascii_digit() || matches!(byte, b'+' | b'-' | b'.' | b'E' | b'e')
        });
        if text.contains(['.', 'E', 'e']) {
            let normalized = if text.ends_with('.') {
                format!("{text}0")
            } else {
                text.replace(".E", ".0E").replace(".e", ".0e")
            };
            normalized
                .parse()
                .map(Token::Real)
                .map_err(|_| self.damaged())
        } else {
            text.parse().map(Token::Integer).map_err(|_| self.damaged())
        }
    }

    fn text(&mut self) -> Result<String, SyntaxError> {
        self.bump();
        let mut raw = Vec::new();
        loop {
            match self.bump() {
                Some(b'\'') if self.peek_byte() == Some(b'\'') => {
                    self.bump();
                    raw.push(b'\'');
                }
                Some(b'\'') => break,
                Some(b'\n' | b'\r') => {}
                Some(byte) => raw.push(byte),
                None => return Err(self.damaged()),
            }
        }
        Ok(decode_text(&String::from_utf8_lossy(&raw)))
    }
}

fn decode_text(raw: &str) -> String {
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
                .and_then(|hex| u8::from_str_radix(hex, 16).ok());
            out.push(code.map_or('?', char::from));
            rest = after.get(2..).unwrap_or_default();
        } else if let Some(after) = rest.strip_prefix("\\S\\") {
            let mut characters = after.chars();
            if let Some(character) = characters.next() {
                out.push(char::from_u32(u32::from(character) + 128).unwrap_or('?'));
            }
            rest = characters.as_str();
        } else if let Some(after) = rest.strip_prefix("\\P") {
            rest = after.get(2..).unwrap_or_default();
        } else {
            out.push('\\');
            rest = rest.get(1..).unwrap_or_default();
        }
    }
    out.push_str(rest);
    out
}

struct Parser<'a> {
    lexer: Lexer<'a>,
    lookahead: Option<Token>,
}

impl<'a> Parser<'a> {
    fn peek(&mut self) -> Result<Option<&Token>, SyntaxError> {
        if self.lookahead.is_none() {
            self.lookahead = self.lexer.next()?;
        }
        Ok(self.lookahead.as_ref())
    }

    fn take(&mut self) -> Result<Token, SyntaxError> {
        match self.lookahead.take() {
            Some(token) => Ok(token),
            None => self.lexer.next()?.ok_or(self.lexer.damaged()),
        }
    }

    fn expect(&mut self, expected: &Token) -> Result<(), SyntaxError> {
        if self.take()? == *expected {
            Ok(())
        } else {
            Err(self.lexer.damaged())
        }
    }

    fn keyword(&mut self) -> Result<String, SyntaxError> {
        match self.take()? {
            Token::Keyword(word) => Ok(word),
            _ => Err(self.lexer.damaged()),
        }
    }

    fn record(&mut self, name: String) -> Result<Record, SyntaxError> {
        self.expect(&Token::Open)?;
        let parameters = self.parameters(0)?;
        Ok(Record { name, parameters })
    }

    fn parameters(&mut self, depth: usize) -> Result<Vec<Parameter>, SyntaxError> {
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

    fn parameter(&mut self, depth: usize) -> Result<Parameter, SyntaxError> {
        Ok(match self.take()? {
            Token::Integer(value) => Parameter::Integer(value),
            Token::Real(value) => Parameter::Real(value),
            Token::Text(text) => Parameter::Text(text),
            Token::Enumeration(value) => Parameter::Enumeration(value),
            Token::Reference(id) => Parameter::Reference(id),
            Token::Binary => Parameter::Binary,
            Token::Dollar => Parameter::Omitted,
            Token::Star => Parameter::Derived,
            Token::Open => Parameter::List(self.parameters(depth + 1)?),
            Token::Keyword(name) => {
                self.expect(&Token::Open)?;
                let mut inner = self.parameters(depth + 1)?;
                let value = if inner.len() == 1 {
                    inner.pop().unwrap_or(Parameter::Omitted)
                } else {
                    Parameter::List(inner)
                };
                Parameter::Typed(name, Box::new(value))
            }
            _ => return Err(self.lexer.damaged()),
        })
    }

    fn instance(&mut self) -> Result<Instance, SyntaxError> {
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
                records.sort_by(|a, b| a.name.cmp(&b.name));
                Ok(Instance::Complex(records))
            }
            _ => Err(self.lexer.damaged()),
        }
    }
}

pub(crate) fn parse(text: &str) -> Result<Exchange, SyntaxError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut parser = Parser {
        lexer: Lexer::new(text),
        lookahead: None,
    };
    match parser.take() {
        Ok(Token::Keyword(word)) if word == "ISO-10303-21" => {}
        _ => return Err(SyntaxError::NotStep),
    }
    parser.expect(&Token::Semicolon)?;
    let mut exchange = Exchange::default();
    loop {
        match parser.keyword()?.as_str() {
            "HEADER" => {
                parser.expect(&Token::Semicolon)?;
                loop {
                    let name = parser.keyword()?;
                    if name == "ENDSEC" {
                        parser.expect(&Token::Semicolon)?;
                        break;
                    }
                    let record = parser.record(name)?;
                    parser.expect(&Token::Semicolon)?;
                    exchange.header.push(record);
                }
            }
            "DATA" => {
                if parser.peek()? == Some(&Token::Open) {
                    parser.take()?;
                    parser.parameters(0)?;
                }
                parser.expect(&Token::Semicolon)?;
                loop {
                    match parser.take()? {
                        Token::Reference(id) => {
                            parser.expect(&Token::Equals)?;
                            let instance = parser.instance()?;
                            parser.expect(&Token::Semicolon)?;
                            exchange.data.insert(id, instance);
                        }
                        Token::Keyword(word) if word == "ENDSEC" => {
                            parser.expect(&Token::Semicolon)?;
                            break;
                        }
                        _ => return Err(parser.lexer.damaged()),
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
            "END-ISO-10303-21" => return Ok(exchange),
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
        assert_eq!(
            exchange.header[0].parameters[0],
            Parameter::Text("a'b".to_owned())
        );
        let point = exchange.data[&1].simple().unwrap();
        assert_eq!(point.name, "CARTESIAN_POINT");
        assert_eq!(
            point.parameters[1],
            Parameter::List(vec![
                Parameter::Real(1.0),
                Parameter::Real(-0.25),
                Parameter::Integer(3)
            ])
        );
        assert_eq!(
            exchange.data[&20].names(),
            ["LENGTH_UNIT", "NAMED_UNIT", "SI_UNIT"]
        );
        let unit = exchange.data[&20].record("SI_UNIT").unwrap();
        assert_eq!(unit.parameters[0].enumeration(), Some("MILLI"));
        let measure = exchange.data[&3].simple().unwrap();
        assert_eq!(measure.parameters[0].real(), Some(1e-7));
        let edge = exchange.data[&4].simple().unwrap();
        assert_eq!(edge.parameters[1], Parameter::Derived);
        assert_eq!(edge.parameters[4].logical(), Some(true));
    }

    #[test]
    fn encoded_text_is_decoded() {
        assert_eq!(decode_text("K\\X2\\00FC\\X0\\hler"), "Kühler");
        assert_eq!(decode_text("\\X2\\D83DDE42\\X0\\"), "🙂");
        assert_eq!(decode_text("\\X4\\0001F642\\X0\\"), "🙂");
        assert_eq!(decode_text("caf\\X\\E9"), "café");
        assert_eq!(decode_text("\\S\\i"), "é");
        assert_eq!(decode_text("a\\\\b"), "a\\b");
    }

    #[test]
    fn damage_is_located_and_other_files_are_refused() {
        assert_eq!(parse("solid cube\nfacet"), Err(SyntaxError::NotStep));
        assert_eq!(
            parse("ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\n#1=LINE('',#2,#3;\nENDSEC;"),
            Err(SyntaxError::Damaged { line: 5 })
        );
        let deep = format!(
            "ISO-10303-21;DATA;#1=A({}{});ENDSEC;END-ISO-10303-21;",
            "(".repeat(200),
            ")".repeat(200)
        );
        assert!(matches!(parse(&deep), Err(SyntaxError::Damaged { .. })));
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
        assert_eq!(exchange.data.len(), 2);
        assert_eq!(
            parse("ISO-10303-21;\nANCHOR;\n<top>=#1;\n"),
            Err(SyntaxError::Damaged { line: 4 })
        );
    }
}
