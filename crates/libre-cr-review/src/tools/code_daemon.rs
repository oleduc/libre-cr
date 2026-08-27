//! Code-daemon-tool client trait + mock impl.
//!
//! Phase 3 swaps `MockCodeDaemonClient` for a real MCP child-process client.
//! The trait + dispatch shape is the contract Phase 3 implements against.

use async_trait::async_trait;

use crate::error::Result;
use crate::provider::ToolSchema;

#[async_trait]
pub trait CodeDaemonClient: Send + Sync {
    /// The set of tools the agent can call. Returned once at startup;
    /// callers may re-discover after a daemon restart.
    async fn list_tools(&self) -> Result<Vec<ToolSchema>>;

    /// Dispatch one tool call. The router injects `repo_path` before calling.
    async fn call(&self, name: &str, input: serde_json::Value) -> Result<serde_json::Value>;

    /// As [`Self::call`], with a caller-chosen deadline for long git work
    /// (`clone_repo`, `prepare_worktree`). The default ignores the deadline —
    /// only the spawned client enforces one.
    async fn call_with_timeout(
        &self,
        name: &str,
        input: serde_json::Value,
        _deadline: std::time::Duration,
    ) -> Result<serde_json::Value> {
        self.call(name, input).await
    }
}

/// Hand-written canned responses for the Phase 2 smoke flow.
pub struct MockCodeDaemonClient;

#[async_trait]
impl CodeDaemonClient for MockCodeDaemonClient {
    async fn list_tools(&self) -> Result<Vec<ToolSchema>> {
        Ok(vec![
            ToolSchema {
                name: "grep".into(),
                description: "Text search via ripgrep".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "repo_path": { "type": "string" },
                        "query": { "type": "string" }
                    },
                    "required": ["query"]
                }),
            },
            ToolSchema {
                name: "read_file".into(),
                description: "Read a file at a ref".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "repo_path": { "type": "string" },
                        "path": { "type": "string" },
                        "ref": { "type": "string" }
                    },
                    "required": ["path"]
                }),
            },
            ToolSchema {
                name: "find_references".into(),
                description: "Find references to a symbol".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "repo_path": { "type": "string" },
                        "identifier": { "type": "string" }
                    },
                    "required": ["identifier"]
                }),
            },
            ToolSchema {
                name: "git_log".into(),
                description: "Show git log".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "repo_path": { "type": "string" },
                        "path": { "type": "string" }
                    }
                }),
            },
        ])
    }

    async fn call(&self, name: &str, input: serde_json::Value) -> Result<serde_json::Value> {
        let echo = |kind: &str| {
            serde_json::json!({
                "mock": true,
                "tool": kind,
                "input": input,
            })
        };
        let v = match name {
            "grep" => serde_json::json!({
                "matches": [
                    {"path": "src/auth.ts", "line": 42, "text": "bcryptHash(password)"},
                ],
                "_mock": true,
            }),
            "read_file" => serde_json::json!({
                "content": "// mocked file content\n",
                "_mock": true,
            }),
            "find_references" => serde_json::json!({
                "references": [
                    {"path": "src/auth.ts", "line": 42, "confidence": "high"},
                    {"path": "src/auth/legacy.ts", "line": 88, "confidence": "medium"},
                ],
                "_mock": true,
            }),
            "git_log" => serde_json::json!({
                "commits": [
                    {"sha": "abc123", "subject": "initial", "author": "alice"},
                ],
                "_mock": true,
            }),
            _ => echo(name),
        };
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_returns_canned_grep() {
        let c = MockCodeDaemonClient;
        let r = c
            .call("grep", serde_json::json!({"query": "x"}))
            .await
            .unwrap();
        assert!(r["matches"].is_array());
    }
}
