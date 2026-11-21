use std::{
    cell::Cell,
    io::{self, Read, Write},
    sync::Arc,
    time::Duration,
};

use ureq::{
    Agent, RequestBuilder, SendBody,
    tls::{TlsConfig, TlsProvider},
    typestate::WithBody,
};

use crate::transport::{Body, Fetched, Method, Request, Response, Transport, TransportError};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(60);
const ANSWER_TIMEOUT: Duration = Duration::from_secs(60);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const MAX_ANSWER_BYTES: u64 = 16 << 20;
const COPY_CHUNK: usize = 64 << 10;
const MAX_DOWNLOAD_BYTES: u64 = 4 << 30;
const DOWNLOAD_SLACK_BYTES: u64 = 1 << 20;
const DOWNLOAD_OVERRUN_FACTOR: u64 = 2;
const MAX_REDIRECTS: u32 = 5;

pub struct Network {
    answers: Agent,
    downloads: Agent,
    uploads: Agent,
}

impl Network {
    pub fn new() -> Self {
        let tls = TlsConfig::builder()
            .provider(TlsProvider::Rustls)
            .unversioned_rustls_crypto_provider(Arc::new(rustls_graviola::default_provider()))
            .build();
        let agent = |body_timeout: Duration, send_timeout: Option<Duration>| {
            Agent::config_builder()
                .tls_config(tls.clone())
                .https_only(true)
                .http_status_as_error(false)
                .max_redirects(MAX_REDIRECTS)
                .timeout_connect(Some(CONNECT_TIMEOUT))
                .timeout_recv_response(Some(RESPONSE_TIMEOUT))
                .timeout_recv_body(Some(body_timeout))
                .timeout_send_body(send_timeout)
                .user_agent(concat!("caditor/", env!("CARGO_PKG_VERSION")))
                .build()
                .new_agent()
        };
        Self {
            answers: agent(ANSWER_TIMEOUT, None),
            downloads: agent(DOWNLOAD_TIMEOUT, None),
            uploads: agent(ANSWER_TIMEOUT, Some(UPLOAD_TIMEOUT)),
        }
    }

    fn call(
        agent: &Agent,
        request: &Request,
    ) -> Result<ureq::http::Response<ureq::Body>, TransportError> {
        let outcome = match request.method {
            Method::Get => authorised(agent.get(&request.url), request).call(),
            Method::Post => with_body(agent.post(&request.url), request),
            Method::Put => with_body(agent.put(&request.url), request),
        };
        outcome.map_err(classify)
    }
}

fn authorised<B>(builder: RequestBuilder<B>, request: &Request) -> RequestBuilder<B> {
    match &request.bearer {
        Some(token) => builder.header("Authorization", format!("Bearer {token}")),
        None => builder,
    }
}

fn with_body(
    builder: RequestBuilder<WithBody>,
    request: &Request,
) -> Result<ureq::http::Response<ureq::Body>, ureq::Error> {
    let builder = authorised(builder, request);
    match &request.body {
        Body::Empty => builder.send_empty(),
        Body::Form(text) => builder
            .header("Content-Type", "application/x-www-form-urlencoded")
            .send(text.as_str()),
        Body::Json(text) => builder
            .header("Content-Type", "application/json")
            .send(text.as_str()),
        Body::JsonApi(text) => builder
            .header("Content-Type", "application/vnd.api+json")
            .send(text.as_str()),
        Body::Bytes(bytes) => builder.send(bytes.as_slice()),
        Body::Multipart(multipart) => builder
            .header("Content-Type", multipart.content_type())
            .send(multipart.encoded().as_slice()),
    }
}

impl Default for Network {
    fn default() -> Self {
        Self::new()
    }
}

fn answered(mut response: ureq::http::Response<ureq::Body>) -> Result<Response, TransportError> {
    let status = response.status().as_u16();
    let retry_after_seconds = retry_after(&response);
    let body = response
        .body_mut()
        .with_config()
        .limit(MAX_ANSWER_BYTES)
        .read_to_vec()
        .map_err(classify)?;
    Ok(Response {
        status,
        body,
        retry_after_seconds,
    })
}

struct Tracked<'a> {
    bytes: &'a [u8],
    position: usize,
    progress: &'a mut dyn FnMut(u64, u64) -> bool,
    stopped: &'a Cell<bool>,
}

