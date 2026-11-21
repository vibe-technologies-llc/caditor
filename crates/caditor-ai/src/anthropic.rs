use std::sync::atomic::AtomicBool;

use serde_json::{Map, Value, json};

use crate::{
    AiError,
    sse::SseParser,
    status_error,
    transport::{HttpRequest, Transport},
    types::{
        Message, RequestConfig, StopReason, StreamEvent, ToolCall, ToolDefinition, TurnResponse,
        base64_encode,
    },
};

const ANTHROPIC_VERSION: &str = "2023-06-01";

struct ActiveTool {
    id: String,
    name: String,
    json_chunks: String,
}

pub fn stream(
    transport: &dyn Transport,
    config: &RequestConfig,
    messages: &[Message],
    tools: &[ToolDefinition],
    cancel: &AtomicBool,
    on_event: &mut dyn FnMut(StreamEvent),
) -> Result<TurnResponse, AiError> {
    if config.api_key.is_empty() {
        return Err(AiError::MissingKey {
            provider: config.provider,
        });
    }

    let url = format!("{}/v1/messages", config.effective_base_url());
    let payload = build_payload(config, messages, tools);
    let body = serde_json::to_string(&payload).map_err(|_| AiError::MalformedStream)?;

    let request = HttpRequest::post_json(url, body)
        .with_header("x-api-key", config.api_key.as_str())
        .with_header("anthropic-version", ANTHROPIC_VERSION)
        .with_header("Accept", "text/event-stream");

    let mut parser = SseParser::default();
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    let mut active_tool: Option<ActiveTool> = None;
    let mut stop_reason = StopReason::EndTurn;

    let status = transport.post_stream(&request, cancel, &mut |line| {
        if let Some(event) = parser.push_line(line) {
            handle_event(
                &event.data,
                &mut text,
                &mut tool_calls,
                &mut active_tool,
                &mut stop_reason,
                on_event,
            )?;
        }
        Ok(())
    })?;

    if !(200..300).contains(&status.status) {
        return Err(status_error(status));
    }

    if let Some(event) = parser.flush() {
        handle_event(
            &event.data,
            &mut text,
            &mut tool_calls,
            &mut active_tool,
            &mut stop_reason,
            on_event,
        )?;
    }
    finish_tool(&mut active_tool, &mut tool_calls, on_event);

    Ok(TurnResponse {
        text,
        tool_calls,
        stop_reason,
    })
}

fn build_payload(config: &RequestConfig, messages: &[Message], tools: &[ToolDefinition]) -> Value {
    let encoded_messages: Vec<Value> = messages
        .iter()
        .map(|message| match message {
            Message::User { text } => json!({
                "role": "user",
                "content": text,
            }),
            Message::Assistant { text, tool_calls } => {
                let mut blocks = Vec::new();

                if !text.is_empty() {
                    blocks.push(json!({
                        "type": "text",
                        "text": text,
                    }));
                }
                for call in tool_calls {
                    blocks.push(json!({
                        "type": "tool_use",
                        "id": call.id,
                        "name": call.name,
                        "input": call.arguments,
                    }));
                }
                if blocks.is_empty() {
                    blocks.push(json!({
                        "type": "text",
                        "text": "",
                    }));
                }
                json!({
                    "role": "assistant",
                    "content": blocks,
                })
            }
            Message::ToolResults(results) => {
                let blocks: Vec<Value> = results
                    .iter()
                    .map(|result| {
                        let content = match &result.image_png {
                            Some(png) => json!([
                                { "type": "text", "text": result.content },
                                {
                                    "type": "image",
                                    "source": {
                                        "type": "base64",
                                        "media_type": "image/png",
                                        "data": base64_encode(png),
                                    }
                                }
                            ]),
                            None => Value::String(result.content.clone()),
                        };
                        let mut item = Map::new();

                        item.insert("type".to_owned(), json!("tool_result"));
                        item.insert("tool_use_id".to_owned(), json!(result.call_id));
                        item.insert("content".to_owned(), content);
                        if result.is_error {
                            item.insert("is_error".to_owned(), json!(true));
                        }
                        Value::Object(item)
                    })
                    .collect();
                json!({
                    "role": "user",
                    "content": blocks,
                })
            }
        })
        .collect();

    let encoded_tools: Vec<Value> = tools
        .iter()
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "input_schema": tool.input_schema,
            })
        })
        .collect();

    let mut payload = Map::new();

    payload.insert("model".to_owned(), json!(config.effective_model()));
    payload.insert("max_tokens".to_owned(), json!(config.max_tokens));
    payload.insert("stream".to_owned(), json!(true));
    if !config.system_prompt.is_empty() {
        payload.insert("system".to_owned(), json!(config.system_prompt));
    }
    payload.insert("messages".to_owned(), Value::Array(encoded_messages));
    if !encoded_tools.is_empty() {
        payload.insert("tools".to_owned(), Value::Array(encoded_tools));
    }
    Value::Object(payload)
}

