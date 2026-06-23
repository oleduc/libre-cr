//! Seeds notes of every severity + a Q&A turn, then asserts the Markdown
//! export groups + orders correctly.

mod common;

use serde_json::json;

#[tokio::test]
async fn markdown_export_orders_and_groups() {
    let h = common::start_server_default().await;
    let c = reqwest::Client::new();
    let resp = c
        .post(format!("http://{}/v1/sessions", h.addr))
        .bearer_auth(&h.token)
        .json(&json!({
            "pr_url":"https://github.com/foo/bar/pull/42",
            "pr_data": {"metadata": {"title": "feat: bcrypt"}}
        }))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    let sid = body["session_id"].as_str().unwrap().to_string();

    for (sev, content) in [
        ("info", "noted as fine"),
        ("critical", "must fix"),
        ("warning", "watch out"),
        ("suggestion", "consider this"),
    ] {
        let r = c
            .post(format!("http://{}/v1/sessions/{}/notes", h.addr, sid))
            .bearer_auth(&h.token)
            .json(&json!({"content": content, "severity": sev}))
            .send()
            .await
            .unwrap();
        assert!(r.status().is_success(), "status {}", r.status());
    }

    let r = c
        .post(format!("http://{}/v1/sessions/{}/export", h.addr, sid))
        .bearer_auth(&h.token)
        .json(&json!({"format":"markdown"}))
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success());
    let v: serde_json::Value = r.json().await.unwrap();
    let md = v["content"].as_str().unwrap();
    let crit = md.find("## Critical").unwrap();
    let warn = md.find("## Warning").unwrap();
    let sugg = md.find("## Suggestions").unwrap();
    let info = md.find("## Info").unwrap();
    assert!(crit < warn && warn < sugg && sugg < info);
    assert!(md.contains("must fix"));
    assert!(md.contains("feat: bcrypt"));
}
