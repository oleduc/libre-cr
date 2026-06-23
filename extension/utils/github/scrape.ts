// Scrape PR metadata from the DOM.
//
// All queries are defensively wrapped so a missing selector becomes a soft
// `null`, never an exception. Selector mismatches surface as a `warnings`
// array on the result for the UI to render.

import type { ScrapedPRData } from "../daemon/client";
import { isPullRequestPage } from "./detect";
import { SELECTORS } from "./selectors";

export interface ScrapeOutput {
  data: ScrapedPRData;
  warnings: string[];
}

function safeQuery<T extends Element>(
  doc: ParentNode,
  selector: string,
  warnings: string[],
  label: string,
): T | null {
  try {
    return doc.querySelector<T>(selector);
  } catch (e) {
    warnings.push(`${label}: ${(e as Error).message}`);
    return null;
  }
}

function safeQueryAll<T extends Element>(
  doc: ParentNode,
  selector: string,
  warnings: string[],
  label: string,
): T[] {
  try {
    return Array.from(doc.querySelectorAll<T>(selector));
  } catch (e) {
    warnings.push(`${label}: ${(e as Error).message}`);
    return [];
  }
}

function textOrNull(el: Element | null): string | null {
  if (!el) return null;
  const t = (el.textContent ?? "").trim();
  return t.length ? t : null;
}

export function scrapePr(doc: ParentNode = globalThis.document): ScrapeOutput {
  const warnings: string[] = [];
  const loc = isPullRequestPage();

  const titleEl = safeQuery<HTMLElement>(doc, SELECTORS.prHeaderTitle, warnings, "title");
  const descEl = safeQuery<HTMLElement>(doc, SELECTORS.prDescription, warnings, "description");
  const authorEl = safeQuery<HTMLElement>(doc, SELECTORS.prAuthor, warnings, "author");
  const baseEl = safeQuery<HTMLElement>(doc, SELECTORS.baseBranch, warnings, "base_branch");
  const headEl = safeQuery<HTMLElement>(doc, SELECTORS.headBranch, warnings, "head_branch");
  const headShaEl = safeQuery<HTMLMetaElement>(doc, SELECTORS.headShaMeta, warnings, "head_sha");
  const head_sha = headShaEl?.getAttribute("content") ?? null;

  const files: string[] = [];
  for (const el of safeQueryAll<HTMLElement>(
    doc,
    SELECTORS.fileRowsInDiff,
    warnings,
    "files_changed",
  )) {
    const path = el.getAttribute("data-tagsearch-path");
    if (path) files.push(path);
  }

  const data: ScrapedPRData = {
    owner: loc?.owner ?? null,
    repo: loc?.repo ?? null,
    number: loc?.number ?? null,
    title: textOrNull(titleEl),
    description: textOrNull(descEl),
    author: textOrNull(authorEl),
    base_branch: textOrNull(baseEl),
    head_branch: textOrNull(headEl),
    head_sha,
    files_changed: files,
  };

  // Soft-warn on a few high-value missing fields.
  if (!data.title) warnings.push("missing title — selectors may need refresh");
  if (loc && data.base_branch === null && data.head_branch === null) {
    warnings.push("missing base/head — selectors may need refresh");
  }

  return { data, warnings };
}

export function prUrl(data: ScrapedPRData): string | null {
  if (!data.owner || !data.repo || !data.number) return null;
  return `https://github.com/${data.owner}/${data.repo}/pull/${data.number}`;
}
