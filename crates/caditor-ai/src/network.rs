use std::{
    io::{self, BufRead, BufReader, Read},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use ureq::{
    Agent,
    tls::{TlsConfig, TlsProvider},
};

use crate::{
    AiError,
    transport::{HttpRequest, StreamStatus, Transport},
    types::{endpoint_allows_http, validate_endpoint},
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_ERROR_BODY: u64 = 64 << 10;
const MAX_LINE_BYTES: usize = 4 << 20;
const MAX_REDIRECTS: u32 = 3;

pub struct Network {
    https: Agent,
    loopback: Agent,
}

impl Network {
    pub fn new() -> Self {
        let tls = TlsConfig::builder()
            .provider(TlsProvider::Rustls)
            .unversioned_rustls_crypto_provider(Arc::new(rustls_graviola::default_provider()))
            .build();
        let make = |https_only: bool| {
            Agent::config_builder()
                .tls_config(tls.clone())
                .https_only(https_only)
                .http_status_as_error(false)
                .max_redirects(MAX_REDIRECTS)
                .timeout_connect(Some(CONNECT_TIMEOUT))
                .timeout_recv_response(Some(RESPONSE_TIMEOUT))
                .user_agent(concat!("caditor/", env!("CARGO_PKG_VERSION")))
                .build()
                .new_agent()
        };
        Self {
            https: make(true),
            loopback: make(false),
        }
    }
}

impl Default for Network {
    fn default() -> Self {
        Self::new()
    }
}

impl Transport for Network {
    fn post_stream(
        &self,
        request: &HttpRequest,
        cancel: &AtomicBool,
        on_line: &mut dyn FnMut(&str) -> Result<(), AiError>,
    ) -> Result<StreamStatus, AiError> {
        if cancel.load(Ordering::Acquire) {
            return Err(AiError::Cancelled);
        }
        if !validate_endpoint(&request.url) {
            return Err(AiError::InvalidEndpoint);
        }
        let agent = if endpoint_allows_http(&request.url) {
            &self.loopback
        } else {
            &self.https
        };
        let mut builder = agent.post(&request.url);
        for (name, value) in &request.headers {
            builder = builder.header(name.as_str(), value.as_str());
        }
        let mut response = builder.send(request.body.as_str()).map_err(classify)?;
        let status = response.status().as_u16();
        let retry_after_seconds = retry_after(&response);
        if !(200..300).contains(&status) {
            let bytes = response
                .body_mut()
                .with_config()
                .limit(MAX_ERROR_BODY)
                .read_to_vec()
                .unwrap_or_default();
            let text = String::from_utf8_lossy(&bytes).trim().to_owned();
            return Ok(StreamStatus {
                status,
                retry_after_seconds,
                error_body: (!text.is_empty()).then_some(text),
            });
        }
        let mut reader = BufReader::new(response.into_body().into_reader());
        let mut buffer = Vec::new();
        loop {
            if cancel.load(Ordering::Acquire) {
                return Err(AiError::Cancelled);
            }
            buffer.clear();
            let read = read_bounded_line(&mut reader, &mut buffer, MAX_LINE_BYTES)
                .map_err(|error| AiError::Io(error.kind()))?;
            if read == 0 {
                break;
            }
            let line = String::from_utf8_lossy(&buffer);
            on_line(&line)?;
        }
        Ok(StreamStatus {
            status,
            retry_after_seconds,
            error_body: None,
        })
    }
}

fn read_bounded_line(
    reader: &mut impl BufRead,
    out: &mut Vec<u8>,
    limit: usize,
) -> io::Result<usize> {
    let mut limited = reader.take(u64::try_from(limit).unwrap_or(u64::MAX));
    limited.read_until(b'\n', out)
}

fn retry_after(response: &ureq::http::Response<ureq::Body>) -> Option<u64> {
    response
        .headers()
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .and_then(|text| text.trim().parse().ok())
}

fn classify(error: ureq::Error) -> AiError {
    match error {
        ureq::Error::Timeout(_) => AiError::TimedOut,
        ureq::Error::HostNotFound | ureq::Error::ConnectionFailed => AiError::Unreachable,
        ureq::Error::Io(error) => match error.kind() {
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => AiError::TimedOut,
            io::ErrorKind::ConnectionRefused
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::NotConnected
            | io::ErrorKind::HostUnreachable
            | io::ErrorKind::NetworkUnreachable => AiError::Unreachable,
            kind => AiError::Io(kind),
        },
        ureq::Error::Tls(_) | ureq::Error::Rustls(_) | ureq::Error::Pem(_) => AiError::Insecure,
        _ => AiError::MalformedStream,
    }
}
