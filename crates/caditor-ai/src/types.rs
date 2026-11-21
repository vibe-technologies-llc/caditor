use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiProvider {
    Claude,
    Gemini,
    OpenAi,
    Custom,
}

impl ApiProvider {
    pub const ALL: [Self; 4] = [Self::Claude, Self::Gemini, Self::OpenAi, Self::Custom];

    pub fn id(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Gemini => "gemini",
            Self::OpenAi => "openai",
            Self::Custom => "custom",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|provider| provider.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude (Anthropic API)",
            Self::Gemini => "Gemini (Google API)",
            Self::OpenAi => "GPT (OpenAI API)",
            Self::Custom => "Custom (OpenAI-compatible)",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Self::Claude => "Claude API",
            Self::Gemini => "Gemini API",
            Self::OpenAi => "GPT API",
            Self::Custom => "Custom API",
        }
    }

    pub fn env_var(self) -> Option<&'static str> {
        match self {
            Self::Claude => Some("ANTHROPIC_API_KEY"),
            Self::Gemini => Some("GEMINI_API_KEY"),
            Self::OpenAi => Some("OPENAI_API_KEY"),
            Self::Custom => None,
        }
    }

    pub fn requires_key(self) -> bool {
        self != Self::Custom
    }

    pub fn default_model(self) -> &'static str {
        match self {
            Self::Claude => "claude-sonnet-4-6",
            Self::Gemini => "gemini-2.5-pro",
            Self::OpenAi => "gpt-5.2",
            Self::Custom => "qwen2.5-coder:14b",
        }
    }

    pub fn preset_models(self) -> &'static [&'static str] {
        match self {
            Self::Claude => &["claude-sonnet-4-6", "claude-opus-4-6", "claude-haiku-4-5"],
            Self::Gemini => &["gemini-2.5-pro", "gemini-2.5-flash", "gemini-2.0-flash"],
            Self::OpenAi => &["gpt-5.2", "gpt-4.1", "gpt-4o", "o4-mini"],
            Self::Custom => &["qwen2.5-coder:14b", "llama3.3:70b"],
        }
    }

    pub fn default_base_url(self) -> &'static str {
        match self {
            Self::Claude => "https://api.anthropic.com",
            Self::Gemini => "https://generativelanguage.googleapis.com",
            Self::OpenAi => "https://api.openai.com/v1",
            Self::Custom => "http://localhost:11434/v1",
        }
    }
}

impl fmt::Display for ApiProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.short_label())
    }
}

#[derive(Clone, PartialEq, Eq, Default)]
pub struct ApiKey(String);

impl ApiKey {
    pub fn new(raw: impl Into<String>) -> Self {
        Self(raw.into().trim().to_owned())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            formatter.write_str("ApiKey(empty)")
        } else {
            formatter.write_str("ApiKey(<redacted>)")
        }
    }
}

#[derive(Debug, Clone)]
pub struct RequestConfig {
    pub provider: ApiProvider,
    pub model: String,
    pub api_key: ApiKey,
    pub base_url: Option<String>,
    pub system_prompt: String,
    pub max_tokens: u32,
}

impl RequestConfig {
    pub fn effective_model(&self) -> &str {
        let trimmed = self.model.trim();
        if trimmed.is_empty() {
            self.provider.default_model()
        } else {
            trimmed
        }
    }

    pub fn effective_base_url(&self) -> &str {
        let custom = self
            .base_url
            .as_deref()
            .map(str::trim)
            .filter(|url| !url.is_empty());
        custom
            .unwrap_or_else(|| self.provider.default_base_url())
            .trim_end_matches('/')
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResult {
    pub call_id: String,
    pub name: String,
    pub content: String,
    pub image_png: Option<Vec<u8>>,
    pub is_error: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    User {
        text: String,
    },
    Assistant {
        text: String,
        tool_calls: Vec<ToolCall>,
    },
    ToolResults(Vec<ToolResult>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamEvent {
    TextDelta(String),
    ToolCallReady(ToolCall),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnResponse {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
    pub stop_reason: StopReason,
}

const BASE64_TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk.first().copied().unwrap_or(0);
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        let triple = (u32::from(first) << 16) | (u32::from(second) << 8) | u32::from(third);
        let index = |shift: u32| -> char {
            let slot = ((triple >> shift) & 0x3F) as usize;
            char::from(BASE64_TABLE.get(slot).copied().unwrap_or(b'A'))
        };
        out.push(index(18));
        out.push(index(12));
        if chunk.len() > 1 {
            out.push(index(6));
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(index(0));
        } else {
            out.push('=');
        }
    }
    out
}

pub fn endpoint_allows_http(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("http://") else {
        return false;
    };
    let host_port = rest.split('/').next().unwrap_or_default();
    if host_port.starts_with('[') {
        let host = host_port
            .strip_prefix('[')
            .and_then(|value| value.split(']').next())
            .unwrap_or_default();
        return host == "::1";
    }
    let host = host_port.split(':').next().unwrap_or_default();
    matches!(host, "localhost" | "127.0.0.1")
}

pub fn validate_endpoint(url: &str) -> bool {
    let trimmed = url.trim();
    if trimmed.contains(char::is_whitespace) {
        return false;
    }
    if let Some(rest) = trimmed.strip_prefix("https://") {
        let host = rest.split('/').next().unwrap_or_default();
        return !host.is_empty();
    }
    endpoint_allows_http(trimmed)
}
