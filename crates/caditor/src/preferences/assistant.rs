use std::path::PathBuf;

use caditor_ai::ApiProvider;
use caditor_file::Settings;

use crate::assistant::{Backend, CliTool};

const BACKEND_KEY: &str = "assistant.backend";
const CLAUDE_MODEL_KEY: &str = "assistant.claude.model";
const GEMINI_MODEL_KEY: &str = "assistant.gemini.model";
const OPENAI_MODEL_KEY: &str = "assistant.openai.model";
const CUSTOM_MODEL_KEY: &str = "assistant.custom.model";
const CUSTOM_URL_KEY: &str = "assistant.custom.base_url";
const CLAUDE_CLI_KEY: &str = "assistant.cli.claude";
const GEMINI_CLI_KEY: &str = "assistant.cli.gemini";
const CODEX_CLI_KEY: &str = "assistant.cli.codex";
const FOLDERS_KEY: &str = "assistant.folders";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssistantPreferences {
    pub backend: Backend,
    pub claude_model: String,
    pub gemini_model: String,
    pub openai_model: String,
    pub custom_model: String,
    pub custom_base_url: String,
    pub claude_cli: String,
    pub gemini_cli: String,
    pub codex_cli: String,
    pub folders: Vec<PathBuf>,
}

impl Default for AssistantPreferences {
    fn default() -> Self {
        Self {
            backend: Backend::Api(ApiProvider::Claude),
            claude_model: ApiProvider::Claude.default_model().to_owned(),
            gemini_model: ApiProvider::Gemini.default_model().to_owned(),
            openai_model: ApiProvider::OpenAi.default_model().to_owned(),
            custom_model: ApiProvider::Custom.default_model().to_owned(),
            custom_base_url: ApiProvider::Custom.default_base_url().to_owned(),
            claude_cli: String::new(),
            gemini_cli: String::new(),
            codex_cli: String::new(),
            folders: Vec::new(),
        }
    }
}

impl AssistantPreferences {
    pub fn from_settings(raw: &Settings) -> Self {
        let default = Self::default();
        let backend = raw
            .text(BACKEND_KEY)
            .and_then(Backend::from_id)
            .unwrap_or(default.backend);
        let read_or = |key: &str, fallback: &str| {
            raw.text(key)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(fallback)
                .to_owned()
        };
        let read_opt = |key: &str| {
            raw.text(key)
                .map(|value| value.trim().to_owned())
                .unwrap_or_default()
        };
        let folders = raw
            .texts(FOLDERS_KEY)
            .unwrap_or_default()
            .into_iter()
            .map(|entry| entry.trim().to_owned())
            .filter(|entry| !entry.is_empty())
            .map(PathBuf::from)
            .collect();
        Self {
            backend,
            claude_model: read_or(CLAUDE_MODEL_KEY, &default.claude_model),
            gemini_model: read_or(GEMINI_MODEL_KEY, &default.gemini_model),
            openai_model: read_or(OPENAI_MODEL_KEY, &default.openai_model),
            custom_model: read_or(CUSTOM_MODEL_KEY, &default.custom_model),
            custom_base_url: read_or(CUSTOM_URL_KEY, &default.custom_base_url),
            claude_cli: read_opt(CLAUDE_CLI_KEY),
            gemini_cli: read_opt(GEMINI_CLI_KEY),
            codex_cli: read_opt(CODEX_CLI_KEY),
            folders,
        }
    }

    pub fn model_for(&self, provider: ApiProvider) -> &str {
        match provider {
            ApiProvider::Claude => self.claude_model.trim(),
            ApiProvider::Gemini => self.gemini_model.trim(),
            ApiProvider::OpenAi => self.openai_model.trim(),
            ApiProvider::Custom => self.custom_model.trim(),
        }
    }

