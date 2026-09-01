//! Routes wiring.

use std::net::SocketAddr;

use axum::{
    extract::{ConnectInfo, Path, Query, State},
    http::{header, HeaderValue, Method, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, patch, post},
    Json, Router,
};
use libre_cr_common::http_api::{
    CodeDaemonHealth, CodeDaemonHealthResponse, CreateSessionResponse, DetectedCredentials,
    ExportResponse, HealthResponse, ListSessionsResponse, ModelsResponse, PairIssueResponse,
    PairRedeemResponse, SearchHit, SearchResponse, SessionDetailResponse, SessionSummary,
    VerbDescriptor,
};
use libre_cr_common::{Selection, PROTOCOL_VERSION};
use serde::Deserialize;
use serde_json::json;
use tokio::net::TcpListener;
use tower_http::cors::{Any, CorsLayer};

use crate::error::{Error, Result};
use crate::storage::Severity;
use crate::verbs::catalog_descriptors;
use crate::worktree::{pr_inputs_from_pr_data, spawn_prepare, PrepareInputs, SessionStatus};

/// Sanitize a user query for an FTS5 MATCH. We strip control characters and
/// internal double quotes, then wrap the whole expression in double quotes so
/// the FTS engine treats it as a single phrase. This sidesteps the FTS5 mini-
/// grammar (NEAR, OR, parens, etc.) entirely; a proper lexer is out of scope
/// for Phase 6 per the plan.
fn sanitize_fts_query(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|c| !c.is_control())
        .map(|c| if c == '"' { ' ' } else { c })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    format!("\"{trimmed}\"")
}

use super::auth::auth_middleware;
use super::export::{build_export, ExportRequest};
use super::state::AppState;
use super::ws::ws_ask_handler;

pub struct ListenInfo {
    pub addr: SocketAddr,
    pub task: tokio::task::JoinHandle<std::io::Result<()>>,
}

pub fn build_router(state: AppState) -> Router {
    // CORS is wide open on purpose. The bearer token is the security boundary;
    // an origin allowlist buys nothing here and broke the content script, whose
    // fetches carry the *page* origin (https://github.com) under MV3, not the
    // extension's. The unauthenticated routes (/v1/health, rate-limited
    // /v1/pair) are reachable by anything on the machine via curl anyway.
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST, Method::PATCH, Method::DELETE])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE]);

    Router::new()
        .route("/v1/sessions", post(create_session).get(list_sessions))
        .route("/v1/sessions/:id", get(get_session).delete(delete_session))
        .route("/v1/sessions/:id/ask", get(ws_ask_handler))
        .route("/v1/sessions/:id/notes", post(create_note))
        .route(
            "/v1/sessions/:id/notes/:note_id",
            patch(update_note).delete(delete_note),
        )
        .route("/v1/sessions/:id/export", post(export_session))
        .route("/v1/search", get(search_global))
        .route("/v1/config", get(get_config).post(post_config))
        .route("/v1/config/validate", post(validate_config))
        .route("/v1/provider/models", post(provider_models))
        .route("/v1/provider/detected", get(provider_detected))
        .route("/v1/health", get(health))
        .route("/v1/health/code-daemon", get(health_code_daemon))
        .route("/v1/pair", post(pair))
        .route("/v1/pair/issue", post(pair_issue))
        .route("/config-ui", get(config_ui))
        .route("/v1/verbs", get(verbs))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .layer(cors)
        .with_state(state)
}

/// Bind, serve, return the addr + task handle. The task ends when the server stops.
pub async fn serve(state: AppState, bind: SocketAddr) -> Result<ListenInfo> {
    let router = build_router(state);
    let listener = TcpListener::bind(bind).await.map_err(Error::from)?;
    let addr = listener.local_addr().map_err(Error::from)?;
    // ConnectInfo so the pairing handler can rate-limit per source IP.
    // See `pair` + `PairingStore::record_failure`.
    let task = tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
    });
    Ok(ListenInfo { addr, task })
}

