// Detect whether the current page is a GitHub PR view.

export interface PRLocation {
  owner: string;
  repo: string;
  number: number;
}

const PR_PATH = /^\/([^/]+)\/([^/]+)\/pull\/(\d+)(?:\/.*)?$/;

export function isPullRequestPage(
  pathname: string = (globalThis as unknown as { location?: { pathname?: string } }).location
    ?.pathname ?? "",
): PRLocation | null {
  const m = pathname.match(PR_PATH);
  if (!m) return null;
  const num = Number(m[3]);
  if (!Number.isFinite(num) || num <= 0) return null;
  return { owner: m[1]!, repo: m[2]!, number: num };
}
