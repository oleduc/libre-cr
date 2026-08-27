// Presentation tool handlers. Per `09-presentation-tools.md`.
//
// All effects are tagged with `data-libre-cr-effect-id` and `data-libre-cr-tag`
// for scoped removal. Annotation text uses `textContent`, never `innerHTML`.

import { CODE_CELL_SEL, ensureFileRendered, fileContainer, findRow } from "../github/diff";

export type PresentationColor = "red" | "yellow" | "green" | "blue" | "purple";
export type PresentationSeverity = "info" | "suggestion" | "warning" | "critical";
export type PresentationScope = "all" | "highlights" | "annotations";

export type PresentationResult =
  | { ok: true; effect_id: string }
  | { ok: false; error: string; message?: string };

export interface PresentationContext {
  /** Auto-increment counter for fresh effect_ids in this session. */
  nextEffectId: () => string;
  /** Document root we apply to — overridable for tests. */
  root: Document;
  /** Whether `open_link` to a new tab is allowed (default: true). */
  allowOpenLinkTab: boolean;
  /** Whether `open_link` to embedded panel is allowed (default: false). */
  allowOpenLinkPanel: boolean;
}

export function makeContext(overrides: Partial<PresentationContext> = {}): PresentationContext {
  let n = 0;
  return {
    nextEffectId: () => `e_${++n}`,
    root: overrides.root ?? globalThis.document,
    allowOpenLinkTab: overrides.allowOpenLinkTab ?? true,
    allowOpenLinkPanel: overrides.allowOpenLinkPanel ?? false,
    ...overrides,
  };
}

const VALID_COLORS: PresentationColor[] = ["red", "yellow", "green", "blue", "purple"];
const VALID_SEVERITIES: PresentationSeverity[] = [
  "info",
  "suggestion",
  "warning",
  "critical",
];

function colorClass(color: PresentationColor): string {
  return `libre-cr-hl-${color}`;
}

function severityClass(sev: PresentationSeverity): string {
  return `libre-cr-sev-${sev}`;
}

/**
 * highlight_lines — overlay a colored background on a range of diff lines.
 */
export function highlightLines(
  ctx: PresentationContext,
  input: {
    file: string;
    start_line: number;
    end_line?: number;
    color?: PresentationColor;
    label?: string;
  },
): PresentationResult {
  if (!input?.file || typeof input.start_line !== "number") {
    return { ok: false, error: "validation_failed", message: "missing file/start_line" };
  }
  const start = input.start_line;
  const end = input.end_line ?? input.start_line;
  const color: PresentationColor = VALID_COLORS.includes(input.color as PresentationColor)
    ? (input.color as PresentationColor)
    : "blue";
  const effectId = ctx.nextEffectId();
  let applied = 0;
  for (let l = start; l <= end; l++) {
    const row = findRow(input.file, l, ctx.root);
    if (!row) continue;
    // Attributes, not classes: GitHub's React rewrites `className` on hover /
    // selection re-renders and would wipe a class; unknown `data-*` survive.
    row.classList.add("libre-cr-effect", colorClass(color));
    row.setAttribute("data-libre-cr-effect-id", effectId);
    row.setAttribute("data-libre-cr-tag", "highlight");
    row.setAttribute("data-libre-cr-color", color);
    if (input.label) {
      row.setAttribute("title", input.label);
      // Caption chip at the right end of the range's first code line, so the
      // reviewer can tie the highlight to the answer without hovering.
      if (applied === 0) {
        const cell = row.querySelector<HTMLElement>(CODE_CELL_SEL);
        if (cell && !cell.querySelector(".libre-cr-label")) {
          const chip = ctx.root.createElement("span");
          chip.className = "libre-cr-label";
          chip.textContent = input.label; // never innerHTML
          cell.appendChild(chip);
        }
      }
    }
    applied++;
  }
  if (applied === 0) {
    return { ok: false, error: "file_not_in_view", message: `no rows for ${input.file}:${start}` };
  }
  return { ok: true, effect_id: effectId };
}

/** annotate_line — inject a row under the target line with summary text. */
export function annotateLine(
  ctx: PresentationContext,
  input: {
    file: string;
    line: number;
    summary: string;
    detail?: string;
    severity?: PresentationSeverity;
  },
): PresentationResult {
  if (!input?.file || typeof input.line !== "number" || !input.summary) {
    return { ok: false, error: "validation_failed" };
  }
  const sev: PresentationSeverity = VALID_SEVERITIES.includes(input.severity as PresentationSeverity)
    ? (input.severity as PresentationSeverity)
    : "info";
  const row = findRow(input.file, input.line, ctx.root);
  if (!row) return { ok: false, error: "file_not_in_view" };
  const effectId = ctx.nextEffectId();
  const tr = ctx.root.createElement("tr");
  tr.setAttribute("data-libre-cr-effect-id", effectId);
  tr.setAttribute("data-libre-cr-tag", "annotation");
  tr.classList.add("libre-cr-effect", "libre-cr-annotation", severityClass(sev));
  const td = ctx.root.createElement("td");
  td.colSpan = 99;
  td.classList.add("libre-cr-annotation-cell");
  const strong = ctx.root.createElement("strong");
  strong.textContent = input.summary; // never innerHTML
  td.appendChild(strong);
  if (input.detail) {
    td.appendChild(ctx.root.createElement("br"));
    const span = ctx.root.createElement("span");
    span.textContent = input.detail; // never innerHTML
    td.appendChild(span);
  }
  tr.appendChild(td);
  row.parentNode?.insertBefore(tr, row.nextSibling);
  return { ok: true, effect_id: effectId };
}

