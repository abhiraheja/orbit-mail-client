# Orbit — a local-first Communication OS

A desktop app that connects your email (and later calendar, contacts, tasks, notes)
into a single knowledge graph, and uses AI to make that graph useful. **All logic lives
in Rust; the web layer only renders.** All user data stays on your machine in SQLite.

See [`docs/PRODUCT-DRAFT.md`](docs/PRODUCT-DRAFT.md) for the full product spec — it is the
source of truth. Read it before contributing.

## v1 wedge: the Open-Loops Engine

One screen that tells you what you're **waiting on**, what you **owe a reply** to, and what
you **promised** — detected from synced mail with pure SQL heuristics, no AI required.

## Stack

- **Shell:** Tauri 2.x
- **Core:** Rust (sync, SQLite, loop detection, audited AI) — all the thinking
- **Frontend:** React + TypeScript (Vite) — rendering only
- **Storage:** SQLite (bundled via `rusqlite`) with FTS5

## Develop

Prerequisites: Rust (stable, MSVC toolchain on Windows) and Node.js.

```sh
npm install
npm run tauri dev      # launches the app (compiles Rust + serves the frontend)
```

Rust core only:

```sh
cd src-tauri
cargo build
cargo test             # loop rules + migrations are unit-tested
```

## Architecture

```
Frontend (React/TS)  — RENDER ONLY
        │  Tauri IPC (commands + events)
Rust core — ALL LOGIC: commands/ → db/ sync/ graph/ loops/ ai/
        │  network (only from Rust)
   IMAP/SMTP servers   •   AI providers
```

Module layout and the IPC contract are documented in the spec (§5, §6, §11).
