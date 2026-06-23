//! OpenAI-compatible chat-completions provider — skeleton.
//!
//! Works against api.openai.com, OpenRouter, Ollama, and any compatible
//! endpoint. The actual network path is wired in Phase 8.

use std::collections::{BTreeMap, VecDeque};

use async_trait::async_trait;
use eventsource_stream::Eventsource;
use futures::stream::BoxStream;
use futures::StreamExt;
use serde_json::json;

use crate::error::{Error, Result};

use super::{ContentBlock, Message, Provider, Role, StreamEvent, ToolSchema};

pub struct OpenAICompatProvider {
    id: String,
    api_key: String,
    model: String,
    max_tokens: u32,
    temperature: f32,
    endpoint: String,
    client: reqwest::Client,
}

impl OpenAICompatProvider {
    pub fn new(api_key: String, model: String, max_tokens: u32, temperature: f32) -> Self {
        Self {
            id: "openai_compat".into(),
            api_key,
            model,
            max_tokens,
            temperature,
            endpoint: "https://api.openai.com/v1/chat/completions".into(),
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .unwrap_or_default(),
        }
    }

    pub fn with_endpoint(mut self, endpoint: String) -> Self {
        if !endpoint.is_empty() {
            self.endpoint = endpoint;
        }
        self
    }

    pub fn build_body(&self, messages: &[Message], tools: &[ToolSchema]) -> serde_json::Value {
        // OpenAI requires each tool result to be its own `role: "tool"`
        // message; we flatten the provider-neutral `ContentBlock::ToolResult`
        // sequence into one such message per block. (Prior versions emitted
        // a single `{_tool_results: […]}` blob which the API rejected.)
        let mut converted: Vec<serde_json::Value> = Vec::with_capacity(messages.len());
        for m in messages {
            let role = match m.role {
                Role::System => "system",
                Role::User => "user",
                Role::Assistant => "assistant",
            };
            let mut content_text = String::new();
            let mut tool_calls: Vec<serde_json::Value> = Vec::new();
            let mut tool_result_msgs: Vec<serde_json::Value> = Vec::new();
            for c in &m.content {
                match c {
                    ContentBlock::Text { text } => content_text.push_str(text),
                    ContentBlock::ToolUse { id, name, input } => {
                        tool_calls.push(json!({
                            "id": id,
                            "type": "function",
                            "function": { "name": name, "arguments": input.to_string() }
                        }));
                    }
                    ContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        ..
                    } => {
                        tool_result_msgs.push(json!({
                            "role": "tool",
                            "tool_call_id": tool_use_id,
                            "content": content,
                        }));
                    }
                }
            }
            // If this message carried tool *results*, splat them as their
            // own top-level messages and skip the wrapper — there's no
            // meaningful role to attach a sibling text to.
            if !tool_result_msgs.is_empty() {
                converted.extend(tool_result_msgs);
                continue;
            }
            let mut msg = json!({ "role": role, "content": content_text });
            if !tool_calls.is_empty() {
                msg["tool_calls"] = json!(tool_calls);
            }
            converted.push(msg);
        }
        let tools_json: Vec<_> = tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.input_schema,
                    }
                })
            })
            .collect();
        let mut body = json!({
            "model": self.model,
            "max_tokens": self.max_tokens,
            "temperature": self.temperature,
            "stream": true,
            // N1: ask for the final usage chunk (sent after the last choice
            // chunk, with an empty `choices` array) so token tallies are real.
            "stream_options": { "include_usage": true },
            "messages": converted,
        });
        if !tools_json.is_empty() {
            body["tools"] = json!(tools_json);
        }
        body
    }
}

#[async_trait]
impl Provider for OpenAICompatProvider {
    fn id(&self) -> &str {
        &self.id
    }

