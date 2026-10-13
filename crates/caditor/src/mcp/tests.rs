use std::{
    io::{self, BufRead, BufReader, Cursor, PipeReader, PipeWriter, Write},
    path::{Path, PathBuf},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use caditor_file::{Retention, StorageConfig};
use serde_json::{Value, json};
use tempfile::TempDir;

use super::{
    files::{self, FileError},
    json_rpc::{self, Line},
    server::{self, Cancellations, PROTOCOL_VERSIONS, base64},
    session::{OpenError, Opening, Session},
    tools::Tool,
};
use crate::{model::PanicFlush, samples::Sample};

const PATIENCE: Duration = Duration::from_secs(30);

pub fn opening(model: PathBuf) -> Opening {
    Opening {
        model,
        folders: Vec::new(),
        storage: StorageConfig::default(),
        panic_flush: PanicFlush::default(),
        retention: Retention::Standard,
    }
}

pub fn saved_plate(folder: &Path) -> PathBuf {
    let path = folder.join("plate.caditor");
    caditor_file::save(&Sample::Plate.document().unwrap(), &path, false).unwrap();
    path
}

struct Gate {
    cancellations: Cancellations,
    request: Value,
}

struct Gated {
    writer: PipeWriter,
    gate: Option<Gate>,
    message: Vec<u8>,
}

impl Write for Gated {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.message.extend_from_slice(bytes);
        self.writer.write_all(bytes)?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()?;
        let message = std::mem::take(&mut self.message);
        let progress = String::from_utf8_lossy(&message).contains("notifications/progress");
        if let (Some(gate), true) = (&self.gate, progress) {
            let deadline = Instant::now() + PATIENCE;
            while !gate.cancellations.is_cancelled(&gate.request) && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(1));
            }
        }
        Ok(())
    }
}

pub struct Client {
    requests: Option<PipeWriter>,
    responses: BufReader<PipeReader>,
    next_id: u64,
    server: Option<JoinHandle<()>>,
    pub notifications: Vec<Value>,
}

impl Client {
    pub fn start(opening: Opening) -> Self {
        Self::start_gated(opening, Cancellations::default(), None)
    }

    fn start_gated(opening: Opening, cancellations: Cancellations, gate: Option<Gate>) -> Self {
        let (input, requests) = io::pipe().unwrap();
        let (responses, output) = io::pipe().unwrap();
        let server = thread::spawn(move || {
            let session = Session::open(opening).unwrap();
            let output = Gated {
                writer: output,
                gate,
                message: Vec::new(),
            };
            let session =
                server::serve_watching(BufReader::new(input), output, session, cancellations);
            if let Some(closing) = session.close() {
                assert!(closing.wait(PATIENCE));
            }
        });
        Self {
            requests: Some(requests),
            responses: BufReader::new(responses),
            next_id: 1,
            server: Some(server),
            notifications: Vec::new(),
        }
    }

    pub fn send(&mut self, message: &Value) {
        let requests = self.requests.as_mut().unwrap();
        writeln!(requests, "{message}").unwrap();
        requests.flush().unwrap();
    }

    pub fn read_message(&mut self) -> Value {
        let mut line = String::new();
        let read = self.responses.read_line(&mut line).unwrap();
        assert!(read > 0, "the server ended without answering");
        serde_json::from_str(&line).unwrap()
    }

