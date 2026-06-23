//! Integration tests for cross-session FTS search.

mod common;

use serde_json::json;

fn url(addr: std::net::SocketAddr, path: &str) -> String {
    format!("http://{addr}{path}")
}

#[tokio::test]
async fn search_happy_path_finds_match_across_sessions() {
    let h = common::start_server_default().await;
    let c = reqwest::Client::new();

    // Seed two sessions, each with a distinctive note.
    let r1 = c
        .post(url(h.addr, "/v1/sessions"))
        .bearer_auth(&h.token)
        .json(&json!({"pr_url":"https://github.com/a/b/pull/300","pr_data":{}}))
        .send()
        .await
        .unwrap();
    let s1: serde_json::Value = r1.json().await.unwrap();
    let sid1 = s1["session_id"].as_str().unwrap().to_string();

    let r2 = c
        .post(url(h.addr, "/v1/sessions"))
        .bearer_auth(&h.token)
        .json(&json!({"pr_url":"https://github.com/a/b/pull/301","pr_data":{}}))
        .send()
        .await
        .unwrap();
    let s2: serde_json::Value = r2.json().await.unwrap();
    let sid2 = s2["session_id"].as_str().unwrap().to_string();

    let _ = c
        .post(url(h.addr, &format!("/v1/sessions/{sid1}/notes")))
        .bearer_auth(&h.token)
        .json(&json!({"content":"bcryptmagicstring is used here","severity":"info"}))
        .send()
        .await
        .unwrap();
    let _ = c
        .post(url(h.addr, &format!("/v1/sessions/{sid2}/notes")))
        .bearer_auth(&h.token)
        .json(&json!({"content":"nothing related to the magic word","severity":"info"}))
        .send()
        .await
        .unwrap();

    // Search returns the seeded match and not the unrelated note.
    let r = c
        .get(url(h.addr, "/v1/search?q=bcryptmagicstring&limit=10"))
        .bearer_auth(&h.token)
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success());
    let body: serde_json::Value = r.json().await.unwrap();
    let results = body["results"].as_array().unwrap();
    assert!(!results.is_empty());
    let first = &results[0];
    assert_eq!(first["session_id"], sid1);
    assert!(first["snippet"]
        .as_str()
        .unwrap()
        .to_ascii_lowercase()
        .contains("bcryptmagicstring"));
}

#[tokio::test]
async fn search_empty_query_returns_empty() {
    let h = common::start_server_default().await;
    let c = reqwest::Client::new();
    let r = c
        .get(url(h.addr, "/v1/search?q="))
        .bearer_auth(&h.token)
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success());
    let body: serde_json::Value = r.json().await.unwrap();
    let arr = body["results"].as_array().unwrap();
    assert!(arr.is_empty());
}

