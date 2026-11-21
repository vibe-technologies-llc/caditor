use std::{
    env, fs,
    io::{self, BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use serde_json::{Value, json};

use crate::{
    assistant::state::{AssistantFailure, CliState, CliTool},
    cloud::private,
    mcp::bridge::{LoopbackBridge, TOKEN_ENV},
};

const CANCEL_POLL: Duration = Duration::from_millis(40);
const TEMP_SUBDIR: &str = "assistant/cli-runtime";

pub struct CliTurnRequest<'a> {
    pub tool: CliTool,
    pub binary: PathBuf,
    pub prompt: &'a str,
    pub system_prompt: &'a str,
    pub session_id: Option<&'a str>,
    pub bridge_port: u16,
    pub bridge_token: &'a str,
    pub state_dir: Option<&'a Path>,
}

#[derive(Debug, Default)]
pub struct CliTurnResult {
    pub session_id: Option<String>,
    pub text: String,
}

pub trait CliBackend: Send + Sync {
    fn discover(&self, tool: CliTool, override_path: &str) -> CliState;

    fn run_turn(
        &self,
        request: &CliTurnRequest<'_>,
        cancel: &Arc<AtomicBool>,
        on_text: &mut dyn FnMut(String),
    ) -> Result<CliTurnResult, AssistantFailure>;
}

pub struct SystemCliBackend;

impl CliBackend for SystemCliBackend {
    fn discover(&self, tool: CliTool, override_path: &str) -> CliState {
        let trimmed = override_path.trim();

        if !trimmed.is_empty() {
            let path = PathBuf::from(trimmed);

            return if path.is_file() {
                CliState::Found(path)
            } else {
                CliState::Missing {
                    expected: trimmed.to_owned(),
                }
            };
        }

        let binary = tool.default_binary();
        match find_on_path(binary) {
            Some(found) => CliState::Found(found),
            None => CliState::Missing {
                expected: binary.to_owned(),
            },
        }
    }

    fn run_turn(
        &self,
        request: &CliTurnRequest<'_>,
        cancel: &Arc<AtomicBool>,
        on_text: &mut dyn FnMut(String),
    ) -> Result<CliTurnResult, AssistantFailure> {
        let exe = env::current_exe().map_err(|error| AssistantFailure::CliLaunch {
            tool: request.tool,
            error_kind: error.kind(),
        })?;
        let runtime_dir =
            make_runtime_dir(request.state_dir, request.bridge_port).map_err(|error| {
                AssistantFailure::CliLaunch {
                    tool: request.tool,
                    error_kind: error.kind(),
                }
            })?;
        let _cleanup = CleanupDir(runtime_dir.clone());

        let mut command = build_command(request, &exe, &runtime_dir).map_err(|error| {
            AssistantFailure::CliLaunch {
                tool: request.tool,
                error_kind: error.kind(),
            }
        })?;

        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = command
            .spawn()
            .map_err(|error| AssistantFailure::CliLaunch {
                tool: request.tool,
                error_kind: error.kind(),
            })?;

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let stderr_handle = thread::Builder::new()
            .name("cli stderr".to_owned())
            .spawn(move || {
                let mut collected = String::new();

                if let Some(stream) = stderr {
                    let mut reader = BufReader::new(stream);

                    let _ = io::Read::read_to_string(&mut reader, &mut collected);
                }
                collected
            })
            .ok();

        let mut result = CliTurnResult::default();
        let (line_tx, line_rx) = std::sync::mpsc::channel::<String>();
        let cancel_watcher = Arc::clone(cancel);

        let stdout_handle = thread::Builder::new()
            .name("cli stdout".to_owned())
            .spawn(move || {
                if let Some(stream) = stdout {
                    let mut reader = BufReader::new(stream);
                    let mut line = String::new();
                    loop {
                        if cancel_watcher.load(Ordering::Acquire) {
                            break;
                        }
                        line.clear();
                        match reader.read_line(&mut line) {
                            Ok(0) | Err(_) => break,
                            Ok(_) => {
                                if line_tx.send(line.clone()).is_err() {
                                    break;
                                }
                            }
                        }
                    }
                }
            })
            .ok();

        let status = loop {
            while let Ok(line) = line_rx.try_recv() {
                parse_cli_line(request.tool, &line, &mut result, on_text);
            }
            if cancel.load(Ordering::Acquire) {
                let _ = child.kill();
                let _ = child.wait();

                return Err(AssistantFailure::Api(caditor_ai::AiError::Cancelled));
            }
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => thread::sleep(CANCEL_POLL),
                Err(error) => {
                    let _ = child.kill();

                    return Err(AssistantFailure::CliLaunch {
                        tool: request.tool,
                        error_kind: error.kind(),
                    });
                }
            }
        };

        if let Some(handle) = stdout_handle {
            let _ = handle.join();
        }
        while let Ok(line) = line_rx.try_recv() {
            parse_cli_line(request.tool, &line, &mut result, on_text);
        }

        let stderr_text = stderr_handle
            .and_then(|handle| handle.join().ok())
            .unwrap_or_default();

        if !status.success() {
            let code = status.code().unwrap_or(-1);
            let output = if !stderr_text.trim().is_empty() {
                stderr_text.trim().to_owned()
            } else if !result.text.trim().is_empty() {
                result.text.trim().to_owned()
            } else {
                "no output was printed".to_owned()
            };

            return Err(AssistantFailure::CliExited {
                tool: request.tool,
                code,
                output,
            });
        }
        Ok(result)
    }
}

