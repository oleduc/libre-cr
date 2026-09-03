//! Anthropic Messages API provider — skeleton.
//!
//! Constructs the request and parses the SSE stream into `StreamEvent`s.
//! The real network path is wired in Phase 8 hardening; for now this builds
//! cleanly and `validate()` returns Ok without contacting the network.

use std::collections::{HashMap, VecDeque};

use async_trait::async_trait;
use eventsource_stream::Eventsource;
use futures::stream::BoxStream;
use futures::StreamExt;
use serde_json::json;

use crate::error::{Error, Result};

use super::{ContentBlock, Message, ModelInfo, Provider, Role, StreamEvent, ToolSchema};

/// Derive the `/v1/models` URL from the configured Messages endpoint. The
/// stored endpoint is the *messages* URL (`.../v1/messages`); we swap a
/// trailing `/messages` for `/models`. For a non-standard endpoint we fall
/// back to `<scheme>://<host>/v1/models`.
fn models_url_from_endpoint(endpoint: &str) -> String {
    // `limit=1000` is the API maximum and comfortably above the catalogue
    // size, so a single page is the whole list (default page size is 20).
    if let Some(prefix) = endpoint.strip_suffix("/messages") {
        return format!("{prefix}/models?limit=1000");
    }
    // Best-effort: keep scheme + authority, force a `/v1/models` path.
    if let Some((scheme, rest)) = endpoint.split_once("://") {
        let host = rest.split('/').next().unwrap_or(rest);
        if !host.is_empty() {
            return format!("{scheme}://{host}/v1/models?limit=1000");
        }
    }
    "https://api.anthropic.com/v1/models?limit=1000".to_string()
}

/// Parse the Anthropic `GET /v1/models` response body. Shape:
/// `{ "data": [ { "type": "model", "id": "claude-…", "display_name": "…" }, … ] }`.
fn parse_models(body: &serde_json::Value) -> Vec<ModelInfo> {
    body.get("data")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    let id = m.get("id").and_then(|s| s.as_str())?.to_string();
                    let display_name = m
                        .get("display_name")
                        .and_then(|s| s.as_str())
                        .map(|s| s.to_string());
                    Some(ModelInfo { id, display_name })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub struct AnthropicProvider {
    id: String,
    api_key: String,
    model: String,
    max_tokens: u32,
    temperature: f32,
    endpoint: String,
    client: reqwest::Client,
}

impl AnthropicProvider {
    fn build_client() -> reqwest::Client {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .unwrap_or_default()
    }

    pub fn new(api_key: String, model: String, max_tokens: u32, temperature: f32) -> Self {
        Self {
            id: "anthropic".into(),
            api_key,
            model,
            max_tokens,
            temperature,
            endpoint: "https://api.anthropic.com/v1/messages".into(),
            client: Self::build_client(),
        }
    }

    pub fn with_endpoint(mut self, endpoint: String) -> Self {
        if !endpoint.trim().is_empty() {
            self.endpoint = super::normalize_endpoint(&endpoint, "/messages");
        }
        self
    }

    /// Test-only accessor for the resolved API key, used to assert the
    /// env-var fallback in `build_provider`.
    #[cfg(test)]
    pub(crate) fn api_key_for_test(&self) -> &str {
        &self.api_key
    }

    /// Assemble the auth/version headers for a request as `(name, value)`
    /// pairs. Pulled out (with [`Self::build_body`]) so the header + system
    /// prompt contract is unit-testable without a live HTTP call.
    pub fn build_headers(&self) -> Vec<(&'static str, String)> {
        vec![
            ("anthropic-version", "2023-06-01".to_string()),
            ("x-api-key", self.api_key.clone()),
        ]
    }

    /// Build the JSON body for the Messages API.
    pub fn build_body(&self, messages: &[Message], tools: &[ToolSchema]) -> serde_json::Value {
        let mut system = String::new();
        let mut converted = Vec::new();
        for m in messages {
            match m.role {
                Role::System => {
                    for c in &m.content {
                        if let ContentBlock::Text { text } = c {
                            if !system.is_empty() {
                                system.push('\n');
                            }
                            system.push_str(text);
                        }
                    }
                }
                Role::User | Role::Assistant => {
                    let role = match m.role {
                        Role::User => "user",
                        Role::Assistant => "assistant",
                        _ => unreachable!(),
                    };
                    converted.push(json!({
                        "role": role,
                        "content": m.content,
                    }));
                }
            }
        }
        let mut body = json!({
            "model": self.model,
            "max_tokens": self.max_tokens,
            "temperature": self.temperature,
            "stream": true,
            "messages": converted,
        });
        if !system.is_empty() {
            body["system"] = json!(system);
        }
        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }
        body
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
    fn id(&self) -> &str {
        &self.id
    }

    async fn stream(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
    ) -> Result<BoxStream<'static, Result<StreamEvent>>> {
        let body = self.build_body(messages, tools);
        let mut req = self
            .client
            .post(&self.endpoint)
            .header("content-type", "application/json");
        for (name, value) in self.build_headers() {
            req = req.header(name, value);
        }
        let resp = req
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Internal(format!("anthropic request: {e}")))?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(Error::ProviderUnauthorized);
        }
        if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(Error::ProviderRateLimited);
        }
        if !resp.status().is_success() {
            let s = resp.status();
            return Err(Error::Internal(format!("anthropic status: {s}")));
        }
        let stream = resp.bytes_stream();
        // Convert reqwest's `Bytes` stream to an `eventsource_stream::Eventsource`.
        let sse = stream
            .map(|r| r.map_err(std::io::Error::other))
            .eventsource();
        Ok(anthropic_event_stream(sse.boxed()).boxed())
    }

    async fn validate(&self) -> Result<()> {
        if self.api_key.is_empty() {
            return Err(Error::ProviderUnauthorized);
        }
        Ok(())
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let url = models_url_from_endpoint(&self.endpoint);
        let mut req = self.client.get(&url);
        for (name, value) in self.build_headers() {
            req = req.header(name, value);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| Error::Internal(format!("anthropic models request: {e}")))?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(Error::ProviderUnauthorized);
        }
        if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(Error::ProviderRateLimited);
        }
        if !resp.status().is_success() {
            let s = resp.status();
            return Err(Error::Internal(format!("anthropic models status: {s}")));
        }
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::Internal(format!("anthropic models parse: {e}")))?;
        Ok(parse_models(&body))
    }
}

