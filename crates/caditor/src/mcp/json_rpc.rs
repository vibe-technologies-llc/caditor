use std::io::{self, BufRead, ErrorKind, Write};

use serde_json::{Map, Value, json};

pub const VERSION: &str = "2.0";
pub const MAX_LINE_BYTES: usize = 16 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Code {
    ParseError,
    InvalidRequest,
    MethodNotFound,
    InvalidParams,
    InternalError,
    ResourceNotFound,
}

impl Code {
    pub fn number(self) -> i64 {
        match self {
            Self::ParseError => -32700,
            Self::InvalidRequest => -32600,
            Self::MethodNotFound => -32601,
            Self::InvalidParams => -32602,
            Self::InternalError => -32603,
            Self::ResourceNotFound => -32002,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RequestId(Value);

impl RequestId {
    pub fn of(value: &Value) -> Option<Self> {
        match value {
            Value::String(_) | Value::Number(_) => Some(Self(value.clone())),
            Value::Null | Value::Bool(_) | Value::Array(_) | Value::Object(_) => None,
        }
    }

    pub fn key(&self) -> String {
        self.0.to_string()
    }

    pub fn value(&self) -> &Value {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    Request {
        id: RequestId,
        method: String,
        params: Value,
    },
    Notification {
        method: String,
        params: Value,
    },
    Response,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Rejection {
    #[error("the message is not UTF-8 text")]
    NotText,
    #[error("the message is not JSON (line {line}, column {column})")]
    NotJson { line: usize, column: usize },
    #[error("the message is longer than {limit} bytes")]
    TooLong { limit: usize },
    #[error("batches of messages are not supported; send one message per line")]
    Batch,
    #[error("a message must be a JSON object")]
    NotAnObject,
    #[error("the message must say \"jsonrpc\": \"2.0\"")]
    WrongVersion,
    #[error("a request id must be a string or a number")]
    UnusableId,
    #[error("the message has no method")]
    NoMethod,
    #[error("the params of a message must be an object or an array")]
    UnusableParams,
}

impl Rejection {
    pub fn code(&self) -> Code {
        match self {
            Self::NotText | Self::NotJson { .. } | Self::TooLong { .. } => Code::ParseError,
            Self::Batch
            | Self::NotAnObject
            | Self::WrongVersion
            | Self::UnusableId
            | Self::NoMethod
            | Self::UnusableParams => Code::InvalidRequest,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Rejected {
    pub id: Option<RequestId>,
    pub reason: Rejection,
}

impl Rejected {
    fn without_id(reason: Rejection) -> Self {
        Self { id: None, reason }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    Text(Vec<u8>),
    TooLong,
    End,
}

pub fn read_line(reader: &mut impl BufRead, limit: usize) -> io::Result<Line> {
    let mut line = Vec::new();
    let mut overflowed = false;
    loop {
        let available = match reader.fill_buf() {
            Ok(available) => available,
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        if available.is_empty() {
            return Ok(match (overflowed, line.is_empty()) {
                (true, _) => Line::TooLong,
                (false, true) => Line::End,
                (false, false) => Line::Text(line),
            });
        }
        let ending = available.iter().position(|byte| *byte == b'\n');
        let taken = ending.map_or(available.len(), |at| at + 1);
        if !overflowed {
            let piece = available.get(..taken).unwrap_or_default();
            if line.len().saturating_add(piece.len()) > limit.saturating_add(1) {
                overflowed = true;
                line = Vec::new();
            } else {
                line.extend_from_slice(piece);
            }
        }
        reader.consume(taken);
        if ending.is_some() {
            return Ok(if overflowed {
                Line::TooLong
            } else {
                Line::Text(line)
            });
        }
    }
}

pub fn parse(line: &[u8]) -> Result<Option<Incoming>, Rejected> {
    let text = std::str::from_utf8(line).map_err(|_| Rejected::without_id(Rejection::NotText))?;
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let value: Value = serde_json::from_str(text).map_err(|error| {
        Rejected::without_id(Rejection::NotJson {
            line: error.line(),
            column: error.column(),
        })
    })?;
    let message = match value {
        Value::Object(message) => message,
        Value::Array(_) => return Err(Rejected::without_id(Rejection::Batch)),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            return Err(Rejected::without_id(Rejection::NotAnObject));
        }
    };
    let id = match message.get("id") {
        None => None,
        Some(id) => Some(RequestId::of(id).ok_or(Rejected::without_id(Rejection::UnusableId))?),
    };
    let rejected = |reason| Rejected {
        id: id.clone(),
        reason,
    };
    if message.get("jsonrpc").and_then(Value::as_str) != Some(VERSION) {
        return Err(rejected(Rejection::WrongVersion));
    }
    let Some(method) = message.get("method") else {
        let answers = message.contains_key("result") || message.contains_key("error");
        return match (answers, &id) {
            (true, Some(_)) => Ok(Some(Incoming::Response)),
            _ => Err(rejected(Rejection::NoMethod)),
        };
    };
    let method = method
        .as_str()
        .ok_or_else(|| rejected(Rejection::NoMethod))?
        .to_owned();
    let params = match message.get("params") {
        None => Value::Null,
        Some(params @ (Value::Object(_) | Value::Array(_))) => params.clone(),
        Some(_) => return Err(rejected(Rejection::UnusableParams)),
    };
    Ok(Some(match id {
        Some(id) => Incoming::Request { id, method, params },
        None => Incoming::Notification { method, params },
    }))
}

pub fn result(id: &RequestId, result: Value) -> Value {
    json!({ "jsonrpc": VERSION, "id": id.value(), "result": result })
}

pub fn error(id: Option<&RequestId>, code: Code, message: &str) -> Value {
    json!({
        "jsonrpc": VERSION,
        "id": id.map_or(Value::Null, |id| id.value().clone()),
        "error": { "code": code.number(), "message": message },
    })
}

pub fn notification(method: &str, params: Map<String, Value>) -> Value {
    json!({ "jsonrpc": VERSION, "method": method, "params": params })
}

pub fn write_message(out: &mut impl Write, message: &Value) -> io::Result<()> {
    serde_json::to_writer(&mut *out, message)?;
    out.write_all(b"\n")?;
    out.flush()
}
