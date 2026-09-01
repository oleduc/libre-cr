import { useState } from "react";

import { Markdown } from "./Markdown";

export interface ToolTraceLite {
  call_id: string;
  name: string;
  preview?: string;
}

export type NoteSeverity = "info" | "suggestion" | "warning" | "critical";

export type Turn =
  | {
      kind: "qa";
      id: string;
      question: string;
      /** The selection the question was asked about, e.g. "src/a.ts:34-36". */
      sel?: string;
      answer: string;
      thinking?: ToolTraceLite[];
      error?: string;
      pending?: boolean;
      collapsed?: boolean;
    }
  | {
      kind: "note";
      id: string;
      noteId?: string;
      content: string;
      severity?: NoteSeverity;
      sourceTurnId?: string;
    };

export interface ConversationTurnProps {
  turn: Turn;
  /**
   * Collapse is a *controlled* prop: the parent (QaPanel) owns collapsed
   * state on each turn and decides who collapses when a new question lands.
   * Clicking a collapsed summary asks the parent to expand it.
   */
  onToggleCollapse?: (turnId: string, collapsed: boolean) => void;
  onSaveAsNote?: (
    sourceTurnId: string,
    text: string,
    severity: NoteSeverity,
  ) => Promise<void> | void;
  onEditNote?: (
    noteId: string,
    text: string,
    severity: NoteSeverity,
  ) => Promise<void> | void;
  onDeleteNote?: (noteId: string) => Promise<void> | void;
}

const SEVERITY_GLYPHS: Record<NoteSeverity, string> = {
  info: "ⓘ", // ⓘ
  suggestion: "◆", // ◆
  warning: "⚠", // ⚠
  critical: "✕", // ✕
};

const SEVERITY_COLORS: Record<NoteSeverity, string> = {
  info: "#0969da",
  suggestion: "#8250df",
  warning: "#9a6700",
  critical: "#cf222e",
};

export function severityGlyph(s: NoteSeverity): string {
  return SEVERITY_GLYPHS[s];
}

