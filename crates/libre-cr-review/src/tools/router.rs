//! Tool router: unified surface over the three categories.

use std::sync::Arc;

use crate::error::{Error, Result};
use crate::provider::ToolSchema;

use super::code_daemon::CodeDaemonClient;
use super::internal::{internal_tool_schemas, InternalContext, INTERNAL_TOOL_NAMES};
use super::presentation::{
    presentation_tool_schemas, PresentationDispatcher, PRESENTATION_TOOL_NAMES,
};

#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct ToolOutcome {
    pub ok: bool,
    pub value: serde_json::Value,
    pub duration_ms: i64,
}

/// Code-daemon tools the review daemon drives itself (worktree orchestration)
/// and therefore never offers to the model. Left visible, the model went
/// repo-hunting when `get_pr_diff` looked empty — `scan_for_repos`, then a
/// `clone_repo` with a guessed URL that failed on a private repo.
const AGENT_HIDDEN_CODE_TOOLS: &[&str] = &[
    "prepare_worktree",
    "list_worktrees",
    "remove_worktree",
    "discover_repo",
    "scan_for_repos",
    "clone_repo",
];

/// Above this, an unnarrowed diff is answered with a file manifest instead of
/// its content.
///
/// Measured on a real PR: 73 files, 643,463 chars of JSON, of which the model
/// received the first 20,000 — three percent, cut mid-structure, most of it
/// `uv.lock` and CI workflow churn because the files arrive in path order. A
/// manifest of the same PR is a few thousand chars and tells the model what it
/// is choosing between, which is what it actually needs before it can pass
/// `paths`. Truncate-and-tell, one level up: the answer to "too big" is a
/// smaller *useful* thing, not the first slice of a large one.
const DIFF_MANIFEST_THRESHOLD_CHARS: usize = 60_000;

/// Replace a too-large diff with the list of files in it, each with its status
/// and size, plus a note saying how to ask for content.
fn manifest_if_huge(diff: serde_json::Value) -> serde_json::Value {
    let size = diff.to_string().chars().count();
    if size <= DIFF_MANIFEST_THRESHOLD_CHARS {
        return diff;
    }
    let Some(files) = diff.get("files").and_then(|f| f.as_array()) else {
        return diff;
    };
    let listed: Vec<serde_json::Value> = files
        .iter()
        .map(|f| {
            let hunks = f.get("hunks").and_then(|h| h.as_array());
            serde_json::json!({
                "path": f.get("path").or_else(|| f.get("file")).cloned().unwrap_or(serde_json::Value::Null),
                "status": f.get("status").cloned().unwrap_or(serde_json::Value::Null),
                "hunks": hunks.map(|h| h.len()).unwrap_or(0),
                "chars": f.to_string().chars().count(),
            })
        })
        .collect();
    serde_json::json!({
        "files_only": true,
        "total_chars": size,
        "files": listed,
        "note": format!(
            "This PR's full diff is {size} chars — too large to return, and far larger than one \
             tool result may hold. Listed above is every changed file with its size. Call \
             get_pr_diff again with `paths` set to the files you actually need; lockfiles, \
             generated files and CI config are usually not among them."
        ),
    })
}

pub struct ToolRouter {
    code: Arc<dyn CodeDaemonClient>,
    internal: InternalContext,
    presentation: Option<Arc<PresentationDispatcher>>,
    code_schemas: Vec<ToolSchema>,
    worktree_path: Option<String>,
    repo_id: Option<String>,
}

impl ToolRouter {
    pub fn new(
        code: Arc<dyn CodeDaemonClient>,
        code_schemas: Vec<ToolSchema>,
        internal: InternalContext,
        worktree_path: Option<String>,
    ) -> Self {
        Self {
            code,
            internal,
            presentation: None,
            code_schemas,
            worktree_path,
            repo_id: None,
        }
    }

    pub fn with_repo_id(mut self, repo_id: Option<String>) -> Self {
        self.repo_id = repo_id;
        self
    }