#[derive(Deserialize)]
struct CreateSessionBody {
    pr_url: String,
    #[serde(default)]
    pr_data: serde_json::Value,
}

async fn create_session(
    State(state): State<AppState>,
    Json(body): Json<CreateSessionBody>,
) -> Result<Json<CreateSessionResponse>> {
    // Pull a head_sha out of the incoming scrape before consuming pr_data.
    let incoming_head_sha = body
        .pr_data
        .get("head_sha")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let sess = state
        .store
        .upsert_session(&body.pr_url, body.pr_data)
        .await?;
    // Diff-change detection: if we have an incoming sha and a stored sha,
    // and they differ, flag the caller. Always update to the latest sha
    // we've observed for this PR.
    let mut pr_diff_changed = false;
    if let Some(new_sha) = incoming_head_sha.as_deref() {
        let prev = state
            .store
            .set_head_sha(&sess.session_id, Some(new_sha))
            .await?;
        if let Some(old) = prev.as_deref() {
            if old != new_sha {
                pr_diff_changed = true;
            }
        }
    }
    let cfg = state.config.snapshot().await;
    let mut worktree_ready = sess.worktree_path.is_some();
    let mut repo_local_path = sess.worktree_path.clone();
    let mut pending_action: Option<&'static str> = None;
    if !worktree_ready && cfg.mock.code_intel {
        // Fake out a worktree so the WS flow can be exercised in Phase 2.
        let fake = format!("/tmp/libre-cr-mock/{}", sess.session_id);
        state
            .store
            .set_worktree(&sess.session_id, Some("mock"), Some(&fake))
            .await?;
        worktree_ready = true;
        repo_local_path = Some(fake);
        state
            .session_status
            .set(
                &sess.session_id,
                SessionStatus::ready(
                    format!("/tmp/libre-cr-mock/{}", sess.session_id),
                    Some("mock".into()),
                ),
            )
            .await;
    } else if !worktree_ready {
        // Kick off the real worktree orchestration in the background.
        let (remote_url, pr_ref) = pr_inputs_from_pr_data(&sess.pr_data, sess.pr_number);
        state
            .session_status
            .set(&sess.session_id, SessionStatus::pending())
            .await;
        spawn_prepare(
            state.store.clone(),
            state.code_daemon.clone(),
            state.session_status.clone(),
            PrepareInputs {
                session_id: sess.session_id.clone(),
                remote_url,
                pr_ref,
            },
        );
        pending_action = Some("worktree_pending");
    }
    Ok(Json(CreateSessionResponse {
        session_id: sess.session_id,
        worktree_ready,
        repo_local_path,
        pending_action: pending_action.map(|s| s.to_string()),
        pr_diff_changed,
        head_sha: incoming_head_sha,
    }))
}

async fn get_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<SessionDetailResponse>> {
    let sess = state.store.get_session(&id).await?.ok_or(Error::NotFound)?;
    let turns = state.store.list_turns(&id).await?;
    let status = state.session_status.get(&id).await;
    let worktree_ready = sess.worktree_path.is_some()
        && status
            .as_ref()
            .map(|s| matches!(s.state, crate::worktree::WorktreeState::Ready))
            .unwrap_or(true);
    let head_sha = sess.head_sha.clone();
    let last_seen_at = sess.last_active_at;
    let turns = turns
        .into_iter()
        .map(|t| serde_json::to_value(t).map_err(Error::from))
        .collect::<Result<Vec<_>>>()?;
    let status = match status {
        Some(s) => Some(serde_json::to_value(s)?),
        None => None,
    };
    Ok(Json(SessionDetailResponse {
        session: SessionSummary::from(sess),
        turns,
        worktree_ready,
        status,
        head_sha,
        last_seen_at,
    }))
}

#[derive(Deserialize)]
struct ListSessionsQuery {
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    since: Option<i64>,
}

