//! `libre-cr config` — open the review daemon's config UI in a browser.

use anyhow::Result;

use crate::config as cfg;

pub async fn run() -> Result<()> {
    let endpoint = cfg::read_endpoint()?.ok_or_else(|| {
        anyhow::anyhow!(
            "no daemon endpoint found at {}. Run `libre-cr start` first.",
            crate::paths::endpoint_file().display()
        )
    })?;
    let token = cfg::read_token()?.ok_or_else(|| {
        anyhow::anyhow!(
            "no daemon token found at {}. Run `libre-cr start` first.",
            crate::paths::token_file().display()
        )
    })?;
    // The HTML page itself is public; the API it submits to is authenticated.
    // Pass the bearer token via `?token=...` so the page can attach it on
    // the in-browser fetch — same pattern the WS upgrade uses.
    let url = format!(
        "{}/config-ui?token={}",
        endpoint.trim_end_matches('/'),
        urlencode(&token)
    );
    println!("Opening config UI");
    // Best-effort: don't fail the call if there's no GUI session.
    let _ = webbrowser::open(&url);
    Ok(())
}

/// Tiny percent-encoder for the subset of characters that can appear in a
/// base64url-no-pad bearer token (`A-Z a-z 0-9 - _`). All of those are
/// already URL-safe, so we only need to escape anything unexpected. Keeps
/// us from pulling in a full URL crate.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'~' {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}
