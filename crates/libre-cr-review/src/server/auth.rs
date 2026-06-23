//! Bearer-token + origin auth middleware.

use axum::{
    extract::State,
    http::{HeaderMap, Request, StatusCode},
    middleware::Next,
    response::Response,
};
use subtle::ConstantTimeEq;

use crate::error::Error;

use super::state::AppState;

// `/config-ui` is a static HTML page with no secrets in markup; the form it
// submits to (`POST /v1/config`) remains token-authenticated. The page reads
// the token from `?token=...` (same trick as the WS upgrade) and stuffs it
// into a `Bearer` header on submit.
const SKIP_AUTH_PATHS: &[&str] = &["/v1/health", "/v1/pair", "/config-ui"];

pub async fn auth_middleware(
    State(state): State<AppState>,
    req: Request<axum::body::Body>,
    next: Next,
) -> std::result::Result<Response, Error> {
    let path = req.uri().path().to_string();
    let skip = SKIP_AUTH_PATHS.iter().any(|p| path == *p);
    if skip {
        return Ok(next.run(req).await);
    }
    let headers = req.headers();
    let query = req.uri().query().unwrap_or("");
    let is_ws_upgrade = headers
        .get("upgrade")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("websocket"));
    check_token(headers, query, is_ws_upgrade, &state.token).map_err(|_| Error::Unauthorized)?;
    // Origin allowlist may change at runtime via /v1/pair; read fresh.
    let allowed = state.config.snapshot().await.server.extension_origin;
    let allowed_str = if allowed.is_empty() {
        state.extension_origin.clone()
    } else {
        allowed
    };
    check_origin(headers, &allowed_str).map_err(|_| Error::OriginRejected)?;
    Ok(next.run(req).await)
}

/// Token check. Header `Authorization: Bearer <t>` is the default. For WebSocket
/// upgrades a `?token=<t>` query-string fallback is also accepted, because the
/// browser `WebSocket` constructor doesn't expose request headers — see
/// `specs/05-browser-extension.md` § Transport from a Content Script.
fn check_token(
    headers: &HeaderMap,
    query: &str,
    is_ws_upgrade: bool,
    expected: &str,
) -> std::result::Result<(), StatusCode> {
    if expected.is_empty() {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let auth = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let prefix = "Bearer ";
    if let Some(presented) = auth.strip_prefix(prefix) {
        return if tokens_eq(presented, expected) {
            Ok(())
        } else {
            Err(StatusCode::UNAUTHORIZED)
        };
    }
    if is_ws_upgrade {
        if let Some(t) = token_from_query(query) {
            return if tokens_eq(&t, expected) {
                Ok(())
            } else {
                Err(StatusCode::UNAUTHORIZED)
            };
        }
    }
    Err(StatusCode::UNAUTHORIZED)
}

/// Constant-time bearer-token comparison. Rejects on length mismatch in
/// constant time (length is not secret, but using `ct_eq` keeps the call
/// site uniform). Without this, `==` short-circuits on the first byte and
/// leaks the secret prefix via response timing.
fn tokens_eq(a: &str, b: &str) -> bool {
    a.as_bytes().ct_eq(b.as_bytes()).into()
}

fn token_from_query(query: &str) -> Option<String> {
    for pair in query.split('&') {
        if let Some(v) = pair.strip_prefix("token=") {
            // Minimal percent-decode for the few likely-present characters.
            let decoded = v
                .replace("%2B", "+")
                .replace("%2F", "/")
                .replace("%3D", "=");
            return Some(decoded);
        }
    }
    None
}

fn check_origin(headers: &HeaderMap, allowed: &str) -> std::result::Result<(), StatusCode> {
    // If no origin allowlist is configured, accept anything once we have a
    // valid token (covers `curl`, internal tests, MCP-CLI). The extension
    // origin only kicks in once paired.
    if allowed.is_empty() {
        return Ok(());
    }
    let origin = headers
        .get("origin")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if origin.is_empty() {
        return Ok(());
    }
    if origin == allowed {
        return Ok(());
    }
    Err(StatusCode::FORBIDDEN)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn header_token_required() {
        let mut h = HeaderMap::new();
        assert!(check_token(&h, "", false, "abc").is_err());
        h.insert("authorization", HeaderValue::from_static("Bearer abc"));
        assert!(check_token(&h, "", false, "abc").is_ok());
        h.insert("authorization", HeaderValue::from_static("Bearer wrong"));
        assert!(check_token(&h, "", false, "abc").is_err());
    }

    #[test]
    fn ws_upgrade_accepts_query_token() {
        let h = HeaderMap::new();
        // No Authorization header, but Upgrade: websocket and matching query.
        assert!(check_token(&h, "token=abc", true, "abc").is_ok());
        assert!(check_token(&h, "token=wrong", true, "abc").is_err());
        // Without the upgrade flag, query is ignored.
        assert!(check_token(&h, "token=abc", false, "abc").is_err());
    }

    #[test]
    fn empty_expected_token_always_rejects() {
        let h = HeaderMap::new();
        assert!(check_token(&h, "", false, "").is_err());
        assert!(check_token(&h, "token=anything", true, "").is_err());
    }

    #[test]
    fn token_from_query_picks_token_param() {
        assert_eq!(token_from_query("token=abc").as_deref(), Some("abc"));
        assert_eq!(
            token_from_query("a=1&token=abc&b=2").as_deref(),
            Some("abc")
        );
        assert_eq!(token_from_query("a=1&b=2").as_deref(), None);
        assert_eq!(token_from_query("").as_deref(), None);
    }

    #[test]
    fn origin_allowed_when_unconfigured() {
        let h = HeaderMap::new();
        assert!(check_origin(&h, "").is_ok());
    }

    #[test]
    fn origin_rejected_on_mismatch() {
        let mut h = HeaderMap::new();
        h.insert("origin", HeaderValue::from_static("https://evil"));
        assert!(check_origin(&h, "chrome-extension://x").is_err());
    }

    #[test]
    fn origin_accepted_on_match() {
        let mut h = HeaderMap::new();
        h.insert("origin", HeaderValue::from_static("chrome-extension://x"));
        assert!(check_origin(&h, "chrome-extension://x").is_ok());
    }
}