async fn list_sessions(
    State(state): State<AppState>,
    Query(q): Query<ListSessionsQuery>,
) -> Result<Json<ListSessionsResponse>> {
    let limit = q.limit.unwrap_or(50).min(500);
    let sessions = state.store.list_sessions(limit, q.since).await?;
    Ok(Json(ListSessionsResponse {
        sessions: sessions.into_iter().map(SessionSummary::from).collect(),
    }))
}

async fn delete_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode> {
    let ok = state.store.delete_session(&id).await?;
    Ok(if ok {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    })
}

#[derive(Deserialize)]
struct CreateNoteBody {
    content: String,
    #[serde(default)]
    anchor: Option<Selection>,
    #[serde(default)]
    severity: Option<String>,
    #[serde(default)]
    source_turn_id: Option<String>,
}

async fn create_note(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<CreateNoteBody>,
) -> Result<Json<serde_json::Value>> {
    let sev = body
        .severity
        .as_deref()
        .and_then(Severity::parse)
        .unwrap_or(Severity::Info);
    let note_id = state
        .store
        .create_note_with_source(
            &id,
            &body.content,
            sev,
            body.anchor,
            body.source_turn_id.as_deref(),
        )
        .await?;
    Ok(Json(json!({ "note_id": note_id })))
}

#[derive(Deserialize)]
struct SearchQuery {
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

async fn search_global(
    State(state): State<AppState>,
    Query(q): Query<SearchQuery>,
) -> Result<Json<SearchResponse>> {
    let raw = q.q.unwrap_or_default();
    let sanitized = sanitize_fts_query(&raw);
    if sanitized.is_empty() {
        return Ok(Json(SearchResponse { results: vec![] }));
    }
    let limit = q.limit.unwrap_or(20).clamp(1, 100);
    let hits = state.store.search_global(&sanitized, limit).await?;
    let results: Vec<SearchHit> = hits
        .into_iter()
        .map(|(session_id, pr_url, turn_id, snippet, score)| SearchHit {
            session_id,
            pr_url,
            turn_id,
            snippet,
            score,
        })
        .collect();
    Ok(Json(SearchResponse { results }))
}

#[derive(Deserialize)]
struct PatchNoteBody {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    severity: Option<String>,
}

async fn update_note(
    State(state): State<AppState>,
    Path((id, note_id)): Path<(String, String)>,
    Json(body): Json<PatchNoteBody>,
) -> Result<StatusCode> {
    let sev = body.severity.as_deref().and_then(Severity::parse);
    let ok = state
        .store
        .update_note(&id, &note_id, body.content.as_deref(), sev)
        .await?;
    Ok(if ok {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    })
}

async fn delete_note(
    State(state): State<AppState>,
    Path((id, note_id)): Path<(String, String)>,
) -> Result<StatusCode> {
    let ok = state.store.delete_note(&id, &note_id).await?;
    Ok(if ok {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    })
}

async fn export_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<ExportRequest>,
) -> Result<Json<ExportResponse>> {
    let sess = state.store.get_session(&id).await?.ok_or(Error::NotFound)?;
    let r = build_export(&state.store, &sess, &body).await?;
    Ok(Json(r))
}

async fn get_config(State(state): State<AppState>) -> Result<Json<serde_json::Value>> {
    let cfg = state.config.snapshot().await;
    let provider = json!({
        "kind": cfg.provider.kind,
        "model": cfg.provider.model,
        "max_tokens": cfg.provider.max_tokens,
        "temperature": cfg.provider.temperature,
        "endpoint": cfg.provider.endpoint,
    });
    Ok(Json(json!({
        "provider": provider,
        "limits": cfg.limits,
        "global_instructions": cfg.global_instructions,
        "mcp_server": cfg.mcp_server,
    })))
}

