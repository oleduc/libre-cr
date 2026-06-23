//! `libre-cr` wrapper CLI library. The binary is a thin entrypoint into
//! [`run`]. Modules expose the supervisor + commands as testable units.
//!
//! See `specs/08-distribution.md` § Wrapper CLI Surface and § Supervision
//! Model for the binding spec.

pub mod commands;
pub mod config;
pub mod doctor;
pub mod logs;
pub mod paths;
pub mod proc;
pub mod supervisor;
pub mod update;

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "libre-cr", version, about = "Libre CR — code review companion", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Start both daemons (idempotent).
    Start {
        /// Set up the OS autostart hook (stub — Phase 7.5).
        #[arg(long)]
        autostart: bool,
    },
    /// Stop both daemons gracefully.
    Stop,
    /// Restart both daemons.
    Restart,
    /// Show health, version, ports, PIDs.
    Status,
    /// Tail both daemons' logs.
    Logs {
        #[arg(short = 'f', long)]
        follow: bool,
        #[arg(short = 'n', long, default_value_t = 200)]
        lines: usize,
    },
    /// Generate a one-time pairing code.
    Pair,
    /// Open the review daemon's config UI in your browser.
    Config,
    /// Diagnose: ports, file perms, code-daemon health.
    Doctor,
    /// Check for updates; apply if user confirms.
    Update,
    /// Stop daemons and (optionally) remove data + logs.
    Uninstall {
        /// Skip confirmation prompts.
        #[arg(long)]
        force: bool,
    },
    /// Print version and exit.
    Version,
}

/// Top-level entrypoint used by `main` and integration tests.
pub async fn run() -> anyhow::Result<()> {
    init_tracing();
    let cli = Cli::parse();
    dispatch(cli.command).await
}

/// Dispatch a parsed command. Exposed for testing.
pub async fn dispatch(cmd: Command) -> anyhow::Result<()> {
    match cmd {
        Command::Start { autostart } => commands::start::run(autostart).await,
        Command::Stop => commands::stop::run().await,
        Command::Restart => commands::restart::run().await,
        Command::Status => commands::status::run().await,
        Command::Logs { follow, lines } => commands::logs::run(follow, lines).await,
        Command::Pair => commands::pair::run().await,
        Command::Config => commands::config::run().await,
        Command::Doctor => commands::doctor::run().await,
        Command::Update => commands::update::run().await,
        Command::Uninstall { force } => commands::uninstall::run(force).await,
        Command::Version => commands::version::run().await,
    }
}

fn init_tracing() {
    // Idempotent: ignore "already initialized" errors so tests can re-enter.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .try_init();
}
