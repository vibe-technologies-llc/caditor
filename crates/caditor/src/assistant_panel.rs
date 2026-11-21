use egui::{
    Align, CornerRadius, Frame, Id, Key, Label, Layout, RichText, Stroke, TextEdit, TextStyle, Ui,
    vec2,
};

use crate::{
    appearance::{
        self, BORDER_WIDTH, CARD_RADIUS, LABEL_SIZE, SPACE_L, SPACE_M, SPACE_S, SPACE_SM, SPACE_XS,
        WIDGET_RADIUS,
    },
    assistant::{
        Assistant, AssistantCommand, Backend, ChatEntry, CliState, EntryKind, ToolActivity,
        ToolStatus,
    },
    commands::{Command, CommandFrame},
    fonts,
    guide::Page,
    icons,
    layout::RightPanel,
    mcp::tools::Tool,
    model::Action,
    preferences::AssistantPreferences,
    widgets::{self, PanelTitle, Tone},
};

pub const TITLE: &str = "AI assistant";
pub const CLOSE: &str = "Close the AI assistant";
pub const NEW_CHAT: &str = "Start a new chat";
pub const SETTINGS: &str = "AI assistant settings";
pub const COMPOSER_HINT: &str = "Ask the assistant to inspect, create or edit the model…";
pub const SEND: &str = "Send";
pub const STOP: &str = "Stop";
pub const RETRY: &str = "Try again";
pub const OPEN_SETTINGS: &str = "Open AI settings";
pub const NOT_WORKING: &str = "The AI assistant is not running right now";
const PANEL: RightPanel = RightPanel { id: "assistant" };
const HEADER: PanelTitle<'static> = PanelTitle {
    glyph: icons::ASSISTANT,
    title: TITLE,
    close: CLOSE,
    help: Some(Page::Assistants),
};
const STARTER_PROMPTS: [(&str, &str); 3] = [
    (
        "Inspect this model",
        "Inspect the open model, list its features, parameters and bodies, and report any issues.",
    ),
    (
        "Create a mounting plate",
        "Create a 60 mm by 40 mm by 8 mm rectangular plate centred on the XY plane with four 5 mm \
         through-holes inset 8 mm from each corner, and render a view.",
    ),
    (
        "Fillet top edges",
        "List the bodies and add a 2 mm fillet to the outer edges where appropriate, then check for \
         any failing features.",
    ),
];
const PREVIEW_MAX_HEIGHT: f32 = 180.0;

pub fn show(
    ui: &mut Ui,
    assistant: &mut Assistant,
    prefs: &AssistantPreferences,
    room: f32,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    let busy = assistant.is_busy();
    let working = busy.then_some(()).ok_or(NOT_WORKING);
    let cancel_command = commands.invoke(Command::CancelAssistant, &working);
    if cancel_command {
        actions.push(Action::Assistant(AssistantCommand::Cancel));
    }

    let has_entries = !assistant.entries().is_empty();
    let header_actions = |ui: &mut Ui| {
        let mut chosen = None;
        let settings = widgets::icon_button(ui, icons::GENERAL, SETTINGS);
        if settings.clicked() {
            chosen = Some(AssistantCommand::OpenSettings);
        }
        let clear = ui
            .add_enabled_ui(has_entries || busy, |ui| {
                widgets::icon_button(ui, icons::NEW_CHAT, NEW_CHAT)
            })
            .inner;
        if clear.clicked() {
            chosen = Some(AssistantCommand::ClearChat);
        }
        chosen
    };

    let mut pending = Vec::new();
    let panel = widgets::side_panel_parts(ui, room, PANEL, &HEADER, header_actions, |ui| {
        toolbar(ui, assistant, prefs, &mut pending);
        ui.add_space(SPACE_S);
        ui.separator();
        ui.add_space(SPACE_S);

        widgets::panel_footer(ui, "assistant-composer", |ui| {
            composer(ui, assistant, prefs, &mut pending);
        });

        widgets::panel_scroll(ui, "assistant-transcript", |ui| {
            transcript(ui, assistant, prefs, &mut pending);
        });
    });

    if let Some(command) = panel.actions {
        pending.push(command);
    }
    if panel.closed {
        assistant.open = false;
    }
    for command in pending {
        actions.push(Action::Assistant(command));
    }
}

