// review daemon
//! WebSocket frame types exchanged on `/v1/sessions/:id/ask`.
//!
//! Per `04-review-daemon.md` § HTTP / WebSocket API and
//! `09-presentation-tools.md` § Protocol.
//!
//! Shared so future test clients in other crates can parse them too.

use serde::{Deserialize, Serialize};

use crate::selection::Selection;

/// First frame from the client opening the WS.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AskInit {
    pub question: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<Selection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verb: Option<String>,
    /// Per-session presentation override. When `true`, the daemon excludes
    /// presentation tools from the turn's tool registration, so the model
    /// can't emit `presentation_call` frames for this turn.
    #[serde(default)]
    pub mute_presentations: bool,
}

/// Usage tally returned at the end of a turn.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct UsageTally {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
}

/// Server → client frame.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerFrame {
    TextDelta {
        text: String,
    },
    ToolCall {
        call_id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        call_id: String,
        result_preview: serde_json::Value,
    },
    PresentationCall {
        call_id: String,
        tool: String,
        input: serde_json::Value,
    },
    Done {
        turn_id: String,
        usage: UsageTally,
    },
    Error {
        message: String,
        recoverable: bool,
    },
}

/// Client → server frame (after the initial `AskInit`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientFrame {
    PresentationResult {
        call_id: String,
        ok: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result: Option<serde_json::Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_frame_text_delta_serializes() {
        let f = ServerFrame::TextDelta {
            text: "hello".into(),
        };
        let s = serde_json::to_string(&f).unwrap();
        assert!(s.contains("\"type\":\"text_delta\""));
        assert!(s.contains("\"text\":\"hello\""));
    }

    #[test]
    fn ask_init_round_trip() {
        let a = AskInit {
            question: "where?".into(),
            selection: None,
            verb: Some("find_callers".into()),
            mute_presentations: false,
        };
        let s = serde_json::to_string(&a).unwrap();
        let b: AskInit = serde_json::from_str(&s).unwrap();
        assert_eq!(b.question, "where?");
        assert_eq!(b.verb.as_deref(), Some("find_callers"));
        assert!(!b.mute_presentations);
    }

    #[test]
    fn ask_init_mute_presentations_defaults_false_and_parses_true() {
        // Older clients omit the field entirely.
        let b: AskInit = serde_json::from_str(r#"{"question":"q"}"#).unwrap();
        assert!(!b.mute_presentations);
        let b: AskInit =
            serde_json::from_str(r#"{"question":"q","mute_presentations":true}"#).unwrap();
        assert!(b.mute_presentations);
    }

    #[test]
    fn presentation_result_round_trip() {
        let f = ClientFrame::PresentationResult {
            call_id: "p_1".into(),
            ok: true,
            result: Some(serde_json::json!({"effect_id":"h_1"})),
            error: None,
            message: None,
        };
        let s = serde_json::to_string(&f).unwrap();
        let b: ClientFrame = serde_json::from_str(&s).unwrap();
        match b {
            ClientFrame::PresentationResult { call_id, ok, .. } => {
                assert_eq!(call_id, "p_1");
                assert!(ok);
            }
        }
    }
}
