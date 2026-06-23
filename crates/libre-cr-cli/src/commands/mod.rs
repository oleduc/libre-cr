//! One module per CLI subcommand. Each exposes `pub async fn run(...) -> Result<()>`.

pub mod config;
pub mod doctor;
pub mod logs;
pub mod pair;
pub mod restart;
pub mod start;
pub mod status;
pub mod stop;
pub mod uninstall;
pub mod update;
pub mod version;