    pub fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        self.response_to(&json!(id))
    }

    pub fn response_to(&mut self, id: &Value) -> Value {
        loop {
            let message = self.read_message();
            if message.get("id") == Some(id) {
                return message;
            }
            self.notifications.push(message);
        }
    }

    pub fn initialize(&mut self, version: &str) -> Value {
        let answer = self.request(
            "initialize",
            json!({
                "protocolVersion": version,
                "capabilities": {},
                "clientInfo": { "name": "test", "version": "1.0.0" }
            }),
        );
        self.send(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
        answer
    }

    pub fn call(&mut self, tool: &str, arguments: Value) -> Value {
        let response = self.request(
            "tools/call",
            json!({ "name": tool, "arguments": arguments }),
        );
        response
            .get("result")
            .cloned()
            .unwrap_or_else(|| panic!("{tool} gave no result: {response}"))
    }

    pub fn tool(&mut self, tool: &str, arguments: Value) -> Value {
        let result = self.call(tool, arguments);
        assert_eq!(result["isError"], json!(false), "{tool}: {result}");
        let text = result["content"][0]["text"].as_str().unwrap();
        let data: Value = serde_json::from_str(text).unwrap();
        if let Some(structured) = result.get("structuredContent") {
            assert_eq!(structured, &data);
        }
        data
    }

    pub fn tool_error(&mut self, tool: &str, arguments: Value) -> String {
        let result = self.call(tool, arguments);
        assert_eq!(result["isError"], json!(true), "{tool}: {result}");
        result["content"][0]["text"].as_str().unwrap().to_owned()
    }

    pub fn finish(mut self) {
        self.requests = None;
        if let Some(server) = self.server.take() {
            server.join().unwrap();
        }
    }
}

fn exchange(session: Session, lines: &[&str]) -> (Vec<Value>, Session) {
    let input = lines.join("\n").into_bytes();
    let mut output = Vec::new();
    let session = server::serve(Cursor::new(input), &mut output, session);
    let messages = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    (messages, session)
}

fn empty_session(folder: &TempDir) -> Session {
    Session::open(opening(folder.path().join("new.caditor"))).unwrap()
}

