//! Provider-agnostic AI + the audit-log chokepoint (spec §3.3, §9).
//!
//! Phase 3. NOT on the v1 critical path: the app must run fully with no provider
//! configured. The single rule that makes the privacy promise enforceable —
//! every outbound call writes `ai_audit_log` *before* sending — is implemented
//! in `audit.rs`, and no code path may reach a provider except through it.
