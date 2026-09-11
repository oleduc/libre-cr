//! ChatGPT subscription provider — the Responses API on OpenAI's ChatGPT
//! backend, authorized by a Plus/Pro sign-in rather than an API key.
//! See `specs/04-review-daemon.md` § ChatGPT subscription provider.
//!
//! Deliberately not a variant of `openai_compat`: that one speaks chat
//! completions, and this backend speaks the Responses API — different content
//! typing, the system prompt as `instructions`, a flat tool schema, and a
//! different event vocabulary. Everything is mapped onto the same
//! `StreamEvent`, so the agent loop, the caps and the panel see no difference.

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use eventsource_stream::Eventsource;
use futures::stream::BoxStream;
use futures::StreamExt;
use serde_json::json;
use tokio::sync::Mutex;

use super::chatgpt_auth::{self, Tokens};
use super::{
    ContentBlock, Message, ModelInfo, Provider, ProviderCapabilities, Role, StreamEvent, ToolSchema,
};
use crate::error::{Error, Result};

const DEFAULT_BASE: &str = "https://chatgpt.com/backend-api/wham";

/// Sent as `client_version` on the model list, and the reason that list is
/// worth fetching rather than hardcoding: the server gates the catalogue on
/// it. Measured against a live account — `1.0.0` returned 8 models, `0.99.0`
/// returned 1, `0.80.0` returned none. Raise this as the backend moves on; too
/// low silently returns fewer models rather than an error.
const CLIENT_VERSION: &str = "1.0.0";
/// The subscription backend signs in rather than taking a key, and rejects
/// sampling parameters outright — see `build_body`.
pub const CAPABILITIES: ProviderCapabilities = ProviderCapabilities {
    api_key: false,
    endpoint: true,
    temperature: false,
    max_tokens: false,
    model: true,
    model_list: true,
};

pub struct ChatGptProvider {
    id: String,
    client: reqwest::Client,
    base: String,
    model: String,
    token_path: PathBuf,
    /// Serialized so two concurrent turns cannot both refresh and clobber
    /// each other's rotated refresh token.
    tokens: Arc<Mutex<Option<Tokens>>>,
}

impl ChatGptProvider {
    pub fn new(model: String, token_path: PathBuf) -> Self {
        Self {
            id: "chatgpt".into(),
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .unwrap_or_default(),
            base: DEFAULT_BASE.to_string(),
            model,
            token_path,
            tokens: Arc::new(Mutex::new(None)),
        }
    }

    pub fn with_base(mut self, base: String) -> Self {
        if !base.trim().is_empty() {
            self.base = base.trim().trim_end_matches('/').to_string();
        }
        self
    }

    /// A valid access token, refreshing when it is close to expiry. Returns
    /// `ProviderUnauthorized` when the user is signed out, which the config UI
    /// renders as "signed out" rather than as a provider fault.
    async fn access(&self) -> Result<Tokens> {
        let mut guard = self.tokens.lock().await;
        if guard.is_none() {
            *guard = chatgpt_auth::load_tokens(&self.token_path)?;
        }
        let current = guard.clone().ok_or(Error::ProviderUnauthorized)?;
        if !current.needs_refresh(chatgpt_auth::now_ms()) {
            return Ok(current);
        }
        match chatgpt_auth::refresh_tokens(&self.client, &current).await {
            Ok(next) => {
                chatgpt_auth::save_tokens(&self.token_path, &next)?;
                *guard = Some(next.clone());
                Ok(next)
            }
            Err(e) => {
                // A refresh that fails is a sign-out, not a retry loop: the
                // stored token is spent and no later attempt can revive it.
                let _ = chatgpt_auth::clear_tokens(&self.token_path);
                *guard = None;
                tracing::warn!(error = %e, "chatgpt refresh failed; signed out");
                Err(Error::ProviderUnauthorized)
            }
        }
    }

