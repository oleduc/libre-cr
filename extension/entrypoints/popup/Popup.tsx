import { Fragment, useEffect, useRef, useState, type ReactNode } from "react";

import { DaemonClient } from "../../utils/daemon/client";
import type { SessionSummary } from "../../utils/daemon/frames";
import { getDaemonAuth } from "../../utils/daemon/storage";

type Status = "loading" | "not_paired" | "connected" | "unreachable";

interface SearchHit {
  session_id: string;
  pr_url: string;
  turn_id: string;
  snippet: string;
  score: number;
}

export function Popup() {
  const [status, setStatus] = useState<Status>("loading");
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [endpoint, setEndpoint] = useState<string | null>(null);
  const [token, setToken] = useState<string | null>(null);
  const [client, setClient] = useState<DaemonClient | null>(null);
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SearchHit[]>([]);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    (async () => {
      const auth = await getDaemonAuth();
      if (!auth) {
        setStatus("not_paired");
        return;
      }
      setEndpoint(auth.endpoint);
      setToken(auth.token);
      const c = new DaemonClient({ endpoint: auth.endpoint, token: auth.token });
      setClient(c);
      try {
        await c.getHealth();
        setStatus("connected");
        try {
          const r = await c.listSessions(50);
          setSessions(r.sessions ?? []);
        } catch {
          // ignore — health still ok
        }
      } catch {
        setStatus("unreachable");
      }
    })();
  }, []);

  useEffect(() => {
    if (!client) return;
    if (debounceRef.current) clearTimeout(debounceRef.current);
    const q = query.trim();
    if (!q) {
      setResults([]);
      return;
    }
    debounceRef.current = setTimeout(() => {
      void client
        .search(q, 20)
        .then((r) => setResults(r.results ?? []))
        .catch(() => setResults([]));
    }, 150);
    return () => {
      if (debounceRef.current) clearTimeout(debounceRef.current);
    };
  }, [query, client]);

  const openOptions = () => {
    try {
      const runtime = (globalThis as unknown as {
        chrome?: { runtime?: { openOptionsPage?: () => void } };
        browser?: { runtime?: { openOptionsPage?: () => void } };
      });
      (runtime.browser?.runtime ?? runtime.chrome?.runtime)?.openOptionsPage?.();
    } catch {
      // ignore
    }
  };

  return (
    <main style={{ fontFamily: "system-ui, sans-serif", padding: 12, minWidth: 280 }}>
      <h1 style={{ fontSize: 14, margin: 0 }}>Libre CR</h1>
      <p style={{ fontSize: 12, color: "#666", marginTop: 6 }}>
        Status: <strong>{statusLabel(status)}</strong>
      </p>
      {status === "connected" ? (
        <input
          type="search"
          value={query}
          placeholder="Search past conversations…"
          onChange={(e) => setQuery(e.target.value)}
          aria-label="Search conversations"
          data-testid="popup-search"
          style={{
            width: "100%",
            padding: "4px 6px",
            fontSize: 12,
            border: "1px solid #d0d7de",
            borderRadius: 4,
            marginBottom: 8,
          }}
        />
      ) : null}
      {query.trim() ? (
        <section data-testid="popup-search-results">
          <h2 style={{ fontSize: 12, margin: "8px 0 4px" }}>Results</h2>
          {results.length === 0 ? (
            <p style={{ fontSize: 12, color: "#666" }}>No matches.</p>
          ) : (
            <ul style={{ listStyle: "none", padding: 0, margin: 0 }}>
              {results.map((r) => (
                <li
                  key={r.turn_id}
                  style={{ fontSize: 12, marginBottom: 6, cursor: "pointer" }}
                  onClick={() => {
                    try {
                      const win = globalThis as unknown as {
                        open?: (u: string, t?: string) => void;
                      };
                      win.open?.(r.pr_url, "_blank");
                    } catch {
                      // ignore
                    }
                  }}
                >
                  <div style={{ color: "#0969da" }}>{r.pr_url}</div>
                  <div style={{ color: "#57606a" }} data-testid="popup-snippet">
                    {renderSnippet(r.snippet)}
                  </div>
                </li>
              ))}
            </ul>
          )}
        </section>
      ) : sessions.length > 0 ? (
        <section>
          <h2 style={{ fontSize: 12, margin: "8px 0 4px" }}>Recent sessions</h2>
          <ul style={{ listStyle: "none", padding: 0, margin: 0 }}>
            {sessions.slice(0, 5).map((s) => (
              <li key={s.session_id} style={{ fontSize: 12, marginBottom: 4 }}>
                <a
                  href={s.pr_url}
                  target="_blank"
                  rel="noreferrer"
                  style={{ color: "#0969da" }}
                >
                  {s.pr_url}
                </a>
              </li>
            ))}
          </ul>
        </section>
      ) : null}
      <div style={{ marginTop: 10, display: "flex", gap: 6 }}>
        {endpoint && token ? (
          <a
            // The config UI reads the bearer token from this query param to
            // authenticate its /v1/config GET+POST (same as `libre-cr config`).
            // Without it the page loads but every call 401s.
            href={`${endpoint}/config-ui?token=${encodeURIComponent(token)}`}
            target="_blank"
            rel="noreferrer"
            style={{ fontSize: 12, color: "#0969da" }}
          >
            Configure daemon
          </a>
        ) : null}
        <button onClick={openOptions} style={{ fontSize: 12 }}>
          {status === "not_paired" ? "Pair extension" : "Options"}
        </button>
      </div>
    </main>
  );
}

// SQLite's `snippet(turns_fts, -1, '[', ']', '…', 12)` wraps matched terms in
// `[` / `]` brackets (see `Store::search_global`). Render the snippet as
// React children — text is escaped by default — and replace the bracketed
// runs with `<mark>` elements for visual highlight. No HTML is ever parsed
// from the daemon-controlled string.
const MARK_REGEX = /\[([^\]]+)\]/g;

export function renderSnippet(snippet: string): ReactNode {
  const matches = Array.from(snippet.matchAll(MARK_REGEX));
  if (matches.length === 0) {
    // No markers — return the raw string so the caller can render it as a
    // plain text node (React escapes it automatically).
    return snippet;
  }
  const nodes: ReactNode[] = [];
  let last = 0;
  matches.forEach((m, i) => {
    const start = m.index ?? 0;
    if (start > last) {
      nodes.push(<Fragment key={`t${i}`}>{snippet.slice(last, start)}</Fragment>);
    }
    nodes.push(<mark key={`m${i}`}>{m[1]}</mark>);
    last = start + m[0].length;
  });
  if (last < snippet.length) {
    nodes.push(<Fragment key={`t-end`}>{snippet.slice(last)}</Fragment>);
  }
  return nodes;
}

function statusLabel(s: Status): string {
  switch (s) {
    case "loading":
      return "checking…";
    case "not_paired":
      return "not paired";
    case "connected":
      return "connected";
    case "unreachable":
      return "unreachable";
  }
}
