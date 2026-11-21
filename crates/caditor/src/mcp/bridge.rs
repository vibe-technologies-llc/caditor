use std::{
    env,
    io::{self, BufRead, BufReader, Write},
    net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream},
    process::ExitCode,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use anyhow::{Result, bail};
use caditor_ai::base64_encode;
use serde_json::{Map, Value, json};

use crate::{
    about,
    mcp::{
        json_rpc::{self, Code, Incoming, Line, MAX_LINE_BYTES, RequestId},
        resources,
        server::{INSTRUCTIONS, PROTOCOL_VERSIONS},
        tools::{Answer, Tool},
    },
};

pub const TOKEN_ENV: &str = "CADITOR_MCP_TOKEN";
const ACCEPT_POLL: Duration = Duration::from_millis(25);
const AUTH_TIMEOUT: Duration = Duration::from_secs(5);
const STRUCTURED_FROM: &str = "2025-06-18";

#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    #[error("{0}")]
    Tool(#[from] crate::mcp::tools::ToolError),
    #[error("{0}")]
    Resource(#[from] crate::mcp::resources::ResourceError),
    #[error("the model window is no longer available")]
    ModelUnavailable,
    #[error("the model changed while running the tool")]
    ModelChanged,
    #[error("the request was stopped")]
    Cancelled,
}

pub struct ToolCallOutcome {
    pub answer: Result<Answer, BridgeError>,
}

pub trait BridgeHandler: Send + Sync + 'static {
    fn call_tool(&self, name: &str, arguments: Value, cancelled: &AtomicBool) -> ToolCallOutcome;

    fn read_resource(&self, uri: &str, cancelled: &AtomicBool) -> Result<Value, BridgeError>;
}

pub struct LoopbackBridge {
    port: u16,
    token: String,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl LoopbackBridge {
    pub fn start(handler: Arc<dyn BridgeHandler>) -> io::Result<Self> {
        let token = random_token()?;
        let address = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0);
        let listener = TcpListener::bind(address)?;
        let port = listener.local_addr()?.port();
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&stop);
        let expected = token.clone();
        let worker = thread::Builder::new()
            .name("mcp bridge".to_owned())
            .spawn(move || accept_loop(listener, expected, stop_flag, handler))?;
        Ok(Self {
            port,
            token,
            stop,
            worker: Some(worker),
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    pub fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = TcpStream::connect(SocketAddrV4::new(Ipv4Addr::LOCALHOST, self.port));
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for LoopbackBridge {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn random_token() -> io::Result<String> {
    let mut bytes = [0_u8; 32];
    graviola::random::fill(&mut bytes).map_err(|_| io::Error::other("random generator failed"))?;
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
    }
    Ok(hex)
}

fn accept_loop(
    listener: TcpListener,
    token: String,
    stop: Arc<AtomicBool>,
    handler: Arc<dyn BridgeHandler>,
) {
    while !stop.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, _)) => {
                if stop.load(Ordering::Acquire) {
                    break;
                }
                let expected = token.clone();
                let stop_client = Arc::clone(&stop);
                let handler = Arc::clone(&handler);
                let _ = thread::Builder::new()
                    .name("mcp bridge client".to_owned())
                    .spawn(move || serve_connection(stream, &expected, &stop_client, &*handler));
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(ACCEPT_POLL);
            }
            Err(error) => {
                log::debug!("mcp bridge accept stopped: {error}");
                break;
            }
        }
    }
}

