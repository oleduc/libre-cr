//! Config loading. Defaults per `03-code-daemon.md` § Configuration.

use crate::util::expand_path;
use serde::Deserialize;
use std::path::PathBuf;

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub storage: StorageConfig,
    #[serde(default)]
    pub worktrees: WorktreeConfig,
    #[serde(default)]
    pub discovery: DiscoveryConfig,
    #[serde(default)]
    pub grep: GrepConfig,
    #[serde(default)]
    pub ast_grep: AstGrepConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StorageConfig {
    pub data_dir: String,
    pub state_db: String,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            data_dir: "~/.local/share/libre-cr-code".to_string(),
            state_db: "~/.local/share/libre-cr-code/state.db".to_string(),
        }
    }
}

impl StorageConfig {
    pub fn data_dir_path(&self) -> PathBuf {
        expand_path(&self.data_dir)
    }
    pub fn state_db_path(&self) -> PathBuf {
        expand_path(&self.state_db)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorktreeConfig {
    pub max_total_bytes: u64,
    pub eviction_check_interval_secs: u64,
}

impl Default for WorktreeConfig {
    fn default() -> Self {
        Self {
            max_total_bytes: 5_368_709_120,
            eviction_check_interval_secs: 3600,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct DiscoveryConfig {
    pub default_roots: Vec<String>,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            default_roots: vec![
                "~/code".to_string(),
                "~/Dev".to_string(),
                "~/src".to_string(),
            ],
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct GrepConfig {
    pub default_max_matches: usize,
}

impl Default for GrepConfig {
    fn default() -> Self {
        Self {
            default_max_matches: 200,
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct AstGrepConfig {
    pub ast_cache_size: usize,
}

impl Default for AstGrepConfig {
    fn default() -> Self {
        Self {
            ast_cache_size: 256,
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize, Default)]
pub struct LoggingConfig {
    #[serde(default)]
    pub file: bool,
}

impl Config {
    /// Default config file path. Per `specs/08-distribution.md` § Configuration
    /// Layout, all wrapper-managed configs live under `~/.config/libre-cr/`.
    pub fn default_path() -> PathBuf {
        if let Some(home) = dirs::config_dir() {
            home.join("libre-cr").join("code.toml")
        } else {
            PathBuf::from("./code.toml")
        }
    }

    /// Legacy path used before the wrapper migrated configs into the shared
    /// `~/.config/libre-cr/` namespace. Kept so a one-time migration on
    /// startup can pick up the user's existing file.
    pub fn legacy_path() -> PathBuf {
        if let Some(home) = dirs::config_dir() {
            home.join("libre-cr-code").join("config.toml")
        } else {
            PathBuf::from("./config.toml")
        }
    }

    /// Load from a path; return defaults if it doesn't exist.
    pub fn load_from(path: &std::path::Path) -> anyhow::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path)?;
        let cfg: Self = toml::from_str(&text)?;
        Ok(cfg)
    }

    /// Load from the default config path. If only the legacy
    /// `~/.config/libre-cr-code/config.toml` exists, copy it into the new
    /// location and load from there.
    pub fn load() -> anyhow::Result<Self> {
        let new_path = Self::default_path();
        let legacy_path = Self::legacy_path();
        if !new_path.exists() && legacy_path.exists() {
            if let Some(parent) = new_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match std::fs::copy(&legacy_path, &new_path) {
                Ok(_) => {
                    tracing::info!(
                        from = %legacy_path.display(),
                        to = %new_path.display(),
                        "migrated code-daemon config to shared libre-cr namespace"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        from = %legacy_path.display(),
                        to = %new_path.display(),
                        error = %e,
                        "could not migrate legacy code-daemon config; loading legacy in place"
                    );
                    return Self::load_from(&legacy_path);
                }
            }
        }
        Self::load_from(&new_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_spec() {
        let cfg = Config::default();
        assert_eq!(cfg.worktrees.max_total_bytes, 5_368_709_120);
        assert_eq!(cfg.worktrees.eviction_check_interval_secs, 3600);
        assert_eq!(cfg.grep.default_max_matches, 200);
        assert_eq!(cfg.ast_grep.ast_cache_size, 256);
        assert_eq!(cfg.discovery.default_roots.len(), 3);
    }

    #[test]
    fn loads_partial_toml() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            tmp.path(),
            r#"
[grep]
default_max_matches = 50
"#,
        )
        .unwrap();
        let cfg = Config::load_from(tmp.path()).unwrap();
        assert_eq!(cfg.grep.default_max_matches, 50);
        // other defaults preserved
        assert_eq!(cfg.worktrees.max_total_bytes, 5_368_709_120);
    }

    #[test]
    fn missing_file_returns_defaults() {
        let cfg = Config::load_from(std::path::Path::new("/nonexistent/path/config.toml")).unwrap();
        assert_eq!(cfg.grep.default_max_matches, 200);
    }

    #[test]
    fn default_path_lives_in_shared_libre_cr_namespace() {
        let p = Config::default_path();
        let s = p.to_string_lossy();
        // Per specs/08-distribution.md § Configuration Layout: the file
        // belongs under `~/.config/libre-cr/`, not `~/.config/libre-cr-code/`.
        assert!(
            s.contains("libre-cr") && s.ends_with("code.toml"),
            "unexpected default_path: {s}"
        );
        assert!(
            !s.contains("libre-cr-code"),
            "default_path must not use the old libre-cr-code namespace: {s}"
        );
    }
}
