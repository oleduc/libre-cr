// Markdown rendering for model output. marked produces the HTML; a small
// allowlist sanitizer walks it so neither markdown-generated nor raw model
// HTML can smuggle scripts, handlers or non-http(s) URLs into the panel
// (C5 kept dangerouslySetInnerHTML out of snippets; this is the one place
// it is allowed, behind the sanitizer).
//
// Copying from the rendered view puts *markdown* back on the clipboard: the
// rendered tag set is exactly the sanitizer allowlist, so a small DOM→md
// serializer (fragmentToMarkdown) round-trips it without a library.

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
      // Fence language survives so copy-as-markdown can restore ```lang.
      if (el.tagName === "CODE" && name === "class" && /^language-[\w-]+$/.test(attr.value)) continue;
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

// --- rendered DOM → markdown (for copy) ------------------------------------

function escapeCell(s: string): string {
  return s.replace(/\|/g, "\\|").replace(/\n+/g, " ").trim();
}

function childrenMd(node: Node, indent: string): string {
  return Array.from(node.childNodes)
    .map((c) => nodeMd(c, indent))
    .join("");
}

function listMd(el: Element, ordered: boolean, indent: string): string {
  let i = 1;
  const items = Array.from(el.children)
    .filter((c) => c.tagName === "LI")
    .map((li) => {
      const marker = ordered ? `${i++}. ` : "- ";
      const body = childrenMd(li, indent + "  ").trim();
      return `${indent}${marker}${body}`;
    });
  return `\n${items.join("\n")}\n`;
}

function tableMd(el: Element, indent: string): string {
  const rows = Array.from(el.querySelectorAll("tr"));
  if (rows.length === 0) return "";
  const cells = (tr: Element) =>
    Array.from(tr.children).map((c) => escapeCell(childrenMd(c, indent)));
  const header = cells(rows[0]!);
  const out = [
    `| ${header.join(" | ")} |`,
    `| ${header.map(() => "---").join(" | ")} |`,
  ];
  for (const tr of rows.slice(1)) out.push(`| ${cells(tr).join(" | ")} |`);
  return `\n${out.join("\n")}\n`;
}

function nodeMd(node: Node, indent: string): string {
  if (node.nodeType === Node.TEXT_NODE) return node.textContent ?? "";
  if (!(node instanceof Element)) return childrenMd(node, indent);
  const el = node;
  const kids = () => childrenMd(el, indent);
  switch (el.tagName) {
    case "P":
      return `\n${kids().trim()}\n`;
    case "STRONG":
    case "B":
      return `**${kids()}**`;
    case "EM":
    case "I":
      return `*${kids()}*`;
    case "DEL":
      return `~~${kids()}~~`;
    case "CODE": {
      if (el.parentElement?.tagName === "PRE") return el.textContent ?? "";
      return "`" + (el.textContent ?? "") + "`";
    }
    case "PRE": {
      const lang =
        el.querySelector("code")?.className.match(/language-([\w-]+)/)?.[1] ?? "";
      const body = (el.textContent ?? "").replace(/\n$/, "");
      return `\n\`\`\`${lang}\n${body}\n\`\`\`\n`;
    }
    case "A": {
      const href = el.getAttribute("href");
      const t = kids();
      return href ? `[${t}](${href})` : t;
    }
    case "UL":
      return listMd(el, false, indent);
    case "OL":
      return listMd(el, true, indent);
    case "BLOCKQUOTE":
      return `\n${kids()
        .trim()
        .split("\n")
        .map((l) => `> ${l}`)
        .join("\n")}\n`;
    case "BR":
      return "\n";
    case "HR":
      return "\n---\n";
    case "H1":
    case "H2":
    case "H3":
    case "H4":
    case "H5":
    case "H6":
      return `\n${"#".repeat(Number(el.tagName[1]))} ${kids().trim()}\n`;
    case "TABLE":
      return tableMd(el, indent);
    case "INPUT":
      return (el as HTMLInputElement).checked || el.hasAttribute("checked")
        ? "[x]"
        : "[ ]";
    default:
      // LI / THEAD / TR / TD reached directly only when a selection starts
      // mid-structure; their children still serialize.
      return kids();
  }
}

/** Serialize a fragment of the rendered answer back to markdown. */
export function fragmentToMarkdown(node: Node): string {
  return childrenMd(node, "").replace(/\n{3,}/g, "\n\n").trim();
}

export function Markdown({ text }: { text: string }) {
  const html = useMemo(() => renderMarkdown(text), [text]);
  const onCopy = (e: React.ClipboardEvent<HTMLDivElement>) => {
    // The panel lives in a shadow root; Chrome exposes its selection on the
    // root, not on window.
    const root = e.currentTarget.getRootNode();
    const sel =
      typeof ShadowRoot !== "undefined" &&
      root instanceof ShadowRoot &&
      "getSelection" in root
        ? (root as unknown as { getSelection(): Selection | null }).getSelection()
        : window.getSelection();
    if (!sel || sel.isCollapsed || sel.rangeCount === 0) return;
    const fragment = sel.getRangeAt(0).cloneContents();
    const md = fragmentToMarkdown(fragment);
    if (!md) return;
    const holder = document.createElement("div");
    holder.appendChild(fragment);
    e.clipboardData.setData("text/plain", md);
    e.clipboardData.setData("text/html", holder.innerHTML);
    e.preventDefault();
  };
  // eslint-disable-next-line react/no-danger -- sanitized above
  return <div className="libre-cr-md" onCopy={onCopy} dangerouslySetInnerHTML={{ __html: html }} />;
}
