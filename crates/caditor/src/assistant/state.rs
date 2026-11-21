use std::{fmt, path::PathBuf, sync::Arc};

use caditor_ai::{AiError, ApiProvider};

use crate::cloud::thumbnails::Thumbnail;

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum CliTool {
    ClaudeCode,
    GeminiCli,
    CodexCli,
}

impl CliTool {
    pub const ALL: [Self; 3] = [Self::ClaudeCode, Self::GeminiCli, Self::CodexCli];

    pub fn id(self) -> &'static str {
        match self {
            Self::ClaudeCode => "cli_claude",
            Self::GeminiCli => "cli_gemini",
            Self::CodexCli => "cli_codex",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tool| tool.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code (local CLI)",
            Self::GeminiCli => "Gemini CLI (local CLI)",
            Self::CodexCli => "Codex CLI (local CLI)",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude CLI",
            Self::GeminiCli => "Gemini CLI",
            Self::CodexCli => "Codex CLI",
        }
    }

    pub fn default_binary(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude",
            Self::GeminiCli => "gemini",
            Self::CodexCli => "codex",
        }
    }
}

impl fmt::Display for CliTool {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.short_label())
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum Backend {
    Api(ApiProvider),
    Cli(CliTool),
}

impl Backend {
    pub const ALL: [Self; 7] = [
        Self::Api(ApiProvider::Claude),
        Self::Api(ApiProvider::Gemini),
        Self::Api(ApiProvider::OpenAi),
        Self::Api(ApiProvider::Custom),
        Self::Cli(CliTool::ClaudeCode),
        Self::Cli(CliTool::GeminiCli),
        Self::Cli(CliTool::CodexCli),
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::Api(provider) => provider.id(),
            Self::Cli(tool) => tool.id(),
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        ApiProvider::from_id(id)
            .map(Self::Api)
            .or_else(|| CliTool::from_id(id).map(Self::Cli))
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Api(provider) => provider.label(),
            Self::Cli(tool) => tool.label(),
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Self::Api(provider) => provider.short_label(),
            Self::Cli(tool) => tool.short_label(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeySource {
    Missing,
    Keyring,
    PrivateFile,
    Environment(&'static str),
}

impl KeySource {
    pub fn is_available(self) -> bool {
        self != Self::Missing
    }

    pub fn label(self) -> String {
        match self {
            Self::Missing => "Not set".to_owned(),
            Self::Keyring => "Saved in system keyring".to_owned(),
            Self::PrivateFile => "Saved in private file".to_owned(),
            Self::Environment(var) => format!("Using {var} from environment"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CliState {
    Checking,
    Found(PathBuf),
    Missing { expected: String },
}

impl CliState {
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Found(_))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolStatus {
    Running,
    Succeeded,
    Failed,
}

#[derive(Clone, Debug)]
pub struct ToolActivity {
    pub id: u64,
    pub tool_name: String,
    pub title: String,
    pub arguments_preview: String,
    pub status: ToolStatus,
    pub progress: Option<String>,
    pub summary: String,
    pub preview: Option<Arc<Thumbnail>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntryKind {
    User,
    Assistant,
    Notice,
}

#[derive(Clone, Debug)]
pub struct ChatEntry {
    pub id: u64,
    pub kind: EntryKind,
    pub text: String,
    pub tools: Vec<ToolActivity>,
    pub failure: Option<AssistantFailure>,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AssistantFailure {
    #[error(transparent)]
    Api(#[from] AiError),
    #[error(
        "The {tool} CLI was not found ({binary}). Install it or set its path in Preferences › General › AI assistant."
    )]
    CliMissing { tool: CliTool, binary: String },
    #[error("Could not start {tool}: {error_kind:?}")]
    CliLaunch {
        tool: CliTool,
        error_kind: std::io::ErrorKind,
    },
    #[error("{tool} exited with status {code}: {output}")]
    CliExited {
        tool: CliTool,
        code: i32,
        output: String,
    },
    #[error("Could not start the local MCP bridge ({error_kind:?}).")]
    BridgeFailed { error_kind: std::io::ErrorKind },
    #[error(
        "The assistant reached the maximum number of tool steps for one turn. Send another message to continue."
    )]
    StepLimitReached,
    #[error("The assistant stopped unexpectedly while answering. Try again.")]
    WorkerPanicked,
    #[error("Could not start the assistant background thread.")]
    ThreadUnavailable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssistantStatus {
    Idle,
    Thinking,
    RunningTool {
        title: String,
        progress: Option<String>,
    },
}

impl AssistantStatus {
    pub fn is_busy(&self) -> bool {
        !matches!(self, Self::Idle)
    }

    pub fn label(&self) -> String {
        match self {
            Self::Idle => "Ready".to_owned(),
            Self::Thinking => "Thinking…".to_owned(),
            Self::RunningTool { title, progress } => match progress {
                Some(step) if !step.is_empty() => format!("{title}: {step}"),
                _ => format!("Running {title}…"),
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssistantCommand {
    OpenSettings,
    SelectBackend(Backend),
    SetModel(ApiProvider, String),
    SendPrompt(String),
    Cancel,
    ClearChat,
    RetryLast,
    SaveApiKey(ApiProvider, String),
    ClearApiKey(ApiProvider),
    RefreshClis,
}
