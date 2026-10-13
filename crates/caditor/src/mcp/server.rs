use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    io::{BufRead, Write},
    panic::{self, AssertUnwindSafe},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Sender},
    },
    thread,
};

use parking_lot::Mutex;
use serde_json::{Map, Value, json};

use crate::{
    about,
    mcp::{
        json_rpc::{self, Code, Incoming, Line, MAX_LINE_BYTES, Rejected, Rejection, RequestId},
        resources::{self, ResourceError},
        session::{Session, Work},
        tools::{Answer, Tool, ToolError},
    },
};

pub const PROTOCOL_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
const STRUCTURED_FROM: &str = "2025-06-18";
const CANCELLED: &str = "notifications/cancelled";
const PROGRESS: &str = "notifications/progress";
const INSTRUCTIONS: &str = "This server works on one caditor model, a parametric CAD model of \
                            features in a tree. Lengths are millimetres and angles degrees; \
                            values are expressions that take units and parameter names. Refer to \
                            features, bodies and sketch entities by the ids or names the list \
                            tools give, and to faces by list_faces keys, never by position. Each \
                            write is one undo step and answers with the features that fail \
                            after it, in plain words. Nothing is written to disk until save or \
                            export. The user guide pages are resources (caditor://guide/...).";

#[derive(Clone, Default)]
pub struct Cancellations(Arc<Mutex<BTreeMap<String, Arc<AtomicBool>>>>);

impl Cancellations {
    fn register(&self, id: &RequestId) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        self.0.lock().insert(id.key(), Arc::clone(&flag));
        flag
    }

    fn cancel(&self, params: &Value) {
        let Some(id) = params.get("requestId").and_then(RequestId::of) else {
            return;
        };
        if let Some(flag) = self.0.lock().get(&id.key()) {
            flag.store(true, Ordering::Release);
        }
    }

    fn forget(&self, id: &RequestId) {
        self.0.lock().remove(&id.key());
    }

    #[cfg(test)]
    pub fn is_cancelled(&self, id: &Value) -> bool {
        RequestId::of(id).is_some_and(|id| {
            self.0
                .lock()
                .get(&id.key())
                .is_some_and(|flag| flag.load(Ordering::Acquire))
        })
    }
}

enum Inbound {
    Request {
        id: RequestId,
        method: String,
        params: Value,
        cancelled: Arc<AtomicBool>,
    },
    Notification {
        method: String,
    },
    Rejected(Rejected),
}

struct Output<W> {
    writer: RefCell<W>,
    broken: Cell<bool>,
}

impl<W: Write> Output<W> {
    fn send(&self, message: &Value) {
        if self.broken.get() {
            return;
        }
        if let Err(error) = json_rpc::write_message(&mut *self.writer.borrow_mut(), message) {
            log::warn!("the client stopped reading: {error}");
            self.broken.set(true);
        }
    }
}

struct Server<W> {
    session: Session,
    output: Output<W>,
    protocol: &'static str,
    cancellations: Cancellations,
}

pub fn serve<R, W>(input: R, output: W, session: Session) -> Session
where
    R: BufRead + Send + 'static,
    W: Write,
{
    serve_watching(input, output, session, Cancellations::default())
}

pub fn serve_watching<R, W>(
    input: R,
    output: W,
    session: Session,
    cancellations: Cancellations,
) -> Session
where
    R: BufRead + Send + 'static,
    W: Write,
{
    let (sender, inbound) = mpsc::channel();
    let reading = cancellations.clone();
    let spawned = thread::Builder::new()
        .name("mcp input".to_owned())
        .spawn(move || read_messages(input, &sender, &reading));
    if let Err(error) = spawned {
        log::error!("could not start reading requests: {error}");
        return session;
    }
    let mut server = Server {
        session,
        output: Output {
            writer: RefCell::new(output),
            broken: Cell::new(false),
        },
        protocol: PROTOCOL_VERSIONS
            .first()
            .copied()
            .unwrap_or(STRUCTURED_FROM),
        cancellations,
    };
    for message in inbound {
        server.handle(message);
        if server.output.broken.get() {
            break;
        }
    }
    server.session
}

