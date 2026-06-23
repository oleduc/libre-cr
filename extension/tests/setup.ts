// Minimal vitest setup — wires the in-memory storage backend.

import { __resetMemoryStore } from "../utils/daemon/storage";
import { beforeEach } from "vitest";

beforeEach(() => {
  __resetMemoryStore();
});
