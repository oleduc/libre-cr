// Mirror of `libre-cr-common::ws_frames` and `libre-cr-common::error`.
//
// The Rust side is the source of truth; these TypeScript shapes simply
// describe what comes over the wire.

import type { Selection } from "../selection";

/**
 * Mirror of `libre-cr-common::PROTOCOL_VERSION`. The daemon reports its own
 * value in `GET /v1/health` (`protocol_version`); a mismatch is surfaced as a
 * soft warning (console + `ui.protocol_mismatch` diagnostics key), never a
 * hard failure — minor versions are wire-compatible by spec.
 */
export const PROTOCOL_VERSION = 1;

export interface AskInit {
  question: string;
  selection?: Selection;
  verb?: string;
  /**
   * Per-session presentation override. When `true`, the daemon excludes
   * presentation tools for this turn (field exists on the Rust `AskInit`).
   * The extension *also* gates locally: a muted session answers any stray
   * `presentation_call` with `{ok: false, error: "presentation_muted"}`
   * instead of executing it (see `utils/presentation/index.ts`).
   */
  mute_presentations?: boolean;
  /**
   * Daemon turn ids whose tool results should be replayed at full fidelity
   * for this ask — the turns the reviewer has expanded in the panel. Ids not
   * belonging to the session are ignored by the daemon.
   */
  context_turn_ids?: string[];
}

export interface UsageTally {
  input_tokens: number;
  output_tokens: number;
}

export type ServerFrame =
  | { type: "text_delta"; text: string }
  | { type: "tool_call"; call_id: string; name: string; input: unknown }
  | { type: "tool_result"; call_id: string; result_preview: unknown }
  | {
      type: "presentation_call";
      call_id: string;
      tool: string;
      input: Record<string, unknown>;
    }
  | { type: "done"; turn_id: string; usage: UsageTally }
  | { type: "error"; message: string; recoverable: boolean };

export type ClientFrame = {
  type: "presentation_result";
  call_id: string;
  ok: boolean;
  result?: Record<string, unknown>;
  error?: string;
  message?: string;
};

export const SERVER_FRAME_TYPES = new Set([
  "text_delta",
  "tool_call",
  "tool_result",
  "presentation_call",
  "done",
  "error",
]);

/**
 * Runtime validator. Returns `null` for unknown / malformed frames so the
 * caller can drop them with a warning rather than crashing the panel.
 */
export function parseServerFrame(raw: string): ServerFrame | null {
  let obj: unknown;
  try {
    obj = JSON.parse(raw);
  } catch {
    return null;
  }
  if (typeof obj !== "object" || obj === null) return null;
  const t = (obj as { type?: unknown }).type;
  if (typeof t !== "string" || !SERVER_FRAME_TYPES.has(t)) return null;
  return obj as ServerFrame;
}

export type ErrorCategory =
  | "unauthorized"
  | "origin_rejected"
  | "validation_failed"
  | "code_daemon_unavailable"
  | "unknown_repo"
  | "unknown_ref"
  | "worktree_busy"
  | "worktree_pending"
  | "worktree_failed"
  | "not_in_workspace"
  | "unsupported_language"
  | "provider_unauthorized"
  | "provider_rate_limited"
  | "provider_timeout"
  | "internal";

export interface ErrorEnvelope {
  error: ErrorCategory;
  message: string;
  recoverable?: boolean;
  details?: unknown;
}

export interface VerbDescriptor {
  id: string;
  label: string;
  required_selection: "any" | "file" | "range" | "symbol";
  description: string;
  suggested_tools: string[];
}

export interface SessionSummary {
  session_id: string;
  pr_url: string;
  pr_number?: number;
  updated_at?: number | string;
  worktree_path?: string | null;
}

export interface CreateSessionResponse {
  session_id: string;
  worktree_ready: boolean;
  repo_local_path: string | null;
  pending_action?: string | null;
  pr_diff_changed?: boolean;
  head_sha?: string | null;
}

/** One stored turn as `GET /v1/sessions/:id` serializes it. */
export interface SessionTurnRow {
  turn_id: string;
  kind: "question" | "note";
  status: "ok" | "cancelled" | "error";
  question?: string;
  answer?: string;
  user_content?: string;
  severity?: "info" | "suggestion" | "warning" | "critical";
  selection?: unknown;
}

export interface GetSessionResponse {
  session: SessionSummary & { pr_data?: unknown; head_sha?: string | null };
  turns: SessionTurnRow[];
  worktree_ready: boolean;
  /** Worktree orchestration status; `error` is set when `state` is `failed`. */
  status?: { state?: unknown; error?: string | null; pending_action?: string | null } | null;
  head_sha?: string | null;
  last_seen_at?: number | null;
}

export interface PairResponse {
  token: string;
  extension_origin: string;
}