    pub fn build_body(&self, messages: &[Message], tools: &[ToolSchema]) -> serde_json::Value {
        let mut instructions = String::new();
        let mut input: Vec<serde_json::Value> = Vec::new();
        for m in messages {
            for c in &m.content {
                match (m.role, c) {
                    (Role::System, ContentBlock::Text { text }) => {
                        if !instructions.is_empty() {
                            instructions.push_str("\n\n");
                        }
                        instructions.push_str(text);
                    }
                    // Content parts are typed `input_text` / `output_text`
                    // here, not `text` — the backend rejects `text`.
                    (role, ContentBlock::Text { text }) => {
                        let (role_s, part) = match role {
                            Role::Assistant => ("assistant", "output_text"),
                            _ => ("user", "input_text"),
                        };
                        input.push(json!({
                            "type": "message",
                            "role": role_s,
                            "content": [{ "type": part, "text": text }],
                        }));
                    }
                    (
                        _,
                        ContentBlock::ToolUse {
                            id,
                            name,
                            input: args,
                        },
                    ) => {
                        input.push(json!({
                            "type": "function_call",
                            "call_id": id,
                            "name": name,
                            "arguments": args.to_string(),
                        }));
                    }
                    (
                        _,
                        ContentBlock::ToolResult {
                            tool_use_id,
                            content,
                            ..
                        },
                    ) => {
                        input.push(json!({
                            "type": "function_call_output",
                            "call_id": tool_use_id,
                            "output": content,
                        }));
                    }
                }
            }
        }
        // Flat tool schema: `{type, name, description, parameters}`, not the
        // chat-completions nesting under `function`.
        let tools_json: Vec<_> = tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.input_schema,
                })
            })
            .collect();
        let mut body = json!({
            "model": self.model,
            "instructions": instructions,
            "input": input,
            // Mandatory: this backend refuses to persist responses.
            "store": false,
            "stream": true,
        });
        if !tools_json.is_empty() {
            body["tools"] = json!(tools_json);
        }
        // No `temperature` and no token cap: the reasoning models this
        // subscription serves reject sampling parameters outright. The config
        // fields still apply to every other provider kind.
        body
    }
}

#[async_trait]
impl Provider for ChatGptProvider {
    fn capabilities(&self) -> ProviderCapabilities {
        CAPABILITIES
    }

    fn id(&self) -> &str {
        &self.id
    }

    async fn stream(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
    ) -> Result<BoxStream<'static, Result<StreamEvent>>> {
        let tokens = self.access().await?;
        let body = self.build_body(messages, tools);
        let resp = self
            .client
            .post(format!("{}/responses", self.base))
            .bearer_auth(&tokens.access)
            .header("ChatGPT-Account-Id", &tokens.account_id)
            .header("originator", chatgpt_auth::ORIGINATOR)
            .header("OpenAI-Beta", "responses=experimental")
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Internal(format!("chatgpt request: {e}")))?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(Error::ProviderUnauthorized);
        }
        if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(Error::ProviderRateLimited);
        }
        if !resp.status().is_success() {
            let s = resp.status();
            // The body carries the reason (an unsupported parameter, a model
            // the plan does not include); a bare status would hide it.
            let detail = resp.text().await.unwrap_or_default();
            let detail: String = detail.chars().take(400).collect();
            return Err(Error::Internal(format!("chatgpt status {s}: {detail}")));
        }
        let sse = resp
            .bytes_stream()
            .map(|r| r.map_err(std::io::Error::other))
            .eventsource();
        Ok(responses_event_stream(sse.boxed()).boxed())
    }

    async fn validate(&self) -> Result<()> {
        self.access().await.map(|_| ())
    }

    /// Ask the backend what this subscription offers.
    ///
    /// An earlier version of this shipped a hardcoded list, which was wrong
    /// twice over: the endpoint does exist, and a list written from memory
    /// offered model ids that do not.
    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let tokens = self.access().await?;
        let url = format!("{}/models?client_version={CLIENT_VERSION}", self.base);
        let resp = self
            .client
            .get(&url)
            .bearer_auth(&tokens.access)
            .header("ChatGPT-Account-Id", &tokens.account_id)
            .header("originator", chatgpt_auth::ORIGINATOR)
            .send()
            .await
            .map_err(|e| Error::Internal(format!("chatgpt models request: {e}")))?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(Error::ProviderUnauthorized);
        }
        if !resp.status().is_success() {
            let s = resp.status();
            let detail: String = resp
                .text()
                .await
                .unwrap_or_default()
                .chars()
                .take(300)
                .collect();
            return Err(Error::Internal(format!(
                "chatgpt models status {s}: {detail}"
            )));
        }
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::Internal(format!("chatgpt models parse: {e}")))?;
        let Some(models) = parse_models(&body) else {
            return Err(Error::Validation(format!(
                "{url} answered without a `models` list, so it is not the ChatGPT backend. \
                 Clear the endpoint field to use the default ({DEFAULT_BASE})."
            )));
        };
        if models.is_empty() {
            // An empty list is what the server returns when it considers the
            // client too old — not a subscription without models. Saying so
            // beats showing an empty dropdown.
            return Err(Error::Validation(format!(
                "{url} returned no models for client_version {CLIENT_VERSION}; it is probably \
                 behind what the server now expects"
            )));
        }
        Ok(models)
    }
}

