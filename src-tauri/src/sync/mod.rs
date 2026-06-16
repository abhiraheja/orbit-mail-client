//! Email sync engine. Runs in the background and emits `sync:*` events.
//!
//! Phase 1b fills this in: a `MailSource` trait abstracts the IMAP client so the
//! sync loop is unit-testable without a live server, with a concrete `async-imap`
//! implementation and a mock that feeds fixture messages.