fn handle_event(
    data: &str,
    text: &mut String,
    tool_calls: &mut Vec<ToolCall>,
    active_tool: &mut Option<ActiveTool>,
    stop_reason: &mut StopReason,
    on_event: &mut dyn FnMut(StreamEvent),
) -> Result<(), AiError> {
    let trimmed = data.trim();

    if trimmed.is_empty() || trimmed == "[DONE]" {
        return Ok(());
    }

    let parsed: Value = serde_json::from_str(trimmed).map_err(|_| AiError::MalformedStream)?;
    let kind = parsed.get("type").and_then(Value::as_str).unwrap_or("");

    match kind {
        "content_block_start" => {
            if let Some(block) = parsed.get("content_block")
                && block.get("type").and_then(Value::as_str) == Some("tool_use")
            {
                let id = block
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let name = block
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();

                *active_tool = Some(ActiveTool {
                    id,
                    name,
                    json_chunks: String::new(),
                });
            }
        }
        "content_block_delta" => {
            if let Some(delta) = parsed.get("delta") {
                match delta.get("type").and_then(Value::as_str).unwrap_or("") {
                    "text_delta" => {
                        if let Some(chunk) = delta.get("text").and_then(Value::as_str)
                            && !chunk.is_empty()
                        {
                            text.push_str(chunk);
                            on_event(StreamEvent::TextDelta(chunk.to_owned()));
                        }
                    }
                    "input_json_delta" => {
                        if let Some(partial) = delta.get("partial_json").and_then(Value::as_str)
                            && let Some(tool) = active_tool.as_mut()
                        {
                            tool.json_chunks.push_str(partial);
                        }
                    }
                    _ => {}
                }
            }
        }
        "content_block_stop" => {
            finish_tool(active_tool, tool_calls, on_event);
        }
        "message_delta" => {
            if let Some(reason) = parsed
                .get("delta")
                .and_then(|delta| delta.get("stop_reason"))
                .and_then(Value::as_str)
            {
                *stop_reason = match reason {
                    "end_turn" => StopReason::EndTurn,
                    "tool_use" => StopReason::ToolUse,
                    "max_tokens" => StopReason::MaxTokens,
                    _ => StopReason::Other,
                };
            }
        }
        "error" => {
            let detail = parsed
                .get("error")
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("the provider reported a stream error")
                .to_owned();

            return Err(AiError::ApiStatus {
                status: 500,
                body: detail,
            });
        }
        _ => {}
    }
    Ok(())
}

fn finish_tool(
    active_tool: &mut Option<ActiveTool>,
    tool_calls: &mut Vec<ToolCall>,
    on_event: &mut dyn FnMut(StreamEvent),
) {
    let Some(tool) = active_tool.take() else {
        return;
    };
    let arguments = if tool.json_chunks.trim().is_empty() {
        Value::Object(Map::new())
    } else {
        serde_json::from_str(&tool.json_chunks).unwrap_or_else(|_| Value::Object(Map::new()))
    };

    let call = ToolCall {
        id: tool.id,
        name: tool.name,
        arguments,
    };

    on_event(StreamEvent::ToolCallReady(call.clone()));
    tool_calls.push(call);
}