/// Per-tool-use accumulator. Anthropic streams the tool input as a sequence
/// of `input_json_delta` events keyed by block index; we have to glue the
/// fragments back together and emit a single `ToolUse` at `content_block_stop`.
#[derive(Debug, Default, Clone)]
struct ToolBlockState {
    id: String,
    name: String,
    partial: String,
}

/// Streaming parser state. Shared between calls of [`drain_anthropic_event`]
/// via `stream::unfold`.
///
/// Usage fidelity (N1): the real Messages API delivers `input_tokens` on
/// `message_start` (`message.usage`), and `output_tokens` + `stop_reason`
/// on `message_delta` — `message_stop` is just `{"type":"message_stop"}`.
/// We track them here and emit the tally with the final `Done`.
#[derive(Default)]
struct AnthropicState {
    tools: HashMap<u64, ToolBlockState>,
    input_tokens: u64,
    output_tokens: u64,
    stop_reason: String,
}

/// Build a stateful `StreamEvent` stream out of a raw SSE stream. Drops the
/// per-fragment `input_json_delta` parsing into a `HashMap<index, …>` so the
/// agent loop sees one well-formed `ToolUse` per tool call.
fn anthropic_event_stream<S>(sse: S) -> impl futures::Stream<Item = Result<StreamEvent>>
where
    S: futures::Stream<
            Item = std::result::Result<
                eventsource_stream::Event,
                eventsource_stream::EventStreamError<std::io::Error>,
            >,
        > + Send
        + 'static,
{
    use futures::stream::{self, StreamExt};

    let state = AnthropicState::default();
    // We use unfold to thread the state plus a small queue: a single SSE
    // event can emit multiple StreamEvents (e.g. on content_block_stop we
    // may flush a buffered ToolUse).
    let initial: (
        AnthropicState,
        VecDeque<Result<StreamEvent>>,
        std::pin::Pin<Box<S>>,
    ) = (state, VecDeque::new(), Box::pin(sse));
    stream::unfold(initial, |(mut state, mut queue, mut sse)| async move {
        loop {
            if let Some(ev) = queue.pop_front() {
                return Some((ev, (state, queue, sse)));
            }
            let Some(next) = sse.next().await else {
                // Early EOF (no message_stop). Flush any parseable buffered
                // tool state instead of silently dropping it — mirrors the
                // OpenAI parser's end-of-stream behavior (N5).
                flush_anthropic_tools(&mut state, &mut queue);
                if let Some(ev) = queue.pop_front() {
                    return Some((ev, (state, queue, sse)));
                }
                return None;
            };
            match next {
                Err(e) => {
                    return Some((
                        Err(Error::Internal(format!("sse: {e}"))),
                        (state, queue, sse),
                    ));
                }
                Ok(ev) => {
                    drain_anthropic_event(&mut state, &ev.event, &ev.data, &mut queue);
                    // loop again to pop whatever this event produced
                }
            }
        }
    })
}

