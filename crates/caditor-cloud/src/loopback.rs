use std::{
    io::{self, Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

use crate::{
    encoding::query_pairs,
    error::Error,
    provider::{Callback, Redirect},
};

pub const CALLBACK_PATH: &str = "/callback";
const POLL: Duration = Duration::from_millis(50);
const READ_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_REQUEST_BYTES: usize = 16 << 10;
const SIGNED_IN: &str = "caditor is signed in. You can close this tab and go back to caditor.";
const DECLINED: &str =
    "The sign-in was not completed. You can close this tab and go back to caditor.";
const NOT_FOUND: &str = "Nothing here.";

pub struct Listener {
    listener: TcpListener,
    port: u16,
}

impl Listener {
    pub fn bind(port: u16) -> Result<Self, Error> {
        let unavailable = |error: io::Error| Error::ListenerUnavailable {
            port,
            kind: error.kind(),
        };
        let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port)))
            .map_err(unavailable)?;
        listener.set_nonblocking(true).map_err(unavailable)?;
        let port = listener.local_addr().map_err(unavailable)?.port();
        Ok(Self { listener, port })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn redirect(&self) -> Redirect {
        Redirect {
            uri: format!("http://localhost:{}{CALLBACK_PATH}", self.port),
        }
    }

    pub fn wait(&self, stop: &AtomicBool, patience: Duration) -> Result<Callback, Error> {
        let deadline = Instant::now() + patience;
        loop {
            if stop.load(Ordering::Relaxed) {
                return Err(Error::Stopped);
            }
            match self.listener.accept() {
                Ok((stream, _)) => {
                    if let Some(outcome) = answer(stream) {
                        return outcome;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return Err(Error::SignInTimedOut);
                    }
                    thread::sleep(POLL);
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => {
                    return Err(Error::ListenerUnavailable {
                        port: self.port,
                        kind: error.kind(),
                    });
                }
            }
        }
    }
}

fn answer(mut stream: TcpStream) -> Option<Result<Callback, Error>> {
    stream.set_nonblocking(false).ok()?;
    stream.set_read_timeout(Some(READ_TIMEOUT)).ok()?;
    let head = read_head(&mut stream)?;
    let target = head.lines().next()?.strip_prefix("GET ")?;
    let target = target.split_whitespace().next()?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != CALLBACK_PATH {
        respond(&mut stream, 404, NOT_FOUND);
        return None;
    }
    let pairs = query_pairs(query);
    let value = |name: &str| {
        pairs
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
    };
    if value("error").is_some() {
        respond(&mut stream, 200, DECLINED);
        return Some(Err(Error::SignInDeclined));
    }
    match (value("code"), value("state")) {
        (Some(code), Some(state)) => {
            respond(&mut stream, 200, SIGNED_IN);
            Some(Ok(Callback { code, state }))
        }
        _ => {
            respond(&mut stream, 404, NOT_FOUND);
            None
        }
    }
}

fn read_head(stream: &mut TcpStream) -> Option<String> {
    let mut head = Vec::new();
    let mut chunk = [0_u8; 1024];
    while !head.windows(4).any(|window| window == b"\r\n\r\n") {
        if head.len() > MAX_REQUEST_BYTES {
            return None;
        }
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            break;
        }
        head.extend_from_slice(chunk.get(..read)?);
    }
    String::from_utf8(head).ok()
}

fn respond(stream: &mut TcpStream, status: u16, message: &str) {
    let reason = if status == 200 { "OK" } else { "Not Found" };
    let page = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>caditor</title></head>\
         <body style=\"font-family:sans-serif;margin:3em\"><p>{message}</p></body></html>"
    );
    let reply = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{page}",
        page.len()
    );
    if let Err(error) = stream.write_all(reply.as_bytes()) {
        log::debug!("the browser went away before the sign-in page was sent: {error}");
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn get(port: u16, target: &str) -> String {
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        write!(stream, "GET {target} HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
        let mut reply = String::new();
        stream.read_to_string(&mut reply).unwrap();
        reply
    }

    fn waiting(listener: Listener) -> thread::JoinHandle<Result<Callback, Error>> {
        thread::spawn(move || listener.wait(&AtomicBool::new(false), Duration::from_secs(20)))
    }

    #[test]
    fn the_redirect_names_localhost_the_port_and_the_callback_path() {
        let listener = Listener::bind(0).unwrap();

        assert_eq!(
            listener.redirect().uri,
            format!("http://localhost:{}/callback", listener.port())
        );
    }

    #[test]
    fn the_browser_returning_with_a_code_ends_the_wait_and_is_told_to_close_the_tab() {
        let listener = Listener::bind(0).unwrap();
        let port = listener.port();
        let waiter = waiting(listener);

        let reply = get(port, "/callback?code=abc%2F1&state=xyz");

        assert!(reply.starts_with("HTTP/1.1 200 OK"));
        assert!(reply.contains("You can close this tab"));
        assert_eq!(
            waiter.join().unwrap().unwrap(),
            Callback {
                code: "abc/1".to_owned(),
                state: "xyz".to_owned()
            }
        );
    }

    #[test]
    fn other_requests_are_turned_away_and_the_wait_goes_on() {
        let listener = Listener::bind(0).unwrap();
        let port = listener.port();
        let waiter = waiting(listener);

        let icon = get(port, "/favicon.ico");
        let empty = get(port, "/callback");
        get(port, "/callback?code=1&state=2");

        assert!(icon.starts_with("HTTP/1.1 404"));
        assert!(empty.starts_with("HTTP/1.1 404"));
        assert!(waiter.join().unwrap().is_ok());
    }

    #[test]
    fn a_refusal_in_the_browser_is_a_declined_sign_in() {
        let listener = Listener::bind(0).unwrap();
        let port = listener.port();
        let waiter = waiting(listener);

        let reply = get(port, "/callback?error=access_denied&state=2");

        assert!(reply.contains("not completed"));
        assert!(matches!(waiter.join().unwrap(), Err(Error::SignInDeclined)));
    }

    #[test]
    fn stopping_ends_the_wait_and_so_does_running_out_of_patience() {
        let listener = Listener::bind(0).unwrap();
        let stop = Arc::new(AtomicBool::new(true));

        assert!(matches!(
            listener.wait(&stop, Duration::from_secs(20)),
            Err(Error::Stopped)
        ));
        assert!(matches!(
            listener.wait(&AtomicBool::new(false), Duration::ZERO),
            Err(Error::SignInTimedOut)
        ));
    }

    #[test]
    fn a_port_already_taken_is_named() {
        let first = Listener::bind(0).unwrap();

        let second = Listener::bind(first.port());

        assert!(matches!(
            second,
            Err(Error::ListenerUnavailable { port, .. }) if port == first.port()
        ));
    }
}