fn serve_connection(
    stream: TcpStream,
    expected_token: &str,
    stop: &AtomicBool,
    handler: &dyn BridgeHandler,
) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(AUTH_TIMEOUT));
    let Ok(mut writer) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(stream);
    let mut auth_line = String::new();
    match reader.read_line(&mut auth_line) {
        Ok(bytes) if bytes > 0 => {
            if !constant_time_eq(auth_line.trim().as_bytes(), expected_token.as_bytes()) {
                return;
            }
        }
        _ => return,
    }
    let _ = reader.get_ref().set_read_timeout(None);
    let mut protocol = PROTOCOL_VERSIONS
        .first()
        .copied()
        .unwrap_or(STRUCTURED_FROM);
    loop {
        if stop.load(Ordering::Acquire) {
            break;
        }
        let line = match json_rpc::read_line(&mut reader, MAX_LINE_BYTES) {
            Ok(line) => line,
            Err(_) => break,
        };
        let message = match line {
            Line::End => break,
            Line::TooLong => {
                let error = json_rpc::error(None, Code::ParseError, "the request line is too long");
                if json_rpc::write_message(&mut writer, &error).is_err() {
                    break;
                }
                continue;
            }
            Line::Text(bytes) => match json_rpc::parse(&bytes) {
                Ok(Some(incoming)) => incoming,
                Ok(None) => continue,
                Err(rejected) => {
                    let error = json_rpc::error(
                        rejected.id.as_ref(),
                        rejected.reason.code(),
                        &rejected.reason.to_string(),
                    );
                    if json_rpc::write_message(&mut writer, &error).is_err() {
                        break;
                    }
                    continue;
                }
            },
        };
        match message {
            Incoming::Request { id, method, params } => {
                let response = respond(&id, &method, params, &mut protocol, stop, handler);
                if json_rpc::write_message(&mut writer, &response).is_err() {
                    break;
                }
            }
            Incoming::Notification { .. } | Incoming::Response => {}
        }
    }
}

fn respond(
    id: &RequestId,
    method: &str,
    params: Value,
    protocol: &mut &'static str,
    stop: &AtomicBool,
    handler: &dyn BridgeHandler,
) -> Value {
    let answered = match method {
        "initialize" => Ok(initialize(&params, protocol)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({
            "tools": Tool::ALL.into_iter().map(Tool::listing).collect::<Vec<_>>()
        })),
        "tools/call" => call_tool(params, protocol, stop, handler),
        "resources/list" => Ok(resources::listing()),
        "resources/templates/list" => Ok(json!({ "resourceTemplates": [] })),
        "resources/read" => read_resource(params, stop, handler),
        _ => Err((
            Code::MethodNotFound,
            format!("this server does not offer {method}"),
        )),
    };
    match answered {
        Ok(result) => json_rpc::result(id, result),
        Err((code, text)) => json_rpc::error(Some(id), code, &text),
    }
}

fn initialize(params: &Value, protocol: &mut &'static str) -> Value {
    let asked = params.get("protocolVersion").and_then(Value::as_str);
    *protocol = asked
        .and_then(|version| {
            PROTOCOL_VERSIONS
                .iter()
                .copied()
                .find(|known| *known == version)
        })
        .or_else(|| PROTOCOL_VERSIONS.first().copied())
        .unwrap_or(STRUCTURED_FROM);
    json!({
        "protocolVersion": *protocol,
        "capabilities": {
            "tools": {},
            "resources": {}
        },
        "serverInfo": {
            "name": about::NAME,
            "version": about::VERSION
        },
        "instructions": INSTRUCTIONS
    })
}

fn call_tool(
    params: Value,
    protocol: &str,
    stop: &AtomicBool,
    handler: &dyn BridgeHandler,
) -> Result<Value, (Code, String)> {
    let name = params.get("name").and_then(Value::as_str).ok_or_else(|| {
        (
            Code::InvalidParams,
            "tools/call needs a tool name".to_owned(),
        )
    })?;
    if Tool::named(name).is_none() {
        return Err((
            Code::InvalidParams,
            format!("there is no tool named {name}"),
        ));
    }
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| Value::Object(Map::new()));
    let outcome = handler.call_tool(name, arguments, stop);
    match outcome.answer {
        Ok(answer) => {
            let text = Value::Object(answer.data.clone()).to_string();
            let mut content = vec![json!({ "type": "text", "text": text })];
            if let Some(png) = answer.image {
                content.push(json!({
                    "type": "image",
                    "data": base64_encode(&png),
                    "mimeType": "image/png"
                }));
            }
            let mut result = Map::new();
            result.insert("content".to_owned(), Value::Array(content));
            if protocol >= STRUCTURED_FROM {
                result.insert("structuredContent".to_owned(), Value::Object(answer.data));
            }
            Ok(Value::Object(result))
        }
        Err(error) => Ok(json!({
            "content": [{ "type": "text", "text": error.to_string() }],
            "isError": true
        })),
    }
}

