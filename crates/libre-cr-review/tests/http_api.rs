//! HTTP API smoke tests against the in-process server.

mod common;

use serde_json::json;

fn url(addr: std::net::SocketAddr, path: &str) -> String {
    format!("http://{addr}{path}")
}

#[tokio::test]
async fn health_is_open() {
    let h = common::start_server_default().await;
    let resp = reqwest::get(url(h.addr, "/v1/health")).await.unwrap();
    assert!(resp.status().is_success());
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["ok"], true);
    // Contract: the wire-protocol version is advertised on /v1/health.
    assert_eq!(body["protocol_version"], libre_cr_common::PROTOCOL_VERSION);
    // I1: code_daemon state comes from the health hook (mock fallback here).
    assert!(body["code_daemon"]["connected"].is_boolean());
}

#[tokio::test]
async fn sessions_require_auth() {
    let h = common::start_server_default().await;
    let resp = reqwest::Client::new()
        .post(url(h.addr, "/v1/sessions"))
        .json(&json!({"pr_url":"https://github.com/a/b/pull/1","pr_data":{}}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn sessions_crud_with_token() {
    let h = common::start_server_default().await;
    let c = reqwest::Client::new();
    let resp = c
        .post(url(h.addr, "/v1/sessions"))
        .bearer_auth(&h.token)
        .json(&json!({"pr_url":"https://github.com/a/b/pull/2","pr_data":{}}))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "status {}", resp.status());
    let body: serde_json::Value = resp.json().await.unwrap();
    let id = body["session_id"].as_str().unwrap().to_string();
    assert!(body["worktree_ready"].as_bool().unwrap()); // mock.code_intel = true

    // GET
    let r = c
        .get(url(h.addr, &format!("/v1/sessions/{id}")))
        .bearer_auth(&h.token)
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success());

    // List
    let r = c
        .get(url(h.addr, "/v1/sessions"))
        .bearer_auth(&h.token)
        .send()
        .await
        .unwrap();
    let v: serde_json::Value = r.json().await.unwrap();
    assert!(v["sessions"].is_array());

    // Delete
    let r = c
        .delete(url(h.addr, &format!("/v1/sessions/{id}")))
        .bearer_auth(&h.token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
}

#[tokio::test]
async fn pairing_redeems_once_via_http() {
    let h = common::start_server_default().await;
    let code = h.pairing.issue().await;
    let c = reqwest::Client::new();
    // Unknown code is rejected.
    let resp = c
        .post(url(h.addr, "/v1/pair"))
        .json(&json!({"code":"deadbeef"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);

    // Issued code redeems once.
    let resp = c
        .post(url(h.addr, "/v1/pair"))
        .json(&json!({"code": code,
                      "extension_origin": "chrome-extension://abc"}))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["token"], h.token);

    // Second redemption fails.
    let resp = c
        .post(url(h.addr, "/v1/pair"))
        .json(&json!({"code": code}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn config_ui_serves_html() {
    // C2: GET /config-ui returns a working HTML page (no auth required for
    // the page itself; the POST it submits is authenticated).
    let h = common::start_server_default().await;
    let resp = reqwest::get(url(h.addr, "/config-ui")).await.unwrap();
    assert!(resp.status().is_success());
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(ct.starts_with("text/html"), "content-type was {ct}");
    let body = resp.text().await.unwrap();
    assert!(body.contains("Libre CR"), "page body must mention Libre CR");
    assert!(
        body.contains("/v1/config"),
        "page must reference the config API"
    );
    // Feature C: the page must expose the new "Fetch models" control and the
    // detected-credentials wiring.
    assert!(
        body.contains("Fetch models"),
        "page must offer the Fetch models button"
    );
    assert!(
        body.contains("/v1/provider/models"),
        "page must reference the provider models API"
    );
    assert!(
        body.contains("/v1/provider/detected"),
        "page must reference the detected credentials API"
    );
}

#[tokio::test]
async fn provider_models_returns_mock_list_and_persists_nothing() {
    // POST /v1/provider/models against the default mock provider returns the
    // canned list, and must not mutate the stored config.
    let h = common::start_server_default().await;
    let c = reqwest::Client::new();

    // Token required.
    let resp = c
        .post(url(h.addr, "/v1/provider/models"))
        .json(&json!({"provider": {"kind": "mock"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);

    // Snapshot the config before the call.
    let before: serde_json::Value = c
        .get(url(h.addr, "/v1/config"))
        .bearer_auth(&h.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let resp = c
        .post(url(h.addr, "/v1/provider/models"))
        .bearer_auth(&h.token)
        .json(&json!({"provider": {"kind": "mock"}}))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "status {}", resp.status());
    let body: serde_json::Value = resp.json().await.unwrap();
    let models = body["models"].as_array().expect("models array");
    assert_eq!(models.len(), 2);
    assert_eq!(models[0]["id"], "mock-fast");
    assert_eq!(models[1]["id"], "mock-smart");

    // Config unchanged — nothing was persisted.
    let after: serde_json::Value = c
        .get(url(h.addr, "/v1/config"))
        .bearer_auth(&h.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(before, after, "listing models must not mutate config");
}

#[tokio::test]
async fn provider_models_unknown_kind_is_400() {
    let h = common::start_server_default().await;
    let c = reqwest::Client::new();
    let resp = c
        .post(url(h.addr, "/v1/provider/models"))
        .bearer_auth(&h.token)
        .json(&json!({"provider": {"kind": "nope-not-a-provider"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        400,
        "unknown kind must be a validation error"
    );
}

#[tokio::test]
async fn provider_detected_requires_auth() {
    let h = common::start_server_default().await;
    let resp = reqwest::get(url(h.addr, "/v1/provider/detected"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn provider_detected_reflects_env_presence() {
    // The endpoint reads the daemon's process environment. We set the vars,
    // assert both report true, then clear them and assert false. The two
    // vars are mutated and read within this single test so no other test
    // observes them mid-flight; we never run two of these concurrently
    // because nothing else touches these exact var names in this binary.
    let h = common::start_server_default().await;
    let c = reqwest::Client::new();

    // Clear first so a polluted parent environment can't make this pass for
    // the wrong reason.
    std::env::remove_var("ANTHROPIC_API_KEY");
    std::env::remove_var("OPENAI_API_KEY");
    let d: serde_json::Value = c
        .get(url(h.addr, "/v1/provider/detected"))
        .bearer_auth(&h.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(d["anthropic"], false);
    assert_eq!(d["openai"], false);

    std::env::set_var("ANTHROPIC_API_KEY", "sk-ant-xxx");
    std::env::set_var("OPENAI_API_KEY", "sk-oai-xxx");
    let d: serde_json::Value = c
        .get(url(h.addr, "/v1/provider/detected"))
        .bearer_auth(&h.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(d["anthropic"], true);
    assert_eq!(d["openai"], true);

    std::env::remove_var("ANTHROPIC_API_KEY");
    std::env::remove_var("OPENAI_API_KEY");
}

#[tokio::test]
async fn config_post_persists_to_disk() {
    // C3: POST /v1/config must rewrite review.toml so changes survive a
    // restart. We point the harness at a tempfile, mutate via HTTP, then
    // reload the file from disk and assert the change is visible.
    use libre_cr_review::config::Config;
    use libre_cr_review::pairing::PairingStore;
    use libre_cr_review::provider::{MockProvider, Provider};
    use libre_cr_review::server::{serve, AppStateBuilder, ConfigStore, ListenInfo};
    use libre_cr_review::storage::{InstallKey, Store};
    use libre_cr_review::tools::code_daemon::{CodeDaemonClient, MockCodeDaemonClient};
    use std::sync::Arc;

    let tmp = tempfile::tempdir().unwrap();
    let cfg_path = tmp.path().join("review.toml");
    let mut cfg = Config::default();
    cfg.mock.code_intel = true;
    cfg.save(&cfg_path).unwrap();

    let store = Store::open_in_memory().unwrap();
    let install_key = Arc::new(InstallKey::from_bytes([0u8; 32]));
    let provider: Arc<dyn Provider> = Arc::new(MockProvider::new(vec![]));
    let code_daemon: Arc<dyn CodeDaemonClient> = Arc::new(MockCodeDaemonClient);
    let state = AppStateBuilder {
        store,
        config: ConfigStore::new(cfg),
        provider,
        code_daemon,
        token: "test-token".into(),
        extension_origin: String::new(),
        install_key,
        health_hook: None,
        config_path: Some(cfg_path.clone()),
    }
    .build();
    let bind: std::net::SocketAddr = "127.0.0.1:0".parse().unwrap();
    let ListenInfo { addr, task: _task } = serve(state, bind).await.unwrap();
    let _drop_guard = PairingStore::new(); // silence unused warn if added

    let c = reqwest::Client::new();
    let resp = c
        .post(url(addr, "/v1/config"))
        .bearer_auth("test-token")
        .json(&json!({"provider": {"model": "custom-x", "max_tokens": 9999}}))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());

    // Reload review.toml from disk — change must be present.
    let reloaded = Config::load(&cfg_path).unwrap();
    assert_eq!(reloaded.provider.model, "custom-x");
    assert_eq!(reloaded.provider.max_tokens, 9999);
}

#[tokio::test]
async fn pair_issue_then_redeem_against_running_daemon() {
    // C1: the CLIs must hit the running daemon's PairingStore via
    // POST /v1/pair/issue. Issuing followed by redeeming must succeed end-
    // to-end without any out-of-band state.
    let h = common::start_server_default().await;
    let c = reqwest::Client::new();

    // Unauthenticated issue is rejected.
    let resp = c
        .post(url(h.addr, "/v1/pair/issue"))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);

    // Token-authenticated issue mints a code.
    let resp = c
        .post(url(h.addr, "/v1/pair/issue"))
        .bearer_auth(&h.token)
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "status {}", resp.status());
    let body: serde_json::Value = resp.json().await.unwrap();
    let code = body["code"].as_str().expect("code").to_string();
    assert!(!code.is_empty(), "code must be non-empty");
    assert!(body["expires_at_epoch_ms"].is_i64());

    // That code redeems against the running daemon — no out-of-band
    // PairingStore needed.
    let resp = c
        .post(url(h.addr, "/v1/pair"))
        .json(&json!({"code": code}))
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "redeem status {}",
        resp.status()
    );
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["token"], h.token);

    // Daemon's pairing store has consumed it.
    assert!(h.pairing.is_empty().await);
}

#[tokio::test]
async fn pair_redeem_rate_limited_after_five_failures() {
    // I11: five wrong-code POSTs from the same source IP trip a per-IP
    // rate limit; the sixth attempt returns 429.
    let h = common::start_server_default().await;
    let c = reqwest::Client::new();
    for _ in 0..5 {
        let resp = c
            .post(url(h.addr, "/v1/pair"))
            .json(&json!({"code": "wrong"}))
            .send()
            .await
            .unwrap();
        // Either 401 (first four) or 429 (when the 5th tips us over).
        assert!(
            resp.status() == 401 || resp.status() == 429,
            "unexpected status {}",
            resp.status()
        );
    }
    let resp = c
        .post(url(h.addr, "/v1/pair"))
        .json(&json!({"code": "wrong"}))
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

#[tokio::test]
async fn origin_check_when_configured() {
    let h = common::start_server_default().await;
    // First pair to register an extension origin
    let code = h.pairing.issue().await;
    let c = reqwest::Client::new();
    let _ = c
        .post(url(h.addr, "/v1/pair"))
        .json(&json!({"code": code, "extension_origin": "chrome-extension://abc"}))
        .send()
        .await
        .unwrap();
    // Now a session with a wrong origin should be rejected.
    let resp = c
        .post(url(h.addr, "/v1/sessions"))
        .bearer_auth(&h.token)
        .header("Origin", "https://evil.example")
        .json(&json!({"pr_url":"https://github.com/a/b/pull/9","pr_data":{}}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403);
    // With the right origin, it succeeds.
    let resp = c
        .post(url(h.addr, "/v1/sessions"))
        .bearer_auth(&h.token)
        .header("Origin", "chrome-extension://abc")
        .json(&json!({"pr_url":"https://github.com/a/b/pull/9","pr_data":{}}))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());
}
