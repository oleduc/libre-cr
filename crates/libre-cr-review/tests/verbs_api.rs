//! `GET /v1/verbs` + selection-enforcement integration tests.

mod common;

use futures::{SinkExt, StreamExt};
use libre_cr_common::ws_frames::ServerFrame;
use serde_json::json;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::protocol::Message;

#[tokio::test]
async fn verbs_returns_phase_b_catalog() {
    let h = common::start_server_default().await;
    let c = reqwest::Client::new();
    let r = c
        .get(format!("http://{}/v1/verbs", h.addr))
        .bearer_auth(&h.token)
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success());
    let body: serde_json::Value = r.json().await.unwrap();
    let arr = body.as_array().expect("verbs endpoint returns an array");
    assert_eq!(arr.len(), 5, "expected exactly five verbs, got {arr:?}");
    let ids: Vec<&str> = arr
        .iter()
        .filter_map(|v| v.get("id").and_then(|x| x.as_str()))
        .collect();
    for want in &[
        "find_callers",
        "show_history",
        "related_tests",
        "compare_to_base",
        "explain",
    ] {
        assert!(ids.contains(want), "missing verb {want} in {ids:?}");
    }
    // Shape check: each entry has id, label, required_selection, description.
    for v in arr {
        for k in &["id", "label", "required_selection", "description"] {
            assert!(v.get(*k).is_some(), "missing field {k} in {v:?}");
        }
    }
}

#[tokio::test]
async fn find_callers_without_symbol_yields_error_frame() {
    let h = common::start_server_default().await;
    let c = reqwest::Client::new();
    let resp = c
        .post(format!("http://{}/v1/sessions", h.addr))
        .bearer_auth(&h.token)
        .json(&json!({"pr_url":"https://github.com/foo/bar/pull/77","pr_data":{}}))
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

    let (mut ws, _resp) = tokio_tungstenite::connect_async(req).await.unwrap();
    // No selection: find_callers requires Symbol — daemon should reject.
    ws.send(Message::Text(
        json!({"question":"who calls?","verb":"find_callers"}).to_string(),
    ))
    .await
    .unwrap();

    let mut got_error = false;
    while let Some(msg) = ws.next().await {
        let Ok(msg) = msg else { break };
        let s = match msg {
            Message::Text(s) => s,
            Message::Close(_) => break,
            _ => continue,
        };
        if let Ok(ServerFrame::Error { message, .. }) = serde_json::from_str::<ServerFrame>(&s) {
            got_error = true;
            assert!(
                message.contains("symbol"),
                "expected symbol-related error, got: {message}"
            );
            break;
        }
    }
    assert!(got_error, "expected an error frame for invalid selection");
}

#[tokio::test]
async fn find_callers_with_symbol_proceeds() {
    let h = common::start_server_with_script(common::happy_two_round_script()).await;
    let c = reqwest::Client::new();
    let resp = c
        .post(format!("http://{}/v1/sessions", h.addr))
        .bearer_auth(&h.token)
        .json(&json!({"pr_url":"https://github.com/foo/bar/pull/78","pr_data":{}}))
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
    let (mut ws, _resp) = tokio_tungstenite::connect_async(req).await.unwrap();
    ws.send(Message::Text(
        json!({
            "question":"who calls?",
            "verb":"find_callers",
            "selection":{"kind":"symbol","file":"src/auth.ts","line":42,"column":8,"identifier":"bcryptHash"}
        })
        .to_string(),
    ))
    .await
    .unwrap();

    let mut got_done = false;
    while let Some(msg) = ws.next().await {
        let Ok(msg) = msg else { break };
        let s = match msg {
            Message::Text(s) => s,
            Message::Close(_) => break,
            _ => continue,
        };
        if let Ok(ServerFrame::Done { .. }) = serde_json::from_str::<ServerFrame>(&s) {
            got_done = true;
            break;
        }
    }
    assert!(got_done, "expected done frame for valid find_callers");
}
