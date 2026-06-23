// E2E suite for the **browser extension ↔ libre-cr-review daemon** consumer.
//
// Spawns the real `libre-cr-review` binary (with the real `libre-cr-code`
// child when available) under a tempdir `$HOME`, then drives it through the
// extension's own `DaemonClient` / `AskSession`. This catches breakages
// that the unit-level vitests can't — the extension talking to a real
// running daemon over the wire.
//
// Run with `pnpm test:e2e` from `extension/` (uses `vitest.config.e2e.ts`).

import { spawn, type ChildProcess } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync, mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

import { DaemonClient } from "../utils/daemon/client";
import { AskSession } from "../utils/daemon/ws";

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = resolve(HERE, "../..");
const TARGET_DIR = resolve(REPO_ROOT, "target/debug");
const REVIEW_BIN = process.platform === "win32"
  ? join(TARGET_DIR, "libre-cr-review.exe")
  : join(TARGET_DIR, "libre-cr-review");
const CODE_BIN = process.platform === "win32"
  ? join(TARGET_DIR, "libre-cr-code.exe")
  : join(TARGET_DIR, "libre-cr-code");

interface DaemonHandle {
  endpoint: string;
  token: string;
  home: string;
  child: ChildProcess;
}

let daemon: DaemonHandle | null = null;
let skipReason: string | null = null;

async function ensureBinariesBuilt(): Promise<boolean> {
  // Lazy build (parallel) via `cargo build -p <name> --bin <name>`. If
  // either fails (no Rust toolchain, broken source), mark the suite as
  // skipped — extensions tests must not fail because of the Rust side.
  if (existsSync(REVIEW_BIN) && existsSync(CODE_BIN)) return true;
  const results = await Promise.all(
    ["libre-cr-review", "libre-cr-code"].map((name) =>
      new Promise<{ name: string; ok: boolean; stderr: string }>((resolveFn) => {
        const proc = spawn(
          "cargo",
          ["build", "-p", name, "--bin", name],
          { cwd: REPO_ROOT, env: { ...process.env, PATH: `${process.env.HOME}/.cargo/bin:${process.env.PATH ?? ""}` } },
        );
        let stderr = "";
        proc.stderr?.on("data", (b) => (stderr += String(b)));
        proc.on("close", (code) => resolveFn({ name, ok: code === 0, stderr }));
        proc.on("error", (e) => resolveFn({ name, ok: false, stderr: String(e) }));
      }),
    ),
  );
  const failed = results.find((r) => !r.ok);
  if (failed) {
    skipReason = `cargo build ${failed.name} failed:\n${failed.stderr.slice(-400)}`;
    return false;
  }
  return existsSync(REVIEW_BIN) && existsSync(CODE_BIN);
}

/**
 * Build a full `review.toml`. All defaulted fields are written explicitly
 * because the Rust side's TOML parser does not honor `#[serde(default)]` on
 * fields nested inside structs that are themselves missing.
 */
function buildToml(codeBin: string, scriptBlock = ""): string {
  return [
    "[server]",
    'bind = "127.0.0.1"',
    "port = 0",
    'endpoint_file = "~/.config/libre-cr/endpoint"',
    'token_file = "~/.config/libre-cr/token"',
    'install_key_file = "~/.config/libre-cr/install_key"',
    'extension_origin = ""',
    "",
    "[storage]",
    'data_dir = "~/.local/share/libre-cr-review"',
    'db = "~/.local/share/libre-cr-review/state.db"',
    "",
    "[provider]",
    'kind = "mock"',
    'api_key_enc = ""',
    'model = "mock-model"',
    "max_tokens = 4096",
    "temperature = 0.0",
    'endpoint = ""',
    "",
    "[code_daemon]",
    'mode = "spawn"',
    `binary = "${codeBin.replace(/\\/g, "\\\\")}"`,
    'external_socket = ""',
    "restart_on_failure = false",
    "max_restarts_per_hour = 1",
    "",
    "[mcp_server]",
    "enabled = true",
    "stdio = true",
    "sse = true",
    "",
    "[global_instructions]",
    'text = ""',
    "",
    "[limits]",
    "max_tool_turns = 25",
    "max_history_messages = 30",
    "session_idle_evict_days = 90",
    "",
    "[mock]",
    "code_intel = true",
    "",
    scriptBlock,
  ].join("\n");
}

/**
 * One-text-delta-then-done provider script — gives the WS test
 * something to observe without having to script tool calls.
 */
const ONE_TURN_SCRIPT = [
  "[[mock.provider_script]]",
  "delay_ms = 0",
  "",
  "[mock.provider_script.event]",
  'type = "text_delta"',
  'text = "answer"',
  "",
  "[[mock.provider_script]]",
  "delay_ms = 0",
  "",
  "[mock.provider_script.event]",
  'type = "done"',
  "input_tokens = 1",
  "output_tokens = 1",
  'stop_reason = "end_turn"',
  "",
].join("\n");