/// Apply the `provider` patch object from a `POST /v1/config` /
/// `POST /v1/config/validate` body onto a config. Plaintext `api_key` is
/// encrypted with the install key before it lands in the config.
fn apply_provider_patch(
    cfg: &mut crate::config::Config,
    body: &serde_json::Value,
    install_key: &crate::storage::InstallKey,
) -> Result<()> {
    let Some(p) = body.get("provider") else {
        return Ok(());
    };
    if let Some(s) = p.get("kind").and_then(|s| s.as_str()) {
        cfg.provider.kind = s.to_string();
    }
    if let Some(s) = p.get("model").and_then(|s| s.as_str()) {
        cfg.provider.model = s.to_string();
    }
    if let Some(n) = p.get("max_tokens").and_then(|n| n.as_u64()) {
        cfg.provider.max_tokens = n as u32;
    }
    if let Some(n) = p.get("temperature").and_then(|n| n.as_f64()) {
        cfg.provider.temperature = n as f32;
    }
    if let Some(s) = p.get("endpoint").and_then(|s| s.as_str()) {
        cfg.provider.endpoint = s.to_string();
    }
    if let Some(s) = p.get("api_key").and_then(|s| s.as_str()) {
        // An explicit empty string clears the saved key (→ env-var fallback).
        cfg.provider.api_key_enc = if s.is_empty() {
            String::new()
        } else {
            crate::storage::encrypt_value(install_key, s)?
        };
    }
    Ok(())
}

async fn post_config(
    State(state): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>> {
    // Build a candidate, prove it constructs a provider, persist it, and
    // only then commit + swap. A typo'd `kind` is rejected with 400 before
    // anything changes (it used to be accepted and brick the next boot),
    // and a failed disk write is a 500 with nothing applied — not a silent
    // `ok: true` that evaporates on restart.
    let provider_changed = body.get("provider").is_some();
    let mut cfg = state.config.0.lock().await;
    let mut candidate = cfg.clone();
    apply_provider_patch(&mut candidate, &body, &state.install_key)?;
    let new_provider = if provider_changed {
        Some(crate::provider::build_provider(
            &candidate,
            &state.install_key,
        )?)
    } else {
        None
    };
    if let Some(path) = state.config_path.as_ref() {
        persist_config_atomic(&candidate, path)
            .map_err(|e| Error::Internal(format!("persist review.toml: {e}")))?;
    }
    *cfg = candidate;
    drop(cfg);
    if let Some(p) = new_provider {
        // RC1: the running provider tracks accepted config mutations.
        state.provider.set(p).await;
    }
    Ok(Json(json!({"ok": true})))
}

/// Atomic write: serialize into `<path>.tmp` then rename over the target.
/// Keeps `review.toml` consistent across process crashes mid-write.
fn persist_config_atomic(cfg: &crate::config::Config, path: &std::path::Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(Error::from)?;
    }
    let body = toml::to_string_pretty(cfg)
        .map_err(|e| Error::Internal(format!("write review.toml: {e}")))?;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp_path = std::path::PathBuf::from(tmp);
    std::fs::write(&tmp_path, body).map_err(Error::from)?;
    std::fs::rename(&tmp_path, path).map_err(Error::from)?;
    Ok(())
}

/// Validate a provider built from the *currently-stored* config, or from
/// the candidate config in the request body when one is supplied. Never the
/// startup snapshot (RC1: it used to validate the stale provider, so a
/// freshly-entered Anthropic key got `ok: true` from the mock).
async fn validate_config(
    State(state): State<AppState>,
    body: Option<Json<serde_json::Value>>,
) -> Result<Json<serde_json::Value>> {
    let mut cfg = state.config.snapshot().await;
    if let Some(Json(b)) = body {
        apply_provider_patch(&mut cfg, &b, &state.install_key)?;
    }
    let provider = crate::provider::build_provider(&cfg, &state.install_key)?;
    provider.validate().await?;
    Ok(Json(json!({"ok": true})))
}

