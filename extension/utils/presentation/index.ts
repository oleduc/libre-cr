// Glue between an `AskSession` and the presentation handlers. The shell
// instantiates one of these per active session.

import type { AskSession } from "../daemon/ws";
import { findRow, scrollIntoViewSettled } from "../github/diff";
import {
  PresentationContext,
  PresentationResult,
  clearPresentation,
  dispatchPresentationCall,
  makeContext,
} from "./handlers";

export interface PresentationEffectRecord {
  effect_id: string;
  tool: string;
  turn_id?: string;
}

/** One presentation call the model made in the current turn, replayable. */
export interface PresentationStep {
  tool: string;
  input: Record<string, unknown>;
}

export interface PresentationManager {
  effects: PresentationEffectRecord[];
  /** Successful presentation calls of the current turn, in order. */
  steps: PresentationStep[];
  /** Re-apply steps 0..=index from a clean page (all steps when omitted). */
  replayTo(index?: number): Promise<void>;
  /** Tour mode: show only step `index` (clean page, apply it, scroll to it). */
  showStep(index: number): Promise<void>;
  /** Forget the recorded steps (start of a new question). */
  resetSteps(): void;
  /** Show or hide the highlight caption chips on the page. */
  setLabelsVisible(visible: boolean): void;
  highlightsCount: number;
  annotationsCount: number;
  attach(session: AskSession): () => void;
  clearAll(): void;
  onChange(handler: (m: PresentationManager) => void): () => void;
  detachAll(): void;
  /**
   * Local mute gate (belt to the daemon's `mute_presentations` suspenders).
   * While muted, incoming `presentation_call` frames are NOT executed; the
   * handler replies `{ok: false, error: "presentation_muted"}` instead of
   * applying any DOM effect. We reply rather than skip `attach()` entirely
   * because the daemon awaits a `presentation_result` for every call — an
   * unanswered call would stall the turn against a daemon that doesn't yet
   * honor the flag.
   */
  setMuted(muted: boolean): void;
}

export interface PresentationManagerOptions {
  context?: Partial<PresentationContext>;
  autoClearOnNewQuestion?: boolean;
}

export function createPresentationManager(
  options: PresentationManagerOptions = {},
): PresentationManager {
  const ctx = makeContext(options.context);
  // A previous manager (panel closed, content script reloaded) may have left
  // the global label-visibility class behind; a fresh manager starts visible.
  ctx.root.documentElement.classList.remove("libre-cr-hide-labels");
  const onChange = new Set<(m: PresentationManager) => void>();

  const state = {
    effects: [] as PresentationEffectRecord[],
    steps: [] as PresentationStep[],
    detachers: [] as (() => void)[],
    muted: false,
    get highlightsCount() {
      return state.effects.filter((e) => e.tool === "highlight_lines").length;
    },
    get annotationsCount() {
      return state.effects.filter((e) => e.tool === "annotate_line").length;
    },
  };

  const fire = () => {
    for (const h of onChange) h(manager);
  };

  const handle = async (
    session: AskSession,
    tool: string,
    call_id: string,
    input: Record<string, unknown>,
  ): Promise<void> => {
    if (state.muted) {
      // Defense in depth: even if the daemon ignores `mute_presentations`
      // and still emits presentation tools, never touch the page DOM while
      // the session is muted. Reply so the agent turn can proceed.
      session.sendPresentationResult(
        call_id,
        false,
        undefined,
        "presentation_muted",
        "Presentations are muted for this session",
      );
      return;
    }
    let outcome: PresentationResult;
    try {
      // Live dispatch: record the effect but never move the viewport — every
      // scroll is reviewer-initiated (tour / replay pass the default).
      outcome = await dispatchPresentationCall(ctx, tool, input, { scroll: false });
    } catch (e) {
      outcome = {
        ok: false,
        error: "internal",
        message: e instanceof Error ? e.message : String(e),
      };
    }
    if (outcome.ok) {
      state.effects.push({ effect_id: outcome.effect_id, tool });
      if (tool !== "clear_presentation" && tool !== "open_link") {
        state.steps.push({ tool, input });
      }
      // Live effects land silently: every scroll is reviewer-initiated (the
      // tour widget opens armed on the first call and waits for a click).
      session.sendPresentationResult(call_id, true, { effect_id: outcome.effect_id });
    } else {
      session.sendPresentationResult(call_id, false, undefined, outcome.error, outcome.message);
    }
    fire();
  };

  const manager: PresentationManager = {
    effects: state.effects,
    steps: state.steps,
    async replayTo(index?: number) {
      const upTo = index === undefined ? state.steps.length - 1 : index;
      clearPresentation(ctx, "all");
      state.effects.length = 0;
      for (const step of state.steps.slice(0, upTo + 1)) {
        const outcome = await dispatchPresentationCall(ctx, step.tool, step.input);
        if (outcome.ok) state.effects.push({ effect_id: outcome.effect_id, tool: step.tool });
      }
      // Stepping is how the reviewer follows the tour: land on the current step.
      const current = state.steps[upTo];
      const line = current && (current.input.start_line ?? current.input.line);
      if (current && typeof current.input.file === "string" && typeof line === "number") {
        const row = findRow(current.input.file, line, ctx.root);
        if (row) await scrollIntoViewSettled(row, "center");
      }
      fire();
    },
    setLabelsVisible(visible: boolean) {
      ctx.root.documentElement.classList.toggle("libre-cr-hide-labels", !visible);
    },
    async showStep(index: number) {
      const step = state.steps[index];
      if (!step) return;
      clearPresentation(ctx, "all");
      state.effects.length = 0;
      const outcome = await dispatchPresentationCall(ctx, step.tool, step.input);
      if (outcome.ok) state.effects.push({ effect_id: outcome.effect_id, tool: step.tool });
      const line = step.input.start_line ?? step.input.line;
      if (typeof step.input.file === "string" && typeof line === "number") {
        const row = findRow(step.input.file, line, ctx.root);
        if (row) await scrollIntoViewSettled(row, "center");
      }
      fire();
    },
    resetSteps() {
      state.steps.length = 0;
      fire();
    },
    get highlightsCount() {
      return state.highlightsCount;
    },
    get annotationsCount() {
      return state.annotationsCount;
    },
    attach(session: AskSession) {
      const off = session.on("presentation_call", (frame) => {
        void handle(session, frame.tool, frame.call_id, frame.input ?? {});
      });
      state.detachers.push(off);
      return off;
    },
    clearAll() {
      clearPresentation(ctx, "all");
      state.effects.length = 0;
      fire();
    },
    onChange(handler) {
      onChange.add(handler);
      return () => onChange.delete(handler);
    },
    detachAll() {
      for (const off of state.detachers) off();
      state.detachers.length = 0;
      // Teardown must not leak the global class: the next manager starts
      // with labelsVisible = true and chips would stay hidden otherwise.
      ctx.root.documentElement.classList.remove("libre-cr-hide-labels");
    },
    setMuted(muted: boolean) {
      state.muted = muted;
    },
  };
  return manager;
}