/** scroll_to — scroll to the row + brief flash highlight. */
export function scrollTo(
  ctx: PresentationContext,
  input: { file: string; line?: number },
): PresentationResult {
  if (!input?.file) return { ok: false, error: "validation_failed" };
  if (typeof input.line !== "number") {
    // No line: bring the file itself into view (line 1 is rarely in a hunk).
    const container = fileContainer(input.file, ctx.root);
    if (!container) return { ok: false, error: "file_not_in_view", message: `no diff rendered for ${input.file}` };
    try {
      container.scrollIntoView({ behavior: "smooth", block: "start" });
    } catch {
      // jsdom
    }
    return { ok: true, effect_id: ctx.nextEffectId() };
  }
  const row = findRow(input.file, input.line, ctx.root);
  if (!row) return { ok: false, error: "file_not_in_view", message: `no row ${input.file}:${input.line}` };
  try {
    row.scrollIntoView({ behavior: "smooth", block: "center" });
  } catch {
    // jsdom or older browsers
  }
  const effectId = ctx.nextEffectId();
  row.classList.add("libre-cr-flash");
  row.setAttribute("data-libre-cr-effect-id", effectId);
  row.setAttribute("data-libre-cr-tag", "flash");
  // Auto-clear flash after a moment so the user is back to normal.
  setTimeout(() => {
    row.classList.remove("libre-cr-flash");
  }, 1400);
  return { ok: true, effect_id: effectId };
}

/** open_link — validate + open. */
export function openLink(
  ctx: PresentationContext,
  input: { url: string; target?: "tab" | "panel" },
): PresentationResult {
  if (!input?.url || typeof input.url !== "string") {
    return { ok: false, error: "url_rejected", message: "missing url" };
  }
  if (!isSafeUrl(input.url)) {
    return { ok: false, error: "url_rejected", message: "scheme not allowed" };
  }
  const target = input.target ?? "tab";
  if (target === "panel" && !ctx.allowOpenLinkPanel) {
    return { ok: false, error: "url_rejected", message: "panel target disabled" };
  }
  if (target === "tab" && !ctx.allowOpenLinkTab) {
    return { ok: false, error: "url_rejected", message: "tab target disabled" };
  }
  const effectId = ctx.nextEffectId();
  if (target === "tab") {
    try {
      (globalThis as unknown as { window?: Window }).window?.open?.(input.url, "_blank");
    } catch {
      // ignore
    }
  }
  return { ok: true, effect_id: effectId };
}

/** clear_presentation — remove tagged elements by scope. */
export function clearPresentation(
  ctx: PresentationContext,
  scope: PresentationScope = "all",
): PresentationResult {
  const tagsToClear: string[] =
    scope === "highlights"
      ? ["highlight"]
      : scope === "annotations"
        ? ["annotation"]
        : ["highlight", "annotation", "flash"];
  let count = 0;
  for (const tag of tagsToClear) {
    const els = ctx.root.querySelectorAll<HTMLElement>(`[data-libre-cr-tag="${tag}"]`);
    for (const el of Array.from(els)) {
      if (tag === "annotation" || tag === "flash") {
        el.parentNode?.removeChild(el);
      } else {
        // For highlights, only strip the markers/classes; leave the row.
        el.removeAttribute("data-libre-cr-effect-id");
        el.removeAttribute("data-libre-cr-tag");
        el.removeAttribute("data-libre-cr-color");
        el.removeAttribute("title");
        el.querySelectorAll(".libre-cr-label").forEach((c) => c.remove());
        el.classList.remove(
          "libre-cr-effect",
          ...Array.from(el.classList).filter((c) => c.startsWith("libre-cr-hl-")),
        );
      }
      count++;
    }
  }
  return { ok: true, effect_id: `cleared:${count}` };
}

export function isSafeUrl(u: string): boolean {
  if (u.startsWith("/")) return !u.startsWith("//"); // relative GitHub paths
  try {
    const parsed = new URL(u);
    return parsed.protocol === "https:" || parsed.protocol === "http:" && parsed.hostname === "127.0.0.1";
  } catch {
    return false;
  }
}

/** Single dispatch entry point. */
export async function dispatchPresentationCall(
  ctx: PresentationContext,
  tool: string,
  input: Record<string, unknown>,
): Promise<PresentationResult> {
  // GitHub virtualizes the diff; give the target file a chance to mount first.
  if (
    typeof input?.file === "string" &&
    (tool === "highlight_lines" || tool === "annotate_line" || tool === "scroll_to")
  ) {
    await ensureFileRendered(input.file, ctx.root);
  }
  switch (tool) {
    case "highlight_lines":
      return highlightLines(ctx, input as Parameters<typeof highlightLines>[1]);
    case "annotate_line":
      return annotateLine(ctx, input as Parameters<typeof annotateLine>[1]);
    case "scroll_to":
      return scrollTo(ctx, input as Parameters<typeof scrollTo>[1]);
    case "open_link":
      return openLink(ctx, input as Parameters<typeof openLink>[1]);
    case "clear_presentation":
      return clearPresentation(
        ctx,
        (input?.scope as PresentationScope | undefined) ?? "all",
      );
    default:
      return { ok: false, error: "unknown_tool", message: tool };
  }
}