/// List the models offered by a *candidate* provider. The body is a provider
/// patch (`{ provider: { kind, api_key?, endpoint? } }`, same shape as
/// `POST /v1/config`). We apply it to a clone of the current config and build
/// an ephemeral provider so the user can fetch a model list for a new
/// provider+key *before* saving. Nothing is persisted.
async fn provider_models(
    State(state): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<ModelsResponse>> {
    let mut cfg = state.config.snapshot().await;
    let saved_endpoint = cfg.provider.endpoint.clone();
    apply_provider_patch(&mut cfg, &body, &state.install_key)?;
    // The stored key is only ever sent to the endpoint it was saved with. A
    // candidate pointing anywhere else must bring its own key (env-var keys,
    // which the user controls, still apply) — otherwise a paired caller
    // could steer the saved key to an arbitrary host and read it there.
    let explicit_key = body
        .get("provider")
        .and_then(|p| p.get("api_key"))
        .is_some();
    if cfg.provider.endpoint != saved_endpoint && !explicit_key {
        cfg.provider.api_key_enc = String::new();
    }
    let provider = crate::provider::build_provider(&cfg, &state.install_key)?;
    let models = provider.list_models().await?;
    Ok(Json(ModelsResponse { models }))
}

/// Report which credentials are detectable in the daemon's environment, so the
/// config UI can offer a one-click "no key needed" option.
///
/// Reflects ambient env vars only (`ANTHROPIC_API_KEY` / `OPENAI_API_KEY`).
async fn provider_detected() -> Json<DetectedCredentials> {
    fn present(var: &str) -> bool {
        std::env::var(var).map(|v| !v.is_empty()).unwrap_or(false)
    }
    Json(DetectedCredentials {
        anthropic: present("ANTHROPIC_API_KEY"),
        openai: present("OPENAI_API_KEY"),
    })
}

async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    // I1: report the real code-daemon state when the CLI wired in the
    // health hook; the mock fallback only exists for in-process tests.
    let code_daemon = if let Some(hook) = &state.health_hook {
        let snap = (hook)().await;
        CodeDaemonHealth {
            connected: snap.connected,
            version: snap.version,
        }
    } else {
        CodeDaemonHealth {
            connected: true,
            version: Some("mock".into()),
        }
    };
    Json(HealthResponse {
        ok: true,
        version: state.version.clone(),
        protocol_version: PROTOCOL_VERSION,
        code_daemon,
    })
}

async fn health_code_daemon(State(state): State<AppState>) -> Json<CodeDaemonHealthResponse> {
    if let Some(hook) = &state.health_hook {
        let snap = (hook)().await;
        return Json(CodeDaemonHealthResponse {
            connected: snap.connected,
            version: snap.version,
            last_error: snap.last_error,
            restart_count: snap.restart_count,
        });
    }
    Json(CodeDaemonHealthResponse {
        connected: true,
        version: Some("mock".into()),
        last_error: None,
        restart_count: 0,
    })
}

#[derive(Deserialize)]
struct PairBody {
    code: String,
    #[serde(default)]
    extension_origin: Option<String>,
}

async fn pair(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(body): Json<PairBody>,
) -> std::result::Result<Json<PairRedeemResponse>, Response<axum::body::Body>> {
    match state.pairing.redeem_from(&body.code, peer.ip()).await {
        crate::pairing::RedeemOutcome::Ok => {}
        crate::pairing::RedeemOutcome::Invalid => {
            return Err((StatusCode::UNAUTHORIZED, "unauthorized").into_response());
        }
        crate::pairing::RedeemOutcome::RateLimited => {
            // 429 with Retry-After in seconds, matching the failure-window
            // length. The extension surfaces this to the user.
            let mut resp = (StatusCode::TOO_MANY_REQUESTS, "rate limited").into_response();
            resp.headers_mut()
                .insert("retry-after", HeaderValue::from_static("60"));
            return Err(resp);
        }
    }
    if let Some(origin) = body.extension_origin.clone() {
        // Persisted for bookkeeping / diagnostics only; CORS no longer keys on it.
        let snapshot = {
            let mut cfg = state.config.0.lock().await;
            cfg.server.extension_origin = origin;
            cfg.clone()
        };
        if let Some(path) = state.config_path.as_ref() {
            // Best-effort: the pairing itself succeeded; a failed write only
            // costs persistence across restarts.
            if let Err(e) = persist_config_atomic(&snapshot, path) {
                tracing::warn!(path = %path.display(), error = %e, "persist review.toml after pair");
            }
        }
    }
    Ok(Json(PairRedeemResponse {
        token: state.token.clone(),
        extension_origin: body
            .extension_origin
            .unwrap_or(state.extension_origin.clone()),
    }))
}

