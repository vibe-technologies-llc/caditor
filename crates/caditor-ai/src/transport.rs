use std::{fmt, sync::atomic::AtomicBool};

use crate::AiError;

#[derive(Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl HttpRequest {
    pub fn post_json(url: String, body: String) -> Self {
        Self {
            url,
            headers: vec![("Content-Type".to_owned(), "application/json".to_owned())],
            body,
        }
    }

    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }
}

impl fmt::Debug for HttpRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HttpRequest")
            .field("url", &self.url)
            .field("header_count", &self.headers.len())
            .field("body_bytes", &self.body.len())
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamStatus {
    pub status: u16,
    pub retry_after_seconds: Option<u64>,
    pub error_body: Option<String>,
}

pub trait Transport: Send + Sync {
    fn post_stream(
        &self,
        request: &HttpRequest,
        cancel: &AtomicBool,
        on_line: &mut dyn FnMut(&str) -> Result<(), AiError>,
    ) -> Result<StreamStatus, AiError>;
}
