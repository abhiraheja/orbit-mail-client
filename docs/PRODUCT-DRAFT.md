# CLAUDE.md — Communication OS

> This file is the source of truth for Claude Code working on this project.
> Read it fully before writing any code. When in doubt, follow the rules here over
> your own defaults. If something here is ambiguous, ask before guessing.

---

## 1. What we are building (and what we are NOT)

We are building a **local-first Communication Operating System** for professionals —
a desktop application that connects a person's email, contacts, and (later) calendar,
tasks, notes, and files into a single **knowledge graph**, and uses AI to make that
graph useful.

The long-term product replaces Outlook + Thunderbird + a lightweight CRM + a task
manager, inside one elegant app. **But we do not build that all at once.**

### The moat is the graph, not the modules

The eight modules (email, calendar, contacts, tasks, notes, files…) are *data
acquisition*. They exist so the graph has something to connect. The value — the thing
no competitor has — is the layer that connects them and answers questions like
*"show me everything about the SSO project."* Keep this in mind: we are not building a
prettier email client. A prettier email client already exists (Spark, Superhuman). Our
advantage only appears at the intersection of data.

### Scope discipline (read this twice)

The single biggest risk to this project is **scope**, not difficulty. The instinct to
add "just the calendar, then notes, then a CRM" is what turns this into a half-built
everything-app that does nothing well.

**The rule: build the v1 wedge until it is genuinely great before adding any second
module.** If a task does not serve the v1 wedge defined in Section 2, do not build it,
even if it is easy and tempting. Flag it for the backlog instead.

---

## 2. The v1 wedge: the Open-Loops Engine

V1 is **not** all eight modules. V1 is one narrow, genuinely differentiated feature
that exercises the whole stack: the **Open-Loops Engine**.

An "open loop" is an unresolved thread of responsibility. The app detects three kinds:

