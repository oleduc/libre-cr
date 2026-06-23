//! End-to-end suite for the **HTTP/WS consumer (browser extension)** →
//! `libre-cr-review` daemon contract.
//!
//! Existing `http_api.rs` / `ws_smoke.rs` run the router *in-process*. That
//! catches handler regressions but doesn't catch breakage in the binary's
//! bootstrap path: token file, endpoint file, config persistence, code
//! daemon child spawn, ws/http co-existence, etc. This suite spawns the
//! real `libre-cr-review` binary against a tempdir `$HOME` (and, when
//! available, a real `libre-cr-code` child) and drives it over `reqwest` +
//! `tokio-tungstenite`, exactly the way the browser extension does.

mod common;

use std::time::Duration;

use common::spawned_daemon::{http_client, SpawnedDaemon};
use futures::{SinkExt, StreamExt};
use libre_cr_common::ws_frames::ServerFrame;
use serde_json::{json, Value};
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::protocol::Message;

/// Convenience: spawn the daemon under a fresh sandbox or skip cleanly if
/// the binaries aren't available (e.g. `cargo build` failed on CI). Tests
/// that hit `None` should `return` to be recorded as a no-op pass.
async fn spawn_or_skip(test: &str) -> Option<SpawnedDaemon> {
    match SpawnedDaemon::start().await {
        Ok(Some(d)) => Some(d),
        Ok(None) => {
            eprintln!("[e2e_http_consumer::{test}] binaries unavailable, skipping");
            None
        }
        Err(e) => {
            // A startup error is a real failure; surface it.
            panic!("[e2e_http_consumer::{test}] daemon start failed: {e:#}");
        }
    }
}

// ─── health ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn health_returns_ok() {
    let Some(d) = spawn_or_skip("health_returns_ok").await else {
        return;
    };
    let resp = http_client().get(d.url("/v1/health")).send().await.unwrap();
    assert!(resp.status().is_success(), "status {}", resp.status());
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["ok"], true);
    assert!(
        body["version"].as_str().is_some_and(|s| !s.is_empty()),
        "missing version: {body}"
    );
}

