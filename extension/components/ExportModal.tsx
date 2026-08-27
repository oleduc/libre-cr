// Export modal per `07-conversation-and-notes.md` § Export. The reviewer
// picks a content scope (notes / notes+context / full transcript) and a
// format (Markdown / GitHub structured), then either copies a Markdown
// rendering to the clipboard or sees the structured JSON for manual paste.

import { useEffect, useRef, useState } from "react";

import type { DaemonClient } from "../utils/daemon/client";

export type ExportContent = "notes_only" | "notes_plus_context" | "full_transcript";
export type ExportFormatChoice = "markdown" | "github_review";
export type SeverityMin = "any" | "info" | "suggestion" | "warning" | "critical";

export interface ExportModalProps {
  client: DaemonClient;
  sessionId: string;
  onClose: () => void;
}

interface ExportResponseShape {
  content: string;
  structure?: {
    body: string;
    event: string;
    comments: Array<{ path: string; line?: number; body: string }>;
  };
}

export async function copyToClipboard(text: string): Promise<boolean> {
  try {
    const nav = (
      globalThis as unknown as { navigator?: { clipboard?: { writeText?: (s: string) => Promise<void> } } }
    ).navigator;
    if (nav?.clipboard?.writeText) {
      await nav.clipboard.writeText(text);
      return true;
    }
  } catch {
    // ignore
  }
  return false;
}

export function buildExportBody(
  content: ExportContent,
  format: ExportFormatChoice,
  severityMin: SeverityMin,
  includeToolIo = false,
): Record<string, unknown> {
  return {
    format: format === "markdown" ? "markdown" : "github_review",
    filter: {
      include_thinking: content !== "notes_only",
      severity_min: severityMin === "any" ? null : severityMin,
      // Debug log: every tool call's input and result. Only meaningful with context.
      include_tool_io: includeToolIo && content !== "notes_only",
    },
  };
}

const FOCUSABLE_SELECTOR =
  'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

