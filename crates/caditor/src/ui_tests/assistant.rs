use std::{
    io::{BufRead, BufReader, Write},
    net::{Ipv4Addr, SocketAddrV4, TcpStream},
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
    thread,
    time::{Duration, Instant},
};

use caditor_ai::{
    ApiProvider,
    fake::{ScriptedStream, ScriptedTransport},
    normalize_schema,
};
use caditor_document::Document;
use serde_json::Value;

use super::Harness;
use crate::{
    app::Workspace,
    assistant::{
        AssistantCommand, AssistantFailure, AssistantServices, Backend, CliState, CliTool,
        cli_runner::{CliBackend, CliTurnRequest, CliTurnResult},
    },
    cloud::keyring::{AiKeyring, FakeAiKeyring},
    commands::Command,
    mcp::tools::Tool,
    model::Action,
};

struct FakeCli {
    calls_bridge: bool,
}

impl CliBackend for FakeCli {
    fn discover(&self, tool: CliTool, _override_path: &str) -> CliState {
        CliState::Found(PathBuf::from(format!(
            "/fake/bin/{}",
            tool.default_binary()
        )))
    }

    fn run_turn(
        &self,
        request: &CliTurnRequest<'_>,
        _cancel: &Arc<AtomicBool>,
        on_text: &mut dyn FnMut(String),
    ) -> Result<CliTurnResult, AssistantFailure> {
        if self.calls_bridge {
            let mut stream =
                TcpStream::connect(SocketAddrV4::new(Ipv4Addr::LOCALHOST, request.bridge_port))
                    .map_err(|error| AssistantFailure::CliLaunch {
                        tool: request.tool,
                        error_kind: error.kind(),
                    })?;
            writeln!(stream, "{}", request.bridge_token).unwrap();
            writeln!(
                stream,
                "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{{\"protocolVersion\":\"2025-11-25\"}}}}"
            )
            .unwrap();
            writeln!(
                stream,
                "{{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{{\"name\":\"set_parameter\",\"arguments\":{{\"name\":\"cli_width\",\"expression\":\"42 mm\"}}}}}}"
            )
            .unwrap();
            stream.flush().unwrap();
            let _ = stream.shutdown(std::net::Shutdown::Write);
            let mut reader = BufReader::new(stream);
            let mut replies = Vec::new();
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap() > 0 {
                replies.push(serde_json::from_str::<Value>(&line).unwrap());
                line.clear();
            }
            assert_eq!(replies.len(), 2);
            assert!(
                replies[1]["result"]["isError"].is_null()
                    || replies[1]["result"]["isError"] == Value::Bool(false)
            );
        }
        let reply = "Set cli_width to 42 mm via live MCP bridge.".to_owned();
        on_text(reply.clone());
        Ok(CliTurnResult {
            session_id: Some("cli-session-1".to_owned()),
            text: reply,
        })
    }
}

fn wait_for_assistant(harness: &mut Harness) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while harness.workspace.assistant.is_busy() {
        assert!(
            Instant::now() < deadline,
            "assistant turn did not finish within deadline"
        );
        harness.frame();
        harness.settle();
        thread::sleep(Duration::from_millis(10));
    }
    harness.frame();
    harness.settle();
}