/// `{ "models": [{ "slug": "gpt-6-astra", … }] }` → models, in server order.
/// Entries without a slug are skipped rather than rendered as blanks.
///
/// `None` means the response carried no `models` list at all, which is not the
/// same as an empty one: the first says we asked the wrong server, the second
/// says this server offered nothing. Conflating them produced a confident
/// wrong diagnosis — an endpoint left pointing at OpenRouter answered 200 with
/// `{data: […]}`, and the daemon blamed its own `client_version`.
fn parse_models(body: &serde_json::Value) -> Option<Vec<ModelInfo>> {
    let arr = body.get("models")?.as_array()?;
    Some(
        arr.iter()
            .filter_map(|m| {
                let slug = m
                    .get("slug")
                    .and_then(|s| s.as_str())
                    .filter(|s| !s.is_empty())?;
                Some(ModelInfo {
                    id: slug.to_string(),
                    display_name: m
                        .get("display_name")
                        .and_then(|s| s.as_str())
                        .filter(|s| !s.is_empty())
                        .map(|s| s.to_string()),
                    // `context_window` is what the model is served with here;
                    // `max_context_window` is what it could be raised to, so
                    // sizing caps from that would overshoot every turn.
                    context_tokens: m.get("context_window").and_then(|n| n.as_u64()),
                })
            })
            .collect(),
    )
}

/// One function call being streamed. Arguments arrive as deltas keyed by the
/// output item's id, so they are buffered and emitted once, whole.
#[derive(Default, Clone)]
struct CallBuf {
    call_id: String,
    name: String,
    args: String,
}

#[derive(Default)]
struct ResponsesState {
    calls: BTreeMap<String, CallBuf>,
    input_tokens: u64,
    output_tokens: u64,
    done_sent: bool,
    stop_reason: Option<String>,
}

fn emit_done(state: &mut ResponsesState, out: &mut VecDeque<Result<StreamEvent>>) {
    if state.done_sent {
        return;
    }
    state.done_sent = true;
    out.push_back(Ok(StreamEvent::Done {
        input_tokens: state.input_tokens,
        output_tokens: state.output_tokens,
        stop_reason: state.stop_reason.clone().unwrap_or_else(|| "stop".into()),
    }));
}

/// Emit every buffered call. Called when the response completes and on early
/// EOF, so a truncated stream never silently drops a tool call.
fn flush_calls(state: &mut ResponsesState, out: &mut VecDeque<Result<StreamEvent>>) {
    for (_, buf) in std::mem::take(&mut state.calls) {
        if buf.call_id.is_empty() && buf.name.is_empty() {
            continue;
        }
        let input: serde_json::Value = if buf.args.trim().is_empty() {
            json!({})
        } else {
            match serde_json::from_str(&buf.args) {
                Ok(v) => v,
                Err(e) => {
                    out.push_back(Err(Error::Internal(format!(
                        "chatgpt tool arguments parse: {e}"
                    ))));
                    continue;
                }
            }
        };
        out.push_back(Ok(StreamEvent::ToolUse {
            id: buf.call_id,
            name: buf.name,
            input,
        }));
    }
}