#[derive(Deserialize, Default)]
struct PairIssueBody {
    #[serde(default)]
    ttl_seconds: Option<u64>,
}

/// Token-authenticated issuer for one-time pairing codes. The CLIs
/// (`libre-cr pair` / `libre-cr-review pair`) call this so codes land in
/// the *running* daemon's `PairingStore` — issuing locally in either CLI
/// is meaningless, since the extension redeems against the daemon.
async fn pair_issue(
    State(state): State<AppState>,
    body: Option<Json<PairIssueBody>>,
) -> Result<Json<PairIssueResponse>> {
    // N2: the requested TTL is actually applied to the issued code, not
    // just echoed back. Clamped to 30 s ..= 15 min.
    let ttl = body
        .and_then(|Json(b)| b.ttl_seconds)
        .unwrap_or(300)
        .clamp(30, 900);
    let code = state
        .pairing
        .issue_with_ttl(std::time::Duration::from_secs(ttl))
        .await;
    let expires_at_epoch_ms =
        (chrono::Utc::now() + chrono::Duration::seconds(ttl as i64)).timestamp_millis();
    Ok(Json(PairIssueResponse {
        code,
        expires_at_epoch_ms,
    }))
}

async fn verbs() -> Json<Vec<VerbDescriptor>> {
    Json(catalog_descriptors())
}

/// Minimal config UI. Static HTML, no templating — the page reads its
/// bearer token from `?token=...` (per spec, same as the WS upgrade) and
/// posts to `/v1/config`. The token never appears in stored markup; it's
/// only ever in the URL the wrapper opens, which is the same surface the
/// user already trusts to launch the daemon.
async fn config_ui() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        CONFIG_UI_HTML,
    )
}

