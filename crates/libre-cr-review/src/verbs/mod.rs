//! Verb registry — Phase 4 catalog.
//!
//! Verbs are prompt-tuned shortcuts on top of the agent loop. See
//! `specs/06-investigation-verbs.md` for the catalog and the verbatim
//! system-prompt addenda.

use libre_cr_common::Selection;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// What kind of selection the verb requires, per spec § Selection
/// Requirement Enforcement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionRequirement {
    /// Works with or without a selection.
    Any,
    /// Requires at least a file selected (any range works).
    File,
    /// Requires a multi-line range.
    Range,
    /// Requires identifier-level resolution.
    Symbol,
}

impl SelectionRequirement {
    pub fn satisfied_by(&self, sel: Option<&Selection>) -> bool {
        match self {
            SelectionRequirement::Any => true,
            SelectionRequirement::File => sel.is_some(),
            SelectionRequirement::Range => matches!(
                sel,
                Some(Selection::Range { .. }) | Some(Selection::Symbol { .. })
            ),
            SelectionRequirement::Symbol => matches!(sel, Some(Selection::Symbol { .. })),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            SelectionRequirement::Any => "any",
            SelectionRequirement::File => "file",
            SelectionRequirement::Range => "range",
            SelectionRequirement::Symbol => "symbol",
        }
    }
}

/// One verb in the catalog. All fields are `&'static` because the catalog is
/// hardcoded at compile time per spec ("plugin model is explicitly deferred").
pub struct Verb {
    pub id: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub required_selection: SelectionRequirement,
    pub system_prompt: &'static str,
    pub suggested_tools: &'static [&'static str],
}

/// `find_callers` — locate references to the selected symbol.
pub const FIND_CALLERS: Verb = Verb {
    id: "find_callers",
    label: "Find callers",
    description: "Where in the codebase is the selected symbol referenced?",
    required_selection: SelectionRequirement::Symbol,
    system_prompt: "You are answering: \"Where in the codebase is the selected symbol referenced?\"\n\
\n\
Use `find_references` to find call sites. If the symbol is a function or method, also use `ast_search` with a call-expression pattern to catch usages the AST-based reference finder might miss.\n\
\n\
Report findings grouped by directory. Distinguish production code from tests by file path heuristics (`test`, `spec`, `__tests__`, `*.test.*`, `*.spec.*`). Flag if the symbol appears only in tests, or only in one file, or has no callers (dead code candidate).\n\
\n\
Be concise. Each finding is `path:line — surrounding context`. Do not paste full functions.\n\
\n\
After the textual answer, call `highlight_lines` on up to ~5 of the most important call sites so the reviewer can scan them. If there are more than 5, highlight the representative ones and describe the rest in text.",
    suggested_tools: &["find_references", "ast_search", "read_file", "highlight_lines"],
};

/// `show_history` — git history for the selection.
pub const SHOW_HISTORY: Verb = Verb {
    id: "show_history",
    label: "Show history",
    description: "When and why was this last changed?",
    required_selection: SelectionRequirement::Range,
    system_prompt: "You are answering: \"What is the history of the selected code?\"\n\
\n\
Use `git_log` and `git_blame` to gather commit history for the selected lines/file. Focus on:\n\
- Most recent change touching the selection.\n\
- The original introduction (look back as far as needed).\n\
- Significant changes between (refactors, bug fixes — use commit messages to judge).\n\
- Authors involved.\n\
\n\
Output as a short timeline (newest first), then a one-paragraph synthesis explaining the trajectory of this code (what it was, what it became, why it likely is the way it is).",
    suggested_tools: &["git_log", "git_blame", "git_show"],
};

