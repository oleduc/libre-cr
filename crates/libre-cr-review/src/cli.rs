//! CLI surface for the review daemon.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use base64::Engine;
use clap::{Parser, Subcommand};
use rand::RngCore;
use tracing::info;

use crate::code_daemon::SpawnedClient;
use crate::config::{expand_path, Config};
use crate::error::{Error, Result};
use crate::provider::{build_provider, Provider};
use crate::server::{serve, AppStateBuilder, ConfigStore};
use crate::storage::{InstallKey, Store};
use crate::tools::code_daemon::{CodeDaemonClient, MockCodeDaemonClient};

#[derive(Parser, Debug)]
#[command(name = "libre-cr-review", version, about, long_about = None)]
pub struct Cli {
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Run the HTTP/WS server (default).
    Serve,
    /// Speak MCP on stdin/stdout — external-client surface
    /// (`ask_about_pr`, `list_sessions`, …). Phase 4.
    McpStdio,
    /// Generate a pairing code and print it. The extension's options page
    /// POSTs it to `/v1/pair` to receive the token.
    Pair,
    /// Print version and exit.
    Version,
}

/// Entry point — wired from `main.rs`.
pub async fn run() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    let cfg_path = match cli.config {
        Some(p) => p,
        None => {
            // Pick up a review.toml stranded at the pre-fix macOS location
            // (Application Support); falls back to loading it in place if the
            // copy fails.
            Config::migrate_macos_legacy()
        }
    };
    match cli.command.unwrap_or(Command::Serve) {
        Command::Version => {
            println!("libre-cr-review {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Command::Serve => {
            let cfg = Config::load(&cfg_path)?;
            run_serve(cfg, cfg_path).await?;
            Ok(())
        }
        Command::McpStdio => {
            // Phase 4
            eprintln!("libre-cr-review mcp-stdio: not implemented (Phase 4)");
            Ok(())
        }
        Command::Pair => {
            let cfg = Config::load(&cfg_path)?;
            issue_pair_code_via_daemon(&cfg).await
        }
    }
}

/// Ask the running daemon to mint a one-time pairing code. The earlier
/// "local PairingStore" implementation issued codes into an in-process
/// store the running daemon never saw, breaking the extension's redeem
/// path. See REVIEW/00-certification.md C1.
async fn issue_pair_code_via_daemon(cfg: &Config) -> anyhow::Result<()> {
    let endpoint_path = expand_path(&cfg.server.endpoint_file);
    let token_path = expand_path(&cfg.server.token_file);
    let endpoint = read_trimmed(&endpoint_path).ok_or_else(|| {
        anyhow::anyhow!(
            "no daemon endpoint at {}. Start `libre-cr-review serve` first.",
            endpoint_path.display()
        )
    })?;
    let token = read_trimmed(&token_path).ok_or_else(|| {
        anyhow::anyhow!(
            "no daemon token at {}. Start `libre-cr-review serve` first.",
            token_path.display()
        )
    })?;
    let url = format!("{}/v1/pair/issue", endpoint.trim_end_matches('/'));
    let resp = reqwest::Client::new()
        .post(&url)
        .bearer_auth(&token)
        .json(&serde_json::json!({}))
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
    println!("{code}");
    Ok(())
}

fn read_trimmed(path: &std::path::Path) -> Option<String> {
    let s = std::fs::read_to_string(path).ok()?;
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

async fn run_serve(cfg: Config, cfg_path: PathBuf) -> Result<()> {
    // Token / endpoint / install_key bootstrap
    let token_path = expand_path(&cfg.server.token_file);
    let endpoint_path = expand_path(&cfg.server.endpoint_file);
    let install_key_path = expand_path(&cfg.server.install_key_file);

    let token = load_or_create_token(&token_path)?;
    let install_key = Arc::new(InstallKey::load_or_create(&install_key_path)?);

    let db_path = expand_path(&cfg.storage.db);
    let store = Store::open(&db_path)?;

    let provider: Arc<dyn Provider> = build_provider(&cfg, &install_key)?;
    // Real code-daemon client, with a graceful fallback to the mock if the
    // child can't be spawned (e.g. binary not on PATH yet — common during
    // first-run setup). Production deployments expect a real spawn.
    let (code_daemon, health_hook): (Arc<dyn CodeDaemonClient>, Option<crate::server::HealthHook>) =
        match SpawnedClient::from_config(&cfg.code_daemon).await {
            Ok(c) => {
                let arc = Arc::new(c);
                let arc_for_hook = arc.clone();
                let hook: crate::server::HealthHook = Arc::new(move || {
                    let a = arc_for_hook.clone();
                    Box::pin(async move { a.health().await })
                });
                (arc as Arc<dyn CodeDaemonClient>, Some(hook))
            }
            Err(e) => {
                tracing::warn!(error = %e, "code daemon unavailable; falling back to mock");
                (
                    Arc::new(MockCodeDaemonClient) as Arc<dyn CodeDaemonClient>,
                    None,
                )
            }
        };

    let extension_origin = cfg.server.extension_origin.clone();
    let state = AppStateBuilder {
        store,
        config: ConfigStore::new(cfg.clone()),
        provider,
        code_daemon,
        token,
        extension_origin,
        install_key,
        health_hook,
        config_path: Some(cfg_path.clone()),
    }
    .build();

    let addr: SocketAddr = format!("{}:{}", cfg.server.bind, cfg.server.port)
        .parse()
        .map_err(|e| Error::Internal(format!("parse bind addr: {e}")))?;
    let info = serve(state, addr).await?;
    if let Some(parent) = endpoint_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&endpoint_path, format!("http://{}", info.addr));
    info!("review daemon listening on {}", info.addr);
    info.task
        .await
        .map_err(|e| Error::Internal(format!("join: {e}")))??;
    Ok(())
}

fn load_or_create_token(path: &std::path::Path) -> Result<String> {
    if path.exists() {
        let s = std::fs::read_to_string(path)?;
        let trimmed = s.trim().to_string();
        if !trimmed.is_empty() {
            return Ok(trimmed);
        }
    }
    let mut buf = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut buf);
    let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, &token)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path)?.permissions();
        perms.set_mode(0o600);
        std::fs::set_permissions(path, perms)?;
    }
    Ok(token)
}
