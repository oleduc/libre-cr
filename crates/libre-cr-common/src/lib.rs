//! Shared types for the Libre CR workspace.
//!
//! Anything that crosses the boundary between two of:
//!   - the code daemon (`libre-cr-code`),
//!   - the review daemon (`libre-cr-review`),
//!   - the wrapper CLI (`libre-cr-cli`),
//!   - the browser extension (via the daemon's HTTP/WS API)
//!
//! lives here. Higher-level domain types stay in their owning crate.

pub mod error;
pub mod http_api;
pub mod selection;
pub mod version;
// review daemon
pub mod ws_frames;

pub use error::{ErrorCategory, ErrorEnvelope};
pub use selection::Selection;
pub use version::PROTOCOL_VERSION;
