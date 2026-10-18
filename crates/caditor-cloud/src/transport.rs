use std::io::Write;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
}

#[derive(Clone, PartialEq, Eq)]
pub enum Body {
    Empty,
    Form(String),
    Json(String),
    JsonApi(String),
    Bytes(Vec<u8>),
}

impl std::fmt::Debug for Body {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => formatter.write_str("Empty"),
            Self::Form(text) => formatter.debug_tuple("Form").field(text).finish(),
            Self::Json(text) => formatter.debug_tuple("Json").field(text).finish(),
            Self::JsonApi(text) => formatter.debug_tuple("JsonApi").field(text).finish(),
            Self::Bytes(bytes) => write!(formatter, "Bytes({} bytes)", bytes.len()),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Request {
    pub method: Method,
    pub url: String,
    pub bearer: Option<String>,
    pub body: Body,
}

impl Request {
    pub fn get(url: String) -> Self {
        Self {
            method: Method::Get,
            url,
            bearer: None,
            body: Body::Empty,
        }
    }

    pub fn post(url: String, body: Body) -> Self {
        Self {
            method: Method::Post,
            url,
            bearer: None,
            body,
        }
    }

    pub fn put(url: String, body: Body) -> Self {
        Self {
            method: Method::Put,
            url,
            bearer: None,
            body,
        }
    }

    pub fn with_bearer(mut self, token: &str) -> Self {
        self.bearer = Some(token.to_owned());
        self
    }
}

impl std::fmt::Debug for Request {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Request")
            .field("method", &self.method)
            .field("url", &self.url)
            .field("authorised", &self.bearer.is_some())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
    pub retry_after_seconds: Option<u64>,
}

impl Response {
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fetched {
    pub status: u16,
    pub retry_after_seconds: Option<u64>,
    pub bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TransportError {
    #[error("caditor could not reach the service. Check the internet connection.")]
    Unreachable,
    #[error("The service did not answer in time. Try again.")]
    TimedOut,
    #[error("The connection to the service could not be made secure.")]
    Insecure,
    #[error("The service sent an answer caditor could not follow.")]
    Malformed,
    #[error("The service sent more than caditor is willing to take.")]
    TooLarge,
    #[error("The transfer was stopped.")]
    Stopped,
    #[error("The connection to the service failed. Check the internet connection and try again.")]
    Io(std::io::ErrorKind),
}

pub trait Transport: Send + Sync {
    fn send(&self, request: &Request) -> Result<Response, TransportError>;

    fn fetch(
        &self,
        request: &Request,
        sink: &mut dyn Write,
        progress: &mut dyn FnMut(u64, Option<u64>) -> bool,
    ) -> Result<Fetched, TransportError>;
}
