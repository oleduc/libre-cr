use serde::{Deserialize, Serialize};

/// A reviewer's selection in the diff view, sent with every question.
///
/// Mirrors the `Selection` type in `specs/05-browser-extension.md`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Selection {
    Line {
        file: String,
        line: u32,
        /// The selected line's text, captured from the diff by the extension.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
    },
    Range {
        file: String,
        start_line: u32,
        end_line: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
    },
    Symbol {
        file: String,
        line: u32,
        column: u32,
        identifier: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
    },
}

impl Selection {
    pub fn file(&self) -> &str {
        match self {
            Selection::Line { file, .. }
            | Selection::Range { file, .. }
            | Selection::Symbol { file, .. } => file,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_line() {
        let s = Selection::Line {
            file: "src/auth.ts".into(),
            line: 42,
            text: None,
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: Selection = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
        assert!(json.contains("\"kind\":\"line\""));
    }

    #[test]
    fn round_trips_symbol() {
        let s = Selection::Symbol {
            file: "src/auth.ts".into(),
            line: 42,
            column: 8,
            identifier: "bcryptHash".into(),
            text: None,
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: Selection = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }
}
