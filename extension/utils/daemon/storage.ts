// Typed wrapper around `browser.storage.local` for the four keys defined in
// `05-browser-extension.md` § State In `browser.storage.local`.
//
// Falls back to an in-memory store when no `browser.storage` is available
// (vitest, options-page preview, etc.).

export interface PanelGeometry {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface StorageShape {
  "daemon.endpoint"?: string;
  "daemon.token"?: string;
  "daemon.extension_origin"?: string;
  "ui.theme_override"?: "system" | "dark" | "light";
  "ui.panel_position"?: Record<string, PanelGeometry>;
  "ui.last_daemon_error"?: { at: number; message: string };
  "ui.last_daemon_ok_at"?: number;
  /** Soft protocol-version mismatch between daemon and extension. */
  "ui.protocol_mismatch"?: { at: number; daemon: number; extension: number };
  /** Keyed by `session_id`, value = head_sha the banner was dismissed for. */
  "ui.diff_change_dismissed"?: Record<string, string>;
  /** Onboarding flags. */
  "onboarding.first_pair_seen"?: boolean;
  /** Per-session presentation mute, keyed by session_id. */
  "session.presentations_muted"?: Record<string, boolean>;
}

type Key = keyof StorageShape;

interface StorageLocalLike {
  get(keys: string[]): Promise<Record<string, unknown>>;
  set(items: Record<string, unknown>): Promise<void>;
  remove(keys: string[]): Promise<void>;
}

function getBackend(): StorageLocalLike | null {
  const b = (globalThis as unknown as { browser?: { storage?: { local?: StorageLocalLike } } })
    .browser?.storage?.local;
  if (b) return b;
  const c = (globalThis as unknown as { chrome?: { storage?: { local?: StorageLocalLike } } })
    .chrome?.storage?.local;
  if (c) return c;
  return null;
}

const memoryStore: Record<string, unknown> = {};

export async function getKey<K extends Key>(key: K): Promise<StorageShape[K] | undefined> {
  const backend = getBackend();
  if (!backend) {
    return memoryStore[key] as StorageShape[K] | undefined;
  }
  try {
    const obj = await backend.get([key]);
    return obj[key] as StorageShape[K] | undefined;
  } catch {
    return undefined;
  }
}

export async function setKey<K extends Key>(key: K, value: StorageShape[K]): Promise<void> {
  const backend = getBackend();
  if (!backend) {
    memoryStore[key] = value;
    return;
  }
  try {
    await backend.set({ [key]: value });
  } catch {
    // ignore
  }
}

export async function removeKey(key: Key): Promise<void> {
  const backend = getBackend();
  if (!backend) {
    delete memoryStore[key];
    return;
  }
  try {
    await backend.remove([key]);
  } catch {
    // ignore
  }
}

export async function getDaemonAuth(): Promise<
  { endpoint: string; token: string; extensionOrigin?: string } | null
> {
  const [endpoint, token, extOrigin] = await Promise.all([
    getKey("daemon.endpoint"),
    getKey("daemon.token"),
    getKey("daemon.extension_origin"),
  ]);
  if (!endpoint || !token) return null;
  return { endpoint, token, extensionOrigin: extOrigin };
}

export async function setDaemonAuth(value: {
  endpoint: string;
  token: string;
  extensionOrigin: string;
}): Promise<void> {
  await Promise.all([
    setKey("daemon.endpoint", value.endpoint),
    setKey("daemon.token", value.token),
    setKey("daemon.extension_origin", value.extensionOrigin),
  ]);
}

export async function clearDaemonAuth(): Promise<void> {
  await Promise.all([
    removeKey("daemon.endpoint"),
    removeKey("daemon.token"),
    removeKey("daemon.extension_origin"),
  ]);
}

export async function getPanelPosition(prUrl: string): Promise<PanelGeometry | undefined> {
  const map = (await getKey("ui.panel_position")) ?? {};
  return map[prUrl];
}

export async function setPanelPosition(prUrl: string, g: PanelGeometry): Promise<void> {
  const map = (await getKey("ui.panel_position")) ?? {};
  map[prUrl] = g;
  await setKey("ui.panel_position", map);
}

/** Internal — exposed for tests. */
export function __resetMemoryStore(): void {
  for (const k of Object.keys(memoryStore)) delete memoryStore[k];
}
