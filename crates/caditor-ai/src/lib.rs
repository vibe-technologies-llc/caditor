mod anthropic;
mod error;
#[cfg(any(test, feature = "test-support"))]
pub mod fake;
mod gemini;
mod network;
mod openai;
mod sse;
mod transport;
mod types;

use std::sync::atomic::AtomicBool;

pub use self::{
    error::AiError,
    gemini::normalize_schema,
    network::Network,
    transport::{HttpRequest, StreamStatus, Transport},
    types::{
        ApiKey, ApiProvider, Message, RequestConfig, StopReason, StreamEvent, ToolCall,
        ToolDefinition, ToolResult, TurnResponse, base64_encode, endpoint_allows_http,
        validate_endpoint,
    },
};

pub fn stream_turn(
    transport: &dyn Transport,
    config: &RequestConfig,
    messages: &[Message],
    tools: &[ToolDefinition],
    cancel: &AtomicBool,
    on_event: &mut dyn FnMut(StreamEvent),
) -> Result<TurnResponse, AiError> {
    match config.provider {
        ApiProvider::Claude => {
            anthropic::stream(transport, config, messages, tools, cancel, on_event)
        }
        ApiProvider::Gemini => gemini::stream(transport, config, messages, tools, cancel, on_event),
        ApiProvider::OpenAi | ApiProvider::Custom => {
            openai::stream(transport, config, messages, tools, cancel, on_event)
        }
    }
}

