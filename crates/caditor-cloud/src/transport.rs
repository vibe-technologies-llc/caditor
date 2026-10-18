use std::io::Write;

use crate::{encoding::percent_encode, error::Error, pkce};

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
    Multipart(Multipart),
}

#[derive(Clone, PartialEq, Eq)]
pub struct Multipart {
    boundary: String,
    fields: Vec<(String, String)>,
    file_field: String,
    file_name: String,
    bytes: Vec<u8>,
}

impl Multipart {
    pub fn new(
        fields: Vec<(String, String)>,
        file_field: &str,
        file_name: &str,
        bytes: Vec<u8>,
    ) -> Result<Self, Error> {
        Ok(Self {
            boundary: pkce::boundary()?,
            fields,
            file_field: file_field.to_owned(),
            file_name: file_name.to_owned(),
            bytes,
        })
    }

    pub fn content_type(&self) -> String {
        format!("multipart/form-data; boundary={}", self.boundary)
    }

    pub fn fields(&self) -> &[(String, String)] {
        &self.fields
    }

    pub fn file_name(&self) -> &str {
        &self.file_name
    }

    pub fn file_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn encoded(&self) -> Vec<u8> {
        let mut body = Vec::with_capacity(self.bytes.len() + 1024);
        for (name, value) in &self.fields {
            body.extend_from_slice(format!("--{}\r\n", self.boundary).as_bytes());
            body.extend_from_slice(
                format!(
                    "Content-Disposition: form-data; name=\"{}\"\r\n\r\n{value}\r\n",
                    header_safe(name)
                )
                .as_bytes(),
            );
        }
        body.extend_from_slice(format!("--{}\r\n", self.boundary).as_bytes());
        body.extend_from_slice(
            format!(
                "Content-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\n\
                 Content-Type: application/octet-stream\r\n\r\n",
                header_safe(&self.file_field),
                header_safe(&self.file_name)
            )
            .as_bytes(),
        );
        body.extend_from_slice(&self.bytes);
        body.extend_from_slice(format!("\r\n--{}--\r\n", self.boundary).as_bytes());
        body
    }
}

fn header_safe(text: &str) -> String {
    if text
        .chars()
        .all(|symbol| (' '..='~').contains(&symbol) && symbol != '"' && symbol != '\\')
    {
        text.to_owned()
    } else {
        percent_encode(text)
    }
}

impl std::fmt::Debug for Body {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => formatter.write_str("Empty"),
            Self::Form(text) => formatter.debug_tuple("Form").field(text).finish(),
            Self::Json(text) => formatter.debug_tuple("Json").field(text).finish(),
            Self::JsonApi(text) => formatter.debug_tuple("JsonApi").field(text).finish(),
            Self::Bytes(bytes) => write!(formatter, "Bytes({} bytes)", bytes.len()),
            Self::Multipart(multipart) => {
                write!(formatter, "Multipart({} bytes)", multipart.bytes.len())
            }
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
    pub fn is_upload(&self) -> bool {
        matches!(self.body, Body::Bytes(_) | Body::Multipart(_))
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_multipart_body_holds_each_field_then_the_file_between_boundaries() {
        let multipart = Multipart::new(
            vec![("translate".to_owned(), "false".to_owned())],
            "file",
            "plate.step",
            b"DATA\r\n--".to_vec(),
        )
        .unwrap();
        let boundary = multipart
            .content_type()
            .strip_prefix("multipart/form-data; boundary=")
            .unwrap()
            .to_owned();

        let body = String::from_utf8(multipart.encoded()).unwrap();

        assert_eq!(
            body,
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"translate\"\r\n\r\nfalse\r\n\
                 --{boundary}\r\nContent-Disposition: form-data; name=\"file\"; \
                 filename=\"plate.step\"\r\nContent-Type: application/octet-stream\r\n\r\n\
                 DATA\r\n--\r\n--{boundary}--\r\n"
            )
        );
    }

    #[test]
    fn every_multipart_has_a_boundary_of_its_own() {
        let make = || Multipart::new(Vec::new(), "file", "a", Vec::new()).unwrap();

        assert_ne!(make().content_type(), make().content_type());
    }

    #[test]
    fn a_debug_print_of_an_upload_shows_its_size_not_its_bytes() {
        let multipart = Multipart::new(Vec::new(), "file", "a", b"secret".to_vec()).unwrap();

        assert_eq!(
            format!("{:?}", Body::Multipart(multipart)),
            "Multipart(6 bytes)"
        );
    }
}