struct CleanupDir(PathBuf);

impl Drop for CleanupDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn make_runtime_dir(state_dir: Option<&Path>, port: u16) -> io::Result<PathBuf> {
    let base = match state_dir {
        Some(dir) => dir.join(TEMP_SUBDIR),
        None => env::temp_dir().join("caditor-assistant"),
    };
    let dir = base.join(format!("bridge-{port}"));

    private::create_dir_all(&dir)?;
    Ok(dir)
}

fn build_command(
    request: &CliTurnRequest<'_>,
    caditor_exe: &Path,
    runtime_dir: &Path,
) -> io::Result<Command> {
    let port_text = request.bridge_port.to_string();
    let exe_str = caditor_exe.display().to_string();
    let mut command = Command::new(&request.binary);

    command.env(TOKEN_ENV, request.bridge_token);
    match request.tool {
        CliTool::ClaudeCode => {
            let config = json!({
                "mcpServers": {
                    "caditor": {
                        "command": exe_str,
                        "args": ["mcp", "--connect", port_text],
                        "env": {
                            TOKEN_ENV: request.bridge_token
                        }
                    }
                }
            });
            let config_path = runtime_dir.join("claude-mcp.json");
            let bytes = serde_json::to_vec(&config).map_err(io::Error::other)?;

            caditor_file::write_privately(&config_path, &bytes)?;
            command
                .arg("-p")
                .arg(request.prompt)
                .arg("--output-format")
                .arg("stream-json")
                .arg("--verbose")
                .arg("--mcp-config")
                .arg(&config_path)
                .arg("--strict-mcp-config")
                .arg("--append-system-prompt")
                .arg(request.system_prompt)
                .arg("--permission-mode")
                .arg("bypassPermissions");
            if let Some(session_id) = request.session_id {
                command.arg("--resume").arg(session_id);
            }
        }
        CliTool::GeminiCli => {
            let gemini_dir = runtime_dir.join(".gemini");

            private::create_dir_all(&gemini_dir)?;

            let settings = json!({
                "mcpServers": {
                    "caditor": {
                        "command": exe_str,
                        "args": ["mcp", "--connect", port_text],
                        "env": {
                            TOKEN_ENV: request.bridge_token
                        }
                    }
                }
            });
            let settings_path = gemini_dir.join("settings.json");
            let bytes = serde_json::to_vec(&settings).map_err(io::Error::other)?;

            caditor_file::write_privately(&settings_path, &bytes)?;

            let combined_prompt = format!(
                "{}\n\nUser request:\n{}",
                request.system_prompt, request.prompt
            );

            command
                .current_dir(runtime_dir)
                .arg("-p")
                .arg(combined_prompt)
                .arg("--output-format")
                .arg("stream-json")
                .arg("--yolo");
        }
        CliTool::CodexCli => {
            let args_toml = format!("[\"mcp\", \"--connect\", \"{port_text}\"]");
            let cmd_toml = serde_json::to_string(&exe_str).map_err(io::Error::other)?;
            let token_toml =
                serde_json::to_string(request.bridge_token).map_err(io::Error::other)?;
            let combined_prompt = format!(
                "{}\n\nUser request:\n{}",
                request.system_prompt, request.prompt
            );

            command
                .arg("exec")
                .arg("--json")
                .arg("--skip-git-repo-check")
                .arg("-c")
                .arg(format!("mcp_servers.caditor.command={cmd_toml}"))
                .arg("-c")
                .arg(format!("mcp_servers.caditor.args={args_toml}"))
                .arg("-c")
                .arg(format!("mcp_servers.caditor.env.{TOKEN_ENV}={token_toml}"))
                .arg(combined_prompt);
        }
    }
    Ok(command)
}

