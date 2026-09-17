// Top-level React component mounted into the Shadow DOM.

import { useCallback, useEffect, useState } from "react";

import { DaemonClient, DaemonError } from "../utils/daemon/client";
import { daemonFetch } from "../utils/daemon/proxy";
import { getDaemonAuth } from "../utils/daemon/storage";
import { scrapePr } from "../utils/github/scrape";
import type { Selection } from "../utils/selection";
import { PANEL_STYLES } from "./styles";
import { QaPanel } from "./QaPanel";
import { SelectionLayer } from "./SelectionLayer";
import type { SessionTurnRow } from "../utils/daemon/frames";
import { selectionLabel } from "../utils/selection";
import type { Turn } from "./ConversationTurn";

/** Whether two selections name the same thing, so re-emitting one is not a
 *  state change. Compared by value: the watcher builds a fresh object each
 *  time it reads the URL hash. */
function sameSelection(a: Selection | null, b: Selection | null): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  return JSON.stringify(a) === JSON.stringify(b);
}

/** Rebuild the panel conversation from the daemon's stored turns. */
export function turnsFromSession(rows: SessionTurnRow[]): Turn[] {
  const out: Turn[] = [];
  for (const r of rows) {
    if (r.kind === "note") {
      out.push({
        kind: "note",
        id: r.turn_id,
        noteId: r.turn_id,
        content: r.user_content ?? "",
        severity: r.severity,
      });
      continue;
    }
    let sel: string | undefined;
    if (r.selection && typeof r.selection === "object") {
      try {
        sel = selectionLabel(r.selection as Selection);
      } catch {
        sel = undefined;
      }
    }
    out.push({
      kind: "qa",
      id: r.turn_id,
      daemonTurnId: r.turn_id,
      presentation: r.presentation,
      question: r.question ?? "",
      sel,
      answer: r.answer ?? "",
      collapsed: true,
      error:
        r.status === "cancelled"
          ? "turn cancelled — no answer"
          : r.status === "error"
            ? "turn failed"
            : undefined,
    });
  }
  return out;
}
import { Shell } from "./Shell";

interface AppState {
  status: "loading" | "not_paired" | "preparing" | "ready" | "error";
  sessionId?: string;
  message?: string;
  warnings: string[];
  prDiffChanged?: boolean;
  /** Why the worktree is not ready: a first clone, or an update to newer
   *  commits. The wait is the same; what the reviewer should expect is not. */
  pendingAction?: string | null;
  headSha?: string | null;
}

export interface ContentAppProps {
  prUrl: string;
  styleEl: HTMLStyleElement;
}

