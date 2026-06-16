# Orbit — Build Status

Phase-wise record of what's built and what's pending. The product spec is
[`PRODUCT-DRAFT.md`](./PRODUCT-DRAFT.md); the build order is the approved plan.
This file is the running ledger — update it as phases land.

**Last updated:** 2026-06-16
**Toolchain:** Rust 1.96.0 (MSVC) · Tauri 2.10.x · React 19 + TS (strict) + Vite 7 · SQLite (rusqlite bundled, FTS5)
**Tests:** 31 Rust unit/integration tests passing · `tsc --noEmit` clean

---

## Architectural laws (must hold throughout — spec §3)

- ✅ **Web renders, never thinks.** No IMAP/SMTP, DB, AI calls, or loop rules in TypeScript. The only bridge is [`src/lib/ipc.ts`](../src/lib/ipc.ts).
- ✅ **All SQL lives in `db/queries.rs`.** No module outside `db/` writes SQL.
- ✅ **Commands stay thin.** Handlers parse input, call a domain module, return.
- ✅ **Credentials never in SQLite.** Passwords go to the OS keychain (`keyring`); the DB stores only a `cred_ref`.
- ✅ **Local-first.** All data in local SQLite; the app works offline except live sync / cloud AI.
- ✅ **AI is optional + audited.** App runs fully with no provider configured; every outbound AI call writes `ai_audit_log` *before* sending.

---

## Phase 0 — Scaffold ✅ COMPLETE  (commit `04e4753`)

| Item | Status |
| --- | --- |
| Tauri 2 + React/TS + Vite scaffold | ✅ |
| Rust module layout per spec §6 (`models/ db/ sync/ graph/ loops/ ai/ commands/` + `state.rs error.rs events.rs`) | ✅ |
| `AppError` enum (thiserror) serializing across IPC | ✅ |
| `db/migrations.rs` — versioned runner (PRAGMA `user_version`); migration 1 = full v1 schema | ✅ |
| `state.rs` — `AppState { db, config, ai, db_path }`; WAL + foreign_keys on | ✅ |
| `events.rs` — typed event names + emit helpers (spec §11) | ✅ |
| Bridge proof: `ping` command + test event + typed `ipc.ts` | ✅ |

**Exit criteria met:** app launches, migrations create the DB, command + event round-trip.

---

## Phase 1 — Open-Loops Engine (the v1 wedge, NO AI) ✅ COMPLETE  (commit `805108b`)

| Item | Status | Where |
| --- | --- | --- |
| Domain models (display-ready `*View` types) | ✅ | [`models/mod.rs`](../src-tauri/src/models/mod.rs) |
| Typed queries; dedup messages on `(account_id, message_id)`; upsert contacts on normalized email | ✅ | [`db/queries.rs`](../src-tauri/src/db/queries.rs) |
| `MailSource` trait (testable without a live server) | ✅ | [`sync/mod.rs`](../src-tauri/src/sync/mod.rs) |
| Ingest: normalize, thread resolution (in-reply-to → references → subject), is_from_me / is_automated | ✅ | [`sync/ingest.rs`](../src-tauri/src/sync/ingest.rs) |
| Sync runner: background, batched, `sync:progress/complete/error` events; never locks DB across a network await | ✅ | [`sync/runner.rs`](../src-tauri/src/sync/runner.rs) |
| Incremental sync state (UIDVALIDITY + last UID per folder) | ✅ | `sync_state` table + runner cursor |
| **Loop detection** — `waiting_on`, `owe_reply`, `promised` heuristics; lifecycle (auto-resolve / snooze / dismiss) | ✅ | [`loops/rules.rs`](../src-tauri/src/loops/rules.rs) |
| Thin commands: add/list/remove account, sync, list/snooze/dismiss loops, get_thread, list_contacts | ✅ | [`commands/mod.rs`](../src-tauri/src/commands/mod.rs) |
| Credentials → OS keychain (cred_ref only in DB) | ✅ | [`secrets.rs`](../src-tauri/src/secrets.rs) |
| Frontend: onboarding form, virtualized loops list, kind filters, sync status | ✅ | [`src/App.tsx`](../src/App.tsx), [`components/`](../src/components/) |

**Verified via mock `MailSource` + fixtures.** Live end-to-end (real IMAP) is pending an account — see Phase 2.

---

## Phase 2 — Reach & Polish 🟡 PARTIAL