    pub fn with_presentation(mut self, p: Arc<PresentationDispatcher>) -> Self {
        self.presentation = Some(p);
        self
    }

    pub fn tools_for_verb(&self, _verb: Option<&str>) -> Vec<ToolSchema> {
        let mut out: Vec<ToolSchema> = self
            .code_schemas
            .iter()
            .filter(|t| !AGENT_HIDDEN_CODE_TOOLS.contains(&t.name.as_str()))
            .cloned()
            .collect();
        out.extend(internal_tool_schemas());
        if self.presentation.is_some() {
            out.extend(presentation_tool_schemas());
        }
        out
    }

    fn category(&self, name: &str) -> Category {
        if INTERNAL_TOOL_NAMES.contains(&name) {
            Category::Internal
        } else if PRESENTATION_TOOL_NAMES.contains(&name) {
            Category::Presentation
        } else if self.code_schemas.iter().any(|t| t.name == name) {
            Category::CodeDaemon
        } else {
            Category::Unknown
        }
    }

    pub async fn dispatch(&self, call: &ToolCall) -> ToolOutcome {
        let started = std::time::Instant::now();
        let r = self.dispatch_inner(call).await;
        let duration_ms = started.elapsed().as_millis() as i64;
        match r {
            Ok(value) => ToolOutcome {
                ok: true,
                value,
                duration_ms,
            },
            Err(e) => ToolOutcome {
                ok: false,
                value: serde_json::json!({
                    "error": e.to_string(),
                }),
                duration_ms,
            },
        }
    }

    /// `origin/<base_branch>` of the PR, when the scrape captured one.
    pub fn base_ref(&self) -> Option<String> {
        self.internal
            .pr_data
            .get("base_branch")
            .and_then(|v| v.as_str())
            .filter(|b| !b.is_empty())
            .map(|b| format!("origin/{b}"))
    }

    /// `get_pr_diff` computed on the prepared worktree: the extension never
    /// scraped hunks, so the honest source is `git_diff` base → PR head.
    /// Falls back to the (empty) scraped payload when there is no worktree.
    async fn pr_diff(&self, input: &serde_json::Value) -> Result<serde_json::Value> {
        let (Some(path), Some(base)) = (&self.worktree_path, self.base_ref()) else {
            return self.internal.call("get_pr_diff", input.clone()).await;
        };
        // Three-dot (merge-base) diff: what the PR page shows. Tip-to-tip made
        // every commit `main` gained since the PR forked look like a deletion.
        let mut args = serde_json::json!({
            "repo_path": path,
            "from_ref": base,
            "to_ref": "HEAD",
            "merge_base": true,
        });
        let narrowed = input
            .get("paths")
            .and_then(|p| p.as_array())
            .is_some_and(|a| !a.is_empty());
        if narrowed {
            args["paths"] = input["paths"].clone();
        }
        let diff = self.code.call("git_diff", args).await?;
        if narrowed {
            return Ok(diff);
        }
        Ok(manifest_if_huge(diff))
    }

    async fn dispatch_inner(&self, call: &ToolCall) -> Result<serde_json::Value> {
        if call.name == "get_pr_diff" {
            return self.pr_diff(&call.input).await;
        }
        if AGENT_HIDDEN_CODE_TOOLS.contains(&call.name.as_str()) {
            return Err(Error::Validation(format!(
                "tool '{}' is managed by the review daemon and not available here; \
                 the PR is already checked out — use the code tools directly",
                call.name
            )));
        }
        match self.category(&call.name) {
            Category::Internal => self.internal.call(&call.name, call.input.clone()).await,
            Category::CodeDaemon => {
                // Inject repo_path / repo_id only when the schema declares
                // them and the caller didn't provide a value.
                let mut input = call.input.clone();
                let schema = self
                    .code_schemas
                    .iter()
                    .find(|t| t.name == call.name)
                    .map(|t| t.input_schema.clone());
                if let Some(obj) = input.as_object_mut() {
                    if let Some(schema) = &schema {
                        if schema_has_property(schema, "repo_path")
                            && !obj.contains_key("repo_path")
                        {
                            if let Some(p) = &self.worktree_path {
                                obj.insert(
                                    "repo_path".into(),
                                    serde_json::Value::String(p.clone()),
                                );
                            }
                        }
                        if schema_has_property(schema, "repo_id") && !obj.contains_key("repo_id") {
                            if let Some(id) = &self.repo_id {
                                obj.insert("repo_id".into(), serde_json::Value::String(id.clone()));
                            }
                        }
                    }
                }
                self.code.call(&call.name, input).await
            }
            Category::Presentation => match &self.presentation {
                Some(p) => {
                    let outcome = p.dispatch(&call.name, call.input.clone()).await?;
                    Ok(outcome.value)
                }
                None => Err(Error::Validation(format!(
                    "presentation tool '{}' not available in this context",
                    call.name
                ))),
            },
            Category::Unknown => Err(Error::Validation(format!("unknown tool: {}", call.name))),
        }
    }