pub(crate) fn status_error(status: StreamStatus) -> AiError {
    match status.status {
        401 | 403 => AiError::Unauthorised,
        429 => AiError::RateLimited {
            retry_after: status.retry_after_seconds,
        },
        code => AiError::ApiStatus {
            status: code,
            body: status
                .error_body
                .unwrap_or_else(|| "the provider gave no details".to_owned()),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use serde_json::{Value, json};

    use super::*;
    use crate::fake::{ScriptedStream, ScriptedTransport};

    fn sample_tool() -> ToolDefinition {
        ToolDefinition {
            name: "add_primitive".to_owned(),
            description: "Adds a box".to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "kind": { "type": "string" },
                    "body": { "type": ["integer", "string"] }
                },
                "required": ["kind"],
                "additionalProperties": false
            }),
        }
    }

    #[test]
    fn api_key_and_http_request_never_leak_secrets_in_debug() {
        let key = ApiKey::new("sk-secret-value-12345");
        let request = HttpRequest::post_json(
            "https://api.openai.com/v1/chat/completions".to_owned(),
            "{\"secret\":\"body\"}".to_owned(),
        )
        .with_header("Authorization", "Bearer sk-secret-value-12345");

        let debug_key = format!("{key:?}");
        let debug_req = format!("{request:?}");

        assert!(!debug_key.contains("sk-secret"));
        assert!(!debug_req.contains("sk-secret"));
        assert!(!debug_req.contains("body\""));
    }

    #[test]
    fn endpoint_validation_allows_https_and_loopback_http_only() {
        assert!(validate_endpoint("https://api.openai.com/v1"));
        assert!(validate_endpoint("http://localhost:11434/v1"));
        assert!(validate_endpoint("http://127.0.0.1:8080/v1"));
        assert!(validate_endpoint("http://[::1]:11434/v1"));
        assert!(!validate_endpoint("http://example.com/v1"));
        assert!(!validate_endpoint("ftp://localhost/v1"));
        assert!(!validate_endpoint("https://bad host/v1"));
    }

    #[test]
    fn base64_encodes_rfc4648_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn anthropic_streams_text_and_tool_calls_and_sends_image_results() {
        let transport = ScriptedTransport::new([Ok(ScriptedStream::ok([
            "event: content_block_start\n",
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n",
            "\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Making a box.\"}}\n",
            "\n",
            "event: content_block_stop\n",
            "data: {\"type\":\"content_block_stop\",\"index\":0}\n",
            "\n",
            "event: content_block_start\n",
            "data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"add_primitive\",\"input\":{}}}\n",
            "\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"kind\\\":\"}}\n",
            "\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"\\\"box\\\"}\"}}\n",
            "\n",
            "event: content_block_stop\n",
            "data: {\"type\":\"content_block_stop\",\"index\":1}\n",
            "\n",
            "event: message_delta\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"}}\n",
            "\n",
        ]))]);
        let config = RequestConfig {
            provider: ApiProvider::Claude,
            model: "claude-sonnet-4-6".to_owned(),
            api_key: ApiKey::new("sk-ant-test"),
            base_url: None,
            system_prompt: "You are a CAD assistant.".to_owned(),
            max_tokens: 1024,
        };
        let messages = vec![
            Message::User {
                text: "Make a box".to_owned(),
            },
            Message::ToolResults(vec![ToolResult {
                call_id: "toolu_0".to_owned(),
                name: "render_view".to_owned(),
                content: "{\"ok\":true}".to_owned(),
                image_png: Some(vec![137, 80, 78, 71]),
                is_error: false,
            }]),
        ];
        let mut events = Vec::new();
        let cancel = AtomicBool::new(false);

        let response = stream_turn(
            &transport,
            &config,
            &messages,
            &[sample_tool()],
            &cancel,
            &mut |event| events.push(event),
        )
        .unwrap();

        assert_eq!(response.text, "Making a box.");
        assert_eq!(response.stop_reason, StopReason::ToolUse);
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].id, "toolu_1");
        assert_eq!(response.tool_calls[0].name, "add_primitive");
        assert_eq!(response.tool_calls[0].arguments, json!({ "kind": "box" }));
        let sent = transport.recorded();
        let body: Value = serde_json::from_str(&sent[0].body).unwrap();
        assert_eq!(
            body["messages"][1]["content"][0]["content"][1]["type"],
            "image"
        );
    }

    #[test]
    fn gemini_normalizes_union_types_and_parses_function_calls() {
        let normalized = normalize_schema(&sample_tool().input_schema);
        assert!(normalized.get("additionalProperties").is_none());
        assert_eq!(
            normalized["properties"]["body"]["anyOf"],
            json!([{ "type": "integer" }, { "type": "string" }])
        );

        let transport = ScriptedTransport::new([Ok(ScriptedStream::ok([
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Checking model.\"}]}}]}\n",
            "\n",
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"functionCall\":{\"name\":\"get_model\",\"args\":{}}}]},\"finishReason\":\"STOP\"}]}\n",
            "\n",
        ]))]);
        let config = RequestConfig {
            provider: ApiProvider::Gemini,
            model: "gemini-2.5-pro".to_owned(),
            api_key: ApiKey::new("AIza-test"),
            base_url: None,
            system_prompt: "CAD".to_owned(),
            max_tokens: 1024,
        };
        let cancel = AtomicBool::new(false);
        let response = stream_turn(
            &transport,
            &config,
            &[Message::User {
                text: "Inspect".to_owned(),
            }],
            &[sample_tool()],
            &cancel,
            &mut |_| {},
        )
        .unwrap();

        assert_eq!(response.text, "Checking model.");
        assert_eq!(response.stop_reason, StopReason::ToolUse);
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].name, "get_model");
    }

    #[test]
    fn openai_streams_incremental_tool_calls_and_allows_keyless_custom_endpoint() {
        let transport = ScriptedTransport::new([Ok(ScriptedStream::ok([
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hi \"}}]}\n",
            "\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"there\"}}]}\n",
            "\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"function\":{\"name\":\"add_\",\"arguments\":\"{\\\"k\\\"\"}}]}}]}\n",
            "\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"name\":\"box\",\"arguments\":\":1}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n",
            "\n",
            "data: [DONE]\n",
            "\n",
        ]))]);
        let config = RequestConfig {
            provider: ApiProvider::Custom,
            model: "qwen2.5".to_owned(),
            api_key: ApiKey::default(),
            base_url: Some("http://localhost:11434/v1".to_owned()),
            system_prompt: "CAD".to_owned(),
            max_tokens: 1024,
        };
        let cancel = AtomicBool::new(false);
        let response = stream_turn(
            &transport,
            &config,
            &[Message::User {
                text: "Go".to_owned(),
            }],
            &[sample_tool()],
            &cancel,
            &mut |_| {},
        )
        .unwrap();

        assert_eq!(response.text, "Hi there");
        assert_eq!(response.stop_reason, StopReason::ToolUse);
        assert_eq!(response.tool_calls[0].id, "call_1");
        assert_eq!(response.tool_calls[0].name, "add_box");
        assert_eq!(response.tool_calls[0].arguments, json!({ "k": 1 }));
    }

    #[test]
    fn unauthorised_and_cancelled_errors_are_typed() {
        let transport = ScriptedTransport::new([Ok(ScriptedStream::error(401, "bad key"))]);
        let config = RequestConfig {
            provider: ApiProvider::OpenAi,
            model: "gpt-5.2".to_owned(),
            api_key: ApiKey::new("bad"),
            base_url: None,
            system_prompt: String::new(),
            max_tokens: 256,
        };
        let cancel = AtomicBool::new(false);
        let err = stream_turn(&transport, &config, &[], &[], &cancel, &mut |_| {}).unwrap_err();
        assert_eq!(err, AiError::Unauthorised);

        let cancelled = AtomicBool::new(true);
        let err = stream_turn(&transport, &config, &[], &[], &cancelled, &mut |_| {}).unwrap_err();
        assert_eq!(err, AiError::Cancelled);
    }
}