fn read_messages(mut input: impl BufRead, sender: &Sender<Inbound>, cancellations: &Cancellations) {
    loop {
        let line = match json_rpc::read_line(&mut input, MAX_LINE_BYTES) {
            Ok(line) => line,
            Err(error) => {
                log::warn!("could not read from the client: {error}");
                return;
            }
        };
        let inbound = match line {
            Line::End => return,
            Line::TooLong => Inbound::Rejected(Rejected {
                id: None,
                reason: Rejection::TooLong {
                    limit: MAX_LINE_BYTES,
                },
            }),
            Line::Text(bytes) => match json_rpc::parse(&bytes) {
                Ok(None | Some(Incoming::Response)) => continue,
                Ok(Some(Incoming::Notification { method, params })) if method == CANCELLED => {
                    cancellations.cancel(&params);
                    continue;
                }
                Ok(Some(Incoming::Notification { method, .. })) => Inbound::Notification { method },
                Ok(Some(Incoming::Request { id, method, params })) => {
                    let cancelled = cancellations.register(&id);
                    Inbound::Request {
                        id,
                        method,
                        params,
                        cancelled,
                    }
                }
                Err(rejected) => Inbound::Rejected(rejected),
            },
        };
        if sender.send(inbound).is_err() {
            return;
        }
    }
}

impl<W: Write> Server<W> {
    fn handle(&mut self, message: Inbound) {
        match message {
            Inbound::Request {
                id,
                method,
                params,
                cancelled,
            } => {
                let response = self.respond(&id, &method, params, &cancelled);
                self.cancellations.forget(&id);
                if !cancelled.load(Ordering::Acquire) {
                    self.output.send(&response);
                }
            }
            Inbound::Notification { method } => {
                log::debug!("notification {method}");
            }
            Inbound::Rejected(rejected) => {
                let error = json_rpc::error(
                    rejected.id.as_ref(),
                    rejected.reason.code(),
                    &rejected.reason.to_string(),
                );
                self.output.send(&error);
            }
        }
    }

