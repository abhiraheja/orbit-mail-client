// Typed wrapper around the Tauri IPC bridge. This is the ONLY place the frontend
// talks to Rust. Components import these functions; they never call `invoke`
// directly, and they never contain business logic (spec §3.1).

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

// --- Commands (request → response) -----------------------------------------

/** Returns the live DB schema version. Phase 0 bridge smoke-test. */
export function ping(): Promise<number> {
  return invoke<number>("ping");
}

/** Asks Rust to emit a one-shot `loops:updated` event. Phase 0 event test. */
export function emitTestEvent(): Promise<void> {
  return invoke<void>("emit_test_event");
}

// --- Events (Rust → frontend) ----------------------------------------------

export interface SyncProgress {
  account_id: number;
  done: number;
  total: number;
}

export interface LoopsUpdated {
  count: number;
}

export function onSyncProgress(
  cb: (p: SyncProgress) => void,
): Promise<UnlistenFn> {
  return listen<SyncProgress>("sync:progress", (e) => cb(e.payload));
}

export function onLoopsUpdated(
  cb: (p: LoopsUpdated) => void,
): Promise<UnlistenFn> {
  return listen<LoopsUpdated>("loops:updated", (e) => cb(e.payload));
}
