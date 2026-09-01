import { useCallback, useEffect, useRef, useState } from "react";

import type { DaemonClient } from "../utils/daemon/client";
import { daemonWsFactory } from "../utils/daemon/proxy";
import { AskSession } from "../utils/daemon/ws";
import type { VerbDescriptor } from "../utils/daemon/frames";
import { getKey, setKey } from "../utils/daemon/storage";
import { createPresentationManager } from "../utils/presentation";
import type { Selection } from "../utils/selection";
import { selectionLabel, selectionSatisfies } from "../utils/selection";
import { ConversationTurn, type NoteSeverity, type Turn } from "./ConversationTurn";
import { ExportModal } from "./ExportModal";
import { TourWidget } from "./TourWidget";

export interface QaPanelProps {
  client: DaemonClient;
  sessionId: string;
  /** Conversation restored from the daemon (page reload / reopened PR). */
  initialTurns?: Turn[];
  prSlug: string;
  selection: Selection | null;
  onClearSelection: () => void;
  onClose: () => void;
  warnings?: string[];
  /** If true, the daemon detected a new head_sha since this session was last seen. */
  prDiffChanged?: boolean;
  /** The current head_sha for this PR (used to scope dismissal). */
  headSha?: string | null;
}

let turnSeq = 0;
const newTurnId = () => `t_${++turnSeq}_${Date.now()}`;

