//! ChatGPT subscription sign-in: OAuth 2.0 with PKCE, against OpenAI's
//! published Codex client. See `specs/04-review-daemon.md`
//! § ChatGPT subscription provider.
//!
//! The client id and redirect URI are OpenAI's, not ours — the redirect is
//! registered against that client, so the callback must land on port 1455.
//! This is the same flow OpenAI's own Codex CLI runs.

use std::path::Path;

use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

pub const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub const AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
pub const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
pub const REDIRECT_URI: &str = "http://localhost:1455/auth/callback";
pub const CALLBACK_PORT: u16 = 1455;
/// Identifies this client to OpenAI. Ours, not a borrowed one.
pub const ORIGINATOR: &str = "libre_cr";
/// Refresh this long before expiry rather than racing it.
const REFRESH_MARGIN_MS: i64 = 30_000;

/// What the sign-in produces, and what the daemon stores.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Tokens {
    pub access: String,
    pub refresh: String,
    /// Unix epoch milliseconds, as OpenAI reports it.
    pub expires_ms: i64,
    /// ChatGPT account the tokens belong to; sent as `ChatGPT-Account-Id`.
    #[serde(default)]
    pub account_id: String,
}

impl Tokens {
    pub fn expired_within(&self, now_ms: i64, margin_ms: i64) -> bool {
        self.expires_ms - margin_ms <= now_ms
    }
    pub fn needs_refresh(&self, now_ms: i64) -> bool {
        self.expired_within(now_ms, REFRESH_MARGIN_MS)
    }
}

/// A PKCE pair. The verifier is kept until the code comes back; the challenge
/// goes in the authorize URL.
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

fn b64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

impl Pkce {
    pub fn generate() -> Self {
        use rand::RngCore;
        let mut raw = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut raw);
        let verifier = b64url(&raw);
        Self {
            challenge: challenge_for(&verifier),
            verifier,
        }
    }
}

/// S256: base64url(sha256(verifier)), unpadded.
pub fn challenge_for(verifier: &str) -> String {
    use sha2::{Digest, Sha256};
    b64url(&Sha256::digest(verifier.as_bytes()))
}

/// The URL the user opens to sign in. The last three parameters are OpenAI's
/// own, as the Codex CLI sends them; without them the consent screen rejects
/// the request.
pub fn authorize_url(challenge: &str, state: &str) -> String {
    let q = [
        ("response_type", "code"),
        ("client_id", CLIENT_ID),
        ("redirect_uri", REDIRECT_URI),
        ("scope", "openid profile email offline_access"),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
        ("state", state),
        ("id_token_add_organizations", "true"),
        ("codex_cli_simplified_flow", "true"),
        ("originator", ORIGINATOR),
    ]
    .iter()
    .map(|(k, v)| format!("{k}={}", urlencode(v)))
    .collect::<Vec<_>>()
    .join("&");
    format!("{AUTHORIZE_URL}?{q}")
}

/// Percent-encode a query value. Small enough not to pull in a URL crate for.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Pull the ChatGPT account id out of an id token. It is a JWT: the middle
/// segment is base64url JSON. We read the claims, we do not verify the
/// signature — the token came from the token endpoint over TLS, and we are
/// only using it to label the account, never to authorize anything.
pub fn account_id_from_id_token(id_token: &str) -> Option<String> {
    let payload = id_token.split('.').nth(1)?;
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&raw).ok()?;
    // Three shapes seen in the wild, newest first.
    for path in [
        &["https://api.openai.com/auth", "chatgpt_account_id"][..],
        &["https://api.openai.com/auth", "user_id"][..],
        &["chatgpt_account_id"][..],
    ] {
        let mut cur = &claims;
        for key in path {
            match cur.get(*key) {
                Some(v) => cur = v,
                None => {
                    cur = &serde_json::Value::Null;
                    break;
                }
            }
        }
        if let Some(s) = cur.as_str() {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

/// Turn a token-endpoint response into [`Tokens`]. `now_ms` is passed in so
/// the expiry maths is testable.
pub fn tokens_from_response(
    body: &serde_json::Value,
    now_ms: i64,
    previous_refresh: Option<&str>,
) -> Result<Tokens> {
    let access = body
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::Validation("token response has no access_token".into()))?;
    // A refresh response may omit the refresh token, meaning "keep the one you
    // have". Dropping it there would sign the user out an hour later.
    let refresh = body
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .or(previous_refresh)
        .unwrap_or_default();
    let expires_in = body
        .get("expires_in")
        .and_then(|v| v.as_i64())
        .unwrap_or(3600);
    let account_id = body
        .get("id_token")
        .and_then(|v| v.as_str())
        .and_then(account_id_from_id_token)
        .unwrap_or_default();
    Ok(Tokens {
        access: access.to_string(),
        refresh: refresh.to_string(),
        expires_ms: now_ms + expires_in * 1000,
        account_id,
    })
}

/// Read stored tokens. A missing file is "signed out", not an error.
pub fn load_tokens(path: &Path) -> Result<Option<Tokens>> {
    match std::fs::read(path) {
        Ok(raw) => Ok(Some(serde_json::from_slice(&raw).map_err(|e| {
            Error::Validation(format!(
                "chatgpt-auth.json is unreadable ({e}); sign in again"
            ))
        })?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::Internal(format!("read chatgpt-auth.json: {e}"))),
    }
}

/// Write tokens `0600`. Same treatment as the daemon's other secrets.
pub fn save_tokens(path: &Path, tokens: &Tokens) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| Error::Internal(format!("create dir: {e}")))?;
    }
    let json = serde_json::to_vec_pretty(tokens)?;
    std::fs::write(path, json).map_err(|e| Error::Internal(format!("write tokens: {e}")))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| Error::Internal(format!("chmod tokens: {e}")))?;
    }
    Ok(())
}

