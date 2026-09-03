// Page-level CSS for presentation effects. These classes land on GitHub's own
// diff rows (outside our shadow root), so the panel stylesheet can't reach
// them. Installed as a constructed stylesheet: CSSOM insertion is not subject
// to the page's `style-src` CSP, unlike an inline <style>.

export const EFFECT_STYLES = `
/* Keyed on data-* attributes: GitHub's React rewrites className on re-render. */
tr[data-libre-cr-tag="highlight"] > td { background: var(--libre-cr-bg) !important; box-shadow: inset 4px 0 0 var(--libre-cr-hl); }
tr[data-libre-cr-tag="highlight"]                            { --libre-cr-hl: #0969da; --libre-cr-bg: rgba(9, 105, 218, 0.16); }
tr[data-libre-cr-tag="highlight"][data-libre-cr-color="red"]    { --libre-cr-hl: #cf222e; --libre-cr-bg: rgba(207, 34, 46, 0.18); }
tr[data-libre-cr-tag="highlight"][data-libre-cr-color="yellow"] { --libre-cr-hl: #bf8700; --libre-cr-bg: rgba(191, 135, 0, 0.22); }
tr[data-libre-cr-tag="highlight"][data-libre-cr-color="green"]  { --libre-cr-hl: #1a7f37; --libre-cr-bg: rgba(26, 127, 55, 0.18); }
tr[data-libre-cr-tag="highlight"][data-libre-cr-color="purple"] { --libre-cr-hl: #8250df; --libre-cr-bg: rgba(130, 80, 223, 0.16); }
tr[data-libre-cr-tag="annotation"] > td.libre-cr-annotation-cell {
  padding: 6px 12px 6px 16px; font: 12px/1.45 system-ui, -apple-system, sans-serif;
  color: #1f2328; background: #f6f8fa; border-left: 4px solid #8c959f; white-space: normal;
}
tr[data-libre-cr-tag="annotation"].libre-cr-sev-suggestion > td { background: #ddf4ff; border-left-color: #0969da; }
tr[data-libre-cr-tag="annotation"].libre-cr-sev-warning > td    { background: #fff8c5; border-left-color: #bf8700; }
tr[data-libre-cr-tag="annotation"].libre-cr-sev-critical > td   { background: #ffebe9; border-left-color: #cf222e; }
tr[data-libre-cr-tag="highlight"] .libre-cr-label {
  float: right; margin-left: 12px; padding: 0 7px; border-radius: 10px; white-space: nowrap;
  font: 600 11px/18px system-ui, -apple-system, sans-serif; color: #fff; background: var(--libre-cr-hl);
  opacity: 0.92; user-select: none; pointer-events: none;
}
html.libre-cr-hide-labels .libre-cr-label { display: none; }
tr[data-libre-cr-tag="flash"] > td { animation: libre-cr-flash 1.4s ease-out; }
@keyframes libre-cr-flash { 0%, 40% { background: rgba(9, 105, 218, 0.35); } 100% { background: transparent; } }
`;

/** Install the effect styles into `doc` once. Safe to call repeatedly. */
export function installEffectStyles(doc: Document = document): void {
  const w = doc.defaultView as (Window & { CSSStyleSheet?: typeof CSSStyleSheet }) | null;
  const marked = doc as Document & { __libreCrEffectStyles?: boolean };
  if (marked.__libreCrEffectStyles) return;
  marked.__libreCrEffectStyles = true;
  try {
    if (w?.CSSStyleSheet && "adoptedStyleSheets" in doc) {
      const sheet = new w.CSSStyleSheet();
      sheet.replaceSync(EFFECT_STYLES);
      doc.adoptedStyleSheets = [...doc.adoptedStyleSheets, sheet];
      return;
    }
  } catch {
    // fall through to a <style> element (older engines / jsdom)
  }
  const el = doc.createElement("style");
  el.setAttribute("data-libre-cr", "effects");
  el.textContent = EFFECT_STYLES;
  doc.head.appendChild(el);
}
