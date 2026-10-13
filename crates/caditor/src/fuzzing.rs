use caditor_file::Settings;
use egui::KeyboardShortcut;
use serde_json::Value;

use crate::{
    commands,
    mcp::{
        json_rpc::{self, Incoming},
        tools::Tool,
    },
    preferences::Preferences,
};

pub struct Settled {
    pub written: Settings,
    pub rewritten: Settings,
}

pub fn preferences(settings: Settings) -> Settled {
    let written = Preferences::from_settings(settings).settings();
    let rewritten = Preferences::from_settings(written.clone()).settings();
    Settled { written, rewritten }
}

pub fn parse_stored(text: &str) -> Option<KeyboardShortcut> {
    commands::parse_stored(text)
}

pub fn stored_text(shortcut: &KeyboardShortcut) -> String {
    commands::stored_text(shortcut)
}

pub fn mcp_lines(bytes: &[u8]) -> usize {
    let mut decoded = 0;
    for line in bytes.split(|byte| *byte == b'\n') {
        let Ok(Some(Incoming::Request { method, params, .. })) = json_rpc::parse(line) else {
            continue;
        };
        let tool = params
            .get("name")
            .and_then(Value::as_str)
            .and_then(Tool::named);
        if let (true, Some(tool)) = (method == "tools/call", tool) {
            let arguments = params.get("arguments").cloned().unwrap_or_default();
            if tool.decode(arguments).is_ok() {
                decoded += 1;
            }
        }
    }
    decoded
}
