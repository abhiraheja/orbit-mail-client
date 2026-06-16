// Typed wrapper around the Tauri IPC bridge. This is the ONLY place the frontend
// talks to Rust. Components import these functions; they never call `invoke`
// directly, and they never contain business logic (spec §3.1).

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

// --- Domain types (mirror the Rust models; display-ready) ------------------

export type LoopKind = "waiting_on" | "owe_reply" | "promised";

export interface Account {
  id: number;
  email: string;
  display_name: string | null;
  provider: string;
  auth_kind: string;
  last_synced: number | null;
  created_at: number;
}

export interface LoopView {
  id: number;
  kind: LoopKind;
  contact_name: string;
  contact_email: string;
  subject: string;
  age: string; // pre-rendered, e.g. "5 days"
  thread_id: number;
  confidence: number;
}

export interface Contact {
  id: number;
  email: string;
  display_name: string | null;
  last_seen: number | null;
}

export interface Message {
  id: number;
  from_email: string;
  to_emails: string[];
  subject: string | null;
  body_text: string | null;
  sent_at: number;
  is_from_me: boolean;
}

export interface ThreadView {
  id: number;
  subject: string | null;
  messages: Message[];
}

export interface BriefingView {
  headline: string;
  total_active: number;
  waiting_on: number;
  owe_reply: number;
  promised: number;
  account_count: number;
  last_synced: string | null; // pre-rendered, e.g. "5 minutes ago"
  top_loops: LoopView[];
}

export type SearchKind = "thread" | "contact";

export interface SearchResult {
  kind: SearchKind;
  title: string;
  subtitle: string;
  thread_id: number | null;
  contact_email: string | null;
}

export interface AddAccountInput {
  email: string;
  display_name: string | null;
  host: string;
  port: number;
  password: string;
}

// --- Commands (request → response) -----------------------------------------

export const ping = (): Promise<number> => invoke("ping");

export const addAccount = (input: AddAccountInput): Promise<Account> =>
  invoke("add_account", { input });

/** Result of email-first provider detection. `method` discriminates the union. */
export type ProviderHint =
  | { label: string; method: "oauth"; provider: string }
  | { label: string; method: "password"; imap_host: string; imap_port: number }
  | { label: string; method: "manual" };

export const detectAccount = (email: string): Promise<ProviderHint> =>
  invoke("detect_account", { email });

/** Run the full OAuth login (opens the browser, captures the redirect). */
export const startOAuthLogin = (provider: string): Promise<Account> =>
  invoke("start_oauth_login", { provider });

export const listAccounts = (): Promise<Account[]> => invoke("list_accounts");

export const removeAccount = (accountId: number): Promise<void> =>
  invoke("remove_account", { accountId });

export const syncAccount = (accountId: number): Promise<void> =>
  invoke("sync_account", { accountId });

export const listLoops = (kind?: LoopKind): Promise<LoopView[]> =>
  invoke("list_loops", { kind: kind ?? null });

export const snoozeLoop = (loopId: number, until: number): Promise<void> =>
  invoke("snooze_loop", { loopId, until });

export const dismissLoop = (loopId: number): Promise<void> =>
  invoke("dismiss_loop", { loopId });

export const getThread = (threadId: number): Promise<ThreadView> =>
  invoke("get_thread", { threadId });

export const listContacts = (): Promise<Contact[]> => invoke("list_contacts");

/** Display-ready daily briefing: counts, last-synced, and the most urgent loops. */
export const getDailyBriefing = (): Promise<BriefingView> =>
  invoke("get_daily_briefing");

/** Keyword search across threads and contacts for the Ctrl+K palette. */
export const search = (query: string): Promise<SearchResult[]> =>
  invoke("search", { query });

export interface AuditEntry {
  id: number;
  timestamp: number;
  provider: string;
  model: string | null;
  purpose: string;
  data_summary: string;
  was_local: boolean;
}

/** Streams ai:token events; resolves with the request_id to correlate them. */
export const draftReply = (threadId: number, instructions: string): Promise<string> =>
  invoke("draft_reply", { threadId, instructions });

/** The "what left my machine" transparency view. */
export const getAiAuditLog = (): Promise<AuditEntry[]> => invoke("get_ai_audit_log");

export interface AiStatus {
  configured: boolean;
  kind: string | null;
  model: string | null;
  local: boolean;
}

export interface SetAiProviderInput {
  kind: string;
  base_url: string | null;
  model: string;
  api_key: string | null;
}

export const getAiStatus = (): Promise<AiStatus> => invoke("get_ai_status");

export const setAiProvider = (input: SetAiProviderInput): Promise<AiStatus> =>
  invoke("set_ai_provider", { input });

export const clearAiProvider = (): Promise<AiStatus> => invoke("clear_ai_provider");

// --- Events (Rust → frontend) ----------------------------------------------

export interface SyncProgress {
  account_id: number;
  done: number;
  total: number;
}
export interface SyncComplete {
  account_id: number;
  new_messages: number;
}
export interface SyncError {
  account_id: number;
  message: string;
}
export interface LoopsUpdated {
  count: number;
}

export const onSyncProgress = (cb: (p: SyncProgress) => void): Promise<UnlistenFn> =>
  listen<SyncProgress>("sync:progress", (e) => cb(e.payload));

export const onSyncComplete = (cb: (p: SyncComplete) => void): Promise<UnlistenFn> =>
  listen<SyncComplete>("sync:complete", (e) => cb(e.payload));

export const onSyncError = (cb: (p: SyncError) => void): Promise<UnlistenFn> =>
  listen<SyncError>("sync:error", (e) => cb(e.payload));

export const onLoopsUpdated = (cb: (p: LoopsUpdated) => void): Promise<UnlistenFn> =>
  listen<LoopsUpdated>("loops:updated", (e) => cb(e.payload));

export interface AiToken {
  request_id: string;
  token: string;
}
export interface AiDone {
  request_id: string;
}

export const onAiToken = (cb: (p: AiToken) => void): Promise<UnlistenFn> =>
  listen<AiToken>("ai:token", (e) => cb(e.payload));

export const onAiDone = (cb: (p: AiDone) => void): Promise<UnlistenFn> =>
  listen<AiDone>("ai:done", (e) => cb(e.payload));