| Item | Status | Notes |
| --- | --- | --- |
| **Daily briefing view** | ✅ | commit `c029525`. [`loops/briefing.rs`](../src-tauri/src/loops/briefing.rs) + [`components/Briefing.tsx`](../src/components/Briefing.tsx). Counts by kind, headline, last-synced, top-5 urgent. |
| **Ctrl+K command palette** | ✅ | commit `c029525`. FTS5 + contact search in [`search.rs`](../src-tauri/src/search.rs); [`components/CommandPalette.tsx`](../src/components/CommandPalette.tsx). Keyboard nav, debounced. |
| **Improved "promised" heuristic** | ✅ | Confidence scoring, negation guard ("I won't…"), deadline boost in [`loops/rules.rs`](../src-tauri/src/loops/rules.rs). |
| **Live IMAP fetch** (real `async-imap`) | 🟡 CODE-COMPLETE, UNVERIFIED | [`sync/imap.rs`](../src-tauri/src/sync/imap.rs). Compiles against real APIs; not yet run against a live server (no test account). **Uncommitted.** |
| **Gmail OAuth** | ⛔ NOT STARTED | Needs OAuth client credentials. Isolate in `sync/oauth.rs`. |
| **M365 (Graph) OAuth** | ⛔ NOT STARTED | Needs an Azure app registration. Separate flow. |

### Known TODOs / caveats in the IMAP layer
- **Sent-folder name** is hardcoded `"Sent"`; varies by server ("Sent Items", "[Gmail]/Sent Mail"). Make configurable / discover via `LIST`.
- **UIDVALIDITY change** is persisted but not acted on — on change we should re-pull from UID 1.
- First pull is bounded to the last `INITIAL_WINDOW` (500) UIDs to keep initial sync fast.

---

## Phase 3 — AI Layer (provider-agnostic, audited, OPTIONAL) 🟡 PARTIAL

| Item | Status | Where |
| --- | --- | --- |
| `AiProvider` trait (name / model / is_local / streamed `complete`) | ✅ | [`ai/provider.rs`](../src-tauri/src/ai/provider.rs) |
| **Audit chokepoint** — writes `ai_audit_log` BEFORE contacting provider; no path bypasses it | ✅ | [`ai/audit.rs`](../src-tauri/src/ai/audit.rs) |
| `AiRegistry` — optional provider, app fully works when none configured | ✅ | [`ai/mod.rs`](../src-tauri/src/ai/mod.rs) |
| Commands: `draft_reply` (streams `ai:token`/`ai:done`), `get_ai_audit_log` | ✅ | [`commands/mod.rs`](../src-tauri/src/commands/mod.rs) |
| IPC bindings + event listeners | ✅ | [`src/lib/ipc.ts`](../src/lib/ipc.ts) |
| **Concrete providers** (OpenAI, Claude, Gemini, Ollama, LM Studio, OpenRouter, Azure, OpenAI-compatible) | ⛔ NOT STARTED | Need API keys / a local model. Each implements the trait only. |
| "What left my machine" transparency UI | ⛔ NOT STARTED | Backend (`get_ai_audit_log`) ready; needs a frontend view. |
| Draft replies in the user's voice (learned from sent mail) | ⛔ POST-V1 | |
| Local semantic search (on-device embeddings, e.g. `fastembed`) | ⛔ POST-V1 | |

**Verified with a fake provider:** tokens stream through, audit row written even when the provider errors, local provider logs `was_local=1`, heuristics work with no provider.

---

## Test coverage (31 Rust tests)

| Module | Tests |
| --- | --- |
| `loops/rules.rs` | 10 (each kind, thresholds, automated suppression, auto-resolve, dismissed-not-recreated, negation, deadline confidence, age formatting) |
| `search.rs` | 6 (prefix build, injection-safety, subject/body/contact hits, blank, `%` escaping) |
| `sync/ingest.rs` | 6 (normalize, threading, dedup, contacts-exclude-me, automated) |
| `loops/briefing.rs` | 3 (empty, counts+headline, top-N cap) |
| `ai/audit.rs` | 3 (stream+audit, audit-on-failure, was_local) |
| `sync/runner.rs` | 2 (full slice, dedup) |
| `db/migrations.rs` | 1 (run + idempotent) |

---

## Commands registered (15)

`ping` · `emit_test_event` · `add_account` · `list_accounts` · `remove_account` ·
`sync_account` · `list_loops` · `snooze_loop` · `dismiss_loop` · `get_thread` ·
`list_contacts` · `get_daily_briefing` · `search` · `draft_reply` · `get_ai_audit_log`

---

## Immediate next steps (when unblocked)

1. **Commit** the live IMAP fetch (`sync/imap.rs`, `Cargo.toml`, `Cargo.lock`).
2. **Live verification** — with an IMAP account + app password: add account → watch background sync → confirm an accurate loops list + briefing within ~1 minute. This is the v1 "definition of done" (spec §2) and the part the spec insists must be proven live.
3. **Gmail OAuth**, then **M365 OAuth** (each isolated in its own flow).
4. **At least one concrete AI provider** (a local Ollama/LM Studio model needs no key and keeps `was_local=1`) + the audit transparency UI.

### Blocked on user input
- A **test email account** (IMAP host + app password) to verify the sync slice live.
- **OAuth client credentials** for Gmail / M365.
- **AI provider** choice + key (or a local model endpoint).