/// `related_tests` — locate tests covering the selection.
pub const RELATED_TESTS: Verb = Verb {
    id: "related_tests",
    label: "Related tests",
    description: "Is this tested? Where?",
    required_selection: SelectionRequirement::File,
    system_prompt: "You are answering: \"Which tests, if any, cover the selected code?\"\n\
\n\
Heuristics:\n\
- Look for sibling test files (`Foo.ts` → `Foo.test.ts`, `Foo.spec.ts`, `__tests__/Foo.ts`).\n\
- Use `find_references` to find tests that import the selected symbol(s).\n\
- Use `grep` for the symbol name within paths matching test patterns.\n\
- Read found test files (`read_file`) and identify which test functions actually exercise the selection.\n\
\n\
Report: list of test functions with paths, and a verdict on coverage. If nothing is found, say so explicitly. Do not invent test names.",
    suggested_tools: &[
        "find_references",
        "grep",
        "read_file",
        "list_dir",
        "scroll_to",
    ],
};

/// `compare_to_base` — describe the PR diff scoped to the selection.
pub const COMPARE_TO_BASE: Verb = Verb {
    id: "compare_to_base",
    label: "Compare to base",
    description: "What did this code look like before this PR? What changed and why?",
    required_selection: SelectionRequirement::Range,
    system_prompt: "You are answering: \"What changed in the selected code between the PR's base branch and head, and what was the apparent intent of the change?\"\n\
\n\
Use `get_pr_metadata` to identify base and head refs. Use `git_diff` between them, scoped to the selected file. Use `git_log` between the base and head refs to see commit-message context.\n\
\n\
Output:\n\
- Before/after summary in plain English (not a diff — describe the change).\n\
- List of relevant commits in the PR that touched this code, with messages.\n\
- Note any subtle behavioral changes the diff implies (return types, error handling, side effects).",
    suggested_tools: &["git_diff", "git_log", "get_pr_metadata", "highlight_lines"],
};

/// `explain` — walk through the selected code in detail.
pub const EXPLAIN: Verb = Verb {
    id: "explain",
    label: "Explain",
    description: "What does this do? Walk me through it.",
    required_selection: SelectionRequirement::File,
    system_prompt: "You are answering: \"What does the selected code do, in detail?\"\n\
\n\
Read the surrounding context (`read_file` with extended range) so the explanation can name what the code interacts with. Use `list_symbols` or `find_definition` for unfamiliar symbols referenced inside the selection.\n\
\n\
Output: a walkthrough that a competent engineer who has never seen this codebase could follow. Reference specific lines. Explain the *why* where you can infer it; mark guesses as guesses.\n\
\n\
At the start, call `scroll_to` the selection so the reviewer is anchored. Use `highlight_lines` *lightly* on the range under discussion as you reference it (one or two highlights, not a rainbow).",
    suggested_tools: &[
        "read_file",
        "list_symbols",
        "find_definition",
        "scroll_to",
        "highlight_lines",
    ],
};

/// The full hardcoded catalog. Order is intentional (matches the spec).
pub const CATALOG: &[&Verb] = &[
    &FIND_CALLERS,
    &SHOW_HISTORY,
    &RELATED_TESTS,
    &COMPARE_TO_BASE,
    &EXPLAIN,
];

pub fn find(id: &str) -> Option<&'static Verb> {
    CATALOG.iter().copied().find(|v| v.id == id)
}

/// Shape returned by `GET /v1/verbs` — the typed wire struct lives in
/// `libre-cr-common` (spec § Adding A Verb (Internal)).
pub use libre_cr_common::http_api::VerbDescriptor;

pub fn catalog_descriptors() -> Vec<VerbDescriptor> {
    CATALOG
        .iter()
        .map(|v| VerbDescriptor {
            id: v.id.to_string(),
            label: v.label.to_string(),
            required_selection: v.required_selection.as_str().to_string(),
            description: v.description.to_string(),
            suggested_tools: v.suggested_tools.iter().map(|s| s.to_string()).collect(),
        })
        .collect()
}