    async fn stream(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
    ) -> Result<BoxStream<'static, Result<StreamEvent>>> {
        let body = self.build_body(messages, tools);
        let resp = self
            .client
            .post(&self.endpoint)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Internal(format!("openai request: {e}")))?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(Error::ProviderUnauthorized);
        }
        if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(Error::ProviderRateLimited);
        }
        if !resp.status().is_success() {
            let s = resp.status();
            return Err(Error::Internal(format!("openai status: {s}")));
        }
        let stream = resp.bytes_stream();
        let sse = stream
            .map(|r| r.map_err(std::io::Error::other))
            .eventsource();
        Ok(openai_event_stream(sse.boxed()).boxed())
    }

    async fn validate(&self) -> Result<()> {
        if self.api_key.is_empty() {
            return Err(Error::ProviderUnauthorized);
        }
        Ok(())
    }
}

/// One in-flight tool call. OpenAI's tool-call stream is chunked across many
/// SSE frames: the `id` and `function.name` arrive in the first chunk, then
/// `function.arguments` arrives in 1+ subsequent chunks, all keyed by the
/// same `index`. Without buffering, we'd emit a fresh `ToolUse` per chunk
/// with un-parseable partial JSON.
#[derive(Debug, Default, Clone)]
struct OpenAiToolBuf {
    id: String,
    name: String,
    args: String,
}

#[derive(Default)]
struct OpenAiState {
    /// Keyed by `tool_calls[].index`. BTreeMap so iteration order on flush is
    /// stable (lowest index first).
    tools: BTreeMap<u64, OpenAiToolBuf>,
    done_sent: bool,
    /// From the final usage chunk (`stream_options.include_usage`): a chunk
    /// with empty `choices` and a `usage` object, sent after the
    /// finish_reason chunk and before `[DONE]`.
    input_tokens: u64,
    output_tokens: u64,
    /// Recorded at the finish_reason chunk; the `Done` event is deferred to
    /// `[DONE]`/EOF so the usage chunk (which arrives later) is included.
    stop_reason: Option<String>,
}