impl Read for Tracked<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let remaining = self.bytes.get(self.position..).unwrap_or_default();
        let count = remaining.len().min(buffer.len()).min(COPY_CHUNK);

        let (Some(from), Some(into)) = (remaining.get(..count), buffer.get_mut(..count)) else {
            return Ok(0);
        };

        if count == 0 {
            return Ok(0);
        }
        into.copy_from_slice(from);
        self.position += count;

        let sent = u64::try_from(self.position).unwrap_or(u64::MAX);
        let total = u64::try_from(self.bytes.len()).unwrap_or(u64::MAX);
        if (self.progress)(sent, total) {
            Ok(count)
        } else {
            self.stopped.set(true);
            Err(io::Error::other("the upload was stopped"))
        }
    }
}

fn retry_after(response: &ureq::http::Response<ureq::Body>) -> Option<u64> {
    response
        .headers()
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .and_then(|text| text.trim().parse().ok())
}

fn classify(error: ureq::Error) -> TransportError {
    match error {
        ureq::Error::Timeout(_) => TransportError::TimedOut,
        ureq::Error::HostNotFound | ureq::Error::ConnectionFailed => TransportError::Unreachable,
        ureq::Error::Io(error) => match error.kind() {
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => TransportError::TimedOut,
            io::ErrorKind::ConnectionRefused
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::NotConnected
            | io::ErrorKind::HostUnreachable
            | io::ErrorKind::NetworkUnreachable => TransportError::Unreachable,
            kind => TransportError::Io(kind),
        },
        ureq::Error::Tls(_) | ureq::Error::Rustls(_) | ureq::Error::Pem(_) => {
            TransportError::Insecure
        }
        ureq::Error::BodyExceedsLimit(_) => TransportError::TooLarge,
        _ => TransportError::Malformed,
    }
}

impl Transport for Network {
    fn send(&self, request: &Request) -> Result<Response, TransportError> {
        let agent = if request.is_upload() {
            &self.uploads
        } else {
            &self.answers
        };
        answered(Self::call(agent, request)?)
    }

    fn send_tracked(
        &self,
        request: &Request,
        progress: &mut dyn FnMut(u64, u64) -> bool,
    ) -> Result<Response, TransportError> {
        let Body::Multipart(multipart) = &request.body else {
            return self.send(request);
        };
        let builder = match request.method {
            Method::Post => self.uploads.post(&request.url),
            Method::Put => self.uploads.put(&request.url),
            Method::Get => return self.send(request),
        };
        let encoded = multipart.encoded();
        let total = u64::try_from(encoded.len()).unwrap_or(u64::MAX);
        let stopped = Cell::new(false);

        let mut body = Tracked {
            bytes: &encoded,
            position: 0,
            progress,
            stopped: &stopped,
        };

        let outcome = authorised(builder, request)
            .header("Content-Type", multipart.content_type())
            .header("Content-Length", total.to_string())
            .send(SendBody::from_reader(&mut body));

        if stopped.get() {
            return Err(TransportError::Stopped);
        }
        answered(outcome.map_err(classify)?)
    }

    fn fetch(
        &self,
        request: &Request,
        sink: &mut dyn Write,
        progress: &mut dyn FnMut(u64, Option<u64>) -> bool,
    ) -> Result<Fetched, TransportError> {
        let response = Self::call(&self.downloads, request)?;
        let status = response.status().as_u16();
        let retry_after_seconds = retry_after(&response);

        if !(200..300).contains(&status) {
            return Ok(Fetched {
                status,
                retry_after_seconds,
                bytes: 0,
            });
        }

        let total = response
            .headers()
            .get("content-length")
            .and_then(|value| value.to_str().ok())
            .and_then(|text| text.trim().parse::<u64>().ok());
        let mut reader = response.into_body().into_reader();

        let bytes = copy_limited(&mut reader, sink, total, progress)?;
        Ok(Fetched {
            status,
            retry_after_seconds,
            bytes,
        })
    }
}

fn download_limit(total: Option<u64>) -> u64 {
    total
        .map(|promised| {
            promised
                .saturating_mul(DOWNLOAD_OVERRUN_FACTOR)
                .saturating_add(DOWNLOAD_SLACK_BYTES)
        })
        .unwrap_or(MAX_DOWNLOAD_BYTES)
        .min(MAX_DOWNLOAD_BYTES)
}