export function ConversationTurn({
  turn,
  onToggleCollapse,
  onSaveAsNote,
  onEditNote,
  onDeleteNote,
}: ConversationTurnProps) {
  const [expanded, setExpanded] = useState(false);
  // Controlled: derived from the prop every render so the parent's
  // collapse-older-turns logic actually takes effect (round-2 E2).
  const collapsedQa = turn.kind === "qa" && turn.collapsed === true;
  const [saveOpen, setSaveOpen] = useState(false);
  const [saveText, setSaveText] = useState("");
  const [saveSeverity, setSaveSeverity] = useState<NoteSeverity>("info");

  // Note edit state
  const [editing, setEditing] = useState(false);
  const [editText, setEditText] = useState("");
  const [editSeverity, setEditSeverity] = useState<NoteSeverity>("info");
  const [deleteConfirm, setDeleteConfirm] = useState(false);

  if (turn.kind === "note") {
    const sev: NoteSeverity = turn.severity ?? "info";
    const color = SEVERITY_COLORS[sev];
    if (editing) {
      return (
        <div className="libre-cr-turn note" data-kind="note" data-testid="note-turn">
          <div style={{ display: "flex", gap: 6, marginBottom: 4 }}>
            <select
              value={editSeverity}
              onChange={(e) => setEditSeverity(e.target.value as NoteSeverity)}
              data-testid="edit-severity"
            >
              <option value="info">Info</option>
              <option value="suggestion">Suggestion</option>
              <option value="warning">Warning</option>
              <option value="critical">Critical</option>
            </select>
          </div>
          <textarea
            value={editText}
            onChange={(e) => setEditText(e.target.value)}
            style={{ width: "100%", minHeight: 60 }}
            data-testid="edit-textarea"
          />
          <div style={{ display: "flex", gap: 6, justifyContent: "flex-end", marginTop: 4 }}>
            <button onClick={() => setEditing(false)}>Cancel</button>
            <button
              className="primary"
              onClick={() => {
                if (turn.noteId && onEditNote) {
                  void Promise.resolve(onEditNote(turn.noteId, editText, editSeverity)).then(() =>
                    setEditing(false),
                  );
                } else {
                  setEditing(false);
                }
              }}
            >
              Save
            </button>
          </div>
        </div>
      );
    }
    return (
      <div className="libre-cr-turn note" data-kind="note" data-testid="note-turn">
        <span
          aria-label={`severity ${sev}`}
          title={sev}
          style={{ color, marginRight: 6, fontWeight: 600 }}
        >
          {SEVERITY_GLYPHS[sev]}
        </span>
        <span style={{ color: "#57606a" }}>Note:</span> {turn.content}
        {turn.noteId ? (
          <div style={{ marginTop: 4, display: "flex", gap: 6, justifyContent: "flex-end" }}>
            <button
              onClick={() => {
                setEditText(turn.content);
                setEditSeverity(sev);
                setEditing(true);
              }}
              data-testid="edit-note"
            >
              Edit
            </button>
            {deleteConfirm ? (
              <button
                onClick={() => {
                  if (turn.noteId && onDeleteNote) {
                    void Promise.resolve(onDeleteNote(turn.noteId));
                  }
                  setDeleteConfirm(false);
                }}
                style={{ color: "#cf222e" }}
                data-testid="confirm-delete"
              >
                Confirm delete
              </button>
            ) : (
              <button onClick={() => setDeleteConfirm(true)} data-testid="delete-note">
                Delete
              </button>
            )}
          </div>
        ) : null}
      </div>
    );
  }

  // QA turn
  if (collapsedQa) {
    return (
      <div className="libre-cr-turn" data-kind="qa" data-testid="qa-turn">
        <div className="q">Q: {turn.question}</div>
        <div
          style={{ color: "#57606a", fontSize: 12, cursor: "pointer" }}
          onClick={() => onToggleCollapse?.(turn.id, false)}
          data-testid="expand-qa"
        >
          {(turn.answer.split("\n")[0] ?? "").slice(0, 80) || "(empty)"} — click to expand
        </div>
      </div>
    );
  }
  return (
    <div className="libre-cr-turn" data-kind="qa" data-testid="qa-turn">
      <div className="q">
        Q: {turn.question}
        {turn.sel ? <span className="libre-cr-ref">{turn.sel}</span> : null}
      </div>
      {turn.answer ? (
        <Markdown text={turn.answer} />
      ) : (
        <div className="a">{turn.pending ? "…" : ""}</div>
      )}
      {turn.error ? <div className="libre-cr-error">{turn.error}</div> : null}
      {turn.thinking && turn.thinking.length > 0 ? (
        <details
          className="libre-cr-thinking"
          open={expanded}
          onToggle={(e) => setExpanded((e.target as HTMLDetailsElement).open)}
        >
          <summary>
            [▾] thinking trace ({turn.thinking.length} tool call
            {turn.thinking.length === 1 ? "" : "s"})
          </summary>
          <ul>
            {turn.thinking.map((t) => (
              <li key={t.call_id}>
                <code>{t.name}</code>
                {t.preview ? ` — ${t.preview.slice(0, 80)}` : ""}
              </li>
            ))}
          </ul>
        </details>
      ) : null}
      {!turn.pending && turn.answer && onSaveAsNote ? (
        <div style={{ position: "relative", marginTop: 4, textAlign: "right" }}>
          {saveOpen ? (
            <div
              data-testid="save-as-note-overlay"
              style={{
                position: "absolute",
                right: 0,
                bottom: "100%",
                background: "#fff",
                border: "1px solid #d0d7de",
                borderRadius: 4,
                padding: 6,
                width: 260,
                boxShadow: "0 4px 12px rgba(0,0,0,0.1)",
                zIndex: 10,
              }}
            >
              <select
                value={saveSeverity}
                onChange={(e) => setSaveSeverity(e.target.value as NoteSeverity)}
                data-testid="save-severity"
                style={{ marginBottom: 4, width: "100%" }}
              >
                <option value="info">Info</option>
                <option value="suggestion">Suggestion</option>
                <option value="warning">Warning</option>
                <option value="critical">Critical</option>
              </select>
              <textarea
                value={saveText}
                onChange={(e) => setSaveText(e.target.value)}
                style={{ width: "100%", minHeight: 60 }}
                data-testid="save-textarea"
              />
              <div style={{ display: "flex", gap: 4, justifyContent: "flex-end", marginTop: 4 }}>
                <button onClick={() => setSaveOpen(false)}>Cancel</button>
                <button
                  className="primary"
                  data-testid="save-as-note-confirm"
                  onClick={() => {
                    if (onSaveAsNote) {
                      void Promise.resolve(
                        onSaveAsNote(turn.id, saveText, saveSeverity),
                      ).then(() => {
                        setSaveOpen(false);
                      });
                    }
                  }}
                >
                  Save
                </button>
              </div>
            </div>
          ) : null}
          <button
            onClick={() => {
              setSaveText(turn.answer);
              setSaveSeverity("info");
              setSaveOpen((v) => !v);
            }}
            data-testid="save-as-note"
            style={{ fontSize: 11 }}
          >
            Save as note ▼
          </button>
        </div>
      ) : null}
    </div>
  );
}
