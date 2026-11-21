use std::{
    collections::VecDeque,
    io::{self, Read, Write},
    net::{Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener, TcpStream},
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
const CONNECTION_PATIENCE: Duration = Duration::from_secs(5);
const WRITE_PATIENCE: Duration = Duration::from_secs(1);
const MAX_REQUEST_BYTES: usize = 16 << 10;
const MAX_OPEN_CONNECTIONS: usize = 32;
const ANY_PORT_TRIES: usize = 8;
const SIGNED_IN: &str = "caditor is signed in. You can close this tab and go back to caditor.";
const DECLINED: &str =
    "The sign-in was not completed. You can close this tab and go back to caditor.";
const OTHER_SIGN_IN: &str =
    "This is not the sign-in caditor is waiting for. You can close this tab.";
const NOT_FOUND: &str = "Nothing here.";

pub struct Listener {
    sockets: Vec<TcpListener>,
    port: u16,
}

impl Listener {
    pub fn bind(port: u16) -> Result<Self, Error> {
        let tries = if port == 0 { ANY_PORT_TRIES } else { 1 };
        let mut bound = Self::bind_both(port);

        for _ in 1..tries {
            if bound.is_ok() {
                break;
            }
            bound = Self::bind_both(port);
        }
        bound
    }

    fn bind_both(port: u16) -> Result<Self, Error> {
        let unavailable = |port: u16| {
            move |error: io::Error| Error::ListenerUnavailable {
                port,
                kind: error.kind(),
            }
        };

        let first = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port)))
            .map_err(unavailable(port))?;
        let port = first.local_addr().map_err(unavailable(port))?.port();
        let mut sockets = vec![first];

        match TcpListener::bind(SocketAddr::from((Ipv6Addr::LOCALHOST, port))) {
            Ok(second) => sockets.push(second),
            Err(error) if is_held(&error) => return Err(unavailable(port)(error)),
            Err(error) => {
                log::debug!("no IPv6 loopback to listen on ({error}); 127.0.0.1 alone answers");
            }
        }
        for socket in &sockets {
            socket.set_nonblocking(true).map_err(unavailable(port))?;
        }

        Ok(Self { sockets, port })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn redirect(&self) -> Redirect {
        Redirect {
            uri: format!("http://localhost:{}{CALLBACK_PATH}", self.port),
        }
    }

    pub fn wait(
        self,
        state: &str,
        stop: &AtomicBool,
        patience: Duration,
    ) -> Result<Callback, Error> {
        let deadline = Instant::now() + patience;
        let mut open = VecDeque::new();
        loop {
            if stop.load(Ordering::Relaxed) {
                return Err(Error::Stopped);
            }

            let now = Instant::now();

            if now >= deadline {
                return Err(Error::SignInTimedOut);
            }

            let mut busy = self.accept(&mut open, now)?;
            let mut kept = VecDeque::with_capacity(open.len());

            while let Some(mut connection) = open.pop_front() {
                match connection.read() {
                    Reading::Waiting if now < connection.deadline => kept.push_back(connection),
                    Reading::Waiting | Reading::Closed => {}
                    Reading::Complete => {
                        busy = true;
                        if let Some(outcome) = connection.answer(state) {
                            return outcome;
                        }
                    }
                }
            }
            open = kept;

            if !busy {
                thread::sleep(POLL);
            }
        }
    }

    fn accept(&self, open: &mut VecDeque<Connection>, now: Instant) -> Result<bool, Error> {
        let mut accepted = false;

        for socket in &self.sockets {
            for _ in 0..MAX_OPEN_CONNECTIONS {
                match socket.accept() {
                    Ok((stream, _)) => {
                        if let Err(error) = stream.set_nonblocking(true) {
                            log::debug!(
                                "a connection to the sign-in listener was dropped: {error}"
                            );
                            continue;
                        }
                        if open.len() >= MAX_OPEN_CONNECTIONS {
                            open.pop_front();
                        }
                        open.push_back(Connection {
                            stream,
                            head: Vec::new(),
                            deadline: now + CONNECTION_PATIENCE,
                        });
                        accepted = true;
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    Err(error) if is_passing(&error) => {}
                    Err(error) => {
                        return Err(Error::ListenerUnavailable {
                            port: self.port,
                            kind: error.kind(),
                        });
                    }
                }
            }
        }
        Ok(accepted)
    }
}

