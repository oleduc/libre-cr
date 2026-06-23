//! Canonicalize git remote URLs to `host/owner/repo` form. Per spec:
//!   `git@github.com:foo/bar.git` -> `github.com/foo/bar`
//!   `https://github.com/foo/bar.git` -> `github.com/foo/bar`
//!   case-fold; strip `.git`.

/// Canonicalize a remote URL. Returns `None` if the URL doesn't parse as a
/// git remote we recognize.
pub fn canonicalize_remote_url(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }

    // git@host:owner/repo(.git)
    if let Some(rest) = s.strip_prefix("git@") {
        if let Some((host, path)) = rest.split_once(':') {
            return Some(canon(host, path));
        }
    }

    // ssh://git@host/owner/repo(.git) or ssh://host/owner/repo
    if let Some(rest) = s.strip_prefix("ssh://") {
        let rest = rest.strip_prefix("git@").unwrap_or(rest);
        if let Some((host, path)) = rest.split_once('/') {
            return Some(canon(host, path));
        }
    }

    // https://host/owner/repo(.git) or http://...
    for prefix in ["https://", "http://", "git://"] {
        if let Some(rest) = s.strip_prefix(prefix) {
            // strip userinfo@ if present
            let rest = match rest.find('@') {
                Some(at) if rest[..at].chars().all(|c| c != '/') => &rest[at + 1..],
                _ => rest,
            };
            if let Some((host, path)) = rest.split_once('/') {
                // strip :port if present
                let host = host.split(':').next().unwrap_or(host);
                return Some(canon(host, path));
            }
        }
    }

    None
}

fn canon(host: &str, path: &str) -> String {
    let path = path.trim_start_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let path = path.trim_end_matches('/');
    format!("{}/{}", host.to_lowercase(), path.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_form() {
        assert_eq!(
            canonicalize_remote_url("git@github.com:foo/bar.git").as_deref(),
            Some("github.com/foo/bar")
        );
        assert_eq!(
            canonicalize_remote_url("git@github.com:Foo/Bar").as_deref(),
            Some("github.com/foo/bar")
        );
    }

    #[test]
    fn https_form() {
        assert_eq!(
            canonicalize_remote_url("https://github.com/foo/bar.git").as_deref(),
            Some("github.com/foo/bar")
        );
        assert_eq!(
            canonicalize_remote_url("https://github.com/foo/bar").as_deref(),
            Some("github.com/foo/bar")
        );
    }

    #[test]
    fn https_with_userinfo() {
        assert_eq!(
            canonicalize_remote_url("https://token@github.com/foo/bar.git").as_deref(),
            Some("github.com/foo/bar")
        );
    }

    #[test]
    fn ssh_url_form() {
        assert_eq!(
            canonicalize_remote_url("ssh://git@github.com/foo/bar.git").as_deref(),
            Some("github.com/foo/bar")
        );
    }

    #[test]
    fn case_folded() {
        assert_eq!(
            canonicalize_remote_url("https://GitHub.com/Foo/Bar.git").as_deref(),
            Some("github.com/foo/bar")
        );
    }

    #[test]
    fn rejects_garbage() {
        assert!(canonicalize_remote_url("").is_none());
        assert!(canonicalize_remote_url("not a url").is_none());
    }

    #[test]
    fn trailing_slash() {
        assert_eq!(
            canonicalize_remote_url("https://github.com/foo/bar/").as_deref(),
            Some("github.com/foo/bar")
        );
    }
}
