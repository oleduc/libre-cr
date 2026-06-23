// Soft protocol-version check against the daemon's `GET /v1/health`.
//
// `libre-cr-common::PROTOCOL_VERSION` is the source of truth; the daemon
// reports it as `protocol_version` in the health body. A mismatch never
// blocks anything (minor versions are wire-compatible by spec) — it just
// logs a console warning and records `ui.protocol_mismatch` so the Options
// diagnostics section can render it. A missing field (older daemon) is
// treated as compatible.

import { PROTOCOL_VERSION } from "./frames";
import { removeKey, setKey } from "./storage";

export async function recordProtocolCheck(health: {
  protocol_version?: number;
}): Promise<boolean> {
  const theirs = health.protocol_version;
  if (typeof theirs !== "number" || theirs === PROTOCOL_VERSION) {
    // Compatible (or pre-versioning daemon) — clear any stale warning.
    await removeKey("ui.protocol_mismatch");
    return true;
  }
  console.warn(
    `[libre-cr] daemon speaks protocol v${theirs} but this extension speaks v${PROTOCOL_VERSION} — ` +
      "update the daemon or the extension if you see odd behavior.",
  );
  await setKey("ui.protocol_mismatch", {
    at: Date.now(),
    daemon: theirs,
    extension: PROTOCOL_VERSION,
  });
  return false;
}