/// Validate that the selection in hand meets the verb's requirement.
/// Returns `Error::Validation` with a clear message otherwise.
pub fn validate_selection(verb_id: &str, selection: Option<&Selection>) -> Result<()> {
    let Some(verb) = find(verb_id) else {
        return Err(Error::Validation(format!("unknown verb: {verb_id}")));
    };
    if !verb.required_selection.satisfied_by(selection) {
        let need = match verb.required_selection {
            SelectionRequirement::Symbol => "a symbol selection (identifier)",
            SelectionRequirement::Range => "a range or symbol selection",
            SelectionRequirement::File => "a file/range selection",
            SelectionRequirement::Any => "no specific selection",
        };
        return Err(Error::Validation(format!(
            "verb '{verb_id}' requires {need}; got {}",
            match selection {
                None => "no selection".to_string(),
                Some(Selection::Line { .. }) => "a line selection".to_string(),
                Some(Selection::Range { .. }) => "a range selection".to_string(),
                Some(Selection::Symbol { .. }) => "a symbol selection".to_string(),
            }
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_has_exactly_five_verbs() {
        assert_eq!(CATALOG.len(), 5);
    }

    #[test]
    fn each_verb_has_nonempty_prompt_and_unique_id() {
        let mut seen = std::collections::HashSet::new();
        for v in CATALOG {
            assert!(!v.system_prompt.trim().is_empty(), "{}", v.id);
            assert!(!v.label.trim().is_empty(), "{}", v.id);
            assert!(!v.description.trim().is_empty(), "{}", v.id);
            assert!(seen.insert(v.id), "duplicate id: {}", v.id);
        }
    }

    #[test]
    fn find_returns_known_verbs() {
        assert!(find("find_callers").is_some());
        assert!(find("explain").is_some());
        assert!(find("nope").is_none());
    }

    #[test]
    fn required_selection_matches_spec() {
        assert_eq!(
            find("find_callers").unwrap().required_selection,
            SelectionRequirement::Symbol
        );
        assert_eq!(
            find("show_history").unwrap().required_selection,
            SelectionRequirement::Range
        );
        assert_eq!(
            find("related_tests").unwrap().required_selection,
            SelectionRequirement::File
        );
        assert_eq!(
            find("compare_to_base").unwrap().required_selection,
            SelectionRequirement::Range
        );
        assert_eq!(
            find("explain").unwrap().required_selection,
            SelectionRequirement::File
        );
    }

    #[test]
    fn descriptors_have_correct_shape() {
        let d = catalog_descriptors();
        assert_eq!(d.len(), 5);
        let fc = &d[0];
        assert_eq!(fc.id, "find_callers");
        assert_eq!(fc.required_selection, "symbol");
    }

    #[test]
    fn validate_selection_rejects_missing_symbol_for_find_callers() {
        let err = validate_selection("find_callers", None).unwrap_err();
        match err {
            Error::Validation(_) => {}
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn validate_selection_accepts_symbol_for_find_callers() {
        let sel = Selection::Symbol {
            file: "a.rs".into(),
            line: 1,
            column: 0,
            identifier: "foo".into(),
            text: None,
        };
        validate_selection("find_callers", Some(&sel)).unwrap();
    }

    #[test]
    fn validate_selection_rejects_line_for_show_history() {
        let sel = Selection::Line {
            file: "a.rs".into(),
            line: 1,
            text: None,
        };
        assert!(validate_selection("show_history", Some(&sel)).is_err());
    }

    #[test]
    fn validate_selection_accepts_range_for_show_history() {
        let sel = Selection::Range {
            file: "a.rs".into(),
            start_line: 1,
            end_line: 5,
            text: None,
        };
        validate_selection("show_history", Some(&sel)).unwrap();
    }

    #[test]
    fn validate_selection_unknown_verb_errors() {
        assert!(validate_selection("nope", None).is_err());
    }

    #[test]
    fn suggested_tools_include_code_daemon_tools_for_find_callers() {
        let v = find("find_callers").unwrap();
        assert!(v.suggested_tools.contains(&"find_references"));
        assert!(v.suggested_tools.contains(&"highlight_lines"));
    }
}
