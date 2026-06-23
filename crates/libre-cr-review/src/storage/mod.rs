//! SQLite-backed storage. Schema per `specs/04-review-daemon.md` § Conversation Storage.

mod crypto;
mod migrations;
mod model;
mod store;

pub use crypto::{decrypt_value, encrypt_value, InstallKey};
pub use migrations::SCHEMA_VERSION;
pub use model::{Note, Session, Severity, ToolTrace, Turn, TurnKind, TurnStatus};
pub use store::Store;
