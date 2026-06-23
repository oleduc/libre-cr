// Daemon spawn helper for browser-driven E2E tests.
//
// Mirrors the patterns established in `extension/e2e/daemon-roundtrip.test.ts`
// (lazy `cargo build`, tempdir $HOME, mock provider with `code_intel = true`)
// but exposes a per-test `start()` API and adds a `pairingCode` so the
// pairing UI can be exercised end-to-end.

import { spawn, type ChildProcess } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const EXT_ROOT = resolve(HERE, "../..");
const REPO_ROOT = resolve(EXT_ROOT, "..");
const TARGET_DIR = resolve(REPO_ROOT, "target/debug");

export const REVIEW_BIN =
  process.platform === "win32"
    ? join(TARGET_DIR, "libre-cr-review.exe")
    : join(TARGET_DIR, "libre-cr-review");
export const CODE_BIN =
  process.platform === "win32"
    ? join(TARGET_DIR, "libre-cr-code.exe")
    : join(TARGET_DIR, "libre-cr-code");

/**
 * The CORS origin the daemon allows. We set this to `https://github.com`
 * because the content script's fetch happens from the page origin (the
 * fixture page is served at github.com via Playwright route interception).
 * Production extensions face the same constraint — the spec calls it out
 * in `05-browser-extension.md` § Transport from a Content Script.
 */
export const EXTENSION_ORIGIN = "https://github.com";

export interface DaemonHandle {
  endpoint: string;
  token: string;
  pairingCode: string;
  home: string;
  child: ChildProcess;
  kill: () => Promise<void>;
}

export interface StartDaemonOptions {
  /**
   * Inline TOML appended after the base `review.toml`. Use this to script the
   * mock provider with `[[mock.provider_script]]` blocks for a specific test.
   */
  scriptBlock?: string;
  /** Pre-set token; defaults to a stable hex string. */
  token?: string;
}

let buildPromise: Promise<boolean> | null = null;

/**
 * Build `libre-cr-review` and `libre-cr-code` if they aren't already on disk.
 * Cached across calls — the first test pays the cost; the rest get hits.
 */
export async function ensureBinariesBuilt(): Promise<boolean> {
  if (existsSync(REVIEW_BIN) && existsSync(CODE_BIN)) return true;
  if (buildPromise) return buildPromise;
  buildPromise = (async () => {
    const results = await Promise.all(
      ["libre-cr-review", "libre-cr-code"].map(
        (name) =>
          new Promise<{ name: string; ok: boolean; stderr: string }>((resolveFn) => {
            const proc = spawn("cargo", ["build", "-p", name, "--bin", name], {
              cwd: REPO_ROOT,
              env: {
                ...process.env,
                PATH: `${process.env.HOME}/.cargo/bin:${process.env.PATH ?? ""}`,
              },
            });
            let stderr = "";
            proc.stderr?.on("data", (b) => (stderr += String(b)));
            proc.on("close", (code) =>
              resolveFn({ name, ok: code === 0, stderr }),
            );
            proc.on("error", (e) =>
              resolveFn({ name, ok: false, stderr: String(e) }),
            );
          }),
      ),
    );
    const failed = results.find((r) => !r.ok);
    if (failed) {
      console.error(
        `[browser-e2e] cargo build ${failed.name} failed:\n${failed.stderr.slice(-1200)}`,
      );
      return false;
    }
    return existsSync(REVIEW_BIN) && existsSync(CODE_BIN);
  })();
  return buildPromise;
}

function buildToml(codeBin: string, scriptBlock = ""): string {
  return [
    "[server]",
    'bind = "127.0.0.1"',
    "port = 0",
    'endpoint_file = "~/.config/libre-cr/endpoint"',
    'token_file = "~/.config/libre-cr/token"',
    'install_key_file = "~/.config/libre-cr/install_key"',
    `extension_origin = "${EXTENSION_ORIGIN}"`,
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

function killChild(c: ChildProcess): Promise<void> {
  return new Promise((resolveFn) => {
    if (c.exitCode !== null) {
      resolveFn();
      return;
    }
    c.once("exit", () => resolveFn());
    c.kill("SIGTERM");
    setTimeout(() => c.kill("SIGKILL"), 1000);
  });
}

/**
 * Spawn a fresh `libre-cr-review`, wait for it to publish its endpoint file,
 * issue a pairing code through `/v1/pair/issue` and return the lot. Caller is
 * responsible for `await handle.kill()` at end-of-test.
 */
export async function startDaemon(
  opts: StartDaemonOptions = {},
): Promise<DaemonHandle> {
  if (!(await ensureBinariesBuilt())) {
    const required = process.env.LIBRE_CR_E2E_REQUIRED === "1";
    throw new Error(
      required
        ? "LIBRE_CR_E2E_REQUIRED=1 but cargo build of libre-cr-review/libre-cr-code failed — refusing to skip; see stderr above"
        : "cargo build of libre-cr-review/libre-cr-code failed — see stderr above",
    );
  }
  const home = mkdtempSync(join(tmpdir(), "lcr-browser-e2e-"));
  mkdirSync(join(home, ".config/libre-cr"), { recursive: true });
  const configPath = join(home, "review.toml");
  const token = opts.token ?? "ext-browser-e2e-token-deadbeef";
  writeFileSync(join(home, ".config/libre-cr/token"), token);
  writeFileSync(configPath, buildToml(CODE_BIN, opts.scriptBlock ?? ""));

  const child = spawn(REVIEW_BIN, ["--config", configPath, "serve"], {
    env: { ...process.env, HOME: home, RUST_LOG: "warn" },
    stdio: ["ignore", "ignore", "pipe"],
  });
  let stderr = "";
  child.stderr?.on("data", (b) => {
    stderr += String(b);
  });
  child.on("error", (e) => {
    // eslint-disable-next-line no-console
    console.error("[browser-e2e] daemon child error:", e);
  });

  const endpointPath = join(home, ".config/libre-cr/endpoint");
  const deadline = Date.now() + 10_000;
  let endpoint = "";
  while (Date.now() < deadline) {
    try {
      const s = readFileSync(endpointPath, "utf8").trim();
      if (s) {
        endpoint = s;
        break;
      }
    } catch {
      /* not there yet */
    }
    await new Promise((r) => setTimeout(r, 50));
  }
  if (!endpoint) {
    child.kill("SIGKILL");
    throw new Error(
      `daemon never published endpoint file. stderr:\n${stderr.slice(-400)}`,
    );
  }

  // Issue a one-time pairing code so tests can drive the pairing UI.
  const issueResp = await fetch(`${endpoint}/v1/pair/issue`, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${token}`,
      "Content-Type": "application/json",
    },
    body: "{}",
  });
  if (!issueResp.ok) {
    await killChild(child);
    throw new Error(`/v1/pair/issue failed: ${issueResp.status}`);
  }
  const body = (await issueResp.json()) as { code: string };

  return {
    endpoint,
    token,
    pairingCode: body.code,
    home,
    child,
    kill: () => killChild(child),
  };
}
