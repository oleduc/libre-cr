//! Agent loop and `FrameSink`. See `specs/04-review-daemon.md` § Agent Loop.

mod loop_;
mod sink;

pub use loop_::{persist_cancelled, run_turn, TurnContext, TurnInput, TurnResult};
pub use sink::{FrameSink, RecordingSink};
