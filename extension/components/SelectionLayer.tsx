// Observes diff line-number clicks and maintains a current `Selection`.

import { useEffect } from "react";
import type { Selection } from "../utils/selection";
import {
  CODE_CELL_SEL,
  FILE_CONTAINER_SEL,
  NUM_CELL_SEL,
  hitTestLine,
  pickIdentifier,
} from "../utils/github/diff";

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
        // Try to pick the identifier from the line text.
        const td = (target as Element | null)?.closest?.("tr")?.querySelector?.(CODE_CELL_SEL);
        if (td) {
          const text = td.textContent ?? "";
          // Best-effort column: use mouse offset proportional to text length.
          const rect = (td as HTMLElement).getBoundingClientRect();
          const col = Math.max(
            0,
            Math.min(text.length - 1, Math.floor(((ev.clientX - rect.left) / rect.width) * text.length)),
          );
          const ident = pickIdentifier(text, col);
          if (ident) {
            onSelect({
              kind: "symbol",
              file: hit.file,
              line: hit.line,
              column: col,
              identifier: ident,
            });
            return;
          }
        }
      }
      if (ev.shiftKey) {
        onSelect({
          kind: "range",
          file: hit.file,
          start_line: hit.line,
          end_line: hit.line,
        });
        return;
      }
      onSelect({ kind: "line", file: hit.file, line: hit.line });
    };
    document.addEventListener("click", click, true);
    return () => document.removeEventListener("click", click, true);
  }, [enabled, onSelect]);
  return null;
}