fn read_resource(
    params: Value,
    stop: &AtomicBool,
    handler: &dyn BridgeHandler,
) -> Result<Value, (Code, String)> {
    let uri = params
        .get("uri")
        .and_then(Value::as_str)
        .ok_or_else(|| (Code::InvalidParams, "resources/read needs a uri".to_owned()))?;
    handler
        .read_resource(uri, stop)
        .map_err(|error| (Code::ResourceNotFound, error.to_string()))
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0_u8;
    for (a, b) in left.iter().zip(right.iter()) {
        diff |= a ^ b;
    }
    diff == 0
}

pub fn connect_stdio(port: u16) -> Result<ExitCode> {
    let token = match env::var(TOKEN_ENV) {
        Ok(token) if !token.trim().is_empty() => token,
        _ => bail!("{TOKEN_ENV} is required to connect to a running caditor window"),
    };
    let mut stream = TcpStream::connect(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port))?;
    writeln!(stream, "{}", token.trim())?;
    stream.flush()?;

    let mut to_server = stream.try_clone()?;
    let reader_handle = thread::Builder::new()
        .name("mcp bridge stdin".to_owned())
        .spawn(move || {
            let mut stdin = BufReader::new(io::stdin());
            let mut line = String::new();
            loop {
                line.clear();
                match stdin.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if to_server.write_all(line.as_bytes()).is_err()
                            || to_server.flush().is_err()
                        {
                            break;
                        }
                    }
                }
            }
            let _ = to_server.shutdown(std::net::Shutdown::Write);
        })?;

    let mut from_server = BufReader::new(stream);
    let mut stdout = io::stdout();
    let mut line = String::new();
    loop {
        line.clear();
        match from_server.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                if stdout.write_all(line.as_bytes()).is_err() || stdout.flush().is_err() {
                    break;
                }
            }
        }
    }
    let _ = reader_handle.join();
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeHandler;

    impl BridgeHandler for FakeHandler {
        fn call_tool(
            &self,
            name: &str,
            _arguments: Value,
            _cancelled: &AtomicBool,
        ) -> ToolCallOutcome {
            let mut map = Map::new();
            map.insert("called".to_owned(), json!(name));
            ToolCallOutcome {
                answer: Ok(Answer::of(map)),
            }
        }

        fn read_resource(&self, uri: &str, _cancelled: &AtomicBool) -> Result<Value, BridgeError> {
            Ok(json!({ "contents": [{ "uri": uri, "mimeType": "text/plain", "text": "ok" }] }))
        }
    }

    #[test]
    fn loopback_bridge_rejects_bad_token_and_serves_valid_mcp_requests() {
        let mut bridge = LoopbackBridge::start(Arc::new(FakeHandler)).unwrap();

        let mut bad =
            TcpStream::connect(SocketAddrV4::new(Ipv4Addr::LOCALHOST, bridge.port())).unwrap();
        writeln!(bad, "wrong-token").unwrap();
        writeln!(bad, "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}}").unwrap();
        bad.flush().unwrap();
        let mut bad_reader = BufReader::new(bad);
        let mut bad_reply = String::new();
        assert_eq!(bad_reader.read_line(&mut bad_reply).unwrap(), 0);

        let mut good =
            TcpStream::connect(SocketAddrV4::new(Ipv4Addr::LOCALHOST, bridge.port())).unwrap();
        writeln!(good, "{}", bridge.token()).unwrap();
        writeln!(
            good,
            "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{{\"protocolVersion\":\"2025-11-25\"}}}}"
        )
        .unwrap();
        writeln!(
            good,
            "{{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{{\"name\":\"get_model\",\"arguments\":{{}}}}}}"
        )
        .unwrap();
        writeln!(
            good,
            "{{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"resources/read\",\"params\":{{\"uri\":\"caditor://model/summary\"}}}}"
        )
        .unwrap();
        good.flush().unwrap();
        let _ = good.shutdown(std::net::Shutdown::Write);

        let mut reader = BufReader::new(good);
        let mut replies = Vec::new();
        let mut line = String::new();
        while reader.read_line(&mut line).unwrap() > 0 {
            replies.push(serde_json::from_str::<Value>(&line).unwrap());
            line.clear();
        }
        assert_eq!(replies.len(), 3);
        assert_eq!(replies[0]["result"]["serverInfo"]["name"], about::NAME);
        assert_eq!(
            replies[1]["result"]["structuredContent"]["called"],
            "get_model"
        );
        assert_eq!(
            replies[2]["result"]["contents"][0]["uri"],
            "caditor://model/summary"
        );
        bridge.shutdown();
    }
}
