import { useEffect, useRef, useState } from "react";

import { getExtensionOrigin, pairWithDaemon } from "../../utils/daemon/pairing";
import {
  clearDaemonAuth,
  getDaemonAuth,
  getKey,
  setKey,
} from "../../utils/daemon/storage";

type Theme = "system" | "dark" | "light";

/**
 * Parse a pairing deep-link of the form
 *   `#pair?endpoint=<url>&code=<code>[&auto=1]`
 * Spec § First-Run Pairing (path B): the daemon's config UI generates a link
 * that opens this options page with pre-filled credentials; if `auto=1` the
 * pairing flow runs without user interaction. Manual paste (path A) remains
 * the fallback for offline / privacy-sensitive users.
 */
export function parsePairDeepLink(
  hash: string,
): { endpoint?: string; code?: string; auto: boolean } | null {
  const trimmed = hash.replace(/^#/, "");
  if (!trimmed.startsWith("pair")) return null;
  const qIdx = trimmed.indexOf("?");
  const qs = qIdx === -1 ? "" : trimmed.slice(qIdx + 1);
  const params = new URLSearchParams(qs);
  return {
    endpoint: params.get("endpoint") ?? undefined,
    code: params.get("code") ?? undefined,
    auto: params.get("auto") === "1",
  };
}

export function Options() {
  const [endpoint, setEndpoint] = useState("http://127.0.0.1:8765");
  const [code, setCode] = useState("");
  const [paired, setPaired] = useState<{ endpoint: string } | null>(null);
  const [theme, setTheme] = useState<Theme>("system");
  const [status, setStatus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [lastErr, setLastErr] = useState<{ at: number; message: string } | null>(null);
  const [lastOk, setLastOk] = useState<number | null>(null);
  const [protoMismatch, setProtoMismatch] = useState<{
    at: number;
    daemon: number;
    extension: number;
  } | null>(null);
  const autoPairFiredRef = useRef(false);

  const doPair = async (ep: string, c: string) => {
    setStatus(null);
    setError(null);
    try {
      const res = await pairWithDaemon({
        endpoint: ep,
        code: c,
        extensionOrigin: getExtensionOrigin(),
      });
      setPaired({ endpoint: res.endpoint });
      setStatus("Paired successfully.");
    } catch (e) {
      setError((e as Error).message);
    }
  };

  useEffect(() => {
    (async () => {
      const a = await getDaemonAuth();
      if (a) setPaired({ endpoint: a.endpoint });
      const t = (await getKey("ui.theme_override")) ?? "system";
      setTheme(t);
      setLastErr((await getKey("ui.last_daemon_error")) ?? null);
      setLastOk((await getKey("ui.last_daemon_ok_at")) ?? null);
      setProtoMismatch((await getKey("ui.protocol_mismatch")) ?? null);

      // Deep-link pre-fill / auto-pair.
      const loc = (globalThis as unknown as { location?: { hash?: string } }).location;
      const parsed = parsePairDeepLink(loc?.hash ?? "");
      if (parsed) {
        if (parsed.endpoint) setEndpoint(parsed.endpoint);
        if (parsed.code) setCode(parsed.code);
        if (parsed.auto && parsed.endpoint && parsed.code && !a && !autoPairFiredRef.current) {
          autoPairFiredRef.current = true;
          await doPair(parsed.endpoint, parsed.code);
        }
      }
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const onPair = async () => {
    await doPair(endpoint, code);
  };

  const onUnpair = async () => {
    await clearDaemonAuth();
    setPaired(null);
    setStatus("Unpaired.");
  };

  const onThemeChange = async (t: Theme) => {
    setTheme(t);
    await setKey("ui.theme_override", t);
  };

  return (
    <main
      style={{ fontFamily: "system-ui, sans-serif", padding: 16, maxWidth: 640 }}
    >
      <h1 style={{ fontSize: 16, marginTop: 0 }}>Libre CR — Options</h1>

      <section>
        <h2 style={{ fontSize: 14 }}>Pairing</h2>
        {paired ? (
          <p style={{ fontSize: 13 }}>
            Paired with <code>{paired.endpoint}</code>.{" "}
            <button onClick={onUnpair}>Unpair</button>
          </p>
        ) : (
          <>
            <p style={{ fontSize: 12, color: "#555" }}>
              Run <code>libre-cr pair</code> in a terminal. Paste the endpoint and
              one-time code below.
            </p>
            <label style={{ display: "block", fontSize: 12 }}>
              Endpoint
              <input
                type="text"
                value={endpoint}
                onChange={(e) => setEndpoint(e.target.value)}
                style={{ width: "100%", padding: 4, marginTop: 2 }}
              />
            </label>
            <label style={{ display: "block", fontSize: 12, marginTop: 6 }}>
              Pairing code
              <input
                type="text"
                value={code}
                onChange={(e) => setCode(e.target.value)}
                style={{ width: "100%", padding: 4, marginTop: 2 }}
              />
            </label>
            <button onClick={onPair} style={{ marginTop: 8 }} disabled={!code}>
              Pair
            </button>
          </>
        )}
        {status ? <p style={{ color: "#1f883d", fontSize: 12 }}>{status}</p> : null}
        {error ? <p style={{ color: "#82071e", fontSize: 12 }}>{error}</p> : null}
      </section>

      <section style={{ marginTop: 16 }}>
        <h2 style={{ fontSize: 14 }}>Theme override</h2>
        {(["system", "dark", "light"] as Theme[]).map((t) => (
          <label key={t} style={{ marginRight: 8, fontSize: 13 }}>
            <input
              type="radio"
              name="theme"
              value={t}
              checked={theme === t}
              onChange={() => onThemeChange(t)}
            />
            {" "}
            {t}
          </label>
        ))}
      </section>

      <section style={{ marginTop: 16 }}>
        <h2 style={{ fontSize: 14 }}>Diagnostics</h2>
        <p style={{ fontSize: 12 }}>
          Last successful daemon call:{" "}
          {lastOk ? new Date(lastOk).toLocaleString() : "never"}
        </p>
        <p style={{ fontSize: 12 }}>
          Last daemon error:{" "}
          {lastErr
            ? `${new Date(lastErr.at).toLocaleString()} — ${lastErr.message}`
            : "none"}
        </p>
        {protoMismatch ? (
          <p style={{ fontSize: 12, color: "#9a6700" }} data-testid="protocol-mismatch">
            Protocol mismatch: daemon speaks v{protoMismatch.daemon}, extension speaks v
            {protoMismatch.extension} (seen{" "}
            {new Date(protoMismatch.at).toLocaleString()}). Update the daemon or the
            extension.
          </p>
        ) : null}
      </section>
    </main>
  );
}
