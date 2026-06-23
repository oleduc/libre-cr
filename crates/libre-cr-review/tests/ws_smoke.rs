//! End-to-end WS smoke test against the mock provider.

mod common;

use futures::{SinkExt, StreamExt};
use libre_cr_common::ws_frames::ServerFrame;
use serde_json::json;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::protocol::Message;

#[tokio::test]
async fn ws_streams_text_and_tool_frames() {
    let h = common::start_server_with_script(common::happy_two_round_script()).await;
    let c = reqwest::Client::new();
    let resp = c
        .post(format!("http://{}/v1/sessions", h.addr))
        .bearer_auth(&h.token)
        .json(&json!({"pr_url":"https://github.com/foo/bar/pull/7","pr_data":{}}))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());
    let body: serde_json::Value = resp.json().await.unwrap();
    let sid = body["session_id"].as_str().unwrap().to_string();

    // Build WS request
    let url = format!("ws://{}/v1/sessions/{}/ask", h.addr, sid);
    let mut req = url.into_client_request().unwrap();
    req.headers_mut().insert(
        "authorization",
        HeaderValue::from_str(&format!("Bearer {}", h.token)).unwrap(),
    );

    let (mut ws, _resp) = tokio_tungstenite::connect_async(req).await.unwrap();
    // Send init
    ws.send(Message::Text(
        json!({"question":"where is bcryptHash used?"}).to_string(),
    ))
    .await
    .unwrap();

    let mut got_done = false;
    let mut got_tool_call = false;
    let mut got_tool_result = false;
    let mut got_text = false;
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
            ServerFrame::ToolCall { name, .. } => {
                got_tool_call = true;
                assert_eq!(name, "grep");
            }
            ServerFrame::ToolResult { .. } => got_tool_result = true,
            ServerFrame::Done { .. } => {
                got_done = true;
                break;
            }
            ServerFrame::Error { message, .. } => panic!("unexpected error frame: {message}"),
            _ => {}
        }
    }
    assert!(got_text, "expected text_delta");
    assert!(got_tool_call, "expected tool_call");
    assert!(got_tool_result, "expected tool_result");
    assert!(got_done, "expected done");
}

#[tokio::test]
async fn ws_cancelled_turn_is_persisted_with_partial_answer() {
    // I3: dropping the WS mid-stream must persist a turn row marked
    // `cancelled` so the session history doesn't silently lose work.
    // The mock provider here streams a few text deltas slowly and never
    // emits Done — perfect for racing the client close against the agent.
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
        // Then "stall" by waiting forever — never emit Done.
        ScriptedEvent {
            delay_ms: 10_000,
            event: StreamEvent::TextDelta { text: "".into() },
        },
    ];
    let h = common::start_server_with_script(script).await;
    let c = reqwest::Client::new();
    let resp = c
        .post(format!("http://{}/v1/sessions", h.addr))
        .bearer_auth(&h.token)
        .json(&json!({"pr_url":"https://github.com/foo/bar/pull/42","pr_data":{}}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    let sid = body["session_id"].as_str().unwrap().to_string();

    let url = format!("ws://{}/v1/sessions/{}/ask", h.addr, sid);
    let mut req = url.into_client_request().unwrap();
    req.headers_mut().insert(
        "authorization",
        HeaderValue::from_str(&format!("Bearer {}", h.token)).unwrap(),
    );
    let (mut ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    ws.send(Message::Text(json!({"question":"hi"}).to_string()))
        .await
        .unwrap();

    // Read at least one text frame so we know the server is streaming.
    let mut saw_text = false;
    for _ in 0..10 {
        match tokio::time::timeout(std::time::Duration::from_millis(500), ws.next()).await {
            Ok(Some(Ok(Message::Text(s)))) => {
                if let Ok(ServerFrame::TextDelta { .. }) = serde_json::from_str::<ServerFrame>(&s) {
                    saw_text = true;
                    break;
                }
            }
            _ => continue,
        }
    }
    assert!(saw_text, "expected at least one text_delta before cancel");

    // Drop the connection — this is what the disconnect handler must
    // detect and persist as cancelled.
    let _ = ws.close(None).await;
    drop(ws);

    // Poll the store for the cancelled turn (the persist happens on the
    // server task; tiny delay).
    let mut turn_found = None;
    for _ in 0..40 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let turns = h.store.list_turns(&sid).await.unwrap();
        if let Some(t) = turns
            .into_iter()
            .find(|t| t.status == libre_cr_review::storage::TurnStatus::Cancelled)
        {
            turn_found = Some(t);
            break;
        }
    }
    let t = turn_found.expect("expected a cancelled turn row to be persisted");
    let ans = t.answer.unwrap_or_default();
    assert!(
        ans.contains("partial"),
        "partial answer should be captured, got: {ans:?}"
    );
}

#[tokio::test]
async fn ws_second_connect_returns_409() {
    let h = common::start_server_with_script(vec![
        // A script that hangs forever (no done) so we hold the busy slot.
        libre_cr_review::config::ScriptedEvent {
            delay_ms: 2000,
            event: libre_cr_review::provider::StreamEvent::TextDelta {
                text: "hold".into(),
            },
        },
    ])
    .await;
    let c = reqwest::Client::new();
    let resp = c
        .post(format!("http://{}/v1/sessions", h.addr))
        .bearer_auth(&h.token)
        .json(&json!({"pr_url":"https://github.com/foo/bar/pull/8","pr_data":{}}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    let sid = body["session_id"].as_str().unwrap().to_string();
    let url = format!("ws://{}/v1/sessions/{}/ask", h.addr, sid);

    let mut req = url.clone().into_client_request().unwrap();
    req.headers_mut().insert(
        "authorization",
        HeaderValue::from_str(&format!("Bearer {}", h.token)).unwrap(),
    );
    let (mut ws1, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    ws1.send(Message::Text(json!({"question":"hold on"}).to_string()))
        .await
        .unwrap();

    // Give the server a moment to register the busy slot.
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;

    // Second connection should be rejected with 409.
    let mut req2 = url.into_client_request().unwrap();
    req2.headers_mut().insert(
        "authorization",
        HeaderValue::from_str(&format!("Bearer {}", h.token)).unwrap(),
    );
    match tokio_tungstenite::connect_async(req2).await {
        Ok(_) => panic!("expected error"),
        Err(tokio_tungstenite::tungstenite::Error::Http(resp)) => {
            assert_eq!(resp.status(), 409);
        }
        Err(e) => panic!("unexpected: {e}"),
    }
    let _ = ws1.close(None).await;
}