pub fn clear_tokens(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(Error::Internal(format!("remove tokens: {e}"))),
    }
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Exchange an authorization code for tokens.
pub async fn exchange_code(client: &reqwest::Client, code: &str, verifier: &str) -> Result<Tokens> {
    let body = serde_json::json!({
        "grant_type": "authorization_code",
        "client_id": CLIENT_ID,
        "code": code,
        "redirect_uri": REDIRECT_URI,
        "code_verifier": verifier,
    });
    let v = post_token(client, &body).await?;
    tokens_from_response(&v, now_ms(), None)
}

/// Redeem a refresh token. Keeps the old refresh token when the response
/// omits one.
pub async fn refresh_tokens(client: &reqwest::Client, current: &Tokens) -> Result<Tokens> {
    let body = serde_json::json!({
        "grant_type": "refresh_token",
        "client_id": CLIENT_ID,
        "refresh_token": current.refresh,
        "scope": "openid profile email offline_access",
    });
    let v = post_token(client, &body).await?;
    let mut next = tokens_from_response(&v, now_ms(), Some(&current.refresh))?;
    if next.account_id.is_empty() {
        next.account_id = current.account_id.clone();
    }
    Ok(next)
}

async fn post_token(
    client: &reqwest::Client,
    body: &serde_json::Value,
) -> Result<serde_json::Value> {
    let resp = client
        .post(TOKEN_URL)
        .json(body)
        .send()
        .await
        .map_err(|e| Error::Internal(format!("chatgpt token request: {e}")))?;
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED
        || resp.status() == reqwest::StatusCode::BAD_REQUEST
    {
        return Err(Error::ProviderUnauthorized);
    }
    if !resp.status().is_success() {
        let s = resp.status();
        return Err(Error::Internal(format!("chatgpt token status: {s}")));
    }
    resp.json()
        .await
        .map_err(|e| Error::Internal(format!("chatgpt token parse: {e}")))
}

/// A sign-in waiting on the browser. Holding the bound listener is the point:
/// the port is checked before the user is sent to OpenAI, so a port clash is
/// reported instead of producing a redirect that lands nowhere.
pub struct PendingSignIn {
    listener: tokio::net::TcpListener,
    state: String,
}

/// True while a sign-in is waiting for its callback. Read by
/// `GET /v1/provider/chatgpt/status` so the config UI can say "waiting for
/// the browser" rather than "signed out".
static PENDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn sign_in_pending() -> bool {
    PENDING.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn clear_pending() {
    PENDING.store(false, std::sync::atomic::Ordering::Relaxed);
}

/// Bind OpenAI's registered callback port. Call before handing the authorize
/// URL to the user.
pub async fn begin_callback(state: &str) -> Result<PendingSignIn> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", CALLBACK_PORT))
        .await
        .map_err(|e| {
            Error::Validation(format!(
                "port {CALLBACK_PORT} is not available ({e}); it is the redirect URI OpenAI \
                 registered for this client, so the sign-in cannot use another one"
            ))
        })?;
    PENDING.store(true, std::sync::atomic::Ordering::Relaxed);
    Ok(PendingSignIn {
        listener,
        state: state.to_string(),
    })
}

/// Wait for the redirect, then exchange the code. The listener answers exactly
/// one callback and closes.
pub async fn finish_callback(
    pending: PendingSignIn,
    client: &reqwest::Client,
    verifier: &str,
) -> Result<Tokens> {
    let code = tokio::time::timeout(
        std::time::Duration::from_secs(300),
        await_code(&pending.listener, &pending.state),
    )
    .await
    .map_err(|_| Error::Validation("sign-in timed out".into()))??;
    exchange_code(client, &code, verifier).await
}