1. **Waiting on** — I sent the last message in a thread and nobody has replied. ("You're
   waiting on a reply from John, 5 days.")
2. **Owe a reply** — Someone sent me a message that expects a response and I haven't
   replied. ("You owe Sarah a reply on the pricing thread.")
3. **Promised** — My own sent mail contains a commitment. ("You promised the SSO doc by
   today.") *Heuristic-detectable in part; AI improves it later.*

The v1 UI is essentially **one screen**: a clean, fast, keyboard-navigable list of open
loops, grouped by type, each with the contact, the thread, the age, and quick actions
(open, snooze, dismiss, draft reply).

### Why this is the right v1

- It needs only email sync — no calendar, CRM, or notes editor.
- The first version needs **no AI at all**: kinds 1 and 2 are pure SQL over synced mail.
  AI is layered on top later, where heuristics fall short (kind 3, intent detection).
- It is genuinely differentiated — everyone feels the pain of dropped balls, and no inbox
  does this well.
- It exercises every part of the architecture (sync, storage, graph, IPC, eventually AI),
  so getting it working proves the foundation.

**Definition of done for v1:** a user adds one email account, the app syncs their mail in
the background with visible progress, and within a minute they see an accurate list of
what they're waiting on and what they owe — fast, and without anything having left their
machine.

---

## 3. Non-negotiable principles

These are architectural laws, not preferences. Do not violate them without explicit
human approval.

### 3.1 The web layer renders; it never thinks

All real logic lives in **Rust**: email sync, the database, all queries, search,
entity resolution, loop detection, and **every call to an AI provider**. The web
frontend only receives already-computed data and displays it, and sends user intentions
back down.

The frontend must **never**:
- talk to an email server (IMAP/SMTP) directly,
- hold or query the database directly,
- call an AI provider (OpenAI, Gemini, Claude, etc.) directly,
- contain business rules about what a "loop" is.

If you find yourself putting logic in TypeScript, stop — it belongs in Rust.

### 3.2 Local-first

All user data lives on the user's machine in SQLite. No forced cloud, no required
backend, no account system to use the core app. The app must be fully functional
offline (except, obviously, syncing new mail and cloud-AI calls).

### 3.3 The AI chokepoint = the privacy promise

Because **all** AI calls route through one Rust module, we have exactly one place that
knows what data leaves the machine. Every outbound AI request must be written to an
**audit log** (see schema) recording: timestamp, provider, model, a description of what
was sent, and why. This audit log powers a future "what left my machine" transparency
view and is the thing that makes "privacy-respecting" true instead of marketing.

There must also be a **fully-local mode**: when the user selects a local provider
(Ollama / LM Studio), no email content ever goes to a third party. Heuristic loop
detection (kinds 1 and 2) must work with **no AI provider configured at all.**

### 3.4 Single, self-contained installer

The shipped artifact is one file per OS (`.exe` / `.dmg` / `.AppImage`). Rust is
compiled *into* the app; there is no separate runtime for the user to install. Never
introduce a dependency that requires the end user to install something separately
(no "install Node", no bundled server process the user manages).

---

## 4. Technology stack

| Layer            | Choice                                   | Notes |
|------------------|------------------------------------------|-------|
| Shell            | **Tauri 2.x**                            | Verify the exact current 2.x version against live docs at setup; tooling moves. |
| Backend / core   | **Rust**                                 | All logic. |
| Frontend         | **React + TypeScript**, built with Vite  | Rendering only. |
| Database         | **SQLite** with **FTS5**                 | Via `rusqlite` (bundled feature) or `sqlx`. Pick `rusqlite` for v1 simplicity. |
| Search (keyword) | **FTS5**                                 | Built into SQLite. |
| Search (semantic)| Local embeddings stored in SQLite        | Defer to post-v1. Use a Rust-native embedder (e.g. `fastembed`) so it stays local. |
| Email            | IMAP/SMTP + OAuth for Gmail/M365         | See Section 8. |
| AI               | Provider-agnostic HTTP client in Rust    | See Section 9. |

**Platform order:** Windows first (ship and polish fully), then macOS, then Linux. The
core is write-once; only platform glue (notifications, tray, paths, signing) differs.

**Frontend libraries you'll likely want (keep them rendering-only):**
- a command-palette component for the Ctrl+K bar,
- a **virtualized list** library (non-negotiable for rendering large inboxes/loop lists
  without dying),
- a rich-text editor for compose (post-v1).

---

## 5. Architecture & the IPC bridge

```
┌─────────────────────────────────────────────┐
│  Frontend (React + TS)  — RENDER ONLY         │
│  • screens, components, state for display     │
│  • invokes commands, listens for events       │
└───────────────┬───────────────────────────────┘
                │  Tauri IPC (commands + events)
┌───────────────┴───────────────────────────────┐
│  Rust core — ALL LOGIC                         │
│  commands/ → db/ sync/ graph/ loops/ ai/       │
│  SQLite (local file)                           │
└───────────────┬───────────────────────────────┘
                │ network (only from Rust)
        IMAP/SMTP servers   •   AI providers
```

### Two IPC patterns — use both

**Commands** = request → response. Frontend asks, waits for a result. Most interactions.
Example: `list_loops()` returns the current loops.

**Events** = Rust pushes to the frontend unprompted. Required for anything long-running
or streaming. **Design for streaming from day one — retrofitting it is painful.** Use
events for:
- email sync progress (`sync:progress` — "1,200 of 40,000"), so the UI never freezes
  behind one giant command,
- loop list updates after a sync (`loops:updated`),
- AI responses streamed token-by-token (`ai:token`), so drafting feels alive.

Rule of thumb: if it can take more than ~200ms or produces incremental output, it's a
background job that emits **events**, not a blocking command.

---

## 6. Rust module layout

```
src-tauri/src/
├── main.rs            # Tauri builder, registers commands, sets up state
├── lib.rs             # wiring
├── state.rs           # shared app state (db pool, config) behind Tauri State
├── error.rs           # one app-wide Error type; commands return Result<T, AppError>
├── models/            # domain types (Message, Contact, Thread, Loop, Account…)
│   └── mod.rs
├── db/                # the ONLY place that touches SQLite
│   ├── mod.rs
│   ├── migrations.rs  # schema versioning; run on startup
│   └── queries.rs     # typed query functions
├── sync/              # email sync engine (IMAP/OAuth), emits sync:* events
│   ├── mod.rs
│   ├── imap.rs
│   └── oauth.rs
├── graph/             # entity resolution: link messages ↔ contacts ↔ threads
│   └── mod.rs
├── loops/             # open-loop detection. HEURISTICS FIRST, AI later.
│   ├── mod.rs
│   └── rules.rs
├── ai/                # provider-agnostic AI + the audit-log chokepoint
│   ├── mod.rs
│   ├── provider.rs    # trait + implementations (openai, gemini, claude, ollama…)
│   └── audit.rs       # writes every outbound call to ai_audit_log
├── commands/          # THIN Tauri command handlers — no logic, just call modules
│   └── mod.rs
└── events.rs          # typed event names + emit helpers
```

**Key discipline:** `commands/` handlers are thin. They parse input, call a function in
`sync/`, `loops/`, `ai/`, etc., and return the result. Business logic never lives in a
command handler.

---

## 7. Database schema (v1)

SQLite. Use integer primary keys, store timestamps as UTC Unix seconds (INTEGER), and
add a `schema_version` mechanism in `db/migrations.rs`. This schema is deliberately
focused on the open-loops wedge — do not add task/note/calendar tables in v1.

```sql
-- Connected email accounts
CREATE TABLE accounts (
    id            INTEGER PRIMARY KEY,
    email         TEXT NOT NULL UNIQUE,
    display_name  TEXT,
    provider      TEXT NOT NULL,        -- 'gmail' | 'm365' | 'imap'
    auth_kind     TEXT NOT NULL,        -- 'oauth' | 'password'
    -- credentials are stored in the OS keychain, NOT here; this holds a reference/id
    cred_ref      TEXT,
    last_synced   INTEGER,              -- UTC unix seconds
    created_at    INTEGER NOT NULL
);

-- People. The seed of the future knowledge graph.
CREATE TABLE contacts (
    id            INTEGER PRIMARY KEY,
    email         TEXT NOT NULL UNIQUE, -- normalized lowercase
    display_name  TEXT,
    last_seen     INTEGER,              -- last time we saw any message to/from them
    created_at    INTEGER NOT NULL
);

-- Threads (conversations)
CREATE TABLE threads (
    id            INTEGER PRIMARY KEY,
    account_id    INTEGER NOT NULL REFERENCES accounts(id),
    subject       TEXT,
    last_message  INTEGER,              -- UTC unix seconds of newest message
    UNIQUE(account_id, id)
);

-- Individual messages
CREATE TABLE messages (
    id            INTEGER PRIMARY KEY,
    account_id    INTEGER NOT NULL REFERENCES accounts(id),
    thread_id     INTEGER REFERENCES threads(id),
    message_id    TEXT,                 -- RFC 822 Message-ID, for dedup
    from_email    TEXT NOT NULL,        -- normalized lowercase
    to_emails     TEXT,                 -- JSON array of normalized addresses
    subject       TEXT,
    snippet       TEXT,                 -- first N chars, for list display
    body_text     TEXT,                 -- plain text body
    sent_at       INTEGER NOT NULL,     -- UTC unix seconds
    is_from_me    INTEGER NOT NULL,     -- 1 if sent by an owned account, else 0
    is_automated  INTEGER NOT NULL DEFAULT 0, -- newsletter/no-reply heuristic
    UNIQUE(account_id, message_id)
);

-- Detected open loops
CREATE TABLE loops (
    id            INTEGER PRIMARY KEY,
    kind          TEXT NOT NULL,        -- 'waiting_on' | 'owe_reply' | 'promised'
    thread_id     INTEGER REFERENCES threads(id),
    contact_id    INTEGER REFERENCES contacts(id),
    message_id    INTEGER REFERENCES messages(id), -- the message the loop hinges on
    detected_at   INTEGER NOT NULL,
    age_anchor    INTEGER NOT NULL,     -- timestamp we measure "how long" from
    status        TEXT NOT NULL DEFAULT 'open', -- 'open' | 'snoozed' | 'dismissed' | 'resolved'
    snoozed_until INTEGER,
    confidence    REAL DEFAULT 1.0      -- 1.0 for heuristics; AI may lower it
);

-- The privacy chokepoint: every outbound AI call is logged here.
CREATE TABLE ai_audit_log (
    id            INTEGER PRIMARY KEY,
    timestamp     INTEGER NOT NULL,
    provider      TEXT NOT NULL,        -- 'openai' | 'gemini' | 'claude' | 'ollama' | …
    model         TEXT,
    purpose       TEXT NOT NULL,        -- 'draft_reply' | 'detect_promise' | …
    data_summary  TEXT NOT NULL,        -- human-readable: what was sent
    was_local     INTEGER NOT NULL      -- 1 if a local model (nothing left machine)
);

-- Full-text search over messages
CREATE VIRTUAL TABLE messages_fts USING fts5(
    subject, body_text, content='messages', content_rowid='id'
);
```

Credentials note: **never store passwords or OAuth tokens in the SQLite file.** Use the
OS keychain (Tauri has a path for this / use a keychain crate). The DB stores only a
reference.

---

## 8. Email sync (the hardest, most underestimated part)

Treat sync as the riskiest engineering in v1. Every dead email client died here.

Requirements:
- **Incremental, not full-refetch.** Track per-account sync state (UID validity / last
  UID for IMAP, history/delta tokens for Gmail/Graph) so re-syncs are cheap.
- **Background + streaming.** Sync runs in a background task and emits `sync:progress`
  events. The UI must never block on it.
- **Resilient.** A 200k-message mailbox must not melt the app. Page through mail; commit
  to SQLite in batches; handle provider-specific quirks, rate limits, and reconnects.
- **Dedup** on `message_id`.

V1 provider order: start with **plain IMAP** (with app-password auth) to get the engine
working end-to-end, then add **Gmail OAuth**, then **Microsoft 365 (Graph) OAuth**. Each
OAuth provider is its own flow; isolate them in `sync/oauth.rs`.

For v1 we only need to read mail (sync inbound + sent). Sending (SMTP) and full compose
can follow once loops detection is proven.

---

## 9. AI layer (provider-agnostic, audited, optional)

Define a single trait that all providers implement:

```rust
// ai/provider.rs (illustrative)
#[async_trait]
pub trait AiProvider {
    fn name(&self) -> &str;
    fn is_local(&self) -> bool;
    async fn complete(&self, req: AiRequest) -> Result<AiStream, AppError>;
}
```

Implementations: OpenAI, Gemini, Claude, Ollama, LM Studio, DeepSeek, OpenRouter, Azure
OpenAI, and a generic "OpenAI-compatible" one. The rest of the app depends only on the
trait, never on a concrete provider.

Rules:
- **Every** `complete()` call writes to `ai_audit_log` *before* sending, via `ai/audit.rs`.
  No code path may reach a provider except through this audited entry point.
- Stream responses back to the frontend as `ai:token` events.
- The app must run with **no provider configured** — AI features degrade gracefully;
  heuristic loops still work.
- Local providers (`is_local() == true`) must guarantee nothing leaves the machine; mark
  `was_local = 1` in the audit log.

AI is **not** in the v1 critical path. Ship heuristic loops first, then add: better
"promised" detection, draft replies in the user's voice (learned from sent mail),
and semantic search.

---

## 10. Open-loop detection rules (v1 = heuristics, no AI)

Implement in `loops/rules.rs`. Run after each sync, then emit `loops:updated`.

**Kind 1 — `waiting_on`:**
> The newest message in a thread `is_from_me = 1`, it was sent to at least one real
> person, and more than **N days** (default 3, user-configurable) have passed with no
> newer inbound message in that thread.
- `contact_id` = primary recipient. `age_anchor` = that sent message's `sent_at`.

**Kind 2 — `owe_reply`:**
> The newest message in a thread `is_from_me = 0`, `is_automated = 0`, it plausibly
> expects a response (contains a question mark, or I am in `to` rather than only `cc`),
> and I have not replied (no newer `is_from_me = 1` message in the thread). Surface after
> a short grace period (default 1 day).
- `contact_id` = sender. `age_anchor` = that inbound message's `sent_at`.

**Kind 3 — `promised` (heuristic seed; AI later):**
> One of my own sent messages contains commitment language — patterns like "I'll send",
> "I will get back to you", "by <weekday/date>", "let me…". Lower `confidence`. This is
> deliberately rough in v1; AI improves precision later.

**Automated-sender heuristic** (`is_automated`): from-address or headers look like
`no-reply@`, `newsletter@`, bulk/list headers, etc. Used to suppress false `owe_reply`s.

**Loop lifecycle:** a loop auto-**resolves** when a qualifying newer message appears
(e.g. they finally reply → `waiting_on` resolves; I reply → `owe_reply` resolves). Users
can `snooze` (sets `snoozed_until`) or `dismiss`. Never delete resolved loops — set
`status`, so the graph keeps history.

---

## 11. The IPC contract (v1)

Commands (Rust → exposed to frontend). Names are `snake_case`; all return
`Result<T, AppError>`.

```
add_account(input) -> Account            # store account + creds in keychain
list_accounts() -> Vec<Account>
sync_account(account_id) -> ()           # starts background sync; emits sync:* events
remove_account(account_id) -> ()

list_loops(filter) -> Vec<LoopView>      # the main v1 screen
snooze_loop(loop_id, until) -> ()
dismiss_loop(loop_id) -> ()

get_thread(thread_id) -> ThreadView      # messages in a thread, for context
list_contacts() -> Vec<Contact>

# AI (post-heuristic):
draft_reply(thread_id, instructions) -> ()   # streams ai:token events
get_ai_audit_log() -> Vec<AuditEntry>        # the "what left my machine" view
```

Events (Rust → frontend):

```
sync:progress   { account_id, done, total }
sync:complete   { account_id, new_messages }
sync:error      { account_id, message }
loops:updated   { count }
ai:token        { request_id, token }
ai:done         { request_id }
```

`LoopView` should be display-ready (contact name, subject, human age string, kind, quick
actions) so the frontend does zero computation.

---

## 12. Coding conventions & how to work

**General**
- Write Rust idiomatically: `Result` + `?`, one `AppError` type, no `unwrap()` in
  non-test code paths that can fail on user data or network.
- Keep `commands/` thin. Logic in domain modules.
- Frontend is TypeScript with strict mode on. No `any`. Display logic only.
- Comment the *why*, not the *what*.

**Process**
- Work in small, reviewable increments. After scaffolding, build the open-loops vertical
  slice end-to-end (sync → store → detect → list) before polishing anything.
- Before adding a dependency, prefer the standard library / existing deps. Justify new
  crates briefly.
- When you hit a Tauri/IMAP/OAuth detail that depends on current library versions, check
  live documentation rather than assuming — these move and this file may lag.
- After each meaningful change, ensure it compiles and the vertical slice still runs.

**Testing**
- Unit-test the loop rules in `loops/rules.rs` with fixture messages — this is the heart
  of the product and must be correct.
- Make sync logic testable without a live server (trait/abstraction over the IMAP client).

**Boundaries to respect**
- No browser storage APIs anywhere; SQLite via Rust is the only persistence.
- No frontend → network. No frontend → provider. No logic in TypeScript.
- No new product modules (calendar/tasks/notes/CRM) without explicit human sign-off.

---

## 13. Build, ship & signing

- Build per-OS installers with Tauri (`.exe`/`.msi`, `.dmg`, `.AppImage`/`.deb`). One
  self-contained file each; Rust compiled in; nothing extra for the user to install.
  (Windows uses the built-in WebView2, present on Win10/11; Tauri's installer can fetch
  it on the rare machine without it.)
- **Windows first.** Get it fully working and polished before the macOS/Linux ports,
  which are adaptations, not rewrites.
- **Signing is deferred to public launch.** For internal users, ship **unsigned** —
  warnings are acceptable and internal users trust the source. Do **not** use a
  self-signed certificate (no trust benefit; can look more suspicious on Windows).
  At public launch: a real OV code-signing certificate for Windows, and an Apple
  Developer account + notarization for macOS. Tauri handles the signing mechanics; the
  certificates/accounts are provided by the human, not generated in code.

---

## 14. Roadmap (build in this order — do not skip ahead)

**Phase 0 — Scaffold.** Tauri 2.x + React/TS + SQLite wired; migrations run on startup;
one command + one event proven across the bridge.

**Phase 1 — The wedge (v1).** IMAP sync (background, streaming) → messages/contacts/
threads in SQLite → heuristic loop detection (kinds 1 & 2) → the single loops screen with
snooze/dismiss. **No AI required.** This is the shippable internal release.

**Phase 2 — Reach & polish.** Gmail + M365 OAuth. Ctrl+K command bar. Daily briefing view.
Kind-3 "promised" loops improved.

**Phase 3 — AI layer.** Provider-agnostic client + audit log + transparency view. Draft
replies in the user's voice. Local semantic search (on-device embeddings).

**Phase 4+ — Expand the graph.** Only now consider calendar, then notes/tasks, then the
"show me everything about project X" cross-module query. Each new module must feed the
graph, not stand alone.

---

## 15. The one-paragraph summary (if you read nothing else)

We are building a local-first Communication OS in **Tauri**: a **Rust** core that does all
the thinking (sync, SQLite storage, loop detection, audited AI) and a **React/TS**
frontend that only renders. V1 is **one feature** — the Open-Loops Engine that tells a
user what they're waiting on, owe, and promised — built first with **pure SQL heuristics,
no AI**, shipped unsigned to internal users as a single self-contained Windows `.exe`.
The web layer never thinks; all data access and AI calls funnel through Rust so the
privacy promise is enforceable. Guard scope ferociously: make the wedge great before
building anything else.