// Pairing helpers.

import { DaemonClient } from "./client";
import { setDaemonAuth } from "./storage";

export interface PairInputs {
  endpoint: string;
  code: string;
  extensionOrigin: string;
}

export interface PairResult {
  endpoint: string;
  token: string;
  extensionOrigin: string;
}

/**
 * POST to `/v1/pair` and persist the resulting `{endpoint, token, extension_origin}`.
 */
export async function pairWithDaemon(inputs: PairInputs): Promise<PairResult> {
  // Auth is unset until pairing succeeds.
  const client = new DaemonClient({ endpoint: inputs.endpoint, token: "" });
  const resp = await client.pair(inputs.code, inputs.extensionOrigin);
  const result: PairResult = {
    endpoint: inputs.endpoint,
    token: resp.token,
    extensionOrigin: resp.extension_origin || inputs.extensionOrigin,
  };
  await setDaemonAuth(result);
  return result;
}

/** The extension's own origin (e.g. `chrome-extension://abcd...`). */
export function getExtensionOrigin(): string {
  const loc = (globalThis as unknown as { location?: { origin?: string } }).location;
  if (loc?.origin && loc.origin.startsWith("chrome-extension://")) return loc.origin;
  if (loc?.origin && loc.origin.startsWith("moz-extension://")) return loc.origin;
  return loc?.origin ?? "";
}