fn toolbar(
    ui: &mut Ui,
    assistant: &Assistant,
    prefs: &AssistantPreferences,
    pending: &mut Vec<AssistantCommand>,
) {
    let backend = prefs.backend;
    let busy = assistant.is_busy();
    ui.horizontal_wrapped(|ui| {
        ui.add_enabled_ui(!busy, |ui| {
            let mut picked = None;
            widgets::combo("assistant-backend", backend.short_label())
                .width(150.0)
                .hover("Choose the AI model provider or local CLI wrapper")
                .show(ui, |ui| {
                    for option in Backend::ALL {
                        let chosen = option == backend;
                        if widgets::menu_option(ui, chosen, option.label()).clicked() && !chosen {
                            picked = Some(option);
                        }
                    }
                });
            if let Some(chosen) = picked {
                pending.push(AssistantCommand::SelectBackend(chosen));
            }
        });

        if let Backend::Api(provider) = backend {
            let current_model = prefs.model_for(provider);
            ui.add_enabled_ui(!busy, |ui| {
                let mut picked_model = None;
                widgets::combo("assistant-model", current_model)
                    .width(150.0)
                    .hover("Choose the model preset or edit the model name in Preferences")
                    .show(ui, |ui| {
                        for preset in provider.preset_models() {
                            let chosen = *preset == current_model;
                            if widgets::menu_option(ui, chosen, preset).clicked() && !chosen {
                                picked_model = Some((*preset).to_owned());
                            }
                        }
                    });
                if let Some(model) = picked_model {
                    pending.push(AssistantCommand::SetModel(provider, model));
                }
            });
        }
    });

    ui.add_space(SPACE_XS);
    ui.horizontal_wrapped(|ui| {
        let tool_count = Tool::ALL.len() + 2;
        if assistant.is_busy() {
            widgets::status_pill(ui, Tone::Info, "MCP active");
        } else if assistant.backend_ready(backend) {
            widgets::status_pill(ui, Tone::Success, format!("MCP: {tool_count} tools"));
        } else {
            let label = match backend {
                Backend::Api(_) => "API key needed",
                Backend::Cli(tool) => match assistant.cli_state(tool) {
                    CliState::Checking => "Checking CLI…",
                    _ => "CLI not found",
                },
            };
            widgets::status_pill(ui, Tone::Warning, label);
        }
    });
}

fn transcript(
    ui: &mut Ui,
    assistant: &Assistant,
    prefs: &AssistantPreferences,
    pending: &mut Vec<AssistantCommand>,
) {
    let backend = prefs.backend;
    if !assistant.backend_ready(backend) {
        setup_callout(ui, assistant, backend, pending);
        ui.add_space(SPACE_M);
    }

    if assistant.entries().is_empty() {
        widgets::empty_state(
            ui,
            icons::ASSISTANT,
            "Chat with Claude, Gemini, GPT or a local CLI to inspect and build your CAD model \
             using caditor's live MCP tools.",
            |ui| {
                ui.vertical_centered_justified(|ui| {
                    for (label, prompt) in STARTER_PROMPTS {
                        let enabled = !assistant.is_busy() && assistant.backend_ready(backend);
                        let response = ui
                            .add_enabled_ui(enabled, |ui| {
                                let button =
                                    widgets::small_button(ui, icons::AUTOMATIC_CONSTRAINTS, label);
                                ui.add(button)
                            })
                            .inner
                            .on_hover_text(prompt);
                        if response.clicked() {
                            pending.push(AssistantCommand::SendPrompt(prompt.to_owned()));
                        }
                        ui.add_space(SPACE_XS);
                    }
                });
            },
        );
        return;
    }

    for entry in assistant.entries() {
        render_entry(ui, entry, assistant.can_retry(), pending);
        ui.add_space(SPACE_M);
    }

    if assistant.is_busy() {
        let cancelled = widgets::progress(assistant.status().label())
            .wrapping()
            .cancel("Stop the AI assistant")
            .show(ui);
        if cancelled {
            pending.push(AssistantCommand::Cancel);
        }
    }
}

fn setup_callout(
    ui: &mut Ui,
    assistant: &Assistant,
    backend: Backend,
    pending: &mut Vec<AssistantCommand>,
) {
    widgets::callout(ui, Tone::Warning, |ui| {
        let detail = match backend {
            Backend::Api(provider) => match provider.env_var() {
                Some(var) => format!(
                    "Add your {} API key in Preferences › General › AI assistant, or set {var} \
                     in your environment.",
                    provider.short_label()
                ),
                None => "Configure your custom OpenAI-compatible endpoint in Preferences › \
                         General › AI assistant."
                    .to_owned(),
            },
            Backend::Cli(tool) => match assistant.cli_state(tool) {
                CliState::Checking => format!("Looking for {} on PATH…", tool.default_binary()),
                CliState::Missing { expected } => format!(
                    "Could not find “{expected}”. Install {} or set its path in Preferences › \
                     General › AI assistant.",
                    tool.label()
                ),
                CliState::Found(_) => String::new(),
            },
        };
        ui.add(Label::new(detail).wrap());
        ui.add_space(SPACE_S);
        ui.horizontal_wrapped(|ui| {
            let settings = widgets::small_button(ui, icons::GENERAL, OPEN_SETTINGS);
            if ui.add(settings).clicked() {
                pending.push(AssistantCommand::OpenSettings);
            }
            if matches!(backend, Backend::Cli(_)) {
                let refresh = widgets::small_button(ui, icons::RELOAD, "Check again");
                if ui.add(refresh).clicked() {
                    pending.push(AssistantCommand::RefreshClis);
                }
            }
        });
    });
}