fn is_held(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::AddrInUse | io::ErrorKind::PermissionDenied
    )
}

fn is_passing(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::Interrupted
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::ConnectionReset
    )
}

struct Connection {
    stream: TcpStream,
    head: Vec<u8>,
    deadline: Instant,
}

enum Reading {
    Waiting,
    Closed,
    Complete,
}

enum Verdict {
    Elsewhere,
    OtherSignIn,
    Declined,
    Taken(Callback),
}

impl Connection {
    fn read(&mut self) -> Reading {
        let mut chunk = [0_u8; 1024];
        loop {
            if self.head.windows(4).any(|window| window == b"\r\n\r\n") {
                return Reading::Complete;
            }
            if self.head.len() > MAX_REQUEST_BYTES {
                return Reading::Closed;
            }
            match self.stream.read(&mut chunk) {
                Ok(0) => return Reading::Complete,
                Ok(read) => match chunk.get(..read) {
                    Some(bytes) => self.head.extend_from_slice(bytes),
                    None => return Reading::Closed,
                },
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    return Reading::Waiting;
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => return Reading::Closed,
            }
        }
    }

    fn answer(mut self, state: &str) -> Option<Result<Callback, Error>> {
        match judge(&self.head, state) {
            Verdict::Elsewhere => {
                self.respond(404, NOT_FOUND);
                None
            }
            Verdict::OtherSignIn => {
                self.respond(404, OTHER_SIGN_IN);
                None
            }
            Verdict::Declined => {
                self.respond(200, DECLINED);
                Some(Err(Error::SignInDeclined))
            }
            Verdict::Taken(callback) => {
                self.respond(200, SIGNED_IN);
                Some(Ok(callback))
            }
        }
    }

    fn respond(&mut self, status: u16, message: &str) {
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
        let written = self
            .stream
            .set_nonblocking(false)
            .and_then(|()| self.stream.set_write_timeout(Some(WRITE_PATIENCE)))
            .and_then(|()| self.stream.write_all(reply.as_bytes()));
        if let Err(error) = written {
            log::debug!("the browser went away before the sign-in page was sent: {error}");
        }
    }
}

