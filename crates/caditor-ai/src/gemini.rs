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
    let url = format!(
        "{}/v1beta/models/{}:streamGenerateContent?alt=sse",
        config.effective_base_url(),
        config.effective_model()
    );
    let payload = build_payload(config, messages, tools);
    let body = serde_json::to_string(&payload).map_err(|_| AiError::MalformedStream)?;
    let request = HttpRequest::post_json(url, body)
        .with_header("x-goog-api-key", config.api_key.as_str())
        .with_header("Accept", "text/event-stream");

    let mut parser = SseParser::default();
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    let mut next_call_index = 1_usize;
    let mut stop_reason = StopReason::EndTurn;

    let status = transport.post_stream(&request, cancel, &mut |line| {
        if let Some(event) = parser.push_line(line) {
            handle_chunk(
                &event.data,
                &mut text,
                &mut tool_calls,
                &mut next_call_index,
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
            &mut tool_calls,
            &mut next_call_index,
            &mut stop_reason,
            on_event,
        )?;
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
    let mut contents = Vec::new();
    for message in messages {
        match message {
            Message::User { text } => {
                contents.push(json!({
                    "role": "user",
                    "parts": [{ "text": text }],
                }));
            }
            Message::Assistant { text, tool_calls } => {
                let mut parts = Vec::new();
                if !text.is_empty() {
                    parts.push(json!({ "text": text }));
                }
                for call in tool_calls {
                    parts.push(json!({
                        "functionCall": {
                            "name": call.name,
                            "args": call.arguments,
                        }
                    }));
                }
                if !parts.is_empty() {
                    contents.push(json!({
                        "role": "model",
                        "parts": parts,
                    }));
                }
            }
            Message::ToolResults(results) => {
                let mut parts = Vec::new();
                for result in results {
                    let parsed_response = serde_json::from_str::<Value>(&result.content)
                        .ok()
                        .filter(Value::is_object)
                        .unwrap_or_else(|| {
                            if result.is_error {
                                json!({ "error": result.content })
                            } else {
                                json!({ "output": result.content })
                            }
                        });
                    parts.push(json!({
                        "functionResponse": {
                            "name": result.name,
                            "response": parsed_response,
                        }
                    }));
                    if let Some(png) = &result.image_png {
                        parts.push(json!({
                            "inlineData": {
                                "mimeType": "image/png",
                                "data": base64_encode(png),
                            }
                        }));
                    }
                }
                if !parts.is_empty() {
                    contents.push(json!({
                        "role": "user",
                        "parts": parts,
                    }));
                }
            }
        }
    }

    let mut payload = Map::new();
    payload.insert("contents".to_owned(), Value::Array(contents));
    if !config.system_prompt.is_empty() {
        payload.insert(
            "systemInstruction".to_owned(),
            json!({ "parts": [{ "text": config.system_prompt }] }),
        );
    }
    payload.insert(
        "generationConfig".to_owned(),
        json!({ "maxOutputTokens": config.max_tokens }),
    );
    if !tools.is_empty() {
        let declarations: Vec<Value> = tools
            .iter()
            .map(|tool| {
                let mut declaration = Map::new();
                declaration.insert("name".to_owned(), json!(tool.name));
                declaration.insert("description".to_owned(), json!(tool.description));
                declaration.insert(
                    "parametersJsonSchema".to_owned(),
                    normalize_schema(&tool.input_schema),
                );
                Value::Object(declaration)
            })
            .collect();
        payload.insert(
            "tools".to_owned(),
            json!([{ "functionDeclarations": declarations }]),
        );
    }
    Value::Object(payload)
}

pub fn normalize_schema(schema: &Value) -> Value {
    match schema {
        Value::Object(map) => {
            let mut out = Map::new();
            for (key, value) in map {
                if key == "additionalProperties" {
                    continue;
                }
                if key == "type"
                    && let Value::Array(types) = value
                {
                    let variants: Vec<Value> = types
                        .iter()
                        .filter_map(Value::as_str)
                        .map(|variant| json!({ "type": variant }))
                        .collect();
                    if !variants.is_empty() {
                        out.insert("anyOf".to_owned(), Value::Array(variants));
                    }
                    continue;
                }
                out.insert(key.clone(), normalize_schema(value));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(normalize_schema).collect()),
        other => other.clone(),
    }
}

fn handle_chunk(
    data: &str,
    text: &mut String,
    tool_calls: &mut Vec<ToolCall>,
    next_call_index: &mut usize,
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
            .unwrap_or("Gemini returned a stream error")
            .to_owned();
        let status = error
            .get("code")
            .and_then(Value::as_u64)
            .and_then(|code| u16::try_from(code).ok())
            .unwrap_or(500);
        return Err(AiError::ApiStatus { status, body });
    }
    let Some(candidates) = parsed.get("candidates").and_then(Value::as_array) else {
        return Ok(());
    };
    let Some(candidate) = candidates.first() else {
        return Ok(());
    };
    if let Some(parts) = candidate
        .get("content")
        .and_then(|content| content.get("parts"))
        .and_then(Value::as_array)
    {
        for part in parts {
            if let Some(chunk) = part.get("text").and_then(Value::as_str)
                && !chunk.is_empty()
            {
                text.push_str(chunk);
                on_event(StreamEvent::TextDelta(chunk.to_owned()));
            }
            if let Some(call) = part.get("functionCall")
                && let Some(name) = call.get("name").and_then(Value::as_str)
            {
                let id = call
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| {
                        let current = *next_call_index;
                        *next_call_index += 1;
                        format!("gemini_call_{current}")
                    });
                let arguments = call
                    .get("args")
                    .cloned()
                    .unwrap_or_else(|| Value::Object(Map::new()));
                let ready = ToolCall {
                    id,
                    name: name.to_owned(),
                    arguments,
                };
                on_event(StreamEvent::ToolCallReady(ready.clone()));
                tool_calls.push(ready);
            }
        }
    }
    if let Some(reason) = candidate.get("finishReason").and_then(Value::as_str) {
        *stop_reason = match reason {
            "STOP" => StopReason::EndTurn,
            "MAX_TOKENS" => StopReason::MaxTokens,
            _ => StopReason::Other,
        };
    }
    Ok(())
}
