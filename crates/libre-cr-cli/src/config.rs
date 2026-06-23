//! Tiny helpers around the daemon's endpoint + token files. We deliberately
//! do not re-parse `review.toml` here — the review daemon owns that schema
//! and the wrapper just needs to *find* the daemon, not configure it.

use anyhow::Result;

use crate::paths;

/// Read the endpoint URL written by the daemon on first start. Returns
/// `Ok(None)` if the file is missing or empty.
pub fn read_endpoint() -> Result<Option<String>> {
    let path = paths::endpoint_file();
    if !path.exists() {
        return Ok(None);
    }
    let s = std::fs::read_to_string(&path)?;
    let trimmed = s.trim();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed.to_string()))
    }
}

/// Read the bearer token. Returns `Ok(None)` if the file is missing.
pub fn read_token() -> Result<Option<String>> {
    let path = paths::token_file();
    if !path.exists() {
        return Ok(None);
    }
    let s = std::fs::read_to_string(&path)?;
    let trimmed = s.trim();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed.to_string()))
    }
}
