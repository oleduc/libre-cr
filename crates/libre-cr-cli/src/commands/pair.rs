//! `libre-cr pair` — ask the running review daemon to mint a one-time
//! pairing code and print it.
//!
//! See `specs/04-review-daemon.md` § Pairing flow. Generating the code
//! locally is meaningless — the extension redeems against the *running*
//! daemon's `PairingStore`, which lives in-process. We POST to
//! `/v1/pair/issue` so the code lands where it can actually be redeemed.

use anyhow::Result;

use crate::config as cfg;

pub async fn run() -> Result<()> {
    let endpoint = cfg::read_endpoint()?.ok_or_else(|| {
        anyhow::anyhow!(
            "no daemon endpoint at {}. Run `libre-cr start` first.",
            crate::paths::endpoint_file().display()
        )
    })?;
    let token = cfg::read_token()?.ok_or_else(|| {
        anyhow::anyhow!(
            "no daemon token at {}. Run `libre-cr start` first.",
            crate::paths::token_file().display()
        )
    })?;
    let url = format!("{}/v1/pair/issue", endpoint.trim_end_matches('/'));
    let resp = reqwest::Client::new()
        .post(&url)
        .bearer_auth(&token)
        // Ask for the daemon's maximum (15 min): loading an extension and
        // filling the form routinely took longer than the 5-minute default.
        .json(&serde_json::json!({ "ttl_seconds": 900 }))
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("contact daemon at {url}: {e}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("daemon returned {status}: {body}");
    }
    let body: serde_json::Value = resp.json().await?;
    let code = body
        .get("code")
        .and_then(|s| s.as_str())
        .ok_or_else(|| anyhow::anyhow!("daemon response missing `code`"))?;
    println!("Pairing code: {code}");
    println!();
    println!("In the libre-cr browser extension's options page, click");
    println!("\"Pair with daemon\" and enter this code. The code is single-use");
    println!("and only valid while the daemon is running on this machine.");
    Ok(())
}
