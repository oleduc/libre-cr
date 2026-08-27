// Glue between an `AskSession` and the presentation handlers. The shell
// instantiates one of these per active session.

import type { AskSession } from "../daemon/ws";
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

export interface PresentationManager {
  effects: PresentationEffectRecord[];
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
  const onChange = new Set<(m: PresentationManager) => void>();

  const state = {
    effects: [] as PresentationEffectRecord[],
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
      outcome = await dispatchPresentationCall(ctx, tool, input);
    } catch (e) {
      outcome = {
        ok: false,
        error: "internal",
        message: e instanceof Error ? e.message : String(e),
      };
    }
    if (outcome.ok) {
      state.effects.push({ effect_id: outcome.effect_id, tool });
      session.sendPresentationResult(call_id, true, { effect_id: outcome.effect_id });
    } else {
      session.sendPresentationResult(call_id, false, undefined, outcome.error, outcome.message);
    }
    fire();
  };

  const manager: PresentationManager = {
    effects: state.effects,
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
    },
    setMuted(muted: boolean) {
      state.muted = muted;
    },
  };
  return manager;
}