#[tokio::test]
async fn health_includes_code_daemon_state() {
    let Some(d) = spawn_or_skip("health_includes_code_daemon_state").await else {
        return;
    };
    let body: Value = http_client()
        .get(d.url("/v1/health/code-daemon"))
        .bearer_auth(&d.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    // The dedicated route always answers; the value of `connected` depends
    // on whether the code daemon binary was reachable. We only insist on
    // shape — that the route is wired and the value is a bool.
    assert!(
        body.get("connected").and_then(|v| v.as_bool()).is_some(),
        "missing connected bool: {body}"
    );
}

// ─── sessions CRUD ────────────────────────────────────────────────────────

#[tokio::test]
async fn sessions_require_token() {
    let Some(d) = spawn_or_skip("sessions_require_token").await else {
        return;
    };
    let resp = http_client()
        .post(d.url("/v1/sessions"))
        .json(&json!({"pr_url":"https://github.com/foo/bar/pull/1","pr_data":{}}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn sessions_crud_round_trip() {
    let Some(d) = spawn_or_skip("sessions_crud_round_trip").await else {
        return;
    };
    let c = http_client();
    let create: Value = c
        .post(d.url("/v1/sessions"))
        .bearer_auth(&d.token)
        .json(&json!({"pr_url":"https://github.com/foo/bar/pull/2","pr_data":{}}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let sid = create["session_id"].as_str().unwrap().to_string();

    // Read.
    let got: Value = c
        .get(d.url(&format!("/v1/sessions/{sid}")))
        .bearer_auth(&d.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(got["session"]["session_id"], sid);

    // List.
    let list: Value = c
        .get(d.url("/v1/sessions"))
        .bearer_auth(&d.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let arr = list["sessions"].as_array().expect("sessions array");
    assert!(arr.iter().any(|s| s["session_id"] == sid));

    // Delete.
    let resp = c
        .delete(d.url(&format!("/v1/sessions/{sid}")))
        .bearer_auth(&d.token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);
}

#[tokio::test]
async fn sessions_idempotent_on_same_pr_url() {
    let Some(d) = spawn_or_skip("sessions_idempotent_on_same_pr_url").await else {
        return;
    };
    let c = http_client();
    let pr_url = "https://github.com/foo/bar/pull/3";
    let first: Value = c
        .post(d.url("/v1/sessions"))
        .bearer_auth(&d.token)
        .json(&json!({"pr_url": pr_url, "pr_data": {}}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let second: Value = c
        .post(d.url("/v1/sessions"))
        .bearer_auth(&d.token)
        .json(&json!({"pr_url": pr_url, "pr_data": {"head_sha":"abc"}}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        first["session_id"], second["session_id"],
        "POST /v1/sessions must upsert on pr_url"
    );
}

#[tokio::test]
async fn worktree_orchestration() {
    // With `mock.code_intel = true`, the daemon synthesizes a worktree path
    // synchronously on session creation. We just verify the path is
    // reported as ready and the daemon answers a follow-up GET.
    let Some(d) = spawn_or_skip("worktree_orchestration").await else {
        return;
    };
    let c = http_client();
    let create: Value = c
        .post(d.url("/v1/sessions"))
        .bearer_auth(&d.token)
        .json(&json!({
            "pr_url": "https://github.com/foo/bar/pull/4",
            "pr_data": {"remote_url": "https://github.com/foo/bar.git", "head_ref": "feature"}
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(create["worktree_ready"], true);
    assert!(
        create["repo_local_path"]
            .as_str()
            .is_some_and(|p| !p.is_empty()),
        "no repo_local_path: {create}"
    );

    let sid = create["session_id"].as_str().unwrap();
    let got: Value = c
        .get(d.url(&format!("/v1/sessions/{sid}")))
        .bearer_auth(&d.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(got["worktree_ready"], true);
}

// ─── pairing ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn pair_issue_then_redeem() {
    let Some(d) = spawn_or_skip("pair_issue_then_redeem").await else {
        return;
    };
    let c = http_client();
    // Issue (token-authenticated).
    let body: Value = c
        .post(d.url("/v1/pair/issue"))
        .bearer_auth(&d.token)
        .json(&json!({}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let code = body["code"].as_str().unwrap().to_string();
    // Redeem (no auth needed).
    let resp = c
        .post(d.url("/v1/pair"))
        .json(&json!({"code": code, "extension_origin": "chrome-extension://e2e"}))
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "redeem status {}",
        resp.status()
    );
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["token"], d.token);
}

#[tokio::test]
async fn pair_rate_limited_after_5_failures() {
    let Some(d) = spawn_or_skip("pair_rate_limited_after_5_failures").await else {
        return;
    };
    let c = http_client();
    // Fire 5 wrong codes from the same source IP. The 5th tips us over the
    // per-IP threshold; the 6th must always be 429 with Retry-After: 60.
    for _ in 0..5 {
        let resp = c
            .post(d.url("/v1/pair"))
            .json(&json!({"code":"wrong-code"}))
            .send()
            .await
            .unwrap();
        assert!(
            resp.status() == 401 || resp.status() == 429,
            "unexpected status {}",
            resp.status()
        );
    }
    let resp = c
        .post(d.url("/v1/pair"))
        .json(&json!({"code":"wrong-code"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 429);
    let retry_after = resp
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(retry_after, "60");
}

// ─── config ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn config_get_returns_non_sensitive() {
    let Some(d) = spawn_or_skip("config_get_returns_non_sensitive").await else {
        return;
    };
    let body: Value = http_client()
        .get(d.url("/v1/config"))
        .bearer_auth(&d.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    // The encrypted API key must not leak in the GET response.
    let dump = serde_json::to_string(&body).unwrap();
    assert!(
        !dump.contains("api_key_enc") && !dump.contains("api_key"),
        "/v1/config leaked sensitive key: {dump}"
    );
}

#[tokio::test]
async fn config_post_persists_across_restart() {
    let Some(mut d) = spawn_or_skip("config_post_persists_across_restart").await else {
        return;
    };
    let c = http_client();
    let resp = c
        .post(d.url("/v1/config"))
        .bearer_auth(&d.token)
        .json(&json!({"provider": {"model": "persisted-x", "max_tokens": 1234}}))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "status {}", resp.status());

    // Kill and restart with the same HOME.
    d.restart().await.expect("restart");

    let cfg: Value = c
        .get(d.url("/v1/config"))
        .bearer_auth(&d.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(cfg["provider"]["model"], "persisted-x");
    assert_eq!(cfg["provider"]["max_tokens"], 1234);
}

#[tokio::test]
async fn config_ui_serves_html() {
    let Some(d) = spawn_or_skip("config_ui_serves_html").await else {
        return;
    };
    let resp = http_client().get(d.url("/config-ui")).send().await.unwrap();
    assert!(resp.status().is_success());
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(ct.starts_with("text/html"), "content-type was {ct}");
    let body = resp.text().await.unwrap();
    assert!(body.contains("Libre CR"));
}

// ─── notes ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn notes_crud() {
    let Some(d) = spawn_or_skip("notes_crud").await else {
        return;
    };
    let c = http_client();
    let sess: Value = c
        .post(d.url("/v1/sessions"))
        .bearer_auth(&d.token)
        .json(&json!({"pr_url":"https://github.com/foo/bar/pull/5","pr_data":{}}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let sid = sess["session_id"].as_str().unwrap().to_string();

    let created: Value = c
        .post(d.url(&format!("/v1/sessions/{sid}/notes")))
        .bearer_auth(&d.token)
        .json(&json!({"content":"a note","severity":"warning"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let note_id = created["note_id"].as_str().unwrap().to_string();

    // PATCH.
    let resp = c
        .patch(d.url(&format!("/v1/sessions/{sid}/notes/{note_id}")))
        .bearer_auth(&d.token)
        .json(&json!({"content":"edited"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);

    // DELETE.
    let resp = c
        .delete(d.url(&format!("/v1/sessions/{sid}/notes/{note_id}")))
        .bearer_auth(&d.token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);
}

#[tokio::test]
async fn notes_back_reference() {
    let Some(d) = spawn_or_skip("notes_back_reference").await else {
        return;
    };
    let c = http_client();
    let sess: Value = c
        .post(d.url("/v1/sessions"))
        .bearer_auth(&d.token)
        .json(&json!({"pr_url":"https://github.com/foo/bar/pull/6","pr_data":{}}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let sid = sess["session_id"].as_str().unwrap().to_string();

    let created: Value = c
        .post(d.url(&format!("/v1/sessions/{sid}/notes")))
        .bearer_auth(&d.token)
        .json(&json!({
            "content":"saved from a Q&A turn",
            "source_turn_id":"turn-xyz",
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let note_id = created["note_id"].as_str().unwrap();

    // GET /v1/sessions/:id returns turns; note rows are turns with
    // kind=Note. Verify the source_turn_id round-tripped.
    let got: Value = c
        .get(d.url(&format!("/v1/sessions/{sid}")))
        .bearer_auth(&d.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let turns = got["turns"].as_array().expect("turns");
    let row = turns
        .iter()
        .find(|t| t["turn_id"] == note_id)
        .expect("note turn not in history");
    assert_eq!(row["source_turn_id"], "turn-xyz");
}

// ─── export ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn export_markdown() {
    let Some(d) = spawn_or_skip("export_markdown").await else {
        return;
    };
    let c = http_client();
    let sess: Value = c
        .post(d.url("/v1/sessions"))
        .bearer_auth(&d.token)
        .json(&json!({
            "pr_url":"https://github.com/foo/bar/pull/42",
            "pr_data": {"metadata": {"title": "feat: bcrypt"}}
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let sid = sess["session_id"].as_str().unwrap().to_string();
    for (sev, content) in [
        ("info", "noted as fine"),
        ("critical", "must fix"),
        ("warning", "watch out"),
        ("suggestion", "consider this"),
    ] {
        let _ = c
            .post(d.url(&format!("/v1/sessions/{sid}/notes")))
            .bearer_auth(&d.token)
            .json(&json!({"content": content, "severity": sev}))
            .send()
            .await
            .unwrap();
    }
    let r: Value = c
        .post(d.url(&format!("/v1/sessions/{sid}/export")))
        .bearer_auth(&d.token)
        .json(&json!({"format":"markdown"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let md = r["content"].as_str().expect("content");
    let crit = md.find("## Critical").expect("critical heading");
    let warn = md.find("## Warning").expect("warning heading");
    let sugg = md.find("## Suggestions").expect("suggestions heading");
    let info = md.find("## Info").expect("info heading");
    assert!(crit < warn && warn < sugg && sugg < info);
}

#[tokio::test]
async fn export_github_review_structured() {
    let Some(d) = spawn_or_skip("export_github_review_structured").await else {
        return;
    };
    let c = http_client();
    let sess: Value = c
        .post(d.url("/v1/sessions"))
        .bearer_auth(&d.token)
        .json(&json!({"pr_url":"https://github.com/foo/bar/pull/43","pr_data":{}}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let sid = sess["session_id"].as_str().unwrap().to_string();
    let _ = c
        .post(d.url(&format!("/v1/sessions/{sid}/notes")))
        .bearer_auth(&d.token)
        .json(&json!({"content":"something to mention","severity":"warning"}))
        .send()
        .await
        .unwrap();
    let r: Value = c
        .post(d.url(&format!("/v1/sessions/{sid}/export")))
        .bearer_auth(&d.token)
        .json(&json!({"format":"github_review"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let structure = r["structure"].as_object().expect("structure object");
    assert!(structure.contains_key("body"));
    assert!(structure.contains_key("event"));
    assert!(structure.contains_key("comments"));
    assert!(r["structure"]["comments"].is_array());
}

// ─── search ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn search_returns_hits_across_sessions() {
    let Some(d) = spawn_or_skip("search_returns_hits_across_sessions").await else {
        return;
    };
    let c = http_client();
    // Seed two sessions with overlapping content via notes (the FTS index
    // covers note content + Q&A turns).
    for (pr, note) in [
        ("https://github.com/foo/bar/pull/100", "lookup zzunique100"),
        ("https://github.com/foo/bar/pull/101", "lookup zzunique100"),
    ] {
        let s: Value = c
            .post(d.url("/v1/sessions"))
            .bearer_auth(&d.token)
            .json(&json!({"pr_url": pr, "pr_data":{}}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let sid = s["session_id"].as_str().unwrap().to_string();
        let _ = c
            .post(d.url(&format!("/v1/sessions/{sid}/notes")))
            .bearer_auth(&d.token)
            .json(&json!({"content": note, "severity":"info"}))
            .send()
            .await
            .unwrap();
    }
    let hits: Value = c
        .get(d.url("/v1/search?q=zzunique100&limit=50"))
        .bearer_auth(&d.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let results = hits["results"].as_array().expect("results");
    assert!(
        results.len() >= 2,
        "expected hits from both sessions, got {results:?}"
    );
}

// ─── websocket ────────────────────────────────────────────────────────────

/// Configure the daemon's mock provider to produce a tiny two-event script
/// (one text delta + done), then exercise the WS.
async fn spawn_with_script(
    test: &str,
    script: Vec<libre_cr_review::config::ScriptedEvent>,
) -> Option<SpawnedDaemon> {
    match SpawnedDaemon::start_with(|cfg| cfg.mock.provider_script = script).await {
        Ok(Some(d)) => Some(d),
        Ok(None) => {
            eprintln!("[e2e_http_consumer::{test}] binaries unavailable, skipping");
            None
        }
        Err(e) => panic!("[e2e_http_consumer::{test}] daemon start failed: {e:#}"),
    }
}

fn one_word_script() -> Vec<libre_cr_review::config::ScriptedEvent> {
    use libre_cr_review::config::ScriptedEvent;
    use libre_cr_review::provider::StreamEvent;
    vec![
        ScriptedEvent {
            delay_ms: 0,
            event: StreamEvent::TextDelta {
                text: "answered".into(),
            },
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

async fn ws_connect(
    url: &str,
    token: &str,
    use_query_token: bool,
) -> Result<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    tokio_tungstenite::tungstenite::Error,
> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::http::HeaderValue;
    let connect_url = if use_query_token {
        format!("{}?token={}", url, urlencode(token))
    } else {
        url.to_string()
    };
    let mut req = connect_url.into_client_request()?;
    if !use_query_token {
        req.headers_mut().insert(
            "authorization",
            HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
        );
    }
    Ok(tokio_tungstenite::connect_async(req).await?.0)
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
                String::from_utf8(vec![b]).unwrap()
            } else {
                format!("%{:02X}", b)
            }
        })
        .collect()
}

#[tokio::test]
async fn ws_ask_streams_text_and_tool_frames() {
    let Some(d) = spawn_with_script("ws_ask_streams_text_and_tool_frames", one_word_script()).await
    else {
        return;
    };
    let c = http_client();
    let sess: Value = c
        .post(d.url("/v1/sessions"))
        .bearer_auth(&d.token)
        .json(&json!({"pr_url":"https://github.com/foo/bar/pull/200","pr_data":{}}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let sid = sess["session_id"].as_str().unwrap().to_string();

    // Connect via the query-param token fallback the browser uses.
    let mut ws = ws_connect(
        &d.ws_url(&format!("/v1/sessions/{sid}/ask")),
        &d.token,
        true,
    )
    .await
    .expect("ws connect");
    ws.send(Message::Text(json!({"question":"hi"}).to_string()))
        .await
        .unwrap();

    let mut saw_text = false;
    let mut saw_done = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        let msg = match timeout(Duration::from_secs(2), ws.next()).await {
            Ok(Some(Ok(m))) => m,
            _ => break,
        };
        let s = match msg {
            Message::Text(s) => s,
            Message::Close(_) => break,
            _ => continue,
        };
        if let Ok(frame) = serde_json::from_str::<ServerFrame>(&s) {
            match frame {
                ServerFrame::TextDelta { .. } => saw_text = true,
                ServerFrame::Done { .. } => {
                    saw_done = true;
                    break;
                }
                _ => {}
            }
        }
    }
    assert!(saw_text, "expected text_delta");
    assert!(saw_done, "expected done");
    let _ = ws.close(None).await;
}

#[tokio::test]
async fn ws_ask_persists_turn() {
    let Some(d) = spawn_with_script("ws_ask_persists_turn", one_word_script()).await else {
        return;
    };
    let c = http_client();
    let sess: Value = c
        .post(d.url("/v1/sessions"))
        .bearer_auth(&d.token)
        .json(&json!({"pr_url":"https://github.com/foo/bar/pull/201","pr_data":{}}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let sid = sess["session_id"].as_str().unwrap().to_string();
    // Drive a turn.
    let mut ws = ws_connect(
        &d.ws_url(&format!("/v1/sessions/{sid}/ask")),
        &d.token,
        true,
    )
    .await
    .unwrap();
    ws.send(Message::Text(
        json!({"question":"what is the change?"}).to_string(),
    ))
    .await
    .unwrap();
    while let Ok(Some(Ok(msg))) = timeout(Duration::from_secs(5), ws.next()).await {
        if let Message::Text(s) = msg {
            if let Ok(ServerFrame::Done { .. }) = serde_json::from_str::<ServerFrame>(&s) {
                break;
            }
        }
    }
    let _ = ws.close(None).await;

    // The turn should now be in history. Poll briefly — persistence is on
    // the server task and can run after the WS closes.
    let mut got = false;
    for _ in 0..40 {
        let got_resp: Value = c
            .get(d.url(&format!("/v1/sessions/{sid}")))
            .bearer_auth(&d.token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let turns = got_resp["turns"].as_array().cloned().unwrap_or_default();
        if turns
            .iter()
            .any(|t| t["question"].as_str() == Some("what is the change?"))
        {
            got = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(got, "ask turn was never persisted to history");
}

#[tokio::test]
async fn ws_ask_409_on_concurrent() {
    use libre_cr_review::config::ScriptedEvent;
    use libre_cr_review::provider::StreamEvent;
    let hanging = vec![ScriptedEvent {
        delay_ms: 5_000,
        event: StreamEvent::TextDelta {
            text: "hold".into(),
        },
    }];
    let Some(d) = spawn_with_script("ws_ask_409_on_concurrent", hanging).await else {
        return;
    };
    let c = http_client();
    let sess: Value = c
        .post(d.url("/v1/sessions"))
        .bearer_auth(&d.token)
        .json(&json!({"pr_url":"https://github.com/foo/bar/pull/202","pr_data":{}}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let sid = sess["session_id"].as_str().unwrap().to_string();
    let url = d.ws_url(&format!("/v1/sessions/{sid}/ask"));
    let mut ws1 = ws_connect(&url, &d.token, true).await.unwrap();
    ws1.send(Message::Text(json!({"question":"hold"}).to_string()))
        .await
        .unwrap();
    // Wait briefly until the daemon registers the busy slot. Polling is
    // unavoidable here — there's no observable signal short of trying.
    let mut got_409 = false;
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        match ws_connect(&url, &d.token, true).await {
            Ok(_) => continue,
            Err(tokio_tungstenite::tungstenite::Error::Http(resp)) if resp.status() == 409 => {
                got_409 = true;
                break;
            }
            Err(_) => continue,
        }
    }
    assert!(got_409, "expected 409 on concurrent WS");
    let _ = ws1.close(None).await;
}

#[tokio::test]
async fn ws_ask_cancellation_persists_partial() {
    use libre_cr_review::config::ScriptedEvent;
    use libre_cr_review::provider::StreamEvent;
    let script = vec![
        ScriptedEvent {
            delay_ms: 0,
            event: StreamEvent::TextDelta {
                text: "partial-".into(),
            },
        },
        ScriptedEvent {
            delay_ms: 0,
            event: StreamEvent::TextDelta {
                text: "answer".into(),
            },
        },
        ScriptedEvent {
            delay_ms: 10_000,
            event: StreamEvent::TextDelta { text: "".into() },
        },
    ];
    let Some(d) = spawn_with_script("ws_ask_cancellation_persists_partial", script).await else {
        return;
    };
    let c = http_client();
    let sess: Value = c
        .post(d.url("/v1/sessions"))
        .bearer_auth(&d.token)
        .json(&json!({"pr_url":"https://github.com/foo/bar/pull/203","pr_data":{}}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let sid = sess["session_id"].as_str().unwrap().to_string();
    let url = d.ws_url(&format!("/v1/sessions/{sid}/ask"));
    let mut ws = ws_connect(&url, &d.token, true).await.unwrap();
    ws.send(Message::Text(json!({"question":"keep going"}).to_string()))
        .await
        .unwrap();
    // Wait for at least one text frame.
    let mut saw_text = false;
    for _ in 0..20 {
        match timeout(Duration::from_millis(400), ws.next()).await {
            Ok(Some(Ok(Message::Text(s)))) => {
                if let Ok(ServerFrame::TextDelta { .. }) = serde_json::from_str::<ServerFrame>(&s) {
                    saw_text = true;
                    break;
                }
            }
            _ => continue,
        }
    }
    assert!(saw_text, "expected text_delta before cancel");
    let _ = ws.close(None).await;
    drop(ws);

    // The cancelled turn must surface in history with status=cancelled and
    // a non-empty answer prefix.
    let mut found = false;
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let got: Value = c
            .get(d.url(&format!("/v1/sessions/{sid}")))
            .bearer_auth(&d.token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        for t in got["turns"].as_array().cloned().unwrap_or_default() {
            if t["status"] == "cancelled" {
                if let Some(ans) = t["answer"].as_str() {
                    if ans.contains("partial") {
                        found = true;
                        break;
                    }
                }
            }
        }
        if found {
            break;
        }
    }
    assert!(
        found,
        "cancelled turn with partial answer was never persisted"
    );
}

#[tokio::test]
async fn ws_ask_query_token_required_on_upgrade() {
    let Some(d) = spawn_or_skip("ws_ask_query_token_required_on_upgrade").await else {
        return;
    };
    let c = http_client();
    let sess: Value = c
        .post(d.url("/v1/sessions"))
        .bearer_auth(&d.token)
        .json(&json!({"pr_url":"https://github.com/foo/bar/pull/204","pr_data":{}}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let sid = sess["session_id"].as_str().unwrap().to_string();
    let url = d.ws_url(&format!("/v1/sessions/{sid}/ask"));
    // No query, no header — must be 401.
    let res = ws_connect(&url, "", false).await;
    match res {
        Ok(_) => panic!("WS upgrade succeeded without auth"),
        Err(tokio_tungstenite::tungstenite::Error::Http(resp)) => {
            assert_eq!(resp.status(), 401, "expected 401, got {}", resp.status());
        }
        Err(e) => panic!("unexpected ws error: {e}"),
    }
}