fn judge(head: &[u8], state: &str) -> Verdict {
    let Ok(head) = str::from_utf8(head) else {
        return Verdict::Elsewhere;
    };
    let Some(target) = head
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("GET "))
        .and_then(|rest| rest.split_whitespace().next())
    else {
        return Verdict::Elsewhere;
    };
    let (path, query) = target.split_once('?').unwrap_or((target, ""));

    if path != CALLBACK_PATH {
        return Verdict::Elsewhere;
    }

    let pairs = query_pairs(query);
    let value = |name: &str| {
        pairs
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
    };

    match (value("state"), value("error"), value("code")) {
        (Some(answered), _, _) if answered != state => Verdict::OtherSignIn,
        (None, _, _) => Verdict::Elsewhere,
        (Some(_), Some(_), _) => Verdict::Declined,
        (Some(_), None, Some(code)) => Verdict::Taken(Callback {
            code,
            state: state.to_owned(),
        }),
        (Some(_), None, None) => Verdict::Elsewhere,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    const STATE: &str = "xyz";

    fn connect(port: u16) -> TcpStream {
        TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap()
    }

    fn get_from(mut stream: TcpStream, target: &str) -> String {
        write!(stream, "GET {target} HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();

        let mut reply = String::new();

        stream.read_to_string(&mut reply).unwrap();
        reply
    }

    fn get(port: u16, target: &str) -> String {
        get_from(connect(port), target)
    }

    fn waiting(listener: Listener) -> thread::JoinHandle<Result<Callback, Error>> {
        thread::spawn(move || {
            listener.wait(STATE, &AtomicBool::new(false), Duration::from_secs(20))
        })
    }

    fn ipv6_loopback() -> Option<TcpListener> {
        TcpListener::bind(SocketAddr::from((Ipv6Addr::LOCALHOST, 0))).ok()
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
                state: STATE.to_owned()
            }
        );
    }

    #[test]
    fn a_browser_trying_the_ipv6_loopback_first_reaches_the_same_listener() {
        if ipv6_loopback().is_none() {
            return;
        }

        let listener = Listener::bind(0).unwrap();
        let port = listener.port();
        let waiter = waiting(listener);

        let stream = TcpStream::connect((Ipv6Addr::LOCALHOST, port)).unwrap();
        let reply = get_from(stream, "/callback?code=c&state=xyz");

        assert!(reply.starts_with("HTTP/1.1 200 OK"));
        assert!(waiter.join().unwrap().is_ok());
    }

    #[test]
    fn a_port_someone_holds_on_the_ipv6_loopback_is_refused_rather_than_shared() {
        let Some(squatter) = ipv6_loopback() else {
            return;
        };
        let port = squatter.local_addr().unwrap().port();

        let listener = Listener::bind(port);

        assert!(matches!(
            listener,
            Err(Error::ListenerUnavailable { port: named, .. }) if named == port
        ));
    }

    #[test]
    fn other_requests_are_turned_away_and_the_wait_goes_on() {
        let listener = Listener::bind(0).unwrap();
        let port = listener.port();
        let waiter = waiting(listener);

        let icon = get(port, "/favicon.ico");
        let empty = get(port, "/callback");
        let codeless = get(port, "/callback?state=xyz");

        get(port, "/callback?code=1&state=xyz");

        assert!(icon.starts_with("HTTP/1.1 404"));
        assert!(empty.starts_with("HTTP/1.1 404"));
        assert!(codeless.starts_with("HTTP/1.1 404"));
        assert!(waiter.join().unwrap().is_ok());
    }

    #[test]
    fn a_callback_or_refusal_for_another_state_is_turned_away_and_the_wait_goes_on() {
        let listener = Listener::bind(0).unwrap();
        let port = listener.port();
        let waiter = waiting(listener);

        let foreign = get(port, "/callback?code=stolen&state=other");
        let unstated = get(port, "/callback?code=stolen");
        let refusal = get(port, "/callback?error=access_denied&state=other");

        get(port, "/callback?code=own&state=xyz");

        assert!(foreign.starts_with("HTTP/1.1 404"));
        assert!(foreign.contains("not the sign-in caditor is waiting for"));
        assert!(!foreign.contains("is signed in"));
        assert!(unstated.starts_with("HTTP/1.1 404"));
        assert!(refusal.starts_with("HTTP/1.1 404"));
        assert_eq!(waiter.join().unwrap().unwrap().code, "own");
    }

    #[test]
    fn a_connection_that_never_finishes_its_request_does_not_hold_the_listener() {
        let listener = Listener::bind(0).unwrap();
        let port = listener.port();
        let waiter = waiting(listener);
        let mut slow = connect(port);

        write!(slow, "GET /callback?code=").unwrap();

        let reply = get(port, "/callback?code=own&state=xyz");

        assert!(reply.starts_with("HTTP/1.1 200 OK"));
        assert_eq!(waiter.join().unwrap().unwrap().code, "own");
    }

    #[test]
    fn a_refusal_in_the_browser_is_a_declined_sign_in() {
        let listener = Listener::bind(0).unwrap();
        let port = listener.port();
        let waiter = waiting(listener);

        let reply = get(port, "/callback?error=access_denied&state=xyz");

        assert!(reply.contains("not completed"));
        assert!(matches!(waiter.join().unwrap(), Err(Error::SignInDeclined)));
    }

    #[test]
    fn stopping_ends_the_wait_and_so_does_running_out_of_patience() {
        let stopped = Listener::bind(0).unwrap();
        let impatient = Listener::bind(0).unwrap();
        let stop = Arc::new(AtomicBool::new(true));

        assert!(matches!(
            stopped.wait(STATE, &stop, Duration::from_secs(20)),
            Err(Error::Stopped)
        ));
        assert!(matches!(
            impatient.wait(STATE, &AtomicBool::new(false), Duration::ZERO),
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