export function ExportModal({ client, sessionId, onClose }: ExportModalProps) {
  const [content, setContent] = useState<ExportContent>("notes_only");
  const [includeToolIo, setIncludeToolIo] = useState(false);
  const [format, setFormat] = useState<ExportFormatChoice>("markdown");
  const [severityMin, setSeverityMin] = useState<SeverityMin>("any");
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<ExportResponseShape | null>(null);
  const [toast, setToast] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const dialogRef = useRef<HTMLDivElement | null>(null);

  // Esc-to-close + Tab focus trap.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
        return;
      }
      if (e.key !== "Tab" || !dialogRef.current) return;
      const focusables = Array.from(
        dialogRef.current.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR),
      ).filter((el) => !el.hasAttribute("aria-hidden"));
      if (focusables.length === 0) return;
      const first = focusables[0]!;
      const last = focusables[focusables.length - 1]!;
      const active = (dialogRef.current.getRootNode() as Document | ShadowRoot).activeElement;
      if (e.shiftKey) {
        if (active === first || !dialogRef.current.contains(active as Node)) {
          e.preventDefault();
          last.focus();
        }
      } else {
        if (active === last) {
          e.preventDefault();
          first.focus();
        }
      }
    };
    document.addEventListener("keydown", onKey, true);
    // Initial focus into the dialog so screen readers and keyboard users
    // land inside the trap.
    queueMicrotask(() => {
      const first = dialogRef.current?.querySelector<HTMLElement>(FOCUSABLE_SELECTOR);
      first?.focus();
    });
    return () => document.removeEventListener("keydown", onKey, true);
  }, [onClose]);

  const onExport = async () => {
    if (busy) return;
    setBusy(true);
    setError(null);
    setResult(null);
    try {
      const body = buildExportBody(content, format, severityMin, includeToolIo);
      const r = (await client.exportSession(sessionId, body)) as ExportResponseShape;
      setResult(r);
      if (format === "markdown") {
        const ok = await copyToClipboard(r.content);
        setToast(ok ? "Copied to clipboard" : "Generated. Copy from preview.");
      } else {
        setToast("Generated — copy as needed below.");
      }
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="libre-cr-export-title"
      data-testid="export-modal"
      ref={dialogRef}
      style={{
        position: "fixed",
        inset: 0,
        background: "rgba(36, 41, 47, 0.4)",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        zIndex: 2147483647,
      }}
    >
      <div
        style={{
          background: "#fff",
          color: "#1f2328",
          borderRadius: 6,
          padding: 16,
          width: 420,
          maxWidth: "92vw",
          maxHeight: "85vh",
          overflow: "auto",
          boxShadow: "0 12px 32px rgba(0,0,0,0.3)",
        }}
      >
        <h2 id="libre-cr-export-title" style={{ margin: 0, fontSize: 16 }}>
          Export review draft
        </h2>
        <section style={{ marginTop: 12 }}>
          <p style={{ margin: "0 0 6px", fontWeight: 600 }}>Content</p>
          <label style={{ display: "block", fontSize: 13 }}>
            <input
              type="radio"
              name="export-content"
              checked={content === "notes_only"}
              onChange={() => setContent("notes_only")}
            />{" "}
            Notes only (clean)
          </label>
          <label style={{ display: "block", fontSize: 13 }}>
            <input
              type="radio"
              name="export-content"
              checked={content === "notes_plus_context"}
              onChange={() => setContent("notes_plus_context")}
            />{" "}
            Notes + investigation context
          </label>
          <label style={{ display: "block", fontSize: 13 }}>
            <input
              type="radio"
              name="export-content"
              checked={content === "full_transcript"}
              onChange={() => setContent("full_transcript")}
            />{" "}
            Full transcript (for personal reference)
          </label>
          <label style={{ display: "block", fontSize: 13, marginTop: 6 }}>
            <input
              type="checkbox"
              data-testid="include-tool-io"
              checked={includeToolIo}
              disabled={content === "notes_only"}
              onChange={(e) => setIncludeToolIo(e.target.checked)}
            />{" "}
            Include tool call log (every input and result — for debugging)
          </label>
        </section>
        <section style={{ marginTop: 12 }}>
          <p style={{ margin: "0 0 6px", fontWeight: 600 }}>Format</p>
          <label style={{ display: "block", fontSize: 13 }}>
            <input
              type="radio"
              name="export-format"
              checked={format === "markdown"}
              onChange={() => setFormat("markdown")}
            />{" "}
            Markdown (clipboard)
          </label>
          <label style={{ display: "block", fontSize: 13 }}>
            <input
              type="radio"
              name="export-format"
              checked={format === "github_review"}
              onChange={() => setFormat("github_review")}
            />{" "}
            GitHub review (structured)
          </label>
        </section>
        <section style={{ marginTop: 12 }}>
          <p style={{ margin: "0 0 6px", fontWeight: 600 }}>Minimum severity</p>
          <select
            value={severityMin}
            onChange={(e) => setSeverityMin(e.target.value as SeverityMin)}
            data-testid="severity-min"
          >
            <option value="any">Any</option>
            <option value="info">Info</option>
            <option value="suggestion">Suggestion</option>
            <option value="warning">Warning</option>
            <option value="critical">Critical</option>
          </select>
        </section>
        {toast ? (
          <div
            data-testid="export-toast"
            style={{
              marginTop: 10,
              padding: "6px 8px",
              background: "#dafbe1",
              border: "1px solid #1f883d",
              borderRadius: 4,
              fontSize: 12,
            }}
          >
            {toast}
          </div>
        ) : null}
        {error ? (
          <div className="libre-cr-error" style={{ marginTop: 10 }}>
            {error}
          </div>
        ) : null}
        {result ? (
          <details
            style={{ marginTop: 10, fontSize: 12 }}
            open={format === "github_review"}
            data-testid="export-preview"
          >
            <summary>Preview</summary>
            {format === "github_review" && result.structure ? (
              <>
                <p style={{ margin: "6px 0 2px", fontWeight: 600 }}>Event: {result.structure.event}</p>
                <p style={{ margin: "2px 0", fontWeight: 600 }}>Body</p>
                <pre
                  style={{
                    background: "#f6f8fa",
                    padding: 6,
                    overflow: "auto",
                    whiteSpace: "pre-wrap",
                    fontSize: 11,
                  }}
                >
                  {result.structure.body}
                </pre>
                {result.structure.comments.length > 0 ? (
                  <>
                    <p style={{ margin: "2px 0", fontWeight: 600 }}>
                      Inline comments ({result.structure.comments.length})
                    </p>
                    <ul>
                      {result.structure.comments.map((c, i) => (
                        <li key={i}>
                          <code>
                            {c.path}
                            {c.line ? `:${c.line}` : ""}
                          </code>{" "}
                          — {c.body}
                          <button
                            style={{ marginLeft: 6 }}
                            onClick={() => void copyToClipboard(c.body)}
                          >
                            copy
                          </button>
                        </li>
                      ))}
                    </ul>
                  </>
                ) : null}
              </>
            ) : (
              <pre
                style={{
                  background: "#f6f8fa",
                  padding: 6,
                  overflow: "auto",
                  whiteSpace: "pre-wrap",
                  fontSize: 11,
                }}
              >
                {result.content}
              </pre>
            )}
          </details>
        ) : null}
        <div style={{ marginTop: 12, display: "flex", gap: 6, justifyContent: "flex-end" }}>
          <button onClick={onClose}>Cancel</button>
          <button className="primary" onClick={onExport} disabled={busy} data-testid="export-go">
            {busy ? "Exporting…" : "Export ▶"}
          </button>
        </div>
      </div>
    </div>
  );
}