pub fn parse_cli_line(
    tool: CliTool,
    raw: &str,
    result: &mut CliTurnResult,
    on_text: &mut dyn FnMut(String),
) {
    let trimmed = raw.trim();

    if trimmed.is_empty() {
        return;
    }

    let Ok(parsed) = serde_json::from_str::<Value>(trimmed) else {
        let mut chunk = trimmed.to_owned();

        chunk.push('\n');
        result.text.push_str(&chunk);
        on_text(chunk);
        return;
    };

    if let Some(session_id) = parsed.get("session_id").and_then(Value::as_str)
        && !session_id.is_empty()
    {
        result.session_id = Some(session_id.to_owned());
    }
    match tool {
        CliTool::ClaudeCode => {
            let kind = parsed.get("type").and_then(Value::as_str).unwrap_or("");
            if kind == "assistant"
                && let Some(contents) = parsed
                    .get("message")
                    .and_then(|message| message.get("content"))
                    .and_then(Value::as_array)
            {
                for item in contents {
                    if item.get("type").and_then(Value::as_str) == Some("text")
                        && let Some(text) = item.get("text").and_then(Value::as_str)
                        && !text.is_empty()
                    {
                        append_delta(text, result, on_text);
                    }
                }
            } else if kind == "result"
                && result.text.is_empty()
                && let Some(text) = parsed.get("result").and_then(Value::as_str)
                && !text.is_empty()
            {
                append_delta(text, result, on_text);
            }
        }
        CliTool::GeminiCli => {
            let kind = parsed.get("type").and_then(Value::as_str).unwrap_or("");
            if kind == "message"
                && parsed.get("role").and_then(Value::as_str) == Some("assistant")
                && let Some(text) = parsed.get("content").and_then(Value::as_str)
                && !text.is_empty()
            {
                append_delta(text, result, on_text);
            } else if let Some(text) = parsed.get("response").and_then(Value::as_str)
                && result.text.is_empty()
                && !text.is_empty()
            {
                append_delta(text, result, on_text);
            }
        }
        CliTool::CodexCli => {
            let kind = parsed.get("type").and_then(Value::as_str).unwrap_or("");
            if matches!(kind, "item.completed" | "item.updated")
                && let Some(item) = parsed.get("item")
                && item.get("type").and_then(Value::as_str) == Some("agent_message")
                && let Some(text) = item.get("text").and_then(Value::as_str)
                && !text.is_empty()
            {
                append_delta(text, result, on_text);
            } else if let Some(text) = parsed.get("message").and_then(Value::as_str)
                && !text.is_empty()
            {
                append_delta(text, result, on_text);
            }
        }
    }
}

fn append_delta(text: &str, result: &mut CliTurnResult, on_text: &mut dyn FnMut(String)) {
    if !result.text.is_empty() && !result.text.ends_with('\n') && !text.starts_with('\n') {
        result.text.push('\n');
        on_text("\n".to_owned());
    }
    result.text.push_str(text);
    on_text(text.to_owned());
}

pub fn find_on_path(binary: &str) -> Option<PathBuf> {
    let path_var = env::var_os("PATH")?;

    for dir in env::split_paths(&path_var) {
        let candidate = dir.join(binary);

        if candidate.is_file() {
            return Some(candidate);
        }
        #[cfg(windows)]
        for ext in [".exe", ".cmd", ".bat"] {
            let with_ext = dir.join(format!("{binary}{ext}"));
            if with_ext.is_file() {
                return Some(with_ext);
            }
        }
    }
    None
}

pub fn start_bridge_for_cli(
    executor: Arc<crate::assistant::executor::LiveMcpExecutor>,
) -> Result<LoopbackBridge, AssistantFailure> {
    LoopbackBridge::start(executor).map_err(|error| AssistantFailure::BridgeFailed {
        error_kind: error.kind(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_claude_gemini_codex_stream_json_and_plain_text() {
        let mut claude = CliTurnResult::default();
        let mut chunks = Vec::new();

        parse_cli_line(
            CliTool::ClaudeCode,
            "{\"type\":\"assistant\",\"session_id\":\"s-1\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"Created plate.\"}]}}",
            &mut claude,
            &mut |chunk| chunks.push(chunk),
        );

        assert_eq!(claude.session_id.as_deref(), Some("s-1"));
        assert_eq!(claude.text, "Created plate.");
        assert_eq!(chunks, vec!["Created plate."]);

        let mut gemini = CliTurnResult::default();

        parse_cli_line(
            CliTool::GeminiCli,
            "{\"type\":\"message\",\"role\":\"assistant\",\"content\":\"Added hole.\"}",
            &mut gemini,
            &mut |_| {},
        );

        assert_eq!(gemini.text, "Added hole.");

        let mut codex = CliTurnResult::default();

        parse_cli_line(
            CliTool::CodexCli,
            "{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"Filleted edges.\"}}",
            &mut codex,
            &mut |_| {},
        );

        assert_eq!(codex.text, "Filleted edges.");

        let mut plain = CliTurnResult::default();

        parse_cli_line(CliTool::CodexCli, "plain output", &mut plain, &mut |_| {});

        assert_eq!(plain.text, "plain output\n");
    }
}