#[tokio::test]
async fn search_requires_auth() {
    let h = common::start_server_default().await;
    let r = reqwest::Client::new()
        .get(url(h.addr, "/v1/search?q=foo"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
}

#[tokio::test]
async fn create_session_flags_pr_diff_changed_on_new_sha() {
    let h = common::start_server_default().await;
    let c = reqwest::Client::new();

    let r1 = c
        .post(url(h.addr, "/v1/sessions"))
        .bearer_auth(&h.token)
        .json(&json!({
            "pr_url": "https://github.com/a/b/pull/400",
            "pr_data": { "head_sha": "deadbeef0000" },
        }))
        .send()
        .await
        .unwrap();
    let b1: serde_json::Value = r1.json().await.unwrap();
    // First time we see a sha — no prior, so no diff change.
    assert_eq!(b1["pr_diff_changed"], false);

    // Same sha again — still false.
    let r2 = c
        .post(url(h.addr, "/v1/sessions"))
        .bearer_auth(&h.token)
        .json(&json!({
            "pr_url": "https://github.com/a/b/pull/400",
            "pr_data": { "head_sha": "deadbeef0000" },
        }))
        .send()
        .await
        .unwrap();
    let b2: serde_json::Value = r2.json().await.unwrap();
    assert_eq!(b2["pr_diff_changed"], false);

    // Different sha — flag flips.
    let r3 = c
        .post(url(h.addr, "/v1/sessions"))
        .bearer_auth(&h.token)
        .json(&json!({
            "pr_url": "https://github.com/a/b/pull/400",
            "pr_data": { "head_sha": "cafebabe1111" },
        }))
        .send()
        .await
        .unwrap();
    let b3: serde_json::Value = r3.json().await.unwrap();
    assert_eq!(b3["pr_diff_changed"], true);
}

#[tokio::test]
async fn ws_free_form_ask_streams_when_verb_is_absent() {
    use futures::{SinkExt, StreamExt};
    use libre_cr_common::ws_frames::ServerFrame;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::http::HeaderValue;
    use tokio_tungstenite::tungstenite::protocol::Message;

    let h = common::start_server_with_script(common::happy_two_round_script()).await;
    let c = reqwest::Client::new();
    let resp = c
        .post(url(h.addr, "/v1/sessions"))
        .bearer_auth(&h.token)
        .json(&json!({"pr_url":"https://github.com/foo/bar/pull/600","pr_data":{}}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    let sid = body["session_id"].as_str().unwrap().to_string();

    let ws_url = format!("ws://{}/v1/sessions/{}/ask", h.addr, sid);
    let mut req = ws_url.into_client_request().unwrap();
    req.headers_mut().insert(
        "authorization",
        HeaderValue::from_str(&format!("Bearer {}", h.token)).unwrap(),
    );
    let (mut ws, _resp) = tokio_tungstenite::connect_async(req).await.unwrap();
    // No `verb`, no `selection` — pure free-form ask.
    ws.send(Message::Text(
        json!({"question":"what is going on here?"}).to_string(),
    ))
    .await
    .unwrap();

    let mut got_text = false;
    let mut got_done = false;
    while let Some(msg) = ws.next().await {
        let Ok(msg) = msg else { break };
        let s = match msg {
            Message::Text(s) => s,
            Message::Close(_) => break,
            _ => continue,
        };
        let f: ServerFrame = match serde_json::from_str(&s) {
            Ok(v) => v,
            Err(_) => continue,
        };
        match f {
            ServerFrame::TextDelta { .. } => got_text = true,
            ServerFrame::Done { .. } => {
                got_done = true;
                break;
            }
            ServerFrame::Error { message, .. } => panic!("unexpected error: {message}"),
            _ => {}
        }
    }
    assert!(got_text, "free-form ask should still stream text");
    assert!(got_done, "free-form ask should still complete");
}

#[tokio::test]
async fn note_with_source_turn_id_round_trips() {
    let h = common::start_server_default().await;
    let c = reqwest::Client::new();
    let r = c
        .post(url(h.addr, "/v1/sessions"))
        .bearer_auth(&h.token)
        .json(&json!({"pr_url":"https://github.com/a/b/pull/500","pr_data":{}}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = r.json().await.unwrap();
    let sid = body["session_id"].as_str().unwrap().to_string();

    let r = c
        .post(url(h.addr, &format!("/v1/sessions/{sid}/notes")))
        .bearer_auth(&h.token)
        .json(&json!({
            "content": "Pinned from an answer",
            "severity": "warning",
            "source_turn_id": "t_source_xyz",
        }))
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "status {}", r.status());

    let r = c
        .get(url(h.addr, &format!("/v1/sessions/{sid}")))
        .bearer_auth(&h.token)
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = r.json().await.unwrap();
    let turns = body["turns"].as_array().unwrap();
    let note = turns
        .iter()
        .find(|t| t["kind"] == "note")
        .expect("note turn");
    assert_eq!(note["source_turn_id"], "t_source_xyz");
    assert_eq!(note["severity"], "warning");
}