#[test]
fn initialize_agrees_on_the_clients_version_or_offers_the_newest() {
    let folder = TempDir::new().unwrap();
    let (messages, session) = exchange(
        empty_session(&folder),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"1999-01-01"}}"#,
            r#"{"jsonrpc":"2.0","id":"three","method":"ping"}"#,
        ],
    );
    drop(session);

    assert_eq!(messages.len(), 3, "{messages:?}");
    assert_eq!(messages[0]["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(messages[0]["result"]["serverInfo"]["name"], "caditor");
    assert_eq!(
        messages[0]["result"]["capabilities"]["tools"]["listChanged"],
        false
    );
    assert_eq!(
        messages[1]["result"]["protocolVersion"],
        PROTOCOL_VERSIONS[0]
    );
    assert_eq!(
        messages[2],
        json!({ "jsonrpc": "2.0", "id": "three", "result": {} })
    );
}

#[test]
fn every_tool_is_listed_with_an_object_schema_and_hints() {
    let folder = TempDir::new().unwrap();
    let (messages, _) = exchange(
        empty_session(&folder),
        &[r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#],
    );

    let tools = messages[0]["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), Tool::ALL.len());
    for tool in tools {
        assert_eq!(tool["inputSchema"]["type"], "object", "{tool}");
        assert!(tool["description"].as_str().unwrap().len() > 20, "{tool}");
        assert_eq!(tool["annotations"]["openWorldHint"], false);
    }
    let read_only = |name: &str| {
        tools.iter().find(|tool| tool["name"] == name).unwrap()["annotations"]["readOnlyHint"]
            .clone()
    };
    assert_eq!(read_only("list_bodies"), true);
    assert_eq!(read_only("add_hole"), false);
}

#[test]
fn malformed_messages_get_json_rpc_errors_and_the_server_carries_on() {
    let folder = TempDir::new().unwrap();
    let (messages, _) = exchange(
        empty_session(&folder),
        &[
            "not json at all",
            "",
            "[]",
            "42",
            r#"{"jsonrpc":"1.0","id":7,"method":"ping"}"#,
            r#"{"jsonrpc":"2.0","id":null,"method":"ping"}"#,
            r#"{"jsonrpc":"2.0","id":8,"method":"no/such/method"}"#,
            r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"frobnicate"}}"#,
            r#"{"jsonrpc":"2.0","id":10,"method":"tools/call","params":"name"}"#,
            r#"{"jsonrpc":"2.0","method":"notifications/whatever"}"#,
            r#"{"jsonrpc":"2.0","id":99,"result":{}}"#,
            r#"{"jsonrpc":"2.0","id":11,"method":"ping"}"#,
        ],
    );

    let codes: Vec<(Value, Value)> = messages
        .iter()
        .map(|message| {
            (
                message["id"].clone(),
                message
                    .pointer("/error/code")
                    .cloned()
                    .unwrap_or(Value::Null),
            )
        })
        .collect();
    assert_eq!(
        codes,
        vec![
            (Value::Null, json!(-32700)),
            (Value::Null, json!(-32600)),
            (Value::Null, json!(-32600)),
            (json!(7), json!(-32600)),
            (Value::Null, json!(-32600)),
            (json!(8), json!(-32601)),
            (json!(9), json!(-32602)),
            (json!(10), json!(-32600)),
            (json!(11), Value::Null),
        ]
    );
    assert_eq!(messages[6]["error"]["message"], "Unknown tool: frobnicate");
}

#[test]
fn a_line_too_long_is_refused_and_the_next_is_read() {
    let mut input = Cursor::new(b"0123456789\nshort\nlast".to_vec());

    assert_eq!(json_rpc::read_line(&mut input, 5).unwrap(), Line::TooLong);
    assert_eq!(
        json_rpc::read_line(&mut input, 5).unwrap(),
        Line::Text(b"short\n".to_vec())
    );
    assert_eq!(
        json_rpc::read_line(&mut input, 5).unwrap(),
        Line::Text(b"last".to_vec())
    );
    assert_eq!(json_rpc::read_line(&mut input, 5).unwrap(), Line::End);
}

#[test]
fn tool_mistakes_are_tool_errors_in_words() {
    let folder = TempDir::new().unwrap();
    let mut client = Client::start(opening(saved_plate(folder.path())));
    client.initialize("2025-06-18");

    let no_body = client.tool_error("list_faces", json!({ "body": "Nowhere" }));
    let wrong_type = client.tool_error("list_faces", json!({ "body": [1, 2] }));
    let unknown_field = client.tool_error("undo", json!({ "steps": 3 }));
    let nothing_to_undo = client.tool_error("undo", json!({}));
    let bad_value = client.tool_error(
        "set_parameter",
        json!({ "name": "length", "expression": "80 mm +" }),
    );
    let not_a_sketch = client.tool_error("get_sketch", json!({ "sketch": "Plate" }));
    client.finish();

    assert_eq!(
        no_body,
        "There is no body “Nowhere”; list_bodies names every body."
    );
    assert!(
        wrong_type.starts_with("The arguments do not fit list_faces"),
        "{wrong_type}"
    );
    assert!(
        unknown_field.contains("unknown field `steps`"),
        "{unknown_field}"
    );
    assert_eq!(nothing_to_undo, "There is nothing to undo.");
    assert!(
        bad_value.starts_with("Expression “80 mm +” cannot be used"),
        "{bad_value}"
    );
    assert!(
        not_a_sketch.starts_with("Plate is not a sketch"),
        "{not_a_sketch}"
    );
}

#[test]
fn reading_tools_describe_the_model_in_words_and_numbers() {
    let folder = TempDir::new().unwrap();
    let mut client = Client::start(opening(saved_plate(folder.path())));
    client.initialize("2025-06-18");

    let model = client.tool("get_model", json!({}));
    let features = client.tool("list_features", json!({}));
    let parameters = client.tool("list_parameters", json!({}));
    let bodies = client.tool("list_bodies", json!({}));
    let sketch = client.tool("get_sketch", json!({ "sketch": "Plate sketch" }));
    client.finish();

    assert_eq!(model["name"], "plate.caditor");
    assert_eq!(model["unsaved_changes"], false);
    assert_eq!(model["failing"], json!([]));
    assert_eq!(features["features"][1]["name"], "Plate");
    assert_eq!(features["features"][1]["kind"], "extrude");
    assert_eq!(features["features"][1]["state"], "up to date");
    assert_eq!(
        features["features"][0]["sketch_status"],
        "Fully constrained"
    );
    assert_eq!(parameters["parameters"][0]["name"], "length");
    assert_eq!(parameters["parameters"][0]["value"], "80 mm");
    let volume = bodies["bodies"][0]["volume"].as_f64().unwrap();
    let expected = 80.0 * 50.0 * 6.0 - 2.0 * std::f64::consts::PI * 16.0 * 6.0;
    assert!((volume - expected).abs() < 1e-6 * expected, "{volume}");
    assert_eq!(
        bodies["bodies"][0]["bounding_box"]["size"],
        json!([80.0, 50.0, 6.0])
    );
    assert_eq!(sketch["degrees_of_freedom"], 0);
    assert_eq!(sketch["lies_on"], "XY plane");
    assert!(
        sketch["constraints"]
            .as_array()
            .unwrap()
            .iter()
            .any(|constraint| {
                constraint["expression"] == "length" && constraint["value"] == json!(80.0)
            })
    );
}

#[test]
fn structured_content_comes_only_with_protocols_that_know_it() {
    let folder = TempDir::new().unwrap();
    let mut client = Client::start(opening(saved_plate(folder.path())));
    client.initialize("2024-11-05");

    let result = client.call("list_parameters", json!({}));
    client.finish();

    assert!(result.get("structuredContent").is_none());
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("\"length\"")
    );
}