fn render_entry(
    ui: &mut Ui,
    entry: &ChatEntry,
    can_retry: bool,
    pending: &mut Vec<AssistantCommand>,
) {
    match entry.kind {
        EntryKind::Notice => {
            ui.horizontal_wrapped(|ui| {
                widgets::icon_label(ui, icons::INFO, appearance::tokens(ui).text_muted);
                ui.add(Label::new(widgets::muted(&entry.text, ui)).wrap());
            });
        }
        EntryKind::User => {
            let tokens = appearance::tokens(ui);
            Frame::new()
                .fill(tokens.accent_surface)
                .stroke(Stroke::new(BORDER_WIDTH, tokens.border))
                .corner_radius(CornerRadius::same(CARD_RADIUS))
                .inner_margin(appearance::margin(SPACE_M, SPACE_SM))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add(
                        Label::new(
                            RichText::new("You")
                                .family(fonts::semibold())
                                .size(LABEL_SIZE)
                                .color(tokens.text_muted),
                        )
                        .selectable(false),
                    );
                    ui.add_space(SPACE_XS);
                    ui.add(Label::new(&entry.text).wrap());
                });
        }
        EntryKind::Assistant => {
            widgets::card(ui, |ui| {
                ui.set_width(ui.available_width());
                let tokens = appearance::tokens(ui);
                ui.horizontal(|ui| {
                    widgets::icon_label(ui, icons::ASSISTANT, tokens.accent);
                    ui.add(
                        Label::new(
                            RichText::new("Assistant")
                                .family(fonts::semibold())
                                .size(LABEL_SIZE)
                                .color(tokens.text_muted),
                        )
                        .selectable(false),
                    );
                });
                if !entry.tools.is_empty() {
                    ui.add_space(SPACE_S);
                    for tool in &entry.tools {
                        render_tool_activity(ui, entry.id, tool);
                        ui.add_space(SPACE_XS);
                    }
                }
                if !entry.text.trim().is_empty() {
                    ui.add_space(SPACE_S);
                    render_formatted_text(ui, &entry.text);
                }
                if let Some(failure) = &entry.failure {
                    ui.add_space(SPACE_S);
                    widgets::callout(ui, Tone::Error, |ui| {
                        ui.add(Label::new(failure.to_string()).wrap());
                        ui.add_space(SPACE_S);
                        ui.horizontal_wrapped(|ui| {
                            if can_retry {
                                let retry = widgets::small_button(ui, icons::RELOAD, RETRY);
                                if ui.add(retry).clicked() {
                                    pending.push(AssistantCommand::RetryLast);
                                }
                            }
                            let settings = widgets::small_button(ui, icons::GENERAL, OPEN_SETTINGS);
                            if ui.add(settings).clicked() {
                                pending.push(AssistantCommand::OpenSettings);
                            }
                        });
                    });
                }
            });
        }
    }
}

fn render_tool_activity(ui: &mut Ui, entry_id: u64, tool: &ToolActivity) {
    let tokens = appearance::tokens(ui);
    let tone = match tool.status {
        ToolStatus::Running => Tone::Info,
        ToolStatus::Succeeded => Tone::Success,
        ToolStatus::Failed => Tone::Error,
    };
    Frame::new()
        .fill(tokens.sunken)
        .stroke(Stroke::new(BORDER_WIDTH, tokens.border))
        .corner_radius(CornerRadius::same(WIDGET_RADIUS))
        .inner_margin(appearance::margin(SPACE_M, SPACE_S))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                match tool.status {
                    ToolStatus::Running => {
                        widgets::spinner(ui);
                    }
                    ToolStatus::Succeeded | ToolStatus::Failed => {
                        widgets::icon_label(ui, tone.icon(), tone.color(&tokens));
                    }
                }
                ui.add(Label::new(widgets::strong(&tool.title)).wrap())
                    .on_hover_text(format!("MCP tool: {}", tool.tool_name));
                if !tool.summary.is_empty() {
                    ui.add(Label::new(widgets::muted(&tool.summary, ui)).wrap());
                } else if let Some(progress) = &tool.progress {
                    ui.add(Label::new(widgets::muted(progress, ui)).wrap());
                }
            });
            if !tool.arguments_preview.is_empty() {
                ui.add_space(SPACE_XS);
                ui.add(
                    Label::new(
                        RichText::new(&tool.arguments_preview)
                            .text_style(TextStyle::Monospace)
                            .size(LABEL_SIZE)
                            .color(tokens.text_muted),
                    )
                    .wrap(),
                );
            }
            if let Some(preview) = &tool.preview {
                ui.add_space(SPACE_S);
                let texture_name = format!("assistant-render-{entry_id}-{}", tool.id);
                let texture = preview.texture(ui.ctx(), &texture_name);
                let raw = texture.size_vec2();
                let max_width = ui.available_width().max(64.0);
                let scale = (max_width / raw.x.max(1.0))
                    .min(PREVIEW_MAX_HEIGHT / raw.y.max(1.0))
                    .min(1.0);
                let size = vec2((raw.x * scale).max(32.0), (raw.y * scale).max(32.0));
                ui.add(
                    egui::Image::from_texture((texture.id(), size)).corner_radius(WIDGET_RADIUS),
                );
            }
        });
}

