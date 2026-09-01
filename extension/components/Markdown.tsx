// Markdown rendering for model output. marked produces the HTML; a small
// allowlist sanitizer walks it so neither markdown-generated nor raw model
// HTML can smuggle scripts, handlers or non-http(s) URLs into the panel
// (C5 kept dangerouslySetInnerHTML out of snippets; this is the one place
// it is allowed, behind the sanitizer).

import { useMemo } from "react";
import { marked } from "marked";

const ALLOWED_TAGS = new Set([
  "P", "A", "UL", "OL", "LI", "CODE", "PRE", "STRONG", "EM", "B", "I", "DEL",
  "TABLE", "THEAD", "TBODY", "TR", "TH", "TD", "BLOCKQUOTE", "BR", "HR",
  "H1", "H2", "H3", "H4", "H5", "H6", "INPUT",
]);

function sanitize(root: HTMLElement): void {
  for (const el of Array.from(root.querySelectorAll("*"))) {
    if (!ALLOWED_TAGS.has(el.tagName)) {
      // Unwrap unknown tags (keep their text), but drop script/style bodies.
      if (el.tagName === "SCRIPT" || el.tagName === "STYLE" || el.tagName === "IFRAME") {
        el.remove();
      } else {
        el.replaceWith(...Array.from(el.childNodes));
      }
      continue;
    }
    for (const attr of Array.from(el.attributes)) {
      const name = attr.name.toLowerCase();
      if (el.tagName === "A" && name === "href" && /^https?:\/\//i.test(attr.value)) continue;
      if (el.tagName === "TH" || el.tagName === "TD") {
        if (name === "align" || name === "colspan" || name === "rowspan") continue;
      }
      if (el.tagName === "INPUT" && (name === "type" || name === "checked" || name === "disabled")) continue;
      el.removeAttribute(attr.name);
    }
    if (el.tagName === "A") {
      el.setAttribute("target", "_blank");
      el.setAttribute("rel", "noopener noreferrer");
    }
    if (el.tagName === "INPUT") {
      // Task-list checkboxes only, always inert.
      if (el.getAttribute("type") !== "checkbox") el.remove();
      else el.setAttribute("disabled", "");
    }
  }
}

export function renderMarkdown(text: string): string {
  const html = marked.parse(text, { async: false, gfm: true, breaks: true }) as string;
  const doc = new DOMParser().parseFromString(`<div>${html}</div>`, "text/html");
  const container = doc.body.firstElementChild as HTMLElement;
  sanitize(container);
  return container.innerHTML;
}

export function Markdown({ text }: { text: string }) {
  const html = useMemo(() => renderMarkdown(text), [text]);
  // eslint-disable-next-line react/no-danger -- sanitized above
  return <div className="libre-cr-md" dangerouslySetInnerHTML={{ __html: html }} />;
}