export function ContentApp({ prUrl, styleEl }: ContentAppProps) {
  const [client, setClient] = useState<DaemonClient | null>(null);
  const [state, setState] = useState<AppState>({ status: "loading", warnings: [] });
  // null = undecided: the panel opens itself only when the PR has history
  // (or something needs attention); a fresh PR stays closed behind the CR
  // button. An explicit user toggle always wins.
  const [open, setOpen] = useState<boolean | null>(null);
  const [history, setHistory] = useState<Turn[] | null>(null);
  const [selection, setSelection] = useState<Selection | null>(null);
  const [prSlug, setPrSlug] = useState(prUrl);

  // Inject styles once.
  useEffect(() => {
    styleEl.textContent = PANEL_STYLES;
  }, [styleEl]);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      const auth = await getDaemonAuth();
      if (cancelled) return;
      if (!auth) {
        setState({ status: "not_paired", warnings: [] });
        return;
      }
      const c = new DaemonClient(
        { endpoint: auth.endpoint, token: auth.token },
        { fetch: daemonFetch() },
      );
      setClient(c);
      const scrape = scrapePr();
      if (scrape.data.owner && scrape.data.repo && scrape.data.number) {
        setPrSlug(`${scrape.data.owner}/${scrape.data.repo}#${scrape.data.number}`);
      }
      const loadHistory = async (sessionId: string) => {
        try {
          const detail = await c.getSession(sessionId);
          if (!cancelled) setHistory(turnsFromSession(detail.turns ?? []));
        } catch {
          if (!cancelled) setHistory([]);
        }
      };
      try {
        const sess = await c.createOrUpdateSession(prUrl, scrape.data);
        if (cancelled) return;
        void loadHistory(sess.session_id);
        if (sess.worktree_ready) {
          setState({
            status: "ready",
            sessionId: sess.session_id,
            warnings: scrape.warnings,
            prDiffChanged: sess.pr_diff_changed,
            headSha: sess.head_sha,
          });
        } else {
          setState({
            status: "preparing",
            sessionId: sess.session_id,
            warnings: scrape.warnings,
            prDiffChanged: sess.pr_diff_changed,
            headSha: sess.head_sha,
            pendingAction: sess.pending_action,
          });
          await pollUntilReady(c, sess.session_id, (error) => {
            if (cancelled) return;
            setState((s) =>
              error === null
                ? { ...s, status: "ready" }
                : { ...s, status: "error", message: `Could not prepare the repo: ${error}` },
            );
          });
        }
      } catch (e) {
        const err = e as DaemonError;
        setState({
          status: "error",
          message: `${err.category}: ${err.message}`,
          warnings: scrape.warnings,
        });
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [prUrl]);

  // Auto-open once things settle: history → open on it; a problem → open so
  // it is visible; a clean, empty session → stay closed behind the button.
  useEffect(() => {
    if (open !== null) return;
    if (state.status === "error" || state.status === "not_paired") {
      setOpen(true);
      return;
    }
    // A branch update is worth seeing: the reviewer has history on this PR (a
    // stored head SHA is how we know it moved), so the panel is about to open
    // anyway — and a message nobody can see is not a message.
    if (state.status === "preparing" && state.pendingAction === "worktree_updating") {
      setOpen(true);
      return;
    }
    if (state.status === "ready" && history !== null) {
      setOpen(history.length > 0);
    }
  }, [open, state.status, state.pendingAction, history]);

  // "Ask about this" on a review comment is an explicit request to ask, so it
  // opens the panel. A line/range selection does not: it rides GitHub's own
  // line-number gesture, and hijacking that to pop a panel open would be rude.
  const onSelect = useCallback((sel: Selection | null) => {
    // Bail out when the same place is selected again. React skips the render
    // for identical state, which is the third guard against a selection loop:
    // the watcher re-emits on re-attach, and an unchanged selection must not
    // become a render.
    setSelection((prev) => (sameSelection(prev, sel) ? prev : sel));
    if (sel?.kind === "comment") setOpen(true);
  }, []);

  if (!open) {
    return (
      <>
        {/* Mounted while collapsed too — otherwise the only way to select
            anything is to open the panel first, which the affordance's whole
            point is to skip. */}
        <SelectionLayer onSelect={onSelect} />
        <button
          type="button"
          className="libre-cr-reopen"
          title="Open Libre CR"
          aria-label="Open Libre CR"
          onClick={() => setOpen(true)}
        >
          CR
        </button>
      </>
    );
  }

  return (
    <>
      <SelectionLayer onSelect={onSelect} />
      <Shell prUrl={prUrl}>
        {state.status === "loading" ? (
          <div className="libre-cr-titlebar">Libre CR — loading…</div>
        ) : null}
        {state.status === "not_paired" ? (
          <>
            <div className="libre-cr-titlebar">Libre CR</div>
            <div className="libre-cr-banner">
              Not paired with a local daemon. Open the extension options to pair.
            </div>
          </>
        ) : null}
        {state.status === "preparing" ? (
          state.pendingAction === "worktree_updating" ? (
            <>
              <div className="libre-cr-titlebar">Libre CR — updating the branch…</div>
              <div className="libre-cr-banner" data-testid="worktree-updating">
                This PR has new commits. Fetching them before answering, so the code read here is
                the code the page is showing.
              </div>
            </>
          ) : (
            <>
              <div className="libre-cr-titlebar">Libre CR — preparing repo…</div>
              <div className="libre-cr-banner">
                Worktree is being prepared. The first visit to a repo clones it, which can take a minute.
              </div>
            </>
          )
        ) : null}
        {state.status === "error" ? (
          <>
            <div className="libre-cr-titlebar">Libre CR — error</div>
            <div className="libre-cr-error">{state.message}</div>
          </>
        ) : null}
        {state.status === "ready" && client && state.sessionId ? (
          <QaPanel
            client={client}
            sessionId={state.sessionId}
            initialTurns={history ?? undefined}
            prSlug={prSlug}
            selection={selection}
            onClearSelection={() => setSelection(null)}
            onClose={() => setOpen(false)}
            warnings={state.warnings}
            prDiffChanged={state.prDiffChanged}
            headSha={state.headSha ?? null}
          />
        ) : null}
      </Shell>
    </>
  );
}

/** Poll until the worktree is ready (`done(null)`) or the daemon reports a
 *  failure (`done(message)`). A first visit to a repo includes a full clone,
 *  so the deadline is generous; failures still surface immediately. */
async function pollUntilReady(
  client: DaemonClient,
  sessionId: string,
  done: (error: string | null) => void,
): Promise<void> {
  const start = Date.now();
  let delay = 500;
  while (Date.now() - start < 10 * 60_000) {
    try {
      const r = await client.getSession(sessionId);
      if (r.worktree_ready) {
        done(null);
        return;
      }
      if (r.status?.error) {
        done(r.status.error);
        return;
      }
    } catch {
      // keep polling
    }
    await new Promise((res) => setTimeout(res, delay));
    delay = Math.min(delay * 1.5, 4000);
  }
  done("timed out waiting for the worktree");
}