/// Emit the terminal `Done` exactly once, with whatever usage/stop_reason
/// the stream delivered.
fn emit_openai_done(state: &mut OpenAiState, out: &mut VecDeque<Result<StreamEvent>>) {
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

fn openai_event_stream<S>(sse: S) -> impl futures::Stream<Item = Result<StreamEvent>>
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

    let initial: (
        OpenAiState,
        VecDeque<Result<StreamEvent>>,
        std::pin::Pin<Box<S>>,
    ) = (OpenAiState::default(), VecDeque::new(), Box::pin(sse));
    stream::unfold(initial, |(mut state, mut queue, mut sse)| async move {
        loop {
            if let Some(ev) = queue.pop_front() {
                return Some((ev, (state, queue, sse)));
            }
            let next = sse.next().await;
            match next {
                None => {
                    // End of stream (server skipped the `[DONE]` sentinel).
                    // Flush any buffered tool calls, then close out.
                    flush_openai_tools(&mut state, &mut queue, "stop");
                    emit_openai_done(&mut state, &mut queue);
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
                Some(Ok(ev)) => {
                    drain_openai_event(&mut state, &ev.data, &mut queue);
                }
            }
        }
    })
}

fn drain_openai_event(
    state: &mut OpenAiState,
    data: &str,
    out: &mut VecDeque<Result<StreamEvent>>,
) {
    if data == "[DONE]" {
        flush_openai_tools(state, out, "stop");
        emit_openai_done(state, out);
        return;
    }
    let v: serde_json::Value = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(e) => {
            out.push_back(Err(Error::Internal(format!("parse sse: {e}"))));
            return;
        }
    };
    // With `stream_options.include_usage`, every chunk carries `usage:null`
    // except the final one, which carries the real tally (and an empty
    // `choices` array).
    if let Some(u) = v.get("usage").filter(|u| u.is_object()) {
        if let Some(n) = u.get("prompt_tokens").and_then(|n| n.as_u64()) {
            state.input_tokens = n;
        }
        if let Some(n) = u.get("completion_tokens").and_then(|n| n.as_u64()) {
            state.output_tokens = n;
        }
    }
    let Some(choice) = v.get("choices").and_then(|c| c.get(0)) else {
        return;
    };
    if let Some(delta) = choice.get("delta") {
        if let Some(s) = delta.get("content").and_then(|x| x.as_str()) {
            if !s.is_empty() {
                out.push_back(Ok(StreamEvent::TextDelta {
                    text: s.to_string(),
                }));
            }
        }
        if let Some(tool_calls) = delta.get("tool_calls").and_then(|x| x.as_array()) {
            for tc in tool_calls {
                let idx = tc.get("index").and_then(|n| n.as_u64()).unwrap_or(0);
                let entry = state.tools.entry(idx).or_default();
                if let Some(id) = tc.get("id").and_then(|s| s.as_str()) {
                    if !id.is_empty() {
                        entry.id = id.to_string();
                    }
                }
                if let Some(name) = tc
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|s| s.as_str())
                {
                    if !name.is_empty() {
                        entry.name = name.to_string();
                    }
                }
                if let Some(args) = tc
                    .get("function")
                    .and_then(|f| f.get("arguments"))
                    .and_then(|s| s.as_str())
                {
                    entry.args.push_str(args);
                }
            }
        }
    }
    if let Some(reason) = choice.get("finish_reason").and_then(|s| s.as_str()) {
        // `tool_calls` and `stop` both close the assistant message; either
        // way flush any buffered tool calls now. The `Done` itself is
        // deferred to `[DONE]` / end-of-stream because the usage chunk
        // arrives *after* the finish_reason chunk (N1) — emitting here
        // would report zeros.
        flush_openai_tools(state, out, reason);
        if matches!(reason, "stop" | "length" | "content_filter" | "tool_calls") {
            state.stop_reason = Some(reason.to_string());
        }
    }
}