async function startDaemon(): Promise<DaemonHandle> {
  // Tempdir $HOME; pass --config explicitly because Rust's `dirs::config_dir()`
  // resolves under `~/Library/...` on macOS, not `~/.config`.
  const home = mkdtempSync(join(tmpdir(), "lcr-e2e-"));
  mkdirSync(join(home, ".config/libre-cr"), { recursive: true });
  const configPath = join(home, "review.toml");
  // Pre-write the token so we know it without reading a file race.
  const token = "ext-e2e-token-deadbeef";
  writeFileSync(join(home, ".config/libre-cr/token"), token);
  writeFileSync(configPath, buildToml(CODE_BIN, ONE_TURN_SCRIPT));

  const child = spawn(
    REVIEW_BIN,
    ["--config", configPath, "serve"],
    {
      env: { ...process.env, HOME: home, RUST_LOG: "warn" },
      stdio: ["ignore", "ignore", "pipe"],
    },
  );
  let stderr = "";
  child.stderr?.on("data", (b) => {
    stderr += String(b);
  });
  child.on("error", (e) => {
    console.error("[daemon-roundtrip] child error:", e);
  });

  // Poll for the endpoint file.
  const endpointPath = join(home, ".config/libre-cr/endpoint");
  const deadline = Date.now() + 10_000;
  while (Date.now() < deadline) {
    try {
      const s = readFileSync(endpointPath, "utf8").trim();
      if (s) {
        return { endpoint: s, token, home, child };
      }
    } catch {
      // not there yet
    }
    await new Promise((r) => setTimeout(r, 50));
  }
  child.kill("SIGKILL");
  throw new Error(`daemon never published endpoint file. stderr:\n${stderr}`);
}

function killChild(c: ChildProcess): Promise<void> {
  return new Promise((resolveFn) => {
    if (c.exitCode !== null) {
      resolveFn();
      return;
    }
    c.once("exit", () => resolveFn());
    c.kill("SIGTERM");
    // Hard-fallback in case SIGTERM is ignored.
    setTimeout(() => c.kill("SIGKILL"), 1000);
  });
}

beforeAll(async () => {
  const ok = await ensureBinariesBuilt();
  if (!ok) {
    // In CI (LIBRE_CR_E2E_REQUIRED=1) a broken Rust build must fail loudly —
    // a vacuous green E2E suite is worse than a red one.
    if (process.env.LIBRE_CR_E2E_REQUIRED === "1") {
      throw new Error(
        `LIBRE_CR_E2E_REQUIRED=1 but the Rust binaries could not be built — refusing to skip the E2E suite. ${skipReason ?? ""}`,
      );
    }
    console.warn(`[daemon-roundtrip] skipping suite — ${skipReason}`);
    return;
  }
  daemon = await startDaemon();
}, 120_000);

afterAll(async () => {
  if (daemon) {
    await killChild(daemon.child);
  }
});

