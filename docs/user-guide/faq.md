# FAQ

### What leaves my machine?

Only the calls to your configured LLM provider. When you ask a question, the prompt sent to the provider can include the PR's scraped content, code the agent read from your local checkout, your conversation history for that PR, and notes you saved on it. Nothing else goes anywhere: no telemetry, no account, no Libre CR server. (If you use the `mock` provider, *nothing* leaves the machine at all.) One nuance worth knowing: notes you save are visible to the LLM in subsequent questions about the same PR — don't put secrets in them.

### Where does my conversation data live?

In a local SQLite database: `~/.local/share/libre-cr-review/state.db` (sessions, Q&A turns, notes, tool traces). Repos and worktrees live under `~/.local/share/libre-cr-code/`. See the [paths table](configuration.md#where-everything-lives). Sessions idle for 90 days are cleaned up (configurable via `limits.session_idle_evict_days`).

### Why is there no GitHub login?

By design. PR content is read by scraping the page you already have open, so no GitHub token is ever needed or stored. Posting reviews back is a manual copy-paste from the [export modal](using.md#export); optional OAuth-backed posting is a possible future addition, not part of the current release.

### Can I use it without an API key?

Yes — [mock provider mode](configuration.md#mock-provider-mode) (`kind = "mock"`, the default) replays scripted answers you define in `review.toml`, which is enough to verify the whole install end-to-end. For real answers you need a key for Anthropic, OpenAI, or any OpenAI-compatible endpoint — including a local Ollama, which needs no cloud key at all.

### I use two machines. Does anything sync?

No, by design. Each machine has its own daemon, pairing, and conversation history. Conversations are grounded in a local checkout at a specific commit — replaying them on a machine without that checkout would be incoherent, and syncing would require a server (rejected) or P2P protocol (out of scope). The durable, shareable artifact is the exported review you post to GitHub.

### How are worktrees managed? Will they eat my disk?

The code daemon fetches each PR's head ref and materializes it as a detached worktree under `~/.local/share/libre-cr-code/worktrees/`. Re-opening a PR reuses (and refreshes) the existing worktree. Total worktree usage is capped at **5 GiB** by default; past that, the least-recently-used worktrees are evicted on an hourly sweep. Both knobs are in [`code.toml`](configuration.md#worktrees).

### Can other MCP clients use the code daemon?

Yes. `libre-cr-code` is a standalone MCP server (symbol search, structural queries, git history, worktree management) with value beyond the review extension — Claude Desktop, Claude Code, or any MCP client can spawn it over stdio. For Claude Desktop, add to `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "libre-cr-code": {
      "command": "libre-cr-code",
      "args": ["mcp-stdio"]
    }
  }
}
```

Then call `scan_for_repos` once and use any tool with the returned `repo_id`.

### Does the assistant ever act on its own?

No. It answers only when you ask — verbs are user-initiated, and presentation effects (highlights, annotations, scrolls) happen only while answering your question and are one click from cleared. It also never edits code, never posts to GitHub, and never summarizes a PR unprompted.