/// Drain the buffered tool calls into `out` as one `ToolUse` per entry,
/// parsing the accumulated argument JSON. Called on `finish_reason` or at
/// end-of-stream.
fn flush_openai_tools(
    state: &mut OpenAiState,
    out: &mut VecDeque<Result<StreamEvent>>,
    _reason: &str,
) {
    let drained: Vec<OpenAiToolBuf> = std::mem::take(&mut state.tools).into_values().collect();
    for entry in drained {
        let input: serde_json::Value = if entry.args.is_empty() {
            json!({})
        } else {
            match serde_json::from_str(&entry.args) {
                Ok(v) => v,
                Err(e) => {
                    out.push_back(Err(Error::Internal(format!(
                        "openai tool arguments parse: {e}"
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

#[cfg(test)]
mod tests {
    use super::*;

    fn drain(chunks: &[&str]) -> Vec<Result<StreamEvent>> {
        let mut state = OpenAiState::default();
        let mut q = VecDeque::new();
        for chunk in chunks {
            drain_openai_event(&mut state, chunk, &mut q);
        }
        q.into_iter().collect()
    }

    #[test]
    fn parses_done() {
        let out = drain(&["[DONE]"]);
        assert_eq!(out.len(), 1);
        assert!(matches!(out[0].as_ref().unwrap(), StreamEvent::Done { .. }));
    }

    #[test]
    fn parses_text_delta() {
        let out = drain(&[r#"{"choices":[{"delta":{"content":"hi"}}]}"#]);
        assert_eq!(out.len(), 1);
        match out[0].as_ref().unwrap() {
            StreamEvent::TextDelta { text } => assert_eq!(text, "hi"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn buffers_tool_arguments_across_chunks_and_emits_once() {
        // C4: arguments fragments accumulate per `index` across SSE chunks.
        // Without this, each chunk emits a fresh ToolUse with broken JSON.
        // Real stream shape with include_usage: finish_reason chunk, then a
        // usage chunk with empty choices, then the [DONE] sentinel.
        let chunks = [
            // First chunk carries id + function.name + opening of arguments.
            r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"grep","arguments":"{\"pat"}}]}}],"usage":null}"#,
            // Middle chunks add to arguments only.
            r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"tern\":\"f"}}]}}],"usage":null}"#,
            r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"oo\"}"}}]}}],"usage":null}"#,
            // finish_reason chunk flushes the buffered tool call.
            r#"{"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}],"usage":null}"#,
            // Usage chunk (empty choices), then sentinel → Done.
            r#"{"choices":[],"usage":{"prompt_tokens":11,"completion_tokens":7}}"#,
            "[DONE]",
        ];
        let out = drain(&chunks);
        assert!(
            out.len() == 2,
            "expected exactly one ToolUse and one Done, got {} events: {:?}",
            out.len(),
            out
        );
        match out[0].as_ref().unwrap() {
            StreamEvent::ToolUse { id, name, input } => {
                assert_eq!(id, "call_1");
                assert_eq!(name, "grep");
                assert_eq!(input, &json!({"pattern": "foo"}));
            }
            other => panic!("expected ToolUse, got {other:?}"),
        }
        match out[1].as_ref().unwrap() {
            StreamEvent::Done {
                input_tokens,
                output_tokens,
                stop_reason,
            } => {
                assert_eq!(*input_tokens, 11);
                assert_eq!(*output_tokens, 7);
                assert_eq!(stop_reason, "tool_calls");
            }
            other => panic!("expected Done, got {other:?}"),
        }
    }

    #[test]
    fn final_usage_chunk_lands_in_done() {
        // N1: with stream_options.include_usage the final pre-[DONE] chunk
        // carries the real token tally.
        let chunks = [
            r#"{"choices":[{"index":0,"delta":{"content":"hello"},"finish_reason":null}],"usage":null}"#,
            r#"{"choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":null}"#,
            r#"{"choices":[],"usage":{"prompt_tokens":21,"completion_tokens":34,"total_tokens":55}}"#,
            "[DONE]",
        ];
        let out = drain(&chunks);
        let done = out.last().unwrap().as_ref().unwrap();
        match done {
            StreamEvent::Done {
                input_tokens,
                output_tokens,
                stop_reason,
            } => {
                assert_eq!(*input_tokens, 21);
                assert_eq!(*output_tokens, 34);
                assert_eq!(stop_reason, "stop");
            }
            other => panic!("expected Done, got {other:?}"),
        }
    }

    #[test]
    fn build_body_requests_stream_usage() {
        let p = OpenAICompatProvider::new("k".into(), "gpt".into(), 4096, 0.0);
        let body = p.build_body(&[], &[]);
        assert_eq!(body["stream_options"]["include_usage"], true);
    }

    #[test]
    fn build_body_splats_tool_results_as_role_tool_messages() {
        let p = OpenAICompatProvider::new("k".into(), "gpt".into(), 4096, 0.0);
        let body = p.build_body(
            &[Message {
                role: Role::User,
                content: vec![
                    ContentBlock::ToolResult {
                        tool_use_id: "call_1".into(),
                        content: "result-a".into(),
                        is_error: false,
                    },
                    ContentBlock::ToolResult {
                        tool_use_id: "call_2".into(),
                        content: "result-b".into(),
                        is_error: false,
                    },
                ],
            }],
            &[],
        );
        let msgs = body["messages"].as_array().expect("messages");
        assert_eq!(msgs.len(), 2, "expected one role:tool message per result");
        for m in msgs {
            assert_eq!(m["role"], "tool");
            assert!(m["tool_call_id"].is_string());
        }
        assert_eq!(msgs[0]["tool_call_id"], "call_1");
        assert_eq!(msgs[1]["tool_call_id"], "call_2");
    }
}
