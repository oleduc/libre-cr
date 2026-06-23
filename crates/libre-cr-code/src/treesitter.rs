//! Tree-sitter integration seam. Phase 1.0: stubs that return
//! `unsupported_language`. Phase 1.1 will wire grammars per spec §
//! "Phase B grammar set".

use crate::error::{ErrorCode, ToolError};

/// Whether the given language has a compiled-in grammar. Phase 1.0: nothing
/// is compiled in.
#[allow(dead_code)]
pub fn has_grammar(_language: &str) -> bool {
    false
}

/// Phase 1.1 will replace this with real tree-sitter parsing.
#[allow(dead_code)]
pub fn unsupported(language: &str) -> ToolError {
    ToolError::new(
        ErrorCode::UnsupportedLanguage,
        format!(
            "language {language:?} not yet supported (Phase 1.0 ships without compiled grammars)"
        ),
    )
}