    pub fn set_model(&mut self, provider: ApiProvider, model: String) {
        let trimmed = model.trim().to_owned();
        let value = if trimmed.is_empty() {
            provider.default_model().to_owned()
        } else {
            trimmed
        };
        match provider {
            ApiProvider::Claude => self.claude_model = value,
            ApiProvider::Gemini => self.gemini_model = value,
            ApiProvider::OpenAi => self.openai_model = value,
            ApiProvider::Custom => self.custom_model = value,
        }
    }

    pub fn cli_program(&self, tool: CliTool) -> &str {
        match tool {
            CliTool::ClaudeCode => self.claude_cli.trim(),
            CliTool::GeminiCli => self.gemini_cli.trim(),
            CliTool::CodexCli => self.codex_cli.trim(),
        }
    }

    pub fn set_cli_program(&mut self, tool: CliTool, program: String) {
        let trimmed = program.trim().to_owned();
        match tool {
            CliTool::ClaudeCode => self.claude_cli = trimmed,
            CliTool::GeminiCli => self.gemini_cli = trimmed,
            CliTool::CodexCli => self.codex_cli = trimmed,
        }
    }

    pub fn write(&self, settings: &mut Settings) {
        let default = Self::default();
        if self.backend == default.backend {
            settings.remove(BACKEND_KEY);
        } else {
            settings.set_text(BACKEND_KEY, self.backend.id());
        }
        write_if_changed(
            settings,
            CLAUDE_MODEL_KEY,
            self.claude_model.trim(),
            &default.claude_model,
        );
        write_if_changed(
            settings,
            GEMINI_MODEL_KEY,
            self.gemini_model.trim(),
            &default.gemini_model,
        );
        write_if_changed(
            settings,
            OPENAI_MODEL_KEY,
            self.openai_model.trim(),
            &default.openai_model,
        );
        write_if_changed(
            settings,
            CUSTOM_MODEL_KEY,
            self.custom_model.trim(),
            &default.custom_model,
        );
        write_if_changed(
            settings,
            CUSTOM_URL_KEY,
            self.custom_base_url.trim(),
            &default.custom_base_url,
        );
        write_optional(settings, CLAUDE_CLI_KEY, self.claude_cli.trim());
        write_optional(settings, GEMINI_CLI_KEY, self.gemini_cli.trim());
        write_optional(settings, CODEX_CLI_KEY, self.codex_cli.trim());
        if self.folders.is_empty() {
            settings.remove(FOLDERS_KEY);
        } else {
            let entries: Vec<String> = self
                .folders
                .iter()
                .map(|path| path.display().to_string())
                .collect();
            settings.set_texts(FOLDERS_KEY, &entries);
        }
    }
}

fn write_if_changed(settings: &mut Settings, key: &str, value: &str, default: &str) {
    if value.is_empty() || value == default {
        settings.remove(key);
    } else {
        settings.set_text(key, value);
    }
}

fn write_optional(settings: &mut Settings, key: &str, value: &str) {
    if value.is_empty() {
        settings.remove(key);
    } else {
        settings.set_text(key, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assistant_preferences_round_trip_and_omit_defaults() {
        let mut settings = Settings::default();
        AssistantPreferences::default().write(&mut settings);
        assert_eq!(settings.text(BACKEND_KEY), None);
        assert_eq!(settings.text(CLAUDE_MODEL_KEY), None);

        let custom = AssistantPreferences {
            backend: Backend::Cli(CliTool::ClaudeCode),
            claude_model: "claude-opus-4-6".to_owned(),
            gemini_model: "gemini-2.5-flash".to_owned(),
            openai_model: "gpt-4.1".to_owned(),
            custom_model: "llama3.3:70b".to_owned(),
            custom_base_url: "http://127.0.0.1:11434/v1".to_owned(),
            claude_cli: "/usr/local/bin/claude".to_owned(),
            gemini_cli: String::new(),
            codex_cli: "/opt/codex".to_owned(),
            folders: vec![PathBuf::from("/tmp/cad-exports")],
        };
        custom.write(&mut settings);
        let loaded = AssistantPreferences::from_settings(&settings);
        assert_eq!(loaded, custom);
    }
}
