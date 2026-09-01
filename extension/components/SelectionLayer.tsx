// Observes diff line-number clicks and maintains a current `Selection`.

import { useEffect } from "react";
import type { Selection } from "../utils/selection";
import {
  CODE_CELL_SEL,
  FILE_CONTAINER_SEL,
  NUM_CELL_SEL,
  hitTestLine,
  pickIdentifier,
  textOfLines,
} from "../utils/github/diff";
import { watchGithubLineSelection } from "../utils/github/gh-selection";

export interface SelectionLayerProps {
  onSelect: (sel: Selection | null) => void;
  enabled?: boolean;
}

export function SelectionLayer({ onSelect, enabled = true }: SelectionLayerProps) {
  useEffect(() => {
    if (!enabled) return;
    const click = (ev: MouseEvent) => {
      const target = ev.target as Node | null;
      // I14: although this listener is document-wide (capture), only clicks
      // that land on diff-table cells inside a per-file diff container (either
      // GitHub DOM — see `utils/github/selectors.ts`) may mutate the
      // selection. Never hijack links, buttons or GitHub's own interactive
      // affordances inside a diff row.
      let el: Element | null = target instanceof Element ? target : null;
      if (!el && target) el = target.parentElement;
      if (!el) return;
      if (el.closest("a, button, input, select, textarea, summary, [role='button']")) return;
      if (!el.closest(FILE_CONTAINER_SEL)) return;
      if (!el.closest(`${NUM_CELL_SEL}, ${CODE_CELL_SEL}`)) return;
      const hit = hitTestLine(target);
      if (!hit) return;
      // Cmd-click → symbol; shift-click → range start/end; plain click → line.
      if (ev.metaKey || ev.ctrlKey) {
        // Try to pick the identifier from the line text — from the *clicked*
        // code cell when there is one (a React replacement row has a left and
        // a right code cell; the row's first match would read deleted text).
        const td =
          el.closest(CODE_CELL_SEL) ??
          (target as Element | null)?.closest?.("tr")?.querySelector?.(CODE_CELL_SEL);
        if (td) {
          const text = td.textContent ?? "";
          // Best-effort column: use mouse offset proportional to text length.
          const rect = (td as HTMLElement).getBoundingClientRect();
          // A zero-width rect (hidden cell, jsdom) would make the ratio NaN
          // and silently disable symbol picking — fall back to column 0.
          const col =
            rect.width > 0
              ? Math.max(
                  0,
                  Math.min(
                    text.length - 1,
                    Math.floor(((ev.clientX - rect.left) / rect.width) * text.length),
                  ),
                )
              : 0;
          const ident = pickIdentifier(text, col);
          if (ident) {
            onSelect({
              kind: "symbol",
              file: hit.file,
              line: hit.line,
              column: col,
              identifier: ident,
              text: textOfLines(hit.file, hit.line, hit.line),
            });
            return;
          }
        }
      }
      // Shift-click is GitHub's range gesture; the hash watcher below turns
      // it into a real range — a local {n,n} pseudo-range would overwrite it.
      if (ev.shiftKey) return;
      onSelect({
        kind: "line",
        file: hit.file,
        line: hit.line,
        text: textOfLines(hit.file, hit.line, hit.line),
      });
    };
    document.addEventListener("click", click, true);
    // GitHub's own gesture: line-number click = one line, shift-click = a
    // range, both published through the URL hash. Riding it gives multi-line
    // selection for free, with GitHub's native row highlight as feedback.
    const unwatch = watchGithubLineSelection(onSelect);
    return () => {
      document.removeEventListener("click", click, true);
      unwatch();
    };
  }, [enabled, onSelect]);
  return null;
}