const CONFIG_UI_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8" />
<title>Libre CR — Configuration</title>
<style>
  body { font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
         max-width: 36rem; margin: 2rem auto; padding: 0 1rem; color: #222; }
  h1 { font-size: 1.4rem; margin-bottom: 0.25rem; }
  p.lede { color: #555; margin-top: 0; }
  label { display: block; margin: 0.75rem 0 0.25rem; font-weight: 600; }
  input, select { width: 100%; padding: 0.4rem; box-sizing: border-box;
                  font: inherit; }
  button { margin-top: 1rem; padding: 0.5rem 1rem; font: inherit; }
  .row { display: grid; grid-template-columns: 1fr 1fr; gap: 0.75rem; }
  #status { margin-top: 0.75rem; font-size: 0.9rem; }
  .ok { color: #064; } .err { color: #803; }
  small { color: #777; }
  .modelRow { display: flex; gap: 0.5rem; align-items: center; }
  .modelRow select { flex: 1; }
  .modelRow button { margin-top: 0; white-space: nowrap; }
  .hint { font-size: 0.85rem; color: #064; margin: 0.25rem 0 0; }
  .modelStatus { font-size: 0.85rem; color: #555; margin: 0.25rem 0 0; }
  .modelStatus.err { color: #803; }
label.inline { display: flex; align-items: center; gap: 6px; font-weight: normal; margin-top: 4px; }
</style>
</head>
<body>
<h1>Libre CR — Configuration</h1>
<p class="lede">Edit the LLM provider settings used by this review daemon. Changes are written to <code>review.toml</code> immediately.</p>

<form id="cfgForm">
  <label for="kind">Provider</label>
  <select id="kind" name="kind">
    <option value="mock">mock (no network)</option>
    <option value="anthropic">anthropic</option>
    <option value="openai_compat">openai_compat</option>
  </select>

  <label for="modelSelect">Model</label>
  <div class="modelRow">
    <select id="modelSelect" name="modelSelect">
      <option value="__manual__">Other / type manually</option>
    </select>
    <button type="button" id="fetchModels">Fetch models</button>
  </div>
  <input id="model" name="model" type="text" autocomplete="off" placeholder="model id" />
  <p id="modelStatus" class="modelStatus" aria-live="polite"></p>

  <div class="row">
    <div>
      <label for="max_tokens">Max tokens</label>
      <input id="max_tokens" name="max_tokens" type="number" min="1" />
    </div>
    <div>
      <label for="temperature">Temperature</label>
      <input id="temperature" name="temperature" type="number" step="0.05" min="0" max="2" />
    </div>
  </div>

  <label for="endpoint">Endpoint <small>(blank = provider default)</small></label>
  <input id="endpoint" name="endpoint" type="text" autocomplete="off" />

  <div id="apiKeyField">
    <label for="api_key">API key <small>(stored encrypted)</small></label>
    <input id="api_key" name="api_key" type="password" autocomplete="off" placeholder="leave blank to keep current" />
    <label class="inline"><input id="clear_key" type="checkbox" /> Clear the saved key (use the environment variable, or none)</label>
  </div>
  <p id="detectedHint" class="hint" hidden></p>

  <button type="submit">Save</button>
  <div id="status" role="status" aria-live="polite"></div>
</form>

<script>
(function () {
  var qs = new URLSearchParams(window.location.search);
  var token = qs.get("token") || "";
  var headers = { "content-type": "application/json" };
  if (token) headers["authorization"] = "Bearer " + token;
  var status = document.getElementById("status");
  var kindEl = document.getElementById("kind");
  var modelEl = document.getElementById("model");
  var modelSelect = document.getElementById("modelSelect");
  var modelStatus = document.getElementById("modelStatus");
  var detectedHint = document.getElementById("detectedHint");
  var apiKeyField = document.getElementById("apiKeyField");
  var apiKeyEl = document.getElementById("api_key");
  var clearKeyEl = document.getElementById("clear_key");
  // Blank field = keep the stored key; the checkbox sends api_key: "" to clear it.
  function apiKeyPatch(target) {
    if (apiKeyEl.value) target.api_key = apiKeyEl.value;
    else if (clearKeyEl.checked) target.api_key = "";
  }
  var endpointEl = document.getElementById("endpoint");
  // Detected ambient credentials, keyed by provider kind.
  var detected = { anthropic: false, openai_compat: false };
  function setStatus(msg, ok) {
    status.textContent = msg;
    status.className = ok ? "ok" : "err";
  }
  function setModelStatus(msg, isErr) {
    modelStatus.textContent = msg || "";
    modelStatus.className = isErr ? "modelStatus err" : "modelStatus";
  }
  var ENV_VARS = { anthropic: "ANTHROPIC_API_KEY", openai_compat: "OPENAI_API_KEY" };
  // Show the "detected key" hint next to the API-key field when the selected
  // provider has an ambient credential in the daemon's environment.
  function updateDetectedHint() {
    var kind = kindEl.value;
    detectedHint.hidden = true;
    detectedHint.textContent = "";
    if (detected[kind] && ENV_VARS[kind]) {
      detectedHint.hidden = false;
      detectedHint.textContent =
        "✓ " + ENV_VARS[kind] +
        " detected in the daemon's environment — leave the key blank to use it.";
    }
  }
  // Keep the free-text model input as the source of truth. The dropdown is a
  // convenience: picking a real model copies it into the text input; picking
  // "Other / type manually" leaves the text input for hand entry.
  modelSelect.addEventListener("change", function () {
    if (modelSelect.value !== "__manual__") {
      modelEl.value = modelSelect.value;
    }
  });
  kindEl.addEventListener("change", updateDetectedHint);

  fetch("/v1/provider/detected", { headers: headers }).then(function (r) {
    if (!r.ok) throw new Error("HTTP " + r.status);
    return r.json();
  }).then(function (d) {
    detected.anthropic = !!d.anthropic;
    detected.openai_compat = !!d.openai;
    updateDetectedHint();
  }).catch(function () { /* hint is best-effort */ });

  fetch("/v1/config", { headers: headers }).then(function (r) {
    if (!r.ok) throw new Error("HTTP " + r.status);
    return r.json();
  }).then(function (cfg) {
    var p = cfg.provider || {};
    kindEl.value = p.kind || "mock";
    modelEl.value = p.model || "";
    document.getElementById("max_tokens").value = p.max_tokens || 4096;
    document.getElementById("temperature").value = p.temperature != null ? p.temperature : 0;
    endpointEl.value = p.endpoint || "";
    updateDetectedHint();
    setStatus("Loaded current settings.", true);
  }).catch(function (e) {
    setStatus("Could not load current settings: " + e.message, false);
  });

  function providerPatch() {
    var patch = { kind: kindEl.value, endpoint: endpointEl.value };
    apiKeyPatch(patch);
    return patch;
  }

  document.getElementById("fetchModels").addEventListener("click", function () {
    setModelStatus("Fetching models…", false);
    fetch("/v1/provider/models", {
      method: "POST", headers: headers,
      body: JSON.stringify({ provider: providerPatch() }),
    }).then(function (r) {
      return r.json().then(function (data) {
        if (!r.ok) {
          var msg = (data && data.message) ? data.message : ("HTTP " + r.status);
          throw new Error(msg);
        }
        return data;
      });
    }).then(function (data) {
      var models = (data && data.models) || [];
      // Rebuild the dropdown, keeping the manual option first.
      modelSelect.innerHTML = "";
      var manual = document.createElement("option");
      manual.value = "__manual__";
      manual.textContent = "Other / type manually";
      modelSelect.appendChild(manual);
      models.forEach(function (m) {
        var opt = document.createElement("option");
        opt.value = m.id;
        opt.textContent = m.display_name ? (m.display_name + " (" + m.id + ")") : m.id;
        modelSelect.appendChild(opt);
      });
      // If the current text value matches a fetched model, preselect it.
      var match = models.some(function (m) { return m.id === modelEl.value; });
      modelSelect.value = match ? modelEl.value : "__manual__";
      setModelStatus("Loaded " + models.length + " model(s). Pick one or type your own.", false);
    }).catch(function (e) {
      setModelStatus("Could not fetch models: " + e.message + " You can still type the model id.", true);
    });
  });

  document.getElementById("cfgForm").addEventListener("submit", function (ev) {
    ev.preventDefault();
    var body = { provider: {
      kind: kindEl.value,
      model: modelEl.value,
      max_tokens: Number(document.getElementById("max_tokens").value),
      temperature: Number(document.getElementById("temperature").value),
      endpoint: endpointEl.value,
    }};
    apiKeyPatch(body.provider);
    fetch("/v1/config", {
      method: "POST", headers: headers, body: JSON.stringify(body),
    }).then(function (r) {
      if (!r.ok) throw new Error("HTTP " + r.status);
      return r.json();
    }).then(function () {
      setStatus("Saved.", true);
      apiKeyEl.value = "";
      clearKeyEl.checked = false;
    }).catch(function (e) {
      setStatus("Save failed: " + e.message, false);
    });
  });
})();
</script>
</body>
</html>
"#;
