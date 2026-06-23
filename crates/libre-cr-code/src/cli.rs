//! CLI entry point. Mirrors `03-code-daemon.md` § CLI Surface.

use crate::config::Config;
use crate::mcp;
use crate::tools::{build_registry, ToolContext};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser, Debug)]
#[command(name = "libre-cr-code", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Speak MCP on stdin/stdout (default mode for parents).
    McpStdio,
    /// Speak MCP over a Unix domain socket.
    McpSocket {
        #[arg(long)]
        path: PathBuf,
    },
    /// Scan configured (or specified) roots and register the repos found.
    Scan {
        #[arg(long, value_delimiter = ',')]
        roots: Vec<String>,
    },
    /// Look up a repo by remote URL and print the local path (exit non-zero on miss).
    Discover { remote_url: String },
    /// Prepare a worktree and print its path.
    Prepare {
        repo_id: String,
        #[arg(name = "ref")]
        ref_name: String,
    },
    /// List all worktrees.
    Worktrees,
    /// Show what eviction would remove without doing it.
    Evict {
        #[arg(long)]
        dry_run: bool,
    },
    /// List all MCP tools and their schemas.
    Tools,
    /// Sanity check the environment.
    Doctor,
    /// Print version and exit.
    Version,
}

pub async fn run() -> anyhow::Result<()> {
    init_tracing();

    let cli = Cli::parse();
    let cfg = Config::load().unwrap_or_default();
    let ctx = ToolContext::new(cfg)?;

    match cli.command.unwrap_or(Command::McpStdio) {
        Command::McpStdio => {
            let registry = Arc::new(build_registry());
            // Spawn background eviction.
            ctx.worktrees.clone().spawn_eviction();
            mcp::run_stdio(ctx, registry).await?;
        }
        Command::McpSocket { path } => {
            let registry = Arc::new(build_registry());
            ctx.worktrees.clone().spawn_eviction();
            mcp::run_socket(&path, ctx, registry).await?;
        }
        Command::Scan { roots } => {
            let registry = build_registry();
            let tool = registry
                .get("scan_for_repos")
                .expect("scan_for_repos registered");
            let input = if roots.is_empty() {
                serde_json::json!({})
            } else {
                serde_json::json!({ "roots": roots })
            };
            let result = tool.call(ctx.clone(), input).await;
            match result {
                Ok(v) => println!("{}", serde_json::to_string_pretty(&v)?),
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            }
        }
        Command::Discover { remote_url } => {
            let registry = build_registry();
            let tool = registry
                .get("discover_repo")
                .expect("discover_repo registered");
            let result = tool
                .call(ctx, serde_json::json!({ "remote_url": remote_url }))
                .await;
            match result {
                Ok(v) if v.get("ok").and_then(|x| x.as_bool()).unwrap_or(false) => {
                    if let Some(p) = v.get("repo_path").and_then(|x| x.as_str()) {
                        println!("{p}");
                    }
                }
                Ok(v) => {
                    eprintln!("{}", serde_json::to_string(&v)?);
                    std::process::exit(2);
                }
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
        }
        Command::Prepare { repo_id, ref_name } => {
            let registry = build_registry();
            let tool = registry
                .get("prepare_worktree")
                .expect("prepare_worktree registered");
            let result = tool
                .call(
                    ctx,
                    serde_json::json!({ "repo_id": repo_id, "ref": ref_name }),
                )
                .await;
            match result {
                Ok(v) => println!("{}", serde_json::to_string_pretty(&v)?),
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
        }
        Command::Worktrees => {
            let registry = build_registry();
            let tool = registry
                .get("list_worktrees")
                .expect("list_worktrees registered");
            let result = tool.call(ctx, serde_json::json!({})).await;
            match result {
                Ok(v) => println!("{}", serde_json::to_string_pretty(&v)?),
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
        }
        Command::Evict { dry_run } => {
            let evicted = ctx.worktrees.evict(dry_run).await?;
            let payload = serde_json::json!({
                "dry_run": dry_run,
                "would_remove": evicted.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>(),
            });
            println!("{}", serde_json::to_string_pretty(&payload)?);
        }
        Command::Tools => {
            let registry = build_registry();
            let tools: Vec<_> = registry
                .all()
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "name": t.name(),
                        "description": t.description(),
                        "inputSchema": t.input_schema(),
                    })
                })
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({ "tools": tools }))?
            );
        }
        Command::Doctor => {
            doctor(&ctx)?;
        }
        Command::Version => {
            println!("libre-cr-code {}", env!("CARGO_PKG_VERSION"));
        }
    }

    Ok(())
}

fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .try_init();
}

fn doctor(ctx: &ToolContext) -> anyhow::Result<()> {
    let git_present = std::process::Command::new("git")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    let data_dir = ctx.data_dir.clone();
    let db_path = ctx.config.storage.state_db_path();

    let report = serde_json::json!({
        "git_cli": git_present,
        "data_dir": data_dir.to_string_lossy(),
        "data_dir_exists": data_dir.exists(),
        "state_db": db_path.to_string_lossy(),
        "state_db_exists": db_path.exists(),
        "registered_repos": ctx.registry.list_repos().map(|r| r.len()).unwrap_or(0),
        "grammars": serde_json::json!({
            "compiled_in": [],
            "note": "Phase 1.0 ships without grammars; ast_search/list_symbols/find_* return unsupported_language",
        }),
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