    pub fn worktree_path(&self) -> Option<&str> {
        self.worktree_path.as_deref()
    }
}

enum Category {
    Internal,
    CodeDaemon,
    Presentation,
    Unknown,
}

/// True if a JSON-schema object declares `name` under its `properties`.
fn schema_has_property(schema: &serde_json::Value, name: &str) -> bool {
    schema
        .get("properties")
        .and_then(|p| p.as_object())
        .map(|m| m.contains_key(name))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    /// An unnarrowed diff on a big PR comes back as a manifest, not as the
    /// first 3% of a JSON blob. Measured: 73 files, 643,463 chars, of which
    /// the model saw 20,000 — mostly `uv.lock` and CI churn, because files
    /// arrive in path order.
    #[test]
    fn a_huge_diff_is_answered_with_a_file_manifest() {
        let file = |path: &str, bytes: usize| {
            serde_json::json!({
                "path": path,
                "status": "modified",
                "hunks": [{"header": "@@", "lines": ["+".repeat(bytes)]}],
            })
        };
        let huge = serde_json::json!({"files": [
            file("uv.lock", 40_000),
            file("src/oauth/authorization_code.py", 30_000),
        ]});
        let out = manifest_if_huge(huge);
        assert_eq!(out["files_only"], true);
        assert_eq!(out["files"].as_array().unwrap().len(), 2);
        assert_eq!(out["files"][0]["path"], "uv.lock");
        // Sizes are what let the model choose; the note says how to ask.
        assert!(out["files"][0]["chars"].as_u64().unwrap() > 40_000);
        assert!(out["note"].as_str().unwrap().contains("`paths`"));
        // No content: that is the whole point.
        assert!(!out.to_string().contains("@@"));

        // A small diff is returned untouched, manifest machinery invisible.
        let small = serde_json::json!({"files": [file("a.rs", 10)]});
        assert_eq!(manifest_if_huge(small.clone()), small);
    }

    use super::*;
    use crate::storage::Store;
    use crate::tools::code_daemon::MockCodeDaemonClient;

    async fn make_router() -> ToolRouter {
        let store = Store::open_in_memory().unwrap();
        let sess = store
            .upsert_session("https://github.com/a/b/pull/1", serde_json::json!({}))
            .await
            .unwrap();
        let mc = Arc::new(MockCodeDaemonClient);
        let schemas = mc.list_tools().await.unwrap();
        let internal = InternalContext {
            session_id: sess.session_id,
            pr_data: serde_json::json!({"metadata":{"title":"hi"}}),
            selection: None,
            store,
        };
        ToolRouter::new(mc, schemas, internal, Some("/tmp/work".into()))
    }

    #[tokio::test]
    async fn dispatches_internal_tool() {
        let r = make_router().await;
        let out = r
            .dispatch(&ToolCall {
                id: "1".into(),
                name: "get_pr_metadata".into(),
                input: serde_json::json!({}),
            })
            .await;
        assert!(out.ok);
        assert_eq!(out.value["title"], "hi");
    }

    #[tokio::test]
    async fn dispatches_code_daemon_tool_and_injects_repo_path() {
        let r = make_router().await;
        let out = r
            .dispatch(&ToolCall {
                id: "2".into(),
                name: "grep".into(),
                input: serde_json::json!({"query":"x"}),
            })
            .await;
        assert!(out.ok);
        // The mock echoes input for unknowns, but grep returns a fixed shape;
        // assert the injection at the input shape via read_file's echo path.
        let out2 = r
            .dispatch(&ToolCall {
                id: "3".into(),
                name: "read_file".into(),
                input: serde_json::json!({"path":"a"}),
            })
            .await;
        assert!(out2.ok);
    }

    #[tokio::test]
    async fn injects_repo_path_only_when_schema_has_it() {
        use crate::provider::ToolSchema;
        struct InspectingClient {
            inner: Arc<MockCodeDaemonClient>,
            last_input: Arc<tokio::sync::Mutex<Option<serde_json::Value>>>,
        }
        #[async_trait::async_trait]
        impl CodeDaemonClient for InspectingClient {
            async fn list_tools(&self) -> crate::error::Result<Vec<ToolSchema>> {
                self.inner.list_tools().await
            }
            async fn call(
                &self,
                name: &str,
                input: serde_json::Value,
            ) -> crate::error::Result<serde_json::Value> {
                *self.last_input.lock().await = Some(input.clone());
                self.inner.call(name, input).await
            }
        }
        let store = Store::open_in_memory().unwrap();
        let sess = store
            .upsert_session("https://github.com/a/b/pull/1", serde_json::json!({}))
            .await
            .unwrap();
        let inner = Arc::new(MockCodeDaemonClient);
        let last = Arc::new(tokio::sync::Mutex::new(None));
        let client = Arc::new(InspectingClient {
            inner: inner.clone(),
            last_input: last.clone(),
        });
        // One schema has repo_path, another doesn't.
        let schemas = vec![
            ToolSchema {
                name: "needs_path".into(),
                description: "".into(),
                input_schema: serde_json::json!({
                    "type":"object",
                    "properties":{"repo_path":{"type":"string"}, "q":{"type":"string"}}
                }),
            },
            ToolSchema {
                name: "no_path".into(),
                description: "".into(),
                input_schema: serde_json::json!({
                    "type":"object",
                    "properties":{"q":{"type":"string"}}
                }),
            },
        ];
        let internal = InternalContext {
            session_id: sess.session_id,
            pr_data: serde_json::json!({}),
            selection: None,
            store,
        };
        let r = ToolRouter::new(client, schemas, internal, Some("/work".into()));
        // With repo_path in schema -> injected.
        let _ = r
            .dispatch(&ToolCall {
                id: "1".into(),
                name: "needs_path".into(),
                input: serde_json::json!({"q":"x"}),
            })
            .await;
        let got = last.lock().await.clone().unwrap();
        assert_eq!(got["repo_path"], "/work");
        // Without repo_path in schema -> NOT injected.
        let _ = r
            .dispatch(&ToolCall {
                id: "2".into(),
                name: "no_path".into(),
                input: serde_json::json!({"q":"y"}),
            })
            .await;
        let got = last.lock().await.clone().unwrap();
        assert!(got.get("repo_path").is_none());
    }

    #[tokio::test]
    async fn injects_repo_id_when_schema_has_it() {
        use crate::provider::ToolSchema;
        struct InspectingClient {
            last_input: Arc<tokio::sync::Mutex<Option<serde_json::Value>>>,
        }
        #[async_trait::async_trait]
        impl CodeDaemonClient for InspectingClient {
            async fn list_tools(&self) -> crate::error::Result<Vec<ToolSchema>> {
                Ok(vec![])
            }
            async fn call(
                &self,
                _name: &str,
                input: serde_json::Value,
            ) -> crate::error::Result<serde_json::Value> {
                *self.last_input.lock().await = Some(input);
                Ok(serde_json::json!({"ok":true}))
            }
        }
        let store = Store::open_in_memory().unwrap();
        let sess = store
            .upsert_session("https://github.com/a/b/pull/2", serde_json::json!({}))
            .await
            .unwrap();
        let last = Arc::new(tokio::sync::Mutex::new(None));
        let client = Arc::new(InspectingClient {
            last_input: last.clone(),
        });
        let schemas = vec![ToolSchema {
            name: "prep".into(),
            description: "".into(),
            input_schema: serde_json::json!({
                "type":"object",
                "properties":{"repo_id":{"type":"string"}, "ref":{"type":"string"}}
            }),
        }];
        let internal = InternalContext {
            session_id: sess.session_id,
            pr_data: serde_json::json!({}),
            selection: None,
            store,
        };
        let r = ToolRouter::new(client, schemas, internal, None)
            .with_repo_id(Some("github.com/a/b".into()));
        let _ = r
            .dispatch(&ToolCall {
                id: "1".into(),
                name: "prep".into(),
                input: serde_json::json!({"ref":"main"}),
            })
            .await;
        let got = last.lock().await.clone().unwrap();
        assert_eq!(got["repo_id"], "github.com/a/b");
    }

    #[tokio::test]
    async fn unknown_tool_errors() {
        let r = make_router().await;
        let out = r
            .dispatch(&ToolCall {
                id: "4".into(),
                name: "nope".into(),
                input: serde_json::json!({}),
            })
            .await;
        assert!(!out.ok);
    }

    #[tokio::test]
    async fn hides_repo_management_tools_and_serves_pr_diff_from_worktree() {
        use std::sync::Mutex as StdMutex;
        struct Recorder(StdMutex<Vec<(String, serde_json::Value)>>);
        #[async_trait::async_trait]
        impl CodeDaemonClient for Recorder {
            async fn list_tools(&self) -> crate::error::Result<Vec<ToolSchema>> {
                Ok(["grep", "clone_repo", "scan_for_repos", "git_diff"]
                    .iter()
                    .map(|n| ToolSchema {
                        name: (*n).into(),
                        description: String::new(),
                        input_schema: serde_json::json!({"type":"object","properties":{"repo_path":{"type":"string"}}}),
                    })
                    .collect())
            }
            async fn call(
                &self,
                name: &str,
                input: serde_json::Value,
            ) -> crate::error::Result<serde_json::Value> {
                self.0.lock().unwrap().push((name.to_string(), input));
                Ok(serde_json::json!({"files": [{"path": "a.rs"}]}))
            }
        }
        let rec = Arc::new(Recorder(StdMutex::new(vec![])));
        let schemas = rec.list_tools().await.unwrap();
        let internal = InternalContext {
            session_id: "s".into(),
            pr_data: serde_json::json!({"base_branch": "main"}),
            selection: None,
            store: crate::storage::Store::open_in_memory().unwrap(),
        };
        let router = ToolRouter::new(rec.clone(), schemas, internal, Some("/wt".into()));

        let offered: Vec<String> = router
            .tools_for_verb(None)
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert!(offered.contains(&"grep".to_string()));
        assert!(!offered
            .iter()
            .any(|n| n == "clone_repo" || n == "scan_for_repos"));

        let out = router
            .dispatch(&ToolCall {
                id: "c1".into(),
                name: "get_pr_diff".into(),
                input: serde_json::json!({"paths": ["a.rs"]}),
            })
            .await;
        assert!(out.ok, "{out:?}");
        {
            let calls = rec.0.lock().unwrap();
            let (name, args) = &calls[0];
            assert_eq!(name, "git_diff");
            assert_eq!(args["repo_path"], "/wt");
            assert_eq!(args["from_ref"], "origin/main");
            assert_eq!(args["to_ref"], "HEAD");
            assert_eq!(args["merge_base"], true);
            assert_eq!(args["paths"][0], "a.rs");
        }

        let refused = router
            .dispatch(&ToolCall {
                id: "c2".into(),
                name: "clone_repo".into(),
                input: serde_json::json!({"remote_url": "x"}),
            })
            .await;
        assert!(!refused.ok);
    }
}