fn copy_limited(
    reader: &mut dyn Read,
    sink: &mut dyn Write,
    total: Option<u64>,
    progress: &mut dyn FnMut(u64, Option<u64>) -> bool,
) -> Result<u64, TransportError> {
    let limit = download_limit(total);
    let mut chunk = vec![0_u8; COPY_CHUNK];
    let mut bytes = 0_u64;
    loop {
        let read = reader
            .read(&mut chunk)
            .map_err(|error| classify(ureq::Error::Io(error)))?;

        if read == 0 {
            return Ok(bytes);
        }
        bytes = bytes.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
        if bytes > limit {
            return Err(TransportError::TooLarge);
        }

        let filled = chunk.get(..read).unwrap_or_default();

        sink.write_all(filled)
            .map_err(|error| TransportError::Io(error.kind()))?;
        if !progress(bytes, total) {
            return Err(TransportError::Stopped);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_all(reader: &mut Tracked<'_>) -> io::Result<Vec<u8>> {
        let mut all = Vec::new();
        let mut buffer = vec![0_u8; 1 << 20];
        loop {
            let read = reader.read(&mut buffer)?;

            if read == 0 {
                return Ok(all);
            }
            all.extend_from_slice(buffer.get(..read).unwrap_or_default());
        }
    }

    #[test]
    fn a_tracked_body_reports_every_chunk_it_hands_over() {
        let bytes = vec![7_u8; COPY_CHUNK * 2 + 10];
        let mut seen = Vec::new();
        let stopped = Cell::new(false);
        let mut progress = |sent: u64, total: u64| {
            seen.push((sent, total));
            true
        };

        let mut reader = Tracked {
            bytes: &bytes,
            position: 0,
            progress: &mut progress,
            stopped: &stopped,
        };

        let read = read_all(&mut reader).unwrap();

        let chunk = COPY_CHUNK as u64;
        let total = bytes.len() as u64;

        assert_eq!(read, bytes);
        assert_eq!(seen, [(chunk, total), (chunk * 2, total), (total, total)]);
        assert!(!stopped.get());
    }

    #[test]
    fn a_tracked_body_stops_where_progress_says_so() {
        let bytes = vec![1_u8; COPY_CHUNK * 3];
        let stopped = Cell::new(false);
        let mut progress = |sent: u64, _: u64| sent < COPY_CHUNK as u64 * 2;

        let mut reader = Tracked {
            bytes: &bytes,
            position: 0,
            progress: &mut progress,
            stopped: &stopped,
        };

        let result = read_all(&mut reader);

        assert!(result.is_err());
        assert!(stopped.get());
        assert_eq!(reader.position, COPY_CHUNK * 2);
    }

    #[test]
    fn the_download_limit_is_a_margin_over_what_the_answer_promised_or_a_fixed_cap() {
        assert_eq!(download_limit(None), MAX_DOWNLOAD_BYTES);
        assert_eq!(
            download_limit(Some(100)),
            100 * DOWNLOAD_OVERRUN_FACTOR + DOWNLOAD_SLACK_BYTES
        );
        assert_eq!(download_limit(Some(u64::MAX)), MAX_DOWNLOAD_BYTES);
    }

    #[test]
    fn a_download_that_runs_past_its_promised_size_is_cut_off_as_too_large() {
        let promised = 1000;
        let body = vec![3_u8; usize::try_from(download_limit(Some(promised))).unwrap() + 1];
        let mut sink = Vec::new();

        let outcome = copy_limited(
            &mut body.as_slice(),
            &mut sink,
            Some(promised),
            &mut |_, _| true,
        );

        assert_eq!(outcome, Err(TransportError::TooLarge));
        assert!(sink.len() < body.len());
    }

    #[test]
    fn a_download_within_its_limit_is_copied_whole() {
        let body = vec![9_u8; COPY_CHUNK + 5];
        let mut sink = Vec::new();

        let outcome = copy_limited(
            &mut body.as_slice(),
            &mut sink,
            Some(body.len() as u64),
            &mut |_, _| true,
        );

        assert_eq!(outcome, Ok(body.len() as u64));
        assert_eq!(sink, body);
    }

    #[test]
    fn a_download_without_a_promised_size_is_still_capped() {
        struct Endless;

        impl Read for Endless {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                Ok(buffer.len())
            }
        }

        let outcome = copy_limited(&mut Endless, &mut io::sink(), None, &mut |_, _| true);

        assert_eq!(outcome, Err(TransportError::TooLarge));
    }
}