#[test]
fn a_call_reports_progress_and_a_cancelled_one_gets_no_answer() {
    let folder = TempDir::new().unwrap();
    let cancellations = Cancellations::default();
    let gate = Gate {
        cancellations: cancellations.clone(),
        request: json!("slow"),
    };
    let mut client = Client::start_gated(
        opening(saved_plate(folder.path())),
        cancellations,
        Some(gate),
    );
    client.initialize("2025-06-18");

    client.send(&json!({
        "jsonrpc": "2.0",
        "id": "slow",
        "method": "tools/call",
        "params": { "name": "list_bodies", "arguments": {}, "_meta": { "progressToken": 5 } }
    }));
    let progress = client.read_message();
    client.send(&json!({
        "jsonrpc": "2.0",
        "method": "notifications/cancelled",
        "params": { "requestId": "slow", "reason": "changed my mind" }
    }));
    let pong = client.request("ping", json!({}));
    let unanswered = std::mem::take(&mut client.notifications);
    let bodies = client.tool("list_bodies", json!({}));
    client.finish();

    assert_eq!(progress["method"], "notifications/progress");
    assert_eq!(progress["params"]["progressToken"], 5);
    assert_eq!(progress["params"]["progress"], 1);
    assert!(
        unanswered
            .iter()
            .all(|message| message.get("id") != Some(&json!("slow"))),
        "{unanswered:?}"
    );
    assert_eq!(pong["result"], json!({}));
    assert_eq!(bodies["bodies"].as_array().unwrap().len(), 1);
}