/// Drain buffered tool blocks (lowest index first) into `out` as `ToolUse`
/// events, surfacing unparseable input buffers as errors. Used on early EOF.
fn flush_anthropic_tools(state: &mut AnthropicState, out: &mut VecDeque<Result<StreamEvent>>) {
    let mut drained: Vec<(u64, ToolBlockState)> =
        std::mem::take(&mut state.tools).into_iter().collect();
    drained.sort_by_key(|(idx, _)| *idx);
    for (_, entry) in drained {
        let input: serde_json::Value = if entry.partial.is_empty() {
            json!({})
        } else {
            match serde_json::from_str(&entry.partial) {
                Ok(v) => v,
                Err(e) => {
                    out.push_back(Err(Error::Internal(format!(
                        "anthropic tool input parse: {e}"
                    ))));
                    continue;
                }
            }
        };
        out.push_back(Ok(StreamEvent::ToolUse {
            id: entry.id,
            name: entry.name,
            input,
        }));
    }
}

/// Apply one SSE event to the parser state, pushing zero or more
/// `StreamEvent`s onto `out`. Pulled out for unit-testing.
fn drain_anthropic_event(
    state: &mut AnthropicState,
    event: &str,
    data: &str,
    out: &mut VecDeque<Result<StreamEvent>>,
) {
    let v: serde_json::Value = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(e) => {
            out.push_back(Err(Error::Internal(format!("parse sse data: {e}"))));
            return;
        }
    };
    let index = v.get("index").and_then(|n| n.as_u64());
    match event {
        "message_start" => {
            // input_tokens arrives here, on message.usage.
            if let Some(n) = v
                .get("message")
                .and_then(|m| m.get("usage"))
                .and_then(|u| u.get("input_tokens"))
                .and_then(|n| n.as_u64())
            {
                state.input_tokens = n;
            }
        }
        "content_block_start" => {
            if let Some(block) = v.get("content_block") {
                if block.get("type").and_then(|s| s.as_str()) == Some("tool_use") {
                    let id = block
                        .get("id")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string();
                    let name = block
                        .get("name")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string();
                    if let Some(idx) = index {
                        // Seed the buffer; do *not* emit yet — we have no input
                        // until input_json_delta events accumulate and the
                        // block stops.
                        state.tools.insert(
                            idx,
                            ToolBlockState {
                                id,
                                name,
                                partial: String::new(),
                            },
                        );
                    }
                }
            }
        }
        "content_block_delta" => {
            let kind = v
                .get("delta")
                .and_then(|d| d.get("type"))
                .and_then(|s| s.as_str());
            match kind {
                Some("text_delta") => {
                    let text = v
                        .get("delta")
                        .and_then(|d| d.get("text"))
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string();
                    out.push_back(Ok(StreamEvent::TextDelta { text }));
                }
                Some("input_json_delta") => {
                    let frag = v
                        .get("delta")
                        .and_then(|d| d.get("partial_json"))
                        .and_then(|s| s.as_str())
                        .unwrap_or("");
                    if let Some(idx) = index {
                        if let Some(entry) = state.tools.get_mut(&idx) {
                            entry.partial.push_str(frag);
                        }
                    }
                }
                _ => {}
            }
        }
        "content_block_stop" => {
            if let Some(idx) = index {
                if let Some(entry) = state.tools.remove(&idx) {
                    let input: serde_json::Value = if entry.partial.is_empty() {
                        json!({})
                    } else {
                        match serde_json::from_str(&entry.partial) {
                            Ok(v) => v,
                            Err(e) => {
                                out.push_back(Err(Error::Internal(format!(
                                    "anthropic tool input parse: {e}"
                                ))));
                                return;
                            }
                        }
                    };
                    out.push_back(Ok(StreamEvent::ToolUse {
                        id: entry.id,
                        name: entry.name,
                        input,
                    }));
                }
            }
        }
        "message_delta" => {
            // The real API carries output_tokens and stop_reason here, not
            // on message_stop. The standard stream sends a single
            // message_delta at message end, so accumulating is equivalent
            // to taking the final value.
            if let Some(n) = v
                .get("usage")
                .and_then(|u| u.get("output_tokens"))
                .and_then(|n| n.as_u64())
            {
                state.output_tokens += n;
            }
            if let Some(s) = v
                .get("delta")
                .and_then(|d| d.get("stop_reason"))
                .and_then(|s| s.as_str())
            {
                state.stop_reason = s.to_string();
            }
        }
        "message_stop" => {
            out.push_back(Ok(StreamEvent::Done {
                input_tokens: state.input_tokens,
                output_tokens: state.output_tokens,
                stop_reason: state.stop_reason.clone(),
            }));
        }
        "error" => {
            // Mid-stream error frame (overloaded_error, invalid_request, …).
            // Surface the API's message instead of letting the stream end
            // with a generic "ended without done" (N5).
            let message = v
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|s| s.as_str())
                .unwrap_or("anthropic stream error")
                .to_string();
            out.push_back(Ok(StreamEvent::Error { message }));
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drain(events: &[(&str, &str)]) -> Vec<Result<StreamEvent>> {
        let mut s = AnthropicState::default();
        let mut q = VecDeque::new();
        for (e, d) in events {
            drain_anthropic_event(&mut s, e, d, &mut q);
        }
        q.into_iter().collect()
    }

    #[test]
    fn parses_text_delta() {
        let out = drain(&[(
            "content_block_delta",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hi"}}"#,
        )]);
        assert_eq!(out.len(), 1);
        assert!(
            matches!(out[0].as_ref().unwrap(), StreamEvent::TextDelta { text } if text == "hi")
        );
    }

    #[test]
    fn parses_message_stop() {
        // Real shape: message_stop carries no payload beyond its type; the
        // stop_reason arrives on message_delta.
        let out = drain(&[
            (
                "message_delta",
                r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":12}}"#,
            ),
            ("message_stop", r#"{"type":"message_stop"}"#),
        ]);
        assert_eq!(out.len(), 1);
        match out[0].as_ref().unwrap() {
            StreamEvent::Done { stop_reason, .. } => assert_eq!(stop_reason, "end_turn"),
            other => panic!("expected Done, got {other:?}"),
        }
    }

    #[test]
    fn usage_from_message_start_and_message_delta_lands_in_done() {
        // N1: input_tokens lives on message_start (message.usage),
        // output_tokens + stop_reason on message_delta. message_stop is
        // bare.
        let out = drain(&[
            (
                "message_start",
                r#"{"type":"message_start","message":{"id":"msg_1","role":"assistant","usage":{"input_tokens":37,"output_tokens":1}}}"#,
            ),
            (
                "content_block_delta",
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hi"}}"#,
            ),
            (
                "message_delta",
                r#"{"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":42}}"#,
            ),
            ("message_stop", r#"{"type":"message_stop"}"#),
        ]);
        let done = out.last().unwrap().as_ref().unwrap();
        match done {
            StreamEvent::Done {
                input_tokens,
                output_tokens,
                stop_reason,
            } => {
                assert_eq!(*input_tokens, 37);
                assert_eq!(*output_tokens, 42);
                assert_eq!(stop_reason, "end_turn");
            }
            other => panic!("expected Done, got {other:?}"),
        }
    }

    #[test]
    fn error_event_surfaces_api_message() {
        // N5: `event: error` must become StreamEvent::Error, not fall into
        // the catch-all.
        let out = drain(&[(
            "error",
            r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
        )]);
        assert_eq!(out.len(), 1);
        match out[0].as_ref().unwrap() {
            StreamEvent::Error { message } => assert_eq!(message, "Overloaded"),
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn early_eof_flushes_buffered_tool_state() {
        // N5: a stream that dies after the input deltas but before
        // content_block_stop must still yield the buffered ToolUse.
        let events: Vec<
            std::result::Result<
                eventsource_stream::Event,
                eventsource_stream::EventStreamError<std::io::Error>,
            >,
        > = vec![
            Ok(eventsource_stream::Event {
                event: "content_block_start".into(),
                data: r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_9","name":"grep","input":{}}}"#.into(),
                id: String::new(),
                retry: None,
            }),
            Ok(eventsource_stream::Event {
                event: "content_block_delta".into(),
                data: r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"q\":\"x\"}"}}"#.into(),
                id: String::new(),
                retry: None,
            }),
            // …and then the connection drops.
        ];
        let sse = futures::stream::iter(events);
        let out: Vec<Result<StreamEvent>> = anthropic_event_stream(sse).collect().await;
        assert_eq!(out.len(), 1, "expected the flushed ToolUse, got {out:?}");
        match out[0].as_ref().unwrap() {
            StreamEvent::ToolUse { id, name, input } => {
                assert_eq!(id, "toolu_9");
                assert_eq!(name, "grep");
                assert_eq!(input, &json!({"q":"x"}));
            }
            other => panic!("expected ToolUse, got {other:?}"),
        }
    }

    #[test]
    fn ignores_unknown() {
        let out = drain(&[("ping", "{}")]);
        assert!(out.is_empty());
    }

    #[test]
    fn buffers_tool_input_across_deltas_and_emits_once() {
        // C4: input_json_delta fragments accumulate per-block-index and emit
        // exactly one well-formed ToolUse on content_block_stop.
        let events = [
            (
                "content_block_start",
                r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"grep","input":{}}}"#,
            ),
            (
                "content_block_delta",
                r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"patte"}}"#,
            ),
            (
                "content_block_delta",
                r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"rn\":\"foo"}}"#,
            ),
            (
                "content_block_delta",
                r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"\"}"}}"#,
            ),
            (
                "content_block_stop",
                r#"{"type":"content_block_stop","index":1}"#,
            ),
            (
                "message_stop",
                r#"{"type":"message_stop","stop_reason":"tool_use"}"#,
            ),
        ];
        let out = drain(&events);
        // First event is the ToolUse with full input, second is Done.
        assert_eq!(
            out.len(),
            2,
            "expected one ToolUse and one Done, got {out:?}"
        );
        match out[0].as_ref().unwrap() {
            StreamEvent::ToolUse { id, name, input } => {
                assert_eq!(id, "toolu_1");
                assert_eq!(name, "grep");
                assert_eq!(input, &json!({"pattern": "foo"}));
            }
            other => panic!("expected ToolUse, got {other:?}"),
        }
        assert!(matches!(out[1].as_ref().unwrap(), StreamEvent::Done { .. }));
    }

    #[test]
    fn empty_tool_input_defaults_to_object() {
        // No deltas between start and stop → emit ToolUse with `{}`.
        let events = [
            (
                "content_block_start",
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"x","name":"noop","input":{}}}"#,
            ),
            (
                "content_block_stop",
                r#"{"type":"content_block_stop","index":0}"#,
            ),
        ];
        let out = drain(&events);
        assert_eq!(out.len(), 1);
        match out[0].as_ref().unwrap() {
            StreamEvent::ToolUse { input, .. } => {
                assert_eq!(input, &json!({}));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn models_url_swaps_messages_for_models() {
        assert_eq!(
            models_url_from_endpoint("https://api.anthropic.com/v1/messages"),
            "https://api.anthropic.com/v1/models?limit=1000"
        );
    }

    #[test]
    fn models_url_handles_nonstandard_endpoint() {
        // A proxy on a custom path → best-effort <scheme>://<host>/v1/models.
        assert_eq!(
            models_url_from_endpoint("https://proxy.example.com:8443/anthropic/relay"),
            "https://proxy.example.com:8443/v1/models?limit=1000"
        );
    }

    #[test]
    fn parse_models_extracts_id_and_display_name() {
        let body = serde_json::json!({
            "data": [
                {"type": "model", "id": "claude-opus-4", "display_name": "Claude Opus 4"},
                {"type": "model", "id": "claude-haiku"}
            ],
            "has_more": false
        });
        let models = parse_models(&body);
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "claude-opus-4");
        assert_eq!(models[0].display_name.as_deref(), Some("Claude Opus 4"));
        assert_eq!(models[1].id, "claude-haiku");
        assert_eq!(models[1].display_name, None);
    }

    #[test]
    fn parse_models_empty_on_missing_data() {
        assert!(parse_models(&serde_json::json!({})).is_empty());
    }

    #[test]
    fn builds_body() {
        let p = AnthropicProvider::new("k".into(), "claude-sonnet".into(), 4096, 0.0);
        let body = p.build_body(
            &[Message {
                role: Role::User,
                content: vec![ContentBlock::Text { text: "hi".into() }],
            }],
            &[],
        );
        assert_eq!(body["model"], "claude-sonnet");
        assert_eq!(body["stream"], true);
    }

    #[test]
    fn api_key_mode_headers_use_x_api_key() {
        let p = AnthropicProvider::new("secret".into(), "m".into(), 100, 0.0);
        let headers = p.build_headers();
        assert!(headers
            .iter()
            .any(|(n, v)| *n == "x-api-key" && v == "secret"));
        assert!(headers
            .iter()
            .any(|(n, v)| *n == "anthropic-version" && v == "2023-06-01"));
    }

    #[test]
    fn system_messages_are_joined_into_system_string() {
        let p = AnthropicProvider::new("k".into(), "m".into(), 100, 0.0);
        let body = p.build_body(
            &[Message {
                role: Role::System,
                content: vec![ContentBlock::Text {
                    text: "You are a code reviewer.".into(),
                }],
            }],
            &[],
        );
        assert_eq!(body["system"], "You are a code reviewer.");
    }
}
