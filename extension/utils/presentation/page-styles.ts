// Page-level CSS for presentation effects. These classes land on GitHub's own
// diff rows (outside our shadow root), so the panel stylesheet can't reach
// them. Installed as a constructed stylesheet: CSSOM insertion is not subject
// to the page's `style-src` CSP, unlike an inline <style>.

export const EFFECT_STYLES = `
tr.libre-cr-effect > td { box-shadow: inset 4px 0 0 var(--libre-cr-hl, #0969da); }
tr.libre-cr-hl-blue   { --libre-cr-hl: #0969da; background: rgba(9, 105, 218, 0.14) !important; }
tr.libre-cr-hl-red    { --libre-cr-hl: #cf222e; background: rgba(207, 34, 46, 0.16) !important; }
tr.libre-cr-hl-yellow { --libre-cr-hl: #bf8700; background: rgba(191, 135, 0, 0.18) !important; }
tr.libre-cr-hl-green  { --libre-cr-hl: #1a7f37; background: rgba(26, 127, 55, 0.16) !important; }
tr.libre-cr-hl-purple { --libre-cr-hl: #8250df; background: rgba(130, 80, 223, 0.14) !important; }
tr.libre-cr-effect.libre-cr-hl-blue > td, tr.libre-cr-effect.libre-cr-hl-red > td,
tr.libre-cr-effect.libre-cr-hl-yellow > td, tr.libre-cr-effect.libre-cr-hl-green > td,
tr.libre-cr-effect.libre-cr-hl-purple > td { background: inherit !important; }
tr.libre-cr-annotation > td.libre-cr-annotation-cell {
  padding: 6px 12px 6px 16px; font: 12px/1.45 system-ui, -apple-system, sans-serif;
  color: #1f2328; background: #fff8c5; border-left: 4px solid #bf8700; white-space: normal;
}
tr.libre-cr-annotation.libre-cr-sev-critical > td, tr.libre-cr-annotation.libre-cr-sev-warning > td { background: #ffebe9; border-left-color: #cf222e; }
tr.libre-cr-annotation.libre-cr-sev-suggestion > td { background: #ddf4ff; border-left-color: #0969da; }
tr.libre-cr-annotation.libre-cr-sev-info > td { background: #f6f8fa; border-left-color: #8c959f; }
tr.libre-cr-flash > td { animation: libre-cr-flash 1.4s ease-out; }
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