    fn respond(
        &mut self,
        id: &RequestId,
        method: &str,
        params: Value,
        cancelled: &Arc<AtomicBool>,
    ) -> Value {
        let answered = match method {
            "initialize" => Ok(self.initialize(&params)),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({
                "tools": Tool::ALL.into_iter().map(Tool::listing).collect::<Vec<_>>()
            })),
            "tools/call" => self.call(params, cancelled),
            "resources/list" => Ok(resources::listing()),
            "resources/templates/list" => Ok(json!({ "resourceTemplates": [] })),
            "resources/read" => self.read_resource(&params, cancelled),
            other => Err((
                Code::MethodNotFound,
                format!("{} does not offer {other}", about::NAME),
            )),
        };
        match answered {
            Ok(result) => json_rpc::result(id, result),
            Err((code, message)) => json_rpc::error(Some(id), code, &message),
        }
    }

    fn initialize(&mut self, params: &Value) -> Value {
        let asked = params.get("protocolVersion").and_then(Value::as_str);
        self.protocol = PROTOCOL_VERSIONS
            .into_iter()
            .find(|version| Some(*version) == asked)
            .or_else(|| PROTOCOL_VERSIONS.first().copied())
            .unwrap_or(STRUCTURED_FROM);
        json!({
            "protocolVersion": self.protocol,
            "capabilities": {
                "tools": { "listChanged": false },
                "resources": { "listChanged": false, "subscribe": false }
            },
            "serverInfo": {
                "name": about::NAME,
                "title": "caditor parametric CAD",
                "version": about::VERSION
            },
            "instructions": INSTRUCTIONS
        })
    }

    fn call(
        &mut self,
        params: Value,
        cancelled: &Arc<AtomicBool>,
    ) -> Result<Value, (Code, String)> {
        let Value::Object(mut params) = params else {
            return Err((Code::InvalidParams, "tools/call takes an object".to_owned()));
        };
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or((
                Code::InvalidParams,
                "tools/call needs the tool's name".to_owned(),
            ))?
            .to_owned();
        let tool = Tool::named(&name)
            .ok_or_else(|| (Code::InvalidParams, format!("Unknown tool: {name}")))?;
        let arguments = params.remove("arguments").unwrap_or(Value::Null);
        let token = progress_token(&params);
        let Server {
            session, output, ..
        } = &mut *self;
        let step = Cell::new(0_u64);
        let report = |message: &str| {
            let Some(token) = &token else {
                return;
            };
            step.set(step.get().saturating_add(1));
            let mut progress = Map::new();
            progress.insert("progressToken".to_owned(), token.clone());
            progress.insert("progress".to_owned(), json!(step.get()));
            progress.insert("message".to_owned(), json!(message));
            output.send(&json_rpc::notification(PROGRESS, progress));
        };
        let work = Work::new(Arc::clone(cancelled), &report);
        let ran = panic::catch_unwind(AssertUnwindSafe(|| {
            tool.decode(arguments)
                .and_then(|call| call.run(session, &work))
        }));
        match ran {
            Ok(Ok(answer)) => Ok(self.answered(answer)),
            Ok(Err(error)) => Ok(failed(&error)),
            Err(_) => {
                self.session.forget_evaluation();
                Err((
                    Code::InternalError,
                    format!(
                        "{} ran into an internal error in {name}; the model keeps every change \
                         made before the call, and the log has the details",
                        about::NAME
                    ),
                ))
            }
        }
    }

    fn answered(&self, answer: Answer) -> Value {
        let Answer { data, image } = answer;
        let text = Value::Object(data.clone()).to_string();
        let mut content = vec![json!({ "type": "text", "text": text })];
        if let Some(png) = image {
            content.push(json!({ "type": "image", "data": base64(&png), "mimeType": "image/png" }));
        }
        let mut result = Map::new();
        result.insert("content".to_owned(), Value::Array(content));
        result.insert("isError".to_owned(), json!(false));
        if self.protocol >= STRUCTURED_FROM {
            result.insert("structuredContent".to_owned(), Value::Object(data));
        }
        Value::Object(result)
    }

    fn read_resource(
        &mut self,
        params: &Value,
        cancelled: &Arc<AtomicBool>,
    ) -> Result<Value, (Code, String)> {
        let uri = params
            .get("uri")
            .and_then(Value::as_str)
            .ok_or((Code::InvalidParams, "resources/read needs a uri".to_owned()))?;
        let quiet = |_: &str| {};
        let work = Work::new(Arc::clone(cancelled), &quiet);
        let session = &mut self.session;
        let read = panic::catch_unwind(AssertUnwindSafe(|| resources::read(session, &work, uri)));
        match read {
            Ok(Ok(contents)) => Ok(contents),
            Ok(Err(error @ ResourceError::NotFound { .. })) => {
                Err((Code::ResourceNotFound, error.to_string()))
            }
            Ok(Err(error)) => Err((Code::InternalError, error.to_string())),
            Err(_) => {
                self.session.forget_evaluation();
                Err((
                    Code::InternalError,
                    format!("{} ran into an internal error reading {uri}", about::NAME),
                ))
            }
        }
    }
}

fn progress_token(params: &Map<String, Value>) -> Option<Value> {
    params
        .get("_meta")
        .and_then(|meta| meta.get("progressToken"))
        .filter(|token| token.is_string() || token.is_number())
        .cloned()
}

fn failed(error: &ToolError) -> Value {
    json!({
        "content": [{ "type": "text", "text": sentence(&error.to_string()) }],
        "isError": true
    })
}

fn sentence(text: &str) -> String {
    let mut characters = text.chars();
    let first: String = characters
        .next()
        .map(char::to_uppercase)
        .into_iter()
        .flatten()
        .collect();
    let mut sentence = format!("{first}{}", characters.as_str());
    if !sentence.ends_with(['.', '?', '!']) {
        sentence.push('.');
    }
    sentence
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64(bytes: &[u8]) -> String {
    let symbol = |index: u32| {
        BASE64
            .get(usize::try_from(index & 63).unwrap_or_default())
            .copied()
            .map_or('=', char::from)
    };
    let mut text = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let [first, second, third] = match *chunk {
            [first] => [first, 0, 0],
            [first, second] => [first, second, 0],
            [first, second, third, ..] => [first, second, third],
            [] => continue,
        };
        let joined = u32::from(first) << 16 | u32::from(second) << 8 | u32::from(third);
        text.push(symbol(joined >> 18));
        text.push(symbol(joined >> 12));
        text.push(if chunk.len() > 1 {
            symbol(joined >> 6)
        } else {
            '='
        });
        text.push(if chunk.len() > 2 { symbol(joined) } else { '=' });
    }
    text
}