describe("extension daemon round-trip", () => {
  it("client_pairing_and_health", async () => {
    if (!daemon) return;
    // Mint a pairing code via the running daemon, redeem via DaemonClient.pair().
    const issueResp = await fetch(`${daemon.endpoint}/v1/pair/issue`, {
      method: "POST",
      headers: {
        Authorization: `Bearer ${daemon.token}`,
        "Content-Type": "application/json",
      },
      body: "{}",
    });
    expect(issueResp.ok).toBe(true);
    const issueBody = (await issueResp.json()) as { code: string };
    expect(issueBody.code).toBeTruthy();

    const unauthed = new DaemonClient({ endpoint: daemon.endpoint, token: "" });
    const pair = await unauthed.pair(issueBody.code, "chrome-extension://e2e");
    expect(pair.token).toBe(daemon.token);

    const client = new DaemonClient({ endpoint: daemon.endpoint, token: pair.token });
    const health = await client.getHealth();
    expect(health.ok).toBe(true);
    expect(typeof health.version).toBe("string");
  });

  it("client_sessions_crud", async () => {
    if (!daemon) return;
    const client = new DaemonClient({ endpoint: daemon.endpoint, token: daemon.token });
    const created = await client.createOrUpdateSession(
      "https://github.com/foo/bar/pull/1001",
      {
        owner: "foo",
        repo: "bar",
        number: 1001,
        title: null,
        description: null,
        author: null,
        base_branch: null,
        head_branch: null,
        head_sha: null,
        files_changed: [],
      },
    );
    expect(typeof created.session_id).toBe("string");
    const got = await client.getSession(created.session_id);
    expect(got.session.session_id).toBe(created.session_id);
    const listed = await client.listSessions(50);
    expect(listed.sessions.some((s) => s.session_id === created.session_id)).toBe(true);
    await client.deleteSession(created.session_id);
    // GET on a deleted session should 404.
    await expect(client.getSession(created.session_id)).rejects.toMatchObject({
      status: 404,
    });
  });

  it("client_export_markdown", async () => {
    if (!daemon) return;
    const client = new DaemonClient({ endpoint: daemon.endpoint, token: daemon.token });
    const sess = await client.createOrUpdateSession(
      "https://github.com/foo/bar/pull/1002",
      {
        owner: "foo",
        repo: "bar",
        number: 1002,
        title: "feat: x",
        description: null,
        author: null,
        base_branch: null,
        head_branch: null,
        head_sha: null,
        files_changed: [],
      },
    );
    await client.addNote(sess.session_id, "must fix this", undefined, "critical");
    const exported = (await client.exportSession(sess.session_id, {
      format: "markdown",
    })) as { content: string };
    expect(exported.content).toContain("## Critical");
    expect(exported.content).toContain("must fix this");
  });

  it("ws_session_streams_frames", async () => {
    if (!daemon) return;
    const client = new DaemonClient({ endpoint: daemon.endpoint, token: daemon.token });
    const sess = await client.createOrUpdateSession(
      "https://github.com/foo/bar/pull/1003",
      {
        owner: "foo",
        repo: "bar",
        number: 1003,
        title: null,
        description: null,
        author: null,
        base_branch: null,
        head_branch: null,
        head_sha: null,
        files_changed: [],
      },
    );
    const session = new AskSession(daemon.endpoint, daemon.token, sess.session_id);
    const frames: string[] = [];
    session.onAny((f) => frames.push(f.type));
    // Default mock provider script is empty — but the agent still emits a
    // `done` frame at end-of-turn. That's enough to verify the AskSession
    // open() → done() round-trip.
    await session.open({ question: "anything to note?" });
    session.close();
    expect(frames).toContain("done");
  });

  it("presentation_call_round_trip", async () => {
    if (!daemon) return;
    // Re-spawn the daemon with a scripted mock provider that emits a
    // presentation tool call. The agent dispatches it as a presentation_call
    // server frame; we reply via AskSession.sendPresentationResult and
    // expect the turn to terminate.
    const home = mkdtempSync(join(tmpdir(), "lcr-e2e-pres-"));
    mkdirSync(join(home, ".config/libre-cr"), { recursive: true });
    const configPath = join(home, "review.toml");
    const token = "ext-e2e-token-pres";
    writeFileSync(join(home, ".config/libre-cr/token"), token);
    const script = [
      "[[mock.provider_script]]",
      "delay_ms = 0",
      "",
      "[mock.provider_script.event]",
      'type = "tool_use"',
      'id = "p1"',
      'name = "scroll_to"',
      'input = { file = "src/main.rs", line = 1 }',
      "",
      "[[mock.provider_script]]",
      "delay_ms = 0",
      "",
      "[mock.provider_script.event]",
      'type = "done"',
      "input_tokens = 1",
      "output_tokens = 1",
      'stop_reason = "tool_use"',
      "",
      "[[mock.provider_script]]",
      "delay_ms = 0",
      "",
      "[mock.provider_script.event]",
      'type = "text_delta"',
      'text = "scrolled"',
      "",
      "[[mock.provider_script]]",
      "delay_ms = 0",
      "",
      "[mock.provider_script.event]",
      'type = "done"',
      "input_tokens = 1",
      "output_tokens = 1",
      'stop_reason = "end_turn"',
      "",
    ].join("\n");
    writeFileSync(configPath, buildToml(CODE_BIN, script));

    const child = spawn(
      REVIEW_BIN,
      ["--config", configPath, "serve"],
      {
        env: { ...process.env, HOME: home, RUST_LOG: "warn" },
        stdio: ["ignore", "ignore", "pipe"],
      },
    );
    try {
      // Poll for endpoint.
      const endpointPath = join(home, ".config/libre-cr/endpoint");
      let endpoint = "";
      const deadline = Date.now() + 10_000;
      while (Date.now() < deadline) {
        try {
          const s = readFileSync(endpointPath, "utf8").trim();
          if (s) {
            endpoint = s;
            break;
          }
        } catch { /* not yet */ }
        await new Promise((r) => setTimeout(r, 50));
      }
      expect(endpoint).not.toBe("");

      const client = new DaemonClient({ endpoint, token });
      const sess = await client.createOrUpdateSession(
        "https://github.com/foo/bar/pull/1004",
        {
          owner: "foo",
          repo: "bar",
          number: 1004,
          title: null,
          description: null,
          author: null,
          base_branch: null,
          head_branch: null,
          head_sha: null,
          files_changed: [],
        },
      );
      const session = new AskSession(endpoint, token, sess.session_id);
      let presentationCallId: string | null = null;
      session.on("presentation_call", (f) => {
        presentationCallId = f.call_id;
        // Acknowledge so the agent can proceed past the dispatch.
        session.sendPresentationResult(f.call_id, true, { acknowledged: true });
      });
      await session.open({ question: "scroll please" });
      session.close();
      expect(presentationCallId).toBeTruthy();
    } finally {
      await killChild(child);
    }
  });
});
