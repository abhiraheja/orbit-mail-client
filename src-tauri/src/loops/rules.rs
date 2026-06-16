//! Heuristic open-loop detection (spec §10). The heart of the product — every
//! rule here is unit-tested with fixture messages.
//!
//! Detection is a pure function of the synced mail plus the user's thresholds.
//! After computing the current set of loops we reconcile against stored loops:
//! new conditions are inserted, conditions that cleared auto-resolve, and the
//! user's `snooze`/`dismiss` decisions are respected (spec §10 lifecycle).

use std::collections::{HashMap, HashSet};

use rusqlite::Connection;

use crate::db::queries::{self, ThreadTip};
use crate::error::Result;
use crate::models::LoopKind;
use crate::state::Config;

const DAY: i64 = 86_400;

/// One detected loop before persistence.
struct Detection {
    kind: LoopKind,
    thread_id: i64,
    contact_email: Option<String>,
    anchor_message_id: i64,
    age_anchor: i64,
    confidence: f64,
}

/// Render a human age string from a duration in seconds. Pre-computed in Rust so
/// the frontend renders it verbatim (spec §11).
pub fn format_age(now: i64, anchor: i64) -> String {
    let secs = (now - anchor).max(0);
    let days = secs / DAY;
    if days >= 14 {
        format!("{} weeks", days / 7)
    } else if days >= 1 {
        if days == 1 { "1 day".into() } else { format!("{days} days") }
    } else {
        let hours = secs / 3600;
        if hours >= 1 {
            if hours == 1 { "1 hour".into() } else { format!("{hours} hours") }
        } else {
            "just now".into()
        }
    }
}

/// Commitment language for the kind-3 "promised" heuristic. Deliberately rough;
/// AI improves precision later (spec §10).
fn contains_commitment(body: &str) -> bool {
    let b = body.to_lowercase();
    const PHRASES: &[&str] = &[
        "i'll send", "i will send", "i'll get back", "i will get back",
        "get back to you", "i'll follow up", "i will follow up", "let me get",
        "let me send", "i'll have", "i will have", "by tomorrow", "by monday",
        "by tuesday", "by wednesday", "by thursday", "by friday", "by end of day",
        "by eod", "i'll circle back", "will circle back",
    ];
    PHRASES.iter().any(|p| b.contains(p))
}

fn parse_emails(json: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(json).unwrap_or_default()
}

/// First recipient who isn't one of my own addresses.
fn primary_recipient(to_json: &str, owner: &HashSet<String>) -> Option<String> {
    parse_emails(to_json).into_iter().find(|e| !owner.contains(e))
}

/// Compute the current set of loops from synced mail. Pure; no DB writes.
fn detect(tips: &[ThreadTip], sent: &[queries::SentMessage], owner: &HashSet<String>, cfg: &Config, now: i64) -> Vec<Detection> {
    let mut out = Vec::new();

    for tip in tips {
        if tip.is_from_me {
            // Kind 1 — waiting_on: my message is newest, sent to a real person,
            // and unanswered past the threshold.
            if let Some(rcpt) = primary_recipient(&tip.to_emails, owner) {
                if now - tip.sent_at > cfg.waiting_on_days * DAY {
                    out.push(Detection {
                        kind: LoopKind::WaitingOn,
                        thread_id: tip.thread_id,
                        contact_email: Some(rcpt.clone()),
                        anchor_message_id: tip.message_id,
                        age_anchor: tip.sent_at,
                        confidence: 1.0,
                    });
                }
            }
        } else if !tip.is_automated {
            // Kind 2 — owe_reply: newest is inbound, not automated, plausibly
            // expects a reply (a question, or I'm a direct To: recipient), and
            // I haven't replied. Surfaced after a short grace period.
            let has_question = tip.body_text.as_deref().is_some_and(|b| b.contains('?'))
                || tip.subject.as_deref().is_some_and(|s| s.contains('?'));
            let i_am_direct = parse_emails(&tip.to_emails).iter().any(|e| owner.contains(e));
            if (has_question || i_am_direct) && now - tip.sent_at > cfg.owe_reply_grace_days * DAY {
                out.push(Detection {
                    kind: LoopKind::OweReply,
                    thread_id: tip.thread_id,
                    contact_email: Some(tip.from_email.clone()),
                    anchor_message_id: tip.message_id,
                    age_anchor: tip.sent_at,
                    confidence: 1.0,
                });
            }
        }
    }

    // Kind 3 — promised: my sent mail contains commitment language and is still
    // the newest message in its thread (the ball is still in my court).
    let tip_msg_ids: HashSet<i64> = tips.iter().map(|t| t.message_id).collect();
    for s in sent {
        if !tip_msg_ids.contains(&s.message_id) {
            continue; // someone has since replied → not an open promise
        }
        if s.body_text.as_deref().is_some_and(contains_commitment) {
            out.push(Detection {
                kind: LoopKind::Promised,
                thread_id: s.thread_id,
                contact_email: primary_recipient(&s.to_emails, owner),
                anchor_message_id: s.message_id,
                age_anchor: s.sent_at,
                confidence: 0.5, // heuristic; AI raises this later
            });
        }
    }

    out
}