#[test]
fn api_assistant_calls_live_mcp_tools_updates_model_and_lands_in_undo_history() {
    let transport = Arc::new(ScriptedTransport::new([
        Ok(ScriptedStream::ok([
            "event: content_block_start\n",
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n",
            "\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Adding a 40x30x10 mm box.\"}}\n",
            "\n",
            "event: content_block_stop\n",
            "data: {\"type\":\"content_block_stop\",\"index\":0}\n",
            "\n",
            "event: content_block_start\n",
            "data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_box\",\"name\":\"add_primitive\",\"input\":{}}}\n",
            "\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"kind\\\":\\\"box\\\",\\\"length\\\":40,\\\"width\\\":30,\\\"height\\\":10}\"}}\n",
            "\n",
            "event: content_block_stop\n",
            "data: {\"type\":\"content_block_stop\",\"index\":1}\n",
            "\n",
            "event: message_delta\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"}}\n",
            "\n",
        ])),
        Ok(ScriptedStream::ok([
            "event: content_block_start\n",
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n",
            "\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Done — created the box.\"}}\n",
            "\n",
            "event: content_block_stop\n",
            "data: {\"type\":\"content_block_stop\",\"index\":0}\n",
            "\n",
            "event: message_delta\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n",
            "\n",
        ])),
    ]));
    let keyring: Arc<dyn AiKeyring> = FakeAiKeyring::reachable();
    let cli = Arc::new(FakeCli {
        calls_bridge: false,
    });
    let mut harness = Harness::starting(None, Document::default(), Workspace::new());
    let prefs = harness.workspace.preferences.assistant.clone();
    harness.workspace.assistant.set_services(
        AssistantServices::for_tests(Arc::clone(&transport) as _, cli, Arc::clone(&keyring), None),
        &prefs,
    );
    harness.perform(Action::Assistant(AssistantCommand::SaveApiKey(
        ApiProvider::Claude,
        "sk-ant-live-test".to_owned(),
    )));
    let deadline = Instant::now() + Duration::from_secs(5);
    while !harness
        .workspace
        .assistant
        .key_source(ApiProvider::Claude)
        .is_available()
    {
        assert!(Instant::now() < deadline, "key save did not arrive");
        harness.frame();
        thread::sleep(Duration::from_millis(5));
    }

    harness.workspace.queue_command(Command::Assistant);
    harness.frame();
    assert!(harness.workspace.assistant.open);

    harness.perform(Action::Assistant(AssistantCommand::SendPrompt(
        "Make a 40x30x10 mm box".to_owned(),
    )));
    wait_for_assistant(&mut harness);

    assert_eq!(harness.model.document().features().len(), 1);
    assert_eq!(harness.model.evaluation().bodies().count(), 1);
    assert!(harness.model.undo_label().is_some());
    assert!(harness.workspace.assistant.entries().iter().any(|entry| {
        entry.text.contains("Done — created the box.")
            && entry
                .tools
                .iter()
                .any(|tool| tool.tool_name == "add_primitive")
    }));

    harness.perform(Action::Undo);
    harness.settle();
    assert_eq!(harness.model.document().features().len(), 0);
}

#[test]
fn cli_wrapper_connects_to_live_loopback_mcp_bridge_and_edits_open_model() {
    let transport = Arc::new(ScriptedTransport::default());
    let keyring: Arc<dyn AiKeyring> = FakeAiKeyring::reachable();
    let cli = Arc::new(FakeCli { calls_bridge: true });
    let mut harness = Harness::starting(None, Document::default(), Workspace::new());
    harness.perform(Action::Assistant(AssistantCommand::SelectBackend(
        Backend::Cli(CliTool::ClaudeCode),
    )));
    let prefs = harness.workspace.preferences.assistant.clone();
    harness.workspace.assistant.set_services(
        AssistantServices::for_tests(transport, cli, keyring, None),
        &prefs,
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while !harness
        .workspace
        .assistant
        .cli_state(CliTool::ClaudeCode)
        .is_available()
    {
        assert!(Instant::now() < deadline, "cli discovery did not arrive");
        harness.frame();
        thread::sleep(Duration::from_millis(5));
    }

    harness.workspace.queue_command(Command::Assistant);
    harness.frame();
    harness.perform(Action::Assistant(AssistantCommand::SendPrompt(
        "Set cli_width to 42 mm".to_owned(),
    )));
    wait_for_assistant(&mut harness);

    assert!(
        harness
            .model
            .document()
            .parameters()
            .iter()
            .any(|param| param.name == "cli_width")
    );
    assert!(harness.workspace.assistant.entries().iter().any(|entry| {
        entry
            .text
            .contains("Set cli_width to 42 mm via live MCP bridge.")
    }));
}

#[test]
fn every_caditor_mcp_tool_schema_normalizes_cleanly_for_gemini() {
    fn has_disallowed(value: &Value) -> bool {
        match value {
            Value::Object(map) => {
                if map.contains_key("additionalProperties") {
                    return true;
                }
                if let Some(Value::Array(_)) = map.get("type") {
                    return true;
                }
                map.values().any(has_disallowed)
            }
            Value::Array(items) => items.iter().any(has_disallowed),
            _ => false,
        }
    }

    for tool in Tool::ALL {
        let normalized = normalize_schema(&tool.input_schema());
        assert!(
            !has_disallowed(&normalized),
            "{} still has unsupported Gemini schema keys: {normalized}",
            tool.name()
        );
    }
}
