//! Open-loop detection. HEURISTICS FIRST (spec §10), AI later.
//!
//! Phase 1c fills `rules.rs` with the three detection rules, run after each sync,
//! emitting `loops:updated`.

pub mod briefing;
pub mod rules;