/// Detect loops from the current mail and reconcile with stored loops. Returns
/// the count of currently-actionable loops.
pub fn detect_and_store(conn: &mut Connection, cfg: &Config, now: i64) -> Result<i64> {
    let owner: HashSet<String> = queries::account_emails(conn)?.into_iter().collect();
    let tips = queries::thread_tips(conn)?;
    let sent = queries::sent_messages(conn)?;
    let detections = detect(&tips, &sent, &owner, cfg, now);

    // Index detections by (kind, thread_id) for reconciliation.
    let desired: HashMap<(String, i64), &Detection> = detections
        .iter()
        .map(|d| ((d.kind.as_str().to_string(), d.thread_id), d))
        .collect();

    let existing = queries::existing_loops(conn)?;
    let mut existing_keys: HashSet<(String, i64)> = HashSet::new();

    let tx = conn.transaction()?;

    // Auto-resolve loops whose condition no longer holds.
    for e in &existing {
        let Some(tid) = e.thread_id else { continue };
        let key = (e.kind.clone(), tid);
        existing_keys.insert(key.clone());
        let still_open = matches!(e.status.as_str(), "open" | "snoozed" | "dismissed");
        if still_open && !desired.contains_key(&key) && matches!(e.status.as_str(), "open" | "snoozed") {
            queries::resolve_loop(&tx, e.id)?;
        }
    }

    // Insert genuinely new loops. If a loop already exists for this (kind,thread)
    // — open, snoozed, or dismissed — leave it alone (respect the user's action).
    for ((kind, thread_id), d) in &desired {
        if existing_keys.contains(&(kind.clone(), *thread_id)) {
            continue;
        }
        let contact_id = match &d.contact_email {
            Some(email) => queries::find_contact_by_email(&tx, email)?,
            None => None,
        };
        queries::insert_loop(
            &tx,
            kind,
            *thread_id,
            contact_id,
            d.anchor_message_id,
            now,
            d.age_anchor,
            d.confidence,
        )?;
    }

    tx.commit()?;
    queries::count_active_loops(conn, now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::ingest::ingest_messages;
    use crate::sync::{Folder, IncomingMessage};

    const NOW: i64 = 1_000_000_000;

    fn db_with_account() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::migrations::run(&conn).unwrap();
        queries::insert_account(&conn, "me@acme.com", None, "imap", "password", None, 0).unwrap();
        // ingest needs &mut; return owned conn, callers re-borrow.
        let _ = &mut conn;
        conn
    }

    fn owner() -> HashSet<String> {
        ["me@acme.com".to_string()].into_iter().collect()
    }

    fn inbound(id: &str, subject: &str, body: &str, sent_at: i64) -> IncomingMessage {
        IncomingMessage {
            message_id: Some(id.into()),
            in_reply_to: None,
            references: vec![],
            from_email: "john@x.com".into(),
            from_name: Some("John".into()),
            to_emails: vec!["me@acme.com".into()],
            cc_emails: vec![],
            subject: Some(subject.into()),
            body_text: Some(body.into()),
            sent_at,
            bulk_headers: false,
        }
    }

    fn outbound(id: &str, subject: &str, body: &str, sent_at: i64) -> IncomingMessage {
        IncomingMessage {
            message_id: Some(id.into()),
            in_reply_to: None,
            references: vec![],
            from_email: "me@acme.com".into(),
            from_name: None,
            to_emails: vec!["john@x.com".into()],
            cc_emails: vec![],
            subject: Some(subject.into()),
            body_text: Some(body.into()),
            sent_at,
            bulk_headers: false,
        }
    }

    fn loops_of_kind(conn: &Connection, kind: LoopKind) -> i64 {
        conn.query_row(
            "SELECT count(*) FROM loops WHERE kind = ?1 AND status = 'open'",
            [kind.as_str()],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[test]
    fn waiting_on_after_threshold() {
        let mut conn = db_with_account();
        // I emailed John 5 days ago; no reply.
        let five_days = NOW - 5 * DAY;
        let batch = vec![outbound("<a>", "Pricing", "here is the quote", five_days)];
        ingest_messages(&mut conn, 1, &owner(), Folder::Sent, &batch, NOW).unwrap();

        detect_and_store(&mut conn, &Config::default(), NOW).unwrap();
        assert_eq!(loops_of_kind(&conn, LoopKind::WaitingOn), 1);
        assert_eq!(loops_of_kind(&conn, LoopKind::OweReply), 0);
    }

    #[test]
    fn waiting_on_not_before_threshold() {
        let mut conn = db_with_account();
        // Sent only 1 day ago; default threshold is 3 days.
        let batch = vec![outbound("<a>", "Pricing", "quote", NOW - DAY)];
        ingest_messages(&mut conn, 1, &owner(), Folder::Sent, &batch, NOW).unwrap();
        detect_and_store(&mut conn, &Config::default(), NOW).unwrap();
        assert_eq!(loops_of_kind(&conn, LoopKind::WaitingOn), 0);
    }

    #[test]
    fn owe_reply_with_question() {
        let mut conn = db_with_account();
        // John asked me a question 2 days ago; grace is 1 day.
        let batch = vec![inbound("<a>", "Quick q", "can you confirm the date?", NOW - 2 * DAY)];
        ingest_messages(&mut conn, 1, &owner(), Folder::Inbox, &batch, NOW).unwrap();
        detect_and_store(&mut conn, &Config::default(), NOW).unwrap();
        assert_eq!(loops_of_kind(&conn, LoopKind::OweReply), 1);
    }

    #[test]
    fn owe_reply_suppressed_for_automated() {
        let mut conn = db_with_account();
        let mut m = inbound("<a>", "Your receipt", "no action needed", NOW - 2 * DAY);
        m.from_email = "no-reply@stripe.com".into();
        ingest_messages(&mut conn, 1, &owner(), Folder::Inbox, &[m], NOW).unwrap();
        detect_and_store(&mut conn, &Config::default(), NOW).unwrap();
        assert_eq!(loops_of_kind(&conn, LoopKind::OweReply), 0);
    }

    #[test]
    fn owe_reply_resolves_when_i_reply() {
        let mut conn = db_with_account();
        let q = inbound("<a>", "Pricing?", "what's the price?", NOW - 3 * DAY);
        ingest_messages(&mut conn, 1, &owner(), Folder::Inbox, &[q], NOW).unwrap();
        detect_and_store(&mut conn, &Config::default(), NOW).unwrap();
        assert_eq!(loops_of_kind(&conn, LoopKind::OweReply), 1);

        // I reply → newest message is mine → owe_reply should auto-resolve.
        let mut reply = outbound("<b>", "Re: Pricing?", "it's $99", NOW - DAY);
        reply.in_reply_to = Some("<a>".into());
        ingest_messages(&mut conn, 1, &owner(), Folder::Sent, &[reply], NOW).unwrap();
        detect_and_store(&mut conn, &Config::default(), NOW).unwrap();

        assert_eq!(loops_of_kind(&conn, LoopKind::OweReply), 0, "should auto-resolve");
        let resolved: i64 = conn
            .query_row(
                "SELECT count(*) FROM loops WHERE kind='owe_reply' AND status='resolved'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(resolved, 1);
    }

    #[test]
    fn promised_detected_from_commitment_language() {
        let mut conn = db_with_account();
        let batch = vec![outbound("<a>", "SSO doc", "I'll send the doc by Friday", NOW - DAY)];
        ingest_messages(&mut conn, 1, &owner(), Folder::Sent, &batch, NOW).unwrap();
        detect_and_store(&mut conn, &Config::default(), NOW).unwrap();
        assert_eq!(loops_of_kind(&conn, LoopKind::Promised), 1);
        // Promised carries lower confidence than the SQL-certain kinds.
        let conf: f64 = conn
            .query_row("SELECT confidence FROM loops WHERE kind='promised'", [], |r| r.get(0))
            .unwrap();
        assert!(conf < 1.0);
    }

    #[test]
    fn dismissed_loop_is_not_recreated() {
        let mut conn = db_with_account();
        let batch = vec![outbound("<a>", "Pricing", "quote", NOW - 5 * DAY)];
        ingest_messages(&mut conn, 1, &owner(), Folder::Sent, &batch, NOW).unwrap();
        detect_and_store(&mut conn, &Config::default(), NOW).unwrap();
        let id: i64 = conn
            .query_row("SELECT id FROM loops WHERE kind='waiting_on'", [], |r| r.get(0))
            .unwrap();
        queries::dismiss_loop(&conn, id).unwrap();

        // Re-running detection must not resurrect the dismissed loop.
        detect_and_store(&mut conn, &Config::default(), NOW).unwrap();
        assert_eq!(loops_of_kind(&conn, LoopKind::WaitingOn), 0);
        let total: i64 = conn
            .query_row("SELECT count(*) FROM loops WHERE kind='waiting_on'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(total, 1, "still exactly one (dismissed) loop, not a duplicate");
    }

    #[test]
    fn age_formatting() {
        assert_eq!(format_age(NOW, NOW - 5 * DAY), "5 days");
        assert_eq!(format_age(NOW, NOW - DAY), "1 day");
        assert_eq!(format_age(NOW, NOW - 3600), "1 hour");
        assert_eq!(format_age(NOW, NOW - 21 * DAY), "3 weeks");
    }
}