#[test]
fn resources_hold_the_model_and_the_user_guide() {
    let folder = TempDir::new().unwrap();
    let mut client = Client::start(opening(saved_plate(folder.path())));
    client.initialize("2025-06-18");

    let listed = client.request("resources/list", json!({}));
    let guide = client.request("resources/read", json!({ "uri": "caditor://guide/hole" }));
    let summary = client.request(
        "resources/read",
        json!({ "uri": "caditor://model/summary" }),
    );
    let missing = client.request("resources/read", json!({ "uri": "caditor://guide/nope" }));
    client.finish();

    let resources = listed["result"]["resources"].as_array().unwrap();
    assert!(
        resources
            .iter()
            .any(|resource| resource["uri"] == "caditor://model/parameters")
    );
    assert!(
        resources
            .iter()
            .any(|resource| resource["uri"] == "caditor://guide/mcp")
    );
    let page = guide["result"]["contents"][0]["text"].as_str().unwrap();
    assert!(page.starts_with("# Hole\n"), "{page}");
    assert!(!page.contains("{command:"), "{page}");
    let model: Value =
        serde_json::from_str(summary["result"]["contents"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(model["bodies"], 1);
    assert_eq!(missing["error"]["code"], -32002);
}

#[test]
fn a_picture_is_drawn_when_a_graphics_adapter_can() {
    let folder = TempDir::new().unwrap();
    let mut client = Client::start(opening(saved_plate(folder.path())));
    client.initialize("2025-06-18");

    let result = client.call(
        "render_view",
        json!({ "view": "top", "width": 320, "height": 200 }),
    );
    let too_big = client.tool_error("render_view", json!({ "width": 100_000 }));
    client.finish();

    assert_eq!(
        too_big,
        "A picture is 64 to 2048 pixels a side, so 100000 pixels cannot be drawn."
    );
    if result["isError"] == json!(true) {
        assert!(
            std::env::var_os("CADITOR_REQUIRE_GPU").is_none(),
            "{result}"
        );
        return;
    }
    assert_eq!(result["content"][1]["type"], "image");
    assert_eq!(result["content"][1]["mimeType"], "image/png");
    assert!(
        result["content"][1]["data"]
            .as_str()
            .unwrap()
            .starts_with("iVBORw0KGgo")
    );
}

#[test]
fn files_are_written_only_inside_the_allowed_folders() {
    let folder = TempDir::new().unwrap();
    let other = TempDir::new().unwrap();
    std::fs::create_dir(folder.path().join("parts")).unwrap();
    let model = folder.path().join("plate.caditor");
    let folders = vec![
        dunce::canonicalize(folder.path()).unwrap(),
        dunce::canonicalize(other.path()).unwrap(),
    ];
    let allowed = |file: &str| files::allowed(&model, &folders, file);

    assert!(allowed("plate.step").is_ok());
    assert!(allowed("parts/plate.stl").is_ok());
    assert!(allowed(&other.path().join("plate.3mf").display().to_string()).is_ok());
    assert!(matches!(
        allowed("../plate.step"),
        Err(FileError::OutsideFolders { .. })
    ));
    assert!(matches!(
        allowed("missing/plate.step"),
        Err(FileError::NoFolder { .. })
    ));
    assert!(matches!(
        allowed("parts/.."),
        Err(FileError::NotAFile { .. })
    ));
}

#[test]
fn only_a_model_file_in_an_existing_folder_is_served() {
    let folder = TempDir::new().unwrap();

    let step = Session::open(opening(folder.path().join("part.step")));
    let nowhere = Session::open(opening(folder.path().join("gone/part.caditor")));
    let fresh = Session::open(opening(folder.path().join("part.caditor"))).unwrap();

    assert!(matches!(step, Err(OpenError::NotAModelFile { .. })));
    assert!(matches!(nowhere, Err(OpenError::NoFolder { .. })));
    assert!(fresh.is_unsaved());
}

#[test]
fn unsaved_changes_stay_in_the_journal_and_hold_the_model_until_recovered() {
    let folder = TempDir::new().unwrap();
    let model = saved_plate(folder.path());
    let mut client = Client::start(opening(model.clone()));
    client.initialize("2025-06-18");
    client.tool(
        "set_parameter",
        json!({ "name": "length", "expression": "90 mm" }),
    );
    client.finish();

    let again = Session::open(opening(model));

    assert!(
        matches!(again, Err(OpenError::Unrecovered { .. })),
        "{:?}",
        again.err()
    );
}

#[test]
fn base64_matches_the_reference_vectors() {
    for (plain, encoded) in [
        ("", ""),
        ("f", "Zg=="),
        ("fo", "Zm8="),
        ("foo", "Zm9v"),
        ("foob", "Zm9vYg=="),
        ("fooba", "Zm9vYmE="),
        ("foobar", "Zm9vYmFy"),
    ] {
        assert_eq!(base64(plain.as_bytes()), encoded);
    }
}