async fn await_code(listener: &tokio::net::TcpListener, expected_state: &str) -> Result<String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    loop {
        let (mut sock, _) = listener
            .accept()
            .await
            .map_err(|e| Error::Internal(format!("callback accept: {e}")))?;
        let mut buf = vec![0u8; 8192];
        let n = sock
            .read(&mut buf)
            .await
            .map_err(|e| Error::Internal(format!("callback read: {e}")))?;
        let req = String::from_utf8_lossy(&buf[..n]).to_string();
        let target = req
            .split_whitespace()
            .nth(1)
            .unwrap_or_default()
            .to_string();
        // The browser also asks for /favicon.ico; answer it and keep waiting.
        if !target.starts_with("/auth/callback") {
            let _ = sock.write_all(b"HTTP/1.1 404 Not Found\r\n\r\n").await;
            continue;
        }
        let params = query_params(&target);
        let page: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n\
            <html><body style=\"font:14px system-ui;padding:3rem\">\
            <h3>Signed in to libre-cr</h3><p>You can close this tab.</p>\
            </body></html>";
        let _ = sock.write_all(page).await;
        let _ = sock.shutdown().await;
        if let Some((_, msg)) = params.iter().find(|(k, _)| k == "error") {
            return Err(Error::Validation(format!("sign-in refused: {msg}")));
        }
        let state = params.iter().find(|(k, _)| k == "state").map(|(_, v)| v);
        if state.map(String::as_str) != Some(expected_state) {
            return Err(Error::Validation("sign-in state mismatch".into()));
        }
        return params
            .iter()
            .find(|(k, _)| k == "code")
            .map(|(_, v)| v.clone())
            .ok_or_else(|| Error::Validation("callback carried no code".into()));
    }
}

fn query_params(target: &str) -> Vec<(String, String)> {
    let Some((_, q)) = target.split_once('?') else {
        return Vec::new();
    };
    q.split('&')
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| (k.to_string(), urldecode(v)))
        .collect()
}

fn urldecode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                let hex = std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(v) => {
                        out.push(v);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(b[i]);
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_matches_rfc_7636_example() {
        // The example pair from RFC 7636 appendix B.
        assert_eq!(
            challenge_for("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn authorize_url_carries_the_openai_specific_parameters() {
        let url = authorize_url("chal", "st");
        for needle in [
            "client_id=app_EMoamEEZ73f0CkXaXp7hrann",
            "code_challenge_method=S256",
            "id_token_add_organizations=true",
            "codex_cli_simplified_flow=true",
            "redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback",
        ] {
            assert!(url.contains(needle), "missing {needle} in {url}");
        }
    }

    #[test]
    fn account_id_comes_from_the_id_token_claims() {
        let claims = serde_json::json!({
            "https://api.openai.com/auth": { "chatgpt_account_id": "acc_123" }
        });
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&claims).unwrap());
        let jwt = format!("header.{payload}.signature");
        assert_eq!(account_id_from_id_token(&jwt).as_deref(), Some("acc_123"));
        assert_eq!(account_id_from_id_token("not a jwt"), None);
    }

    /// A refresh response that omits `refresh_token` means "keep yours".
    /// Dropping it would sign the user out an hour later.
    #[test]
    fn refresh_keeps_the_previous_refresh_token() {
        let body = serde_json::json!({ "access_token": "new", "expires_in": 3600 });
        let t = tokens_from_response(&body, 1_000, Some("old-refresh")).unwrap();
        assert_eq!(t.refresh, "old-refresh");
        assert_eq!(t.expires_ms, 1_000 + 3_600_000);
    }

    #[test]
    fn refresh_is_due_before_expiry_not_after() {
        let t = Tokens {
            access: "a".into(),
            refresh: "r".into(),
            expires_ms: 100_000,
            account_id: String::new(),
        };
        assert!(!t.needs_refresh(60_000));
        assert!(t.needs_refresh(80_000), "should refresh inside the margin");
        assert!(t.needs_refresh(100_001));
    }

    #[test]
    fn tokens_round_trip_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("chatgpt-auth.json");
        assert!(
            load_tokens(&path).unwrap().is_none(),
            "missing = signed out"
        );
        let t = Tokens {
            access: "a".into(),
            refresh: "r".into(),
            expires_ms: 5,
            account_id: "acc".into(),
        };
        save_tokens(&path, &t).unwrap();
        assert_eq!(load_tokens(&path).unwrap().unwrap(), t);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        clear_tokens(&path).unwrap();
        assert!(load_tokens(&path).unwrap().is_none());
    }

    #[test]
    fn callback_query_is_decoded() {
        let p = query_params("/auth/callback?code=abc%2Fdef&state=xy");
        assert_eq!(p[0], ("code".into(), "abc/def".into()));
        assert_eq!(p[1], ("state".into(), "xy".into()));
    }
}
