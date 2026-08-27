//! Integration coverage for the round-2 certification fixes:
//! RC1 (provider hot-reload), N3 (origin persistence + dynamic CORS),
//! N4 (busy-slot RAII release), E1 (mute_presentations), and the
//! post_config error-propagation fix.

mod common;

use std::sync::Arc;

use futures::{stream::BoxStream, SinkExt, StreamExt};
use libre_cr_common::ws_frames::ServerFrame;
use libre_cr_review::config::{Config, ScriptedEvent};
use libre_cr_review::provider::{MockProvider, Provider, StreamEvent, ToolSchema};
use serde_json::json;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::protocol::Message;

fn url(addr: std::net::SocketAddr, path: &str) -> String {
    format!("http://{addr}{path}")
}

fn text_script(text: &str) -> Vec<ScriptedEvent> {
    vec![
        ScriptedEvent {
            delay_ms: 0,
            event: StreamEvent::TextDelta { text: text.into() },
        },
        ScriptedEvent {
            delay_ms: 0,
            event: StreamEvent::Done {
                input_tokens: 1,
                output_tokens: 1,
                stop_reason: "end_turn".into(),
            },
        },
    ]
}

async fn create_session(h: &common::Harness, pr: u64) -> String {
    let resp = reqwest::Client::new()
        .post(url(h.addr, "/v1/sessions"))
        .bearer_auth(&h.token)
        .json(&json!({"pr_url": format!("https://github.com/a/b/pull/{pr}"), "pr_data": {}}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    body["session_id"].as_str().unwrap().to_string()
}

async fn ws_connect(
    h: &common::Harness,
    sid: &str,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let ws_url = format!("ws://{}/v1/sessions/{}/ask", h.addr, sid);
    let mut req = ws_url.into_client_request().unwrap();
    req.headers_mut().insert(
        "authorization",
        HeaderValue::from_str(&format!("Bearer {}", h.token)).unwrap(),
    );
    let (ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    ws
}

/// Run one ask over the WS and collect the streamed text until `done`.
async fn ask_and_collect_text(h: &common::Harness, sid: &str, init: serde_json::Value) -> String {
    let mut ws = ws_connect(h, sid).await;
    ws.send(Message::Text(init.to_string())).await.unwrap();
    let mut text = String::new();
    while let Some(msg) = ws.next().await {
        let Ok(Message::Text(s)) = msg else {
            break;
        };
        match serde_json::from_str::<ServerFrame>(&s) {
            Ok(ServerFrame::TextDelta { text: t }) => text.push_str(&t),
            Ok(ServerFrame::Done { .. }) => break,
            Ok(ServerFrame::Error { message, .. }) => panic!("error frame: {message}"),
            _ => continue,
        }
    }
    text
}

#[tokio::test]
async fn config_post_swaps_running_provider() {
    // RC1: the stored config scripts "from-new-provider"; the startup
    // provider scripts "from-old-provider". After POST /v1/config the ask
    // must stream the *stored* config's script.
    let mut cfg = Config::default();
    cfg.mock.code_intel = true;
    cfg.provider.kind = "mock".into();
    cfg.mock.provider_script = text_script("from-new-provider");
    let startup_provider = Arc::new(MockProvider::new(text_script("from-old-provider")));
    let h = common::start_server_with(cfg, startup_provider, None).await;
    let sid = create_session(&h, 9001).await;

    let resp = reqwest::Client::new()
        .post(url(h.addr, "/v1/config"))
        .bearer_auth(&h.token)
        .json(&json!({"provider": {"kind": "mock"}}))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "status {}", resp.status());

    let text = ask_and_collect_text(&h, &sid, json!({"question": "hi"})).await;
    assert!(
        text.contains("from-new-provider"),
        "expected the rebuilt provider's script, got {text:?}"
    );
    assert!(!text.contains("from-old-provider"));
}

#[tokio::test]
async fn validate_uses_current_or_candidate_config_not_startup_snapshot() {
    let h = common::start_server_default().await;
    let c = reqwest::Client::new();

    // Stored config is the mock provider → validates fine.
    let resp = c
        .post(url(h.addr, "/v1/config/validate"))
        .bearer_auth(&h.token)
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "status {}", resp.status());

    // Candidate in the body: anthropic with no key → provider_unauthorized,
    // without mutating the stored config.
    let resp = c
        .post(url(h.addr, "/v1/config/validate"))
        .bearer_auth(&h.token)
        .json(&json!({"provider": {"kind": "anthropic", "model": "claude-x"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 502, "anthropic with empty key must fail");

    // Now actually switch the stored config to anthropic (no key) — a
    // body-less validate must check the *new* config, not the startup mock.
    let resp = c
        .post(url(h.addr, "/v1/config"))
        .bearer_auth(&h.token)
        .json(&json!({"provider": {"kind": "anthropic", "model": "claude-x"}}))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());
    let resp = c
        .post(url(h.addr, "/v1/config/validate"))
        .bearer_auth(&h.token)
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        502,
        "validate must reflect the stored config change"
    );
}

#[tokio::test]
async fn config_post_rejects_unknown_provider_kind() {
    let h = common::start_server_default().await;
    let resp = reqwest::Client::new()
        .post(url(h.addr, "/v1/config"))
        .bearer_auth(&h.token)
        .json(&json!({"provider": {"kind": "definitely-not-a-provider"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    // The bad kind must not have been committed.
    let cfg: serde_json::Value = reqwest::Client::new()
        .get(url(h.addr, "/v1/config"))
        .bearer_auth(&h.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(cfg["provider"]["kind"], "mock");
}

#[tokio::test]
async fn pair_persists_origin_and_cors_is_permissive() {
    // N3 (revised): the origin learned via /v1/pair still lands in review.toml
    // for diagnostics, but CORS is `*` — the content script's fetches carry
    // the page origin under MV3, so an allowlist can't work; the bearer token
    // is the boundary.
    let tmp = tempfile::tempdir().unwrap();
    let cfg_path = tmp.path().join("review.toml");
    let mut cfg = Config::default();
    cfg.mock.code_intel = true;
    cfg.save(&cfg_path).unwrap();
    let h = common::start_server_with(
        Config::load(&cfg_path).unwrap(),
        Arc::new(MockProvider::new(vec![])),
        Some(cfg_path.clone()),
    )
    .await;
    let c = reqwest::Client::new();
    let origin = "chrome-extension://abcdefgh";

    let code = h.pairing.issue().await;
    let resp = c
        .post(url(h.addr, "/v1/pair"))
        .json(&json!({"code": code, "extension_origin": origin}))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());

    // Any origin — the page origin a content script actually sends included —
    // gets a wildcard allowance.
    for o in [origin, "https://github.com"] {
        let resp = c
            .get(url(h.addr, "/v1/health"))
            .header("Origin", o)
            .send()
            .await
            .unwrap();
        let acao = resp
            .headers()
            .get("access-control-allow-origin")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert_eq!(acao, "*", "CORS must be permissive for {o}");
    }

    // And the origin survives on disk.
    let reloaded = Config::load(&cfg_path).unwrap();
    assert_eq!(reloaded.server.extension_origin, origin);
}

#[tokio::test]
async fn config_post_returns_500_when_persist_fails() {
    // The config path's parent is a *file*, so the atomic write cannot
    // create the directory — the API must say so, not return ok:true.
    let tmp = tempfile::tempdir().unwrap();
    let blocker = tmp.path().join("blocker");
    std::fs::write(&blocker, "i am a file").unwrap();
    let mut cfg = Config::default();
    cfg.mock.code_intel = true;
    let h = common::start_server_with(
        cfg,
        Arc::new(MockProvider::new(vec![])),
        Some(blocker.join("review.toml")),
    )
    .await;
    let resp = reqwest::Client::new()
        .post(url(h.addr, "/v1/config"))
        .bearer_auth(&h.token)
        .json(&json!({"provider": {"model": "new-model"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 500);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body.get("error").is_some(),
        "expected an ErrorEnvelope, got {body}"
    );
    // Nothing was committed in memory either.
    let cfg: serde_json::Value = reqwest::Client::new()
        .get(url(h.addr, "/v1/config"))
        .bearer_auth(&h.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_ne!(cfg["provider"]["model"], "new-model");
}

#[tokio::test]
async fn busy_slot_released_after_failed_init_frame() {
    // N4: a handler that errors right after claiming the busy slot (here:
    // unparseable AskInit) must release it so the next ask succeeds.
    let h = common::start_server_with_script(text_script("second-ask-works")).await;
    let sid = create_session(&h, 9002).await;

    let mut ws = ws_connect(&h, &sid).await;
    ws.send(Message::Text("this is not json".into()))
        .await
        .unwrap();
    // Drain until the server closes on us.
    while let Some(Ok(msg)) = ws.next().await {
        if matches!(msg, Message::Close(_)) {
            break;
        }
    }
    drop(ws);

    // Second ask on the same session must not 409 and must complete.
    let text = ask_and_collect_text(&h, &sid, json!({"question": "again"})).await;
    assert!(text.contains("second-ask-works"));
}

/// Provider that records the tool list offered on each `stream` call and
/// replays a one-shot text script.
struct ToolRecordingProvider {
    tools_seen: Arc<tokio::sync::Mutex<Vec<Vec<String>>>>,
}

#[async_trait::async_trait]
impl Provider for ToolRecordingProvider {
    fn id(&self) -> &str {
        "tool-recorder"
    }
    async fn stream(
        &self,
        _messages: &[libre_cr_review::provider::Message],
        tools: &[ToolSchema],
    ) -> libre_cr_review::error::Result<
        BoxStream<'static, libre_cr_review::error::Result<StreamEvent>>,
    > {
        self.tools_seen
            .lock()
            .await
            .push(tools.iter().map(|t| t.name.clone()).collect());
        let events = vec![
            Ok(StreamEvent::TextDelta { text: "ok".into() }),
            Ok(StreamEvent::Done {
                input_tokens: 0,
                output_tokens: 0,
                stop_reason: "end_turn".into(),
            }),
        ];
        Ok(futures::stream::iter(events).boxed())
    }
    async fn validate(&self) -> libre_cr_review::error::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn mute_presentations_excludes_presentation_tools() {
    // E1: with mute_presentations the provider must not be offered any
    // presentation tools; without it, they're present.
    const PRESENTATION: &[&str] = &[
        "highlight_lines",
        "annotate_line",
        "scroll_to",
        "open_link",
        "clear_presentation",
    ];
    let tools_seen = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let provider = Arc::new(ToolRecordingProvider {
        tools_seen: tools_seen.clone(),
    });
    let h = common::start_server_with_provider(provider).await;
    let sid = create_session(&h, 9003).await;

    let _ = ask_and_collect_text(
        &h,
        &sid,
        json!({"question": "muted", "mute_presentations": true}),
    )
    .await;
    let _ = ask_and_collect_text(&h, &sid, json!({"question": "unmuted"})).await;

    let seen = tools_seen.lock().await.clone();
    assert_eq!(seen.len(), 2, "expected two recorded turns");
    let muted = &seen[0];
    let unmuted = &seen[1];
    for p in PRESENTATION {
        assert!(
            !muted.iter().any(|t| t == p),
            "muted turn offered presentation tool {p}: {muted:?}"
        );
        assert!(
            unmuted.iter().any(|t| t == p),
            "unmuted turn missing presentation tool {p}: {unmuted:?}"
        );
    }
}