fn responses_event_stream<S>(sse: S) -> impl futures::Stream<Item = Result<StreamEvent>>
where
    S: futures::Stream<
            Item = std::result::Result<
                eventsource_stream::Event,
                eventsource_stream::EventStreamError<std::io::Error>,
            >,
        > + Send
        + 'static,
{
    use futures::stream;
    let initial: (
        ResponsesState,
        VecDeque<Result<StreamEvent>>,
        std::pin::Pin<Box<S>>,
    ) = (ResponsesState::default(), VecDeque::new(), Box::pin(sse));
    stream::unfold(initial, |(mut state, mut queue, mut sse)| async move {
        loop {
            if let Some(ev) = queue.pop_front() {
                return Some((ev, (state, queue, sse)));
            }
            match sse.next().await {
                None => {
                    flush_calls(&mut state, &mut queue);
                    emit_done(&mut state, &mut queue);
                    if let Some(ev) = queue.pop_front() {
                        return Some((ev, (state, queue, sse)));
                    }
                    return None;
                }
                Some(Err(e)) => {
                    return Some((
                        Err(Error::Internal(format!("sse: {e}"))),
                        (state, queue, sse),
                    ));
                }
                Some(Ok(ev)) => drain_response_event(&mut state, &ev.data, &mut queue),
            }
        }
    })
}

