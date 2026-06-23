//! Tool router. Three categories per spec § Tool Router:
//!   1. Code-daemon tools (mocked in Phase 2)
//!   2. Internal review-daemon tools (PR-aware)
//!   3. Presentation tools (only when there is an active WS)

pub mod code_daemon;
pub mod internal;
pub mod presentation;
pub mod router;

pub use code_daemon::{CodeDaemonClient, MockCodeDaemonClient};
pub use presentation::{PresentationDispatcher, PresentationOutcome};
pub use router::{ToolCall, ToolOutcome, ToolRouter};
