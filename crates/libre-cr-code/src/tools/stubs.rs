//! Phase 1.0 stubs for tree-sitter and ast-grep tools. Registered so the
//! surface is complete; bodies will be filled in Phase 1.1.

use crate::error::{ErrorCode, ToolError};
use crate::tools::context::ToolContext;
use crate::tools::registry::{Tool, ToolFuture};
use serde_json::{json, Value};
use std::sync::Arc;

fn unsupported_envelope(message: &str) -> Value {
    json!({
        "ok": false,
        "error": ErrorCode::UnsupportedLanguage.as_str(),
        "message": message,
        "details": { "phase": "1.0", "todo": "compile in tree-sitter grammars" },
    })
}

pub struct AstSearch;

impl Tool for AstSearch {
    fn name(&self) -> &'static str {
        "ast_search"
    }
    fn description(&self) -> &'static str {
        "ast-grep structural search. Phase 1.0: stub returning unsupported_language."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["repo_path", "language", "pattern"],
            "properties": {
                "repo_path": { "type": "string" },
                "language": { "type": "string" },
                "pattern": { "type": "string" },
                "ref": { "type": "string" },
                "paths": { "type": "array" }
            }
        })
    }
    fn call<'a>(&'a self, _ctx: Arc<ToolContext>, _input: Value) -> ToolFuture<'a> {
        Box::pin(async move {
            Ok(unsupported_envelope(
                "ast_search not yet implemented in Phase 1.0",
            ))
        })
    }
}

pub struct ListSymbols;

impl Tool for ListSymbols {
    fn name(&self) -> &'static str {
        "list_symbols"
    }
    fn description(&self) -> &'static str {
        "Tree-sitter tags. Phase 1.0: stub returning unsupported_language."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["repo_path", "file"],
            "properties": {
                "repo_path": { "type": "string" },
                "file": { "type": "string" },
                "ref": { "type": "string" }
            }
        })
    }
    fn call<'a>(&'a self, _ctx: Arc<ToolContext>, _input: Value) -> ToolFuture<'a> {
        Box::pin(async move {
            Ok(unsupported_envelope(
                "list_symbols not yet implemented in Phase 1.0",
            ))
        })
    }
}

pub struct FindDefinition;

impl Tool for FindDefinition {
    fn name(&self) -> &'static str {
        "find_definition"
    }
    fn description(&self) -> &'static str {
        "Find definitions for a symbol. Phase 1.0: stub returning low-confidence empty results."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["repo_path", "file", "line", "column"],
            "properties": {
                "repo_path": { "type": "string" },
                "file": { "type": "string" },
                "line": { "type": "integer" },
                "column": { "type": "integer" },
                "ref": { "type": "string" }
            }
        })
    }
    fn call<'a>(&'a self, _ctx: Arc<ToolContext>, _input: Value) -> ToolFuture<'a> {
        Box::pin(async move {
            // Returns the stub with confidence: low so the seam is intact.
            Ok(json!({
                "ok": false,
                "error": ErrorCode::UnsupportedLanguage.as_str(),
                "message": "find_definition is a stub in Phase 1.0",
                "definitions": [],
                "confidence": "low",
                "details": { "phase": "1.0" },
            }))
        })
    }
}

pub struct FindReferences;

impl Tool for FindReferences {
    fn name(&self) -> &'static str {
        "find_references"
    }
    fn description(&self) -> &'static str {
        "Find references to a symbol. Phase 1.0: stub returning low-confidence empty results."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["repo_path", "file", "line", "column"],
            "properties": {
                "repo_path": { "type": "string" },
                "file": { "type": "string" },
                "line": { "type": "integer" },
                "column": { "type": "integer" },
                "ref": { "type": "string" }
            }
        })
    }
    fn call<'a>(&'a self, _ctx: Arc<ToolContext>, _input: Value) -> ToolFuture<'a> {
        Box::pin(async move {
            Ok(json!({
                "ok": false,
                "error": ErrorCode::UnsupportedLanguage.as_str(),
                "message": "find_references is a stub in Phase 1.0",
                "references": [],
                "confidence": "low",
                "details": { "phase": "1.0" },
            }))
        })
    }
}

// Suppress unused-import warning on `ToolError`.
#[allow(dead_code)]
fn _suppress() -> ToolError {
    ToolError::new(ErrorCode::Internal, "")
}