fn drain_response_event(
    state: &mut ResponsesState,
    data: &str,
    out: &mut VecDeque<Result<StreamEvent>>,
) {
    if data.trim().is_empty() || data == "[DONE]" {
        return;
    }
    let v: serde_json::Value = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(e) => {
            out.push_back(Err(Error::Internal(format!("parse sse: {e}"))));
            return;
        }
    };
    let kind = v.get("type").and_then(|t| t.as_str()).unwrap_or_default();
    let item_id = |v: &serde_json::Value| -> String {
        v.get("item_id")
            .or_else(|| v.get("item").and_then(|i| i.get("id")))
            .and_then(|s| s.as_str())
            .unwrap_or_default()
            .to_string()
    };
    match kind {
        "response.output_text.delta" => {
            if let Some(s) = v.get("delta").and_then(|d| d.as_str()) {
                if !s.is_empty() {
                    out.push_back(Ok(StreamEvent::TextDelta {
                        text: s.to_string(),
                    }));
                }
            }
        }
        "response.output_item.added" => {
            let item = v.get("item").cloned().unwrap_or_default();
            if item.get("type").and_then(|t| t.as_str()) == Some("function_call") {
                let entry = state.calls.entry(item_id(&v)).or_default();
                // `call_id` is what a result must be addressed to; `id` is the
                // streaming item. They are not interchangeable.
                entry.call_id = item
                    .get("call_id")
                    .and_then(|s| s.as_str())
                    .unwrap_or_default()
                    .to_string();
                entry.name = item
                    .get("name")
                    .and_then(|s| s.as_str())
                    .unwrap_or_default()
                    .to_string();
                if let Some(a) = item.get("arguments").and_then(|s| s.as_str()) {
                    entry.args.push_str(a);
                }
            }
        }
        "response.function_call_arguments.delta" => {
            if let Some(d) = v.get("delta").and_then(|d| d.as_str()) {
                state.calls.entry(item_id(&v)).or_default().args.push_str(d);
            }
        }
        "response.output_item.done" => {
            let item = v.get("item").cloned().unwrap_or_default();
            if item.get("type").and_then(|t| t.as_str()) == Some("function_call") {
                let entry = state.calls.entry(item_id(&v)).or_default();
                // The terminal item carries the complete arguments; prefer it
                // over the deltas we accumulated.
                if let Some(a) = item.get("arguments").and_then(|s| s.as_str()) {
                    if !a.is_empty() {
                        entry.args = a.to_string();
                    }
                }
                if entry.call_id.is_empty() {
                    entry.call_id = item
                        .get("call_id")
                        .and_then(|s| s.as_str())
                        .unwrap_or_default()
                        .to_string();
                }
                if entry.name.is_empty() {
                    entry.name = item
                        .get("name")
                        .and_then(|s| s.as_str())
                        .unwrap_or_default()
                        .to_string();
                }
            }
        }
        "response.completed" | "response.incomplete" => {
            if let Some(u) = v.get("response").and_then(|r| r.get("usage")) {
                state.input_tokens = u
                    .get("input_tokens")
                    .and_then(|n| n.as_u64())
                    .unwrap_or_default();
                state.output_tokens = u
                    .get("output_tokens")
                    .and_then(|n| n.as_u64())
                    .unwrap_or_default();
            }
            state.stop_reason = Some(if state.calls.is_empty() {
                "stop".into()
            } else {
                "tool_use".into()
            });
            flush_calls(state, out);
            emit_done(state, out);
        }
        "response.failed" | "error" => {
            let message = v
                .get("response")
                .and_then(|r| r.get("error"))
                .and_then(|e| e.get("message"))
                .or_else(|| v.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("chatgpt stream error")
                .to_string();
            out.push_back(Ok(StreamEvent::Error { message }));
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider() -> ChatGptProvider {
        ChatGptProvider::new("gpt-5.2-codex".into(), PathBuf::from("/nonexistent"))
    }

    #[test]
    fn system_prompt_becomes_instructions_and_parts_are_typed() {
        let msgs = vec![
            Message {
                role: Role::System,
                content: vec![ContentBlock::Text {
                    text: "be terse".into(),
                }],
            },
            Message {
                role: Role::User,
                content: vec![ContentBlock::Text {
                    text: "why?".into(),
                }],
            },
        ];
        let body = provider().build_body(&msgs, &[]);
        assert_eq!(body["instructions"], "be terse");
        // Exactly one input item: the system message is not repeated there.
        assert_eq!(body["input"].as_array().unwrap().len(), 1);
        assert_eq!(body["input"][0]["content"][0]["type"], "input_text");
        assert_eq!(body["store"], false);
        // Sampling parameters are rejected by this backend.
        assert!(body.get("temperature").is_none());
        assert!(body.get("max_tokens").is_none());
    }

    #[test]
    fn tool_calls_and_results_round_trip_as_responses_items() {
        let msgs = vec![
            Message {
                role: Role::Assistant,
                content: vec![ContentBlock::ToolUse {
                    id: "call_1".into(),
                    name: "grep".into(),
                    input: serde_json::json!({"pattern": "a"}),
                }],
            },
            Message {
                role: Role::User,
                content: vec![ContentBlock::ToolResult {
                    tool_use_id: "call_1".into(),
                    content: "{\"matches\":[]}".into(),
                    is_error: false,
                }],
            },
        ];
        let body = provider().build_body(&msgs, &[]);
        assert_eq!(body["input"][0]["type"], "function_call");
        assert_eq!(body["input"][0]["call_id"], "call_1");
        assert_eq!(body["input"][0]["arguments"], "{\"pattern\":\"a\"}");
        assert_eq!(body["input"][1]["type"], "function_call_output");
        assert_eq!(body["input"][1]["call_id"], "call_1");
    }

    #[test]
    fn tools_are_flat_not_nested_under_function() {
        let tools = vec![ToolSchema {
            name: "grep".into(),
            description: "search".into(),
            input_schema: serde_json::json!({"type": "object"}),
        }];
        let body = provider().build_body(&[], &tools);
        assert_eq!(body["tools"][0]["name"], "grep");
        assert!(body["tools"][0].get("function").is_none());
    }

    fn drain(chunks: &[&str]) -> Vec<Result<StreamEvent>> {
        let mut state = ResponsesState::default();
        let mut q = VecDeque::new();
        for c in chunks {
            drain_response_event(&mut state, c, &mut q);
        }
        flush_calls(&mut state, &mut q);
        emit_done(&mut state, &mut q);
        q.into_iter().collect()
    }

    #[test]
    fn text_deltas_and_usage_map_onto_stream_events() {
        let out = drain(&[
            r#"{"type":"response.output_text.delta","delta":"hel"}"#,
            r#"{"type":"response.output_text.delta","delta":"lo"}"#,
            r#"{"type":"response.completed","response":{"usage":{"input_tokens":11,"output_tokens":3}}}"#,
        ]);
        let texts: Vec<String> = out
            .iter()
            .filter_map(|e| match e {
                Ok(StreamEvent::TextDelta { text }) => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, vec!["hel", "lo"]);
        match out.last().unwrap() {
            Ok(StreamEvent::Done {
                input_tokens,
                output_tokens,
                ..
            }) => {
                assert_eq!((*input_tokens, *output_tokens), (11, 3));
            }
            other => panic!("expected Done, got {other:?}"),
        }
    }

    /// Arguments arrive in fragments keyed by the *item* id, while a result
    /// must be addressed to the *call* id. One `ToolUse`, whole, either way.
    #[test]
    fn function_call_arguments_are_buffered_and_emitted_once() {
        let out = drain(&[
            r#"{"type":"response.output_item.added","item":{"id":"it_1","type":"function_call","call_id":"call_9","name":"grep"}}"#,
            r#"{"type":"response.function_call_arguments.delta","item_id":"it_1","delta":"{\"pat"}"#,
            r#"{"type":"response.function_call_arguments.delta","item_id":"it_1","delta":"tern\":\"x\"}"}"#,
            r#"{"type":"response.completed","response":{"usage":{"input_tokens":1,"output_tokens":1}}}"#,
        ]);
        let calls: Vec<_> = out
            .iter()
            .filter_map(|e| match e {
                Ok(StreamEvent::ToolUse { id, name, input }) => {
                    Some((id.clone(), name.clone(), input.clone()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(calls.len(), 1, "one call, not one per fragment");
        assert_eq!(calls[0].0, "call_9");
        assert_eq!(calls[0].1, "grep");
        assert_eq!(calls[0].2["pattern"], "x");
    }

    #[test]
    fn a_truncated_stream_still_yields_its_call() {
        // No `response.completed`: the connection died mid-response.
        let out = drain(&[
            r#"{"type":"response.output_item.added","item":{"id":"it_1","type":"function_call","call_id":"c1","name":"grep","arguments":"{}"}}"#,
        ]);
        assert!(out
            .iter()
            .any(|e| matches!(e, Ok(StreamEvent::ToolUse { name, .. }) if name == "grep")));
    }

    #[test]
    fn stream_errors_surface_the_api_message() {
        let out = drain(&[
            r#"{"type":"response.failed","response":{"error":{"message":"model not available on this plan"}}}"#,
        ]);
        assert!(out.iter().any(
            |e| matches!(e, Ok(StreamEvent::Error { message }) if message.contains("not available"))
        ));
    }

    #[test]
    fn models_come_from_the_backend_shape() {
        // The shape the live endpoint returns, trimmed.
        let body = serde_json::json!({"models": [
            {"slug": "gpt-6-astra", "display_name": "GPT-6 Astra", "tool_mode": "code_mode_only",
             "context_window": 272000, "max_context_window": 872000},
            {"slug": "gpt-5.6-sol"},
            {"no_slug": true},
        ]});
        let models = parse_models(&body).unwrap();
        let ids: Vec<String> = models.iter().map(|m| m.id.clone()).collect();
        assert_eq!(ids, vec!["gpt-6-astra", "gpt-5.6-sol"]);
        assert_eq!(models[0].display_name.as_deref(), Some("GPT-6 Astra"));
        // The window it is served with, not the one it could be raised to.
        assert_eq!(models[0].context_tokens, Some(272_000));
        assert_eq!(models[1].context_tokens, None);
        // Empty list: this server has nothing for us (a stale client_version).
        assert_eq!(
            parse_models(&serde_json::json!({"models": []})),
            Some(vec![])
        );
        // No list at all: we asked something that is not this backend. An
        // endpoint left pointing at OpenRouter returns exactly this.
        assert_eq!(
            parse_models(&serde_json::json!({"data": [{"id": "x"}], "total_count": 1})),
            None
        );
    }

    #[tokio::test]
    async fn signed_out_is_unauthorized_not_a_fault() {
        let p = provider();
        assert!(matches!(
            p.validate().await,
            Err(Error::ProviderUnauthorized)
        ));
    }
}