fn render_formatted_text(ui: &mut Ui, text: &str) {
    let tokens = appearance::tokens(ui);
    let mut in_code = false;
    let mut code_lines = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            if in_code {
                render_code_block(ui, &code_lines.join("\n"));
                code_lines.clear();
                in_code = false;
            } else {
                in_code = true;
            }
            continue;
        }
        if in_code {
            code_lines.push(line);
            continue;
        }
        if trimmed.is_empty() {
            ui.add_space(SPACE_XS);
            continue;
        }
        if let Some(heading) = trimmed
            .strip_prefix("### ")
            .or_else(|| trimmed.strip_prefix("## "))
            .or_else(|| trimmed.strip_prefix("# "))
        {
            ui.add_space(SPACE_XS);
            ui.add(Label::new(widgets::section_title(heading)).wrap());
            continue;
        }
        if let Some(item) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            ui.horizontal_top(|ui| {
                widgets::icon_label(ui, icons::BULLET, tokens.text_muted);
                ui.add(Label::new(item).wrap());
            });
            continue;
        }
        ui.add(Label::new(line).wrap());
    }
    if in_code && !code_lines.is_empty() {
        render_code_block(ui, &code_lines.join("\n"));
    }
}

fn render_code_block(ui: &mut Ui, code: &str) {
    let tokens = appearance::tokens(ui);
    Frame::new()
        .fill(tokens.sunken)
        .stroke(Stroke::new(BORDER_WIDTH, tokens.border))
        .corner_radius(CornerRadius::same(WIDGET_RADIUS))
        .inner_margin(appearance::margin(SPACE_M, SPACE_S))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(
                Label::new(
                    RichText::new(code)
                        .text_style(TextStyle::Monospace)
                        .size(LABEL_SIZE),
                )
                .wrap(),
            );
        });
    ui.add_space(SPACE_XS);
}

fn composer_id() -> Id {
    Id::unique("assistant-composer-input")
}

fn composer(
    ui: &mut Ui,
    assistant: &mut Assistant,
    prefs: &AssistantPreferences,
    pending: &mut Vec<AssistantCommand>,
) {
    let busy = assistant.is_busy();
    let ready = assistant.backend_ready(prefs.backend);
    let can_send = !busy && ready && !assistant.composer.trim().is_empty();
    let focused_before = ui.memory(|memory| memory.has_focus(composer_id()));
    let submitted_by_enter = focused_before
        && can_send
        && ui.input_mut(|input| {
            if input.modifiers.is_none() && input.key_pressed(Key::Enter) {
                input.consume_key(egui::Modifiers::NONE, Key::Enter)
            } else {
                false
            }
        });

    let response = widgets::text_field(ui, |ui| {
        widgets::spoken_as(
            ui.add(
                TextEdit::multiline(&mut assistant.composer)
                    .id(composer_id())
                    .desired_rows(3)
                    .desired_width(f32::INFINITY)
                    .hint_text(COMPOSER_HINT),
            ),
            COMPOSER_HINT,
        )
    });
    if std::mem::take(&mut assistant.focus_composer) {
        response.request_focus();
    }

    ui.add_space(SPACE_S);
    ui.horizontal(|ui| {
        ui.add(
            Label::new(widgets::muted("Enter to send, Shift+Enter for newline", ui))
                .selectable(false),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if busy {
                if ui.add(widgets::danger_button(STOP)).clicked() {
                    pending.push(AssistantCommand::Cancel);
                }
            } else {
                let clicked = ui
                    .add_enabled_ui(can_send, |ui| {
                        ui.add(widgets::primary_icon_button(ui, icons::SEND, SEND))
                    })
                    .inner
                    .clicked();
                if clicked || submitted_by_enter {
                    let text = std::mem::take(&mut assistant.composer);
                    pending.push(AssistantCommand::SendPrompt(text));
                }
            }
        });
    });
    ui.add_space(SPACE_L);
}