export function QaPanel(props: QaPanelProps) {
  const [verbs, setVerbs] = useState<VerbDescriptor[]>([]);
  const [turns, setTurns] = useState<Turn[]>(props.initialTurns ?? []);
  // A late-arriving restore only fills an untouched conversation.
  const restoredRef = useRef(false);
  useEffect(() => {
    if (restoredRef.current || !props.initialTurns?.length) return;
    restoredRef.current = true;
    setTurns((t) => (t.length === 0 ? props.initialTurns! : t));
  }, [props.initialTurns]);
  const [question, setQuestion] = useState("");
  const [asking, setAsking] = useState(false);
  const [effects, setEffects] = useState({ highlights: 0, annotations: 0, steps: 0 });
  /** Replay cursor into the current turn's presentation steps (-1 = none). */
  const [stepIndex, setStepIndex] = useState(-1);
  const [labelsVisible, setLabelsVisible] = useState(true);
  const [tourOpen, setTourOpen] = useState(false);
  /** Armed = opened by the assistant's presentation, waiting for the reviewer's first click. */
  const [tourArmed, setTourArmed] = useState(false);
  const autoOpenedRef = useRef(false);
  const openTour = useCallback((i: number) => {
    setTourOpen(true);
    setTourArmed(false);
    setStepIndex(i);
    void presentationRef.current.showStep(i);
  }, []);
  const [error, setError] = useState<string | null>(null);
  const [exportOpen, setExportOpen] = useState(false);
  const [bannerDismissed, setBannerDismissed] = useState(false);
  const [showFirstPair, setShowFirstPair] = useState(false);
  const [presentationsMuted, setPresentationsMuted] = useState(false);
  const presentationRef = useRef(createPresentationManager());

  useEffect(() => {
    const m = presentationRef.current;
    const off = m.onChange((mm) => {
      setEffects({
        highlights: mm.highlightsCount,
        annotations: mm.annotationsCount,
        steps: mm.steps.length,
      });
      // The assistant's first presentation call of a turn opens the tour,
      // armed: nothing scrolls until the reviewer clicks.
      if (mm.steps.length > 0 && !autoOpenedRef.current) {
        autoOpenedRef.current = true;
        setTourArmed(true);
        setTourOpen(true);
      }
    });
    return () => {
      off();
      // Spec § Effect Lifecycle: clear all DOM effects and detach WS listeners
      // on unmount (Turbo nav, content-script tearDown, panel close).
      m.clearAll();
      m.detachAll();
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    props.client
      .getVerbs()
      .then((v) => {
        if (!cancelled) setVerbs(v);
      })
      .catch((e) => {
        if (!cancelled) setError(`Failed to load verbs: ${(e as Error).message}`);
      });
    return () => {
      cancelled = true;
    };
  }, [props.client]);

  // Per-session diff-change banner dismissal.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      const map = (await getKey("ui.diff_change_dismissed")) ?? {};
      const stored = map[props.sessionId];
      if (!cancelled && stored && props.headSha && stored === props.headSha) {
        setBannerDismissed(true);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [props.sessionId, props.headSha]);

  // Per-session presentation mute toggle — restore on mount.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      const map = (await getKey("session.presentations_muted")) ?? {};
      if (!cancelled && map[props.sessionId]) {
        setPresentationsMuted(true);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [props.sessionId]);

  const togglePresentationsMuted = useCallback(async () => {
    const next = !presentationsMuted;
    setPresentationsMuted(next);
    const map = (await getKey("session.presentations_muted")) ?? {};
    if (next) {
      map[props.sessionId] = true;
    } else {
      delete map[props.sessionId];
    }
    await setKey("session.presentations_muted", map);
  }, [presentationsMuted, props.sessionId]);

  // Local mute gate (E1): keep the presentation manager's muted flag in sync
  // so `presentation_call` frames are answered with `presentation_muted`
  // instead of executed — even if the daemon doesn't honor the
  // `mute_presentations` AskInit flag.
  useEffect(() => {
    presentationRef.current.setMuted(presentationsMuted);
  }, [presentationsMuted]);

  // First-pair onboarding banner — one-time after pairing.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      const seen = await getKey("onboarding.first_pair_seen");
      if (!cancelled && !seen) {
        setShowFirstPair(true);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const dismissDiffBanner = () => {
    setBannerDismissed(true);
    if (props.headSha) {
      void (async () => {
        const map = (await getKey("ui.diff_change_dismissed")) ?? {};
        map[props.sessionId] = props.headSha as string;
        await setKey("ui.diff_change_dismissed", map);
      })();
    }
  };

  const dismissFirstPair = () => {
    setShowFirstPair(false);
    void setKey("onboarding.first_pair_seen", true);
  };

  const runAsk = useCallback(
    async (questionText: string, verb?: string) => {
      if (asking) return;
      setAsking(true);
      setError(null);
      // Auto-clear previous effects and start a fresh replay list.
      presentationRef.current.clearAll();
      presentationRef.current.resetSteps();
      setStepIndex(-1);
      setTourOpen(false);
      setTourArmed(false);
      autoOpenedRef.current = false;
      const turnId = newTurnId();
      // Mark earlier qa turns collapsed.
      setTurns((all) =>
        all.map((t) => (t.kind === "qa" ? { ...t, collapsed: true } : t)),
      );
      const newTurn: Turn = {
        kind: "qa",
        id: turnId,
        question: questionText,
        sel: props.selection ? selectionLabel(props.selection) : undefined,
        answer: "",
        thinking: [],
        pending: true,
        collapsed: false,
      };
      setTurns((t) => [...t, newTurn]);
      const session = new AskSession(
        props.client.auth.endpoint,
        props.client.auth.token,
        props.sessionId,
        { wsFactory: daemonWsFactory() },
      );
      presentationRef.current.attach(session);
      session.on("text_delta", (f) => {
        setTurns((all) =>
          all.map((t) =>
            t.kind === "qa" && t.id === turnId ? { ...t, answer: t.answer + f.text } : t,
          ),
        );
      });
      session.on("tool_call", (f) => {
        setTurns((all) =>
          all.map((t) =>
            t.kind === "qa" && t.id === turnId
              ? { ...t, thinking: [...(t.thinking ?? []), { call_id: f.call_id, name: f.name }] }
              : t,
          ),
        );
      });
      session.on("tool_result", (f) => {
        setTurns((all) =>
          all.map((t) => {
            if (t.kind !== "qa" || t.id !== turnId) return t;
            const preview = JSON.stringify(f.result_preview).slice(0, 200);
            return {
              ...t,
              thinking: (t.thinking ?? []).map((tt) =>
                tt.call_id === f.call_id ? { ...tt, preview } : tt,
              ),
            };
          }),
        );
      });
      try {
        await session.open({
          question: questionText,
          selection: props.selection ?? undefined,
          verb,
          mute_presentations: presentationsMuted || undefined,
        });
      } catch (e) {
        const msg = (e as Error).message;
        setTurns((all) =>
          all.map((t) =>
            t.kind === "qa" && t.id === turnId ? { ...t, error: msg, pending: false } : t,
          ),
        );
        setError(msg);
      } finally {
        setTurns((all) =>
          all.map((t) => (t.kind === "qa" && t.id === turnId ? { ...t, pending: false } : t)),
        );
        session.close();
        setAsking(false);
      }
    },
    [asking, presentationsMuted, props.client, props.selection, props.sessionId],
  );

  const onAsk = () => {
    const q = question.trim();
    if (!q) return;
    setQuestion("");
    void runAsk(q);
  };

  const onAddNote = () => {
    const q = question.trim();
    if (!q) return;
    setQuestion("");
    void props.client
      .addNote(props.sessionId, q, props.selection ?? undefined)
      .then((r) => {
        setTurns((t) => [
          ...t,
          { kind: "note", id: `n_${Date.now()}`, noteId: r.note_id, content: q, severity: "info" },
        ]);
      })
      .catch((e) => setError(`Note failed: ${(e as Error).message}`));
  };

  const onSaveAsNote = useCallback(
    async (sourceTurnId: string, text: string, severity: NoteSeverity) => {
      try {
        const r = await props.client.addNote(
          props.sessionId,
          text,
          props.selection ?? undefined,
          severity,
          sourceTurnId,
        );
        setTurns((t) => [
          ...t,
          {
            kind: "note",
            id: `n_${Date.now()}`,
            noteId: r.note_id,
            content: text,
            severity,
            sourceTurnId,
          },
        ]);
      } catch (e) {
        setError(`Save as note failed: ${(e as Error).message}`);
      }
    },
    [props.client, props.selection, props.sessionId],
  );

  const onEditNote = useCallback(
    async (noteId: string, text: string, severity: NoteSeverity) => {
      try {
        await props.client.updateNote(props.sessionId, noteId, { content: text, severity });
        setTurns((all) =>
          all.map((t) =>
            t.kind === "note" && t.noteId === noteId ? { ...t, content: text, severity } : t,
          ),
        );
      } catch (e) {
        setError(`Edit note failed: ${(e as Error).message}`);
      }
    },
    [props.client, props.sessionId],
  );

  const onDeleteNote = useCallback(
    async (noteId: string) => {
      try {
        await props.client.deleteNote(props.sessionId, noteId);
        setTurns((all) => all.filter((t) => !(t.kind === "note" && t.noteId === noteId)));
      } catch (e) {
        setError(`Delete note failed: ${(e as Error).message}`);
      }
    },
    [props.client, props.sessionId],
  );

  const onToggleCollapse = useCallback((turnId: string, collapsed: boolean) => {
    setTurns((all) =>
      all.map((t) => (t.kind === "qa" && t.id === turnId ? { ...t, collapsed } : t)),
    );
  }, []);

  const showDiffBanner = props.prDiffChanged === true && !bannerDismissed;

  return (
    <>
      <div className="libre-cr-titlebar">
        <span>Libre CR · {props.prSlug}</span>
        <span style={{ display: "flex", gap: 4 }}>
          <button
            onClick={() => void togglePresentationsMuted()}
            aria-label={presentationsMuted ? "unmute presentations" : "mute presentations"}
            aria-pressed={presentationsMuted}
            title={
              presentationsMuted
                ? "Presentations muted for this session"
                : "Mute presentation tools for this session"
            }
            data-testid="toggle-mute-presentations"
          >
            {presentationsMuted ? "🔇" : "🔊"}
          </button>
          <button
            onClick={() => setExportOpen(true)}
            aria-label="export"
            title="Export review draft"
            data-testid="open-export"
          >
            ⇪
          </button>
          <button onClick={props.onClose} aria-label="close" title="Close">
            ×
          </button>
        </span>
      </div>
      {showDiffBanner ? (
        <div className="libre-cr-banner" data-testid="diff-changed-banner">
          PR diff changed since your last review · review your notes
          <button
            style={{ marginLeft: 8 }}
            onClick={dismissDiffBanner}
            aria-label="dismiss"
            data-testid="diff-changed-dismiss"
          >
            Dismiss
          </button>
        </div>
      ) : null}
      {showFirstPair ? (
        <div className="libre-cr-banner" data-testid="first-pair-banner">
          Paired! Ask a question with the verbs below, or type something free-form.
          <button
            style={{ marginLeft: 8 }}
            onClick={dismissFirstPair}
            aria-label="dismiss"
            data-testid="first-pair-dismiss"
          >
            Got it
          </button>
        </div>
      ) : null}
      {props.warnings && props.warnings.length > 0 ? (
        <div className="libre-cr-banner">
          Selector warnings:
          <ul style={{ margin: "4px 0 0 16px" }}>
            {props.warnings.slice(0, 3).map((w, i) => (
              <li key={i}>{w}</li>
            ))}
          </ul>
        </div>
      ) : null}
      <div className="libre-cr-selection">
        <span>
          {props.selection ? `Selection: ${selectionLabel(props.selection)}` : "No selection"}
        </span>
        {props.selection ? (
          <button onClick={props.onClearSelection} aria-label="clear selection">
            ×
          </button>
        ) : null}
      </div>
      <div
        className="libre-cr-conversation"
        role="log"
        aria-live="polite"
        aria-relevant="additions text"
      >
        {turns.length === 0 ? (
          <div data-testid="empty-state" style={{ color: "#57606a", fontStyle: "italic" }}>
            <strong>What is Libre CR?</strong> Ask focused questions about this PR. Use a verb
            for guided investigations (find callers, history, related tests…) or type a
            free-form question below.
          </div>
        ) : null}
        {turns.map((t) => (
          <ConversationTurn
            key={t.id}
            turn={t}
            onToggleCollapse={onToggleCollapse}
            onSaveAsNote={onSaveAsNote}
            onEditNote={onEditNote}
            onDeleteNote={onDeleteNote}
          />
        ))}
      </div>
      <div className="libre-cr-verbs" data-testid="verbs">
        {verbs.map((v) => {
          const enabled = selectionSatisfies(v.required_selection, props.selection);
          return (
            <button
              key={v.id}
              disabled={!enabled || asking}
              title={v.description}
              onClick={() => void runAsk(v.label, v.id)}
            >
              {v.label}
            </button>
          );
        })}
        <span
          data-testid="freeform-hint"
          style={{ color: "#57606a", fontSize: 11, marginLeft: 4, alignSelf: "center" }}
        >
          · or type a free-form question
        </span>
      </div>
      <div className="libre-cr-input">
        <textarea
          value={question}
          placeholder="Ask a question about the selection..."
          onChange={(e) => setQuestion(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              onAsk();
            }
          }}
          disabled={asking}
        />
        <div className="libre-cr-input-actions">
          <button onClick={onAddNote} disabled={!question.trim() || asking}>
            Add note
          </button>
          <button
            className="primary"
            onClick={onAsk}
            disabled={!question.trim() || asking}
          >
            {asking ? "Asking…" : "Ask ▶"}
          </button>
        </div>
      </div>
      <div className="libre-cr-footer">
        <span>
          {effects.highlights} highlight{effects.highlights === 1 ? "" : "s"} ·{" "}
          {effects.annotations} annotation{effects.annotations === 1 ? "" : "s"}
        </span>
        {effects.steps > 0 ? (
          <button
            className="primary"
            data-testid="start-tour"
            title="Step through what the assistant highlighted, with its explanation"
            onClick={() => openTour(0)}
          >
            Tour ({effects.steps})
          </button>
        ) : null}
        <button
          aria-pressed={labelsVisible}
          title={labelsVisible ? "Hide the captions next to highlights" : "Show the captions next to highlights"}
          onClick={() => {
            const v = !labelsVisible;
            setLabelsVisible(v);
            presentationRef.current.setLabelsVisible(v);
          }}
        >
          {labelsVisible ? "Captions on" : "Captions off"}
        </button>
        <button
          onClick={() => presentationRef.current.clearAll()}
          title="Remove highlights and annotations from the diff (the conversation stays)"
        >
          Clear highlights
        </button>
      </div>
      {error ? <div className="libre-cr-error">{error}</div> : null}
      {tourOpen && effects.steps > 0 ? (
        <TourWidget
          steps={presentationRef.current.steps}
          index={Math.max(0, stepIndex)}
          armed={tourArmed}
          onStart={() => openTour(0)}
          onStep={openTour}
          onShowAll={() => {
            setStepIndex(effects.steps - 1);
            void presentationRef.current.replayTo();
          }}
          onClose={() => setTourOpen(false)}
        />
      ) : null}
      {exportOpen ? (
        <ExportModal
          client={props.client}
          sessionId={props.sessionId}
          onClose={() => setExportOpen(false)}
        />
      ) : null}
    </>
  );
}
