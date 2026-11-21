use std::{collections::BTreeMap, sync::atomic::AtomicBool};

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

#[derive(Default)]
struct PendingTool {
    id: String,
    name: String,
    arguments: String,
}

pub fn stream(
    transport: &dyn Transport,
    config: &RequestConfig,
    messages: &[Message],
    tools: &[ToolDefinition],
    cancel: &AtomicBool,
    on_event: &mut dyn FnMut(StreamEvent),
) -> Result<TurnResponse, AiError> {
    if config.provider.requires_key() && config.api_key.is_empty() {
        return Err(AiError::MissingKey {
            provider: config.provider,
        });
    }

    let url = format!("{}/chat/completions", config.effective_base_url());
    let payload = build_payload(config, messages, tools);
    let body = serde_json::to_string(&payload).map_err(|_| AiError::MalformedStream)?;

    let mut request = HttpRequest::post_json(url, body).with_header("Accept", "text/event-stream");

    if !config.api_key.is_empty() {
        request = request.with_header(
            "Authorization",
            format!("Bearer {}", config.api_key.as_str()),
        );
    }

    let mut parser = SseParser::default();
    let mut text = String::new();
    let mut pending_tools: BTreeMap<usize, PendingTool> = BTreeMap::new();
    let mut stop_reason = StopReason::EndTurn;

    let status = transport.post_stream(&request, cancel, &mut |line| {
        if let Some(event) = parser.push_line(line) {
            handle_chunk(
                &event.data,
                &mut text,
                &mut pending_tools,
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
        handle_chunk(
            &event.data,
            &mut text,
            &mut pending_tools,
            &mut stop_reason,
            on_event,
        )?;
    }

    let mut tool_calls = Vec::new();

    for (index, pending) in pending_tools {
        if pending.name.is_empty() {
            continue;
        }

        let id = if pending.id.is_empty() {
            format!("openai_call_{}", index + 1)
        } else {
            pending.id
        };
        let arguments = if pending.arguments.trim().is_empty() {
            Value::Object(Map::new())
        } else {
            serde_json::from_str(&pending.arguments).unwrap_or_else(|_| Value::Object(Map::new()))
        };

        let call = ToolCall {
            id,
            name: pending.name,
            arguments,
        };

        on_event(StreamEvent::ToolCallReady(call.clone()));
        tool_calls.push(call);
    }

    if !tool_calls.is_empty() {
        stop_reason = StopReason::ToolUse;
    }

    Ok(TurnResponse {
        text,
        tool_calls,
        stop_reason,
    })
}

fn build_payload(config: &RequestConfig, messages: &[Message], tools: &[ToolDefinition]) -> Value {
    let mut encoded = Vec::new();

    if !config.system_prompt.is_empty() {
        encoded.push(json!({
            "role": "system",
            "content": config.system_prompt,
        }));
    }
    for message in messages {
        match message {
            Message::User { text } => {
                encoded.push(json!({
                    "role": "user",
                    "content": text,
                }));
            }
            Message::Assistant { text, tool_calls } => {
                let mut item = Map::new();

                item.insert("role".to_owned(), json!("assistant"));
                if text.is_empty() && !tool_calls.is_empty() {
                    item.insert("content".to_owned(), Value::Null);
                } else {
                    item.insert("content".to_owned(), json!(text));
                }
                if !tool_calls.is_empty() {
                    let calls: Vec<Value> = tool_calls
                        .iter()
                        .map(|call| {
                            let arguments = serde_json::to_string(&call.arguments)
                                .unwrap_or_else(|_| "{}".to_owned());
                            json!({
                                "id": call.id,
                                "type": "function",
                                "function": {
                                    "name": call.name,
                                    "arguments": arguments,
                                }
                            })
                        })
                        .collect();

                    item.insert("tool_calls".to_owned(), Value::Array(calls));
                }
                encoded.push(Value::Object(item));
            }
            Message::ToolResults(results) => {
                let mut images = Vec::new();

                for result in results {
                    encoded.push(json!({
                        "role": "tool",
                        "tool_call_id": result.call_id,
                        "content": result.content,
                    }));
                    if let Some(png) = &result.image_png {
                        images.push((result.name.clone(), png));
                    }
                }
                if !images.is_empty() {
                    let mut parts = vec![json!({
                        "type": "text",
                        "text": "Rendered model image from the preceding tool call:",
                    })];

                    for (_, png) in images {
                        parts.push(json!({
                            "type": "image_url",
                            "image_url": {
                                "url": format!("data:image/png;base64,{}", base64_encode(png)),
                            }
                        }));
                    }
                    encoded.push(json!({
                        "role": "user",
                        "content": parts,
                    }));
                }
            }
        }
    }

    let mut payload = Map::new();

    payload.insert("model".to_owned(), json!(config.effective_model()));
    payload.insert("stream".to_owned(), json!(true));
    payload.insert("max_completion_tokens".to_owned(), json!(config.max_tokens));
    payload.insert("messages".to_owned(), Value::Array(encoded));
    if !tools.is_empty() {
        let encoded_tools: Vec<Value> = tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.input_schema,
                    }
                })
            })
            .collect();

        payload.insert("tools".to_owned(), Value::Array(encoded_tools));
    }
    Value::Object(payload)
}

fn handle_chunk(
    data: &str,
    text: &mut String,
    pending_tools: &mut BTreeMap<usize, PendingTool>,
    stop_reason: &mut StopReason,
    on_event: &mut dyn FnMut(StreamEvent),
) -> Result<(), AiError> {
    let trimmed = data.trim();

    if trimmed.is_empty() || trimmed == "[DONE]" {
        return Ok(());
    }

    let parsed: Value = serde_json::from_str(trimmed).map_err(|_| AiError::MalformedStream)?;

    if let Some(error) = parsed.get("error") {
        let body = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("OpenAI returned a stream error")
            .to_owned();

        return Err(AiError::ApiStatus { status: 500, body });
    }

    let Some(choices) = parsed.get("choices").and_then(Value::as_array) else {
        return Ok(());
    };
    let Some(choice) = choices.first() else {
        return Ok(());
    };

    if let Some(delta) = choice.get("delta") {
        if let Some(chunk) = delta.get("content").and_then(Value::as_str)
            && !chunk.is_empty()
        {
            text.push_str(chunk);
            on_event(StreamEvent::TextDelta(chunk.to_owned()));
        }
        if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
            for call in calls {
                let index = call
                    .get("index")
                    .and_then(Value::as_u64)
                    .and_then(|value| usize::try_from(value).ok())
                    .unwrap_or(0);
                let entry = pending_tools.entry(index).or_default();

                if let Some(id) = call.get("id").and_then(Value::as_str)
                    && !id.is_empty()
                {
                    entry.id = id.to_owned();
                }
                if let Some(function) = call.get("function") {
                    if let Some(name) = function.get("name").and_then(Value::as_str)
                        && !name.is_empty()
                    {
                        entry.name.push_str(name);
                    }
                    if let Some(arguments) = function.get("arguments").and_then(Value::as_str) {
                        entry.arguments.push_str(arguments);
                    }
                }
            }
        }
    }
    if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
        *stop_reason = match reason {
            "stop" => StopReason::EndTurn,
            "tool_calls" => StopReason::ToolUse,
            "length" => StopReason::MaxTokens,
            _ => StopReason::Other,
        };
    }
    Ok(())
}
