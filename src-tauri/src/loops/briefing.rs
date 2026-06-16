//! Daily briefing composition (spec §14, Phase 2).
//!
//! A pure read over already-detected loops — it never runs detection or touches
//! mail. Everything it returns is display-ready (headline + human age strings),
//! so the frontend renders it verbatim and computes nothing (spec §11).

use rusqlite::Connection;

use crate::db::queries;
use crate::error::Result;
use crate::loops::rules::format_age;
use crate::models::{BriefingView, LoopKind};

/// How many "do these first" loops the briefing surfaces.
const TOP_N: usize = 5;

/// Build the briefing from the current set of active loops. Assumes detection
/// has already run (it runs after every sync); this only reads.
pub fn build(conn: &Connection, now: i64) -> Result<BriefingView> {
    let counts = queries::active_loop_counts(conn, now)?;
    let total_active = counts.waiting_on + counts.owe_reply + counts.promised;
    let account_count = queries::count_accounts(conn)?;
    let last_synced = queries::latest_sync(conn)?.map(|ts| format!("{} ago", format_age(now, ts)));

    // Reuse the main-screen query (oldest first) and take the most urgent few.
    let top_loops = queries::list_loop_views(conn, now, None)?
        .into_iter()
        .take(TOP_N)
        .map(|r| {
            let kind = LoopKind::from_db_str(&r.kind);
            let age = format_age(now, r.age_anchor);
            r.into_view(kind, age)
        })
        .collect();

    let headline = headline(total_active, &counts);

    Ok(BriefingView {
        headline,
        total_active,
        waiting_on: counts.waiting_on,
        owe_reply: counts.owe_reply,
        promised: counts.promised,
        account_count,
        last_synced,
        top_loops,
    })
}

/// Compose the one-line headline. Only mentions the kinds that are non-zero so
/// it reads naturally ("3 open loops — 2 to reply, 1 promised").
fn headline(total: i64, c: &queries::LoopKindCounts) -> String {
    if total == 0 {
        return "You're all caught up — no open loops.".into();
    }
    let mut parts = Vec::new();
    if c.owe_reply > 0 {
        parts.push(format!("{} to reply", c.owe_reply));
    }
    if c.waiting_on > 0 {
        parts.push(format!("{} waiting on others", c.waiting_on));
    }
    if c.promised > 0 {
        parts.push(format!("{} promised", c.promised));
    }
    let noun = if total == 1 { "open loop" } else { "open loops" };
    format!("{total} {noun} — {}.", parts.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loops::rules::detect_and_store;
    use crate::state::Config;
    use crate::sync::ingest::ingest_messages;
    use crate::sync::{Folder, IncomingMessage};
    use std::collections::HashSet;

    const DAY: i64 = 86_400;
    const NOW: i64 = 1_000_000_000;

    fn db_with_account() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::migrations::run(&conn).unwrap();
        queries::insert_account(&conn, "me@acme.com", None, "imap", "password", None, 0).unwrap();
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

    #[test]
    fn empty_when_no_loops() {
        let conn = db_with_account();
        let b = build(&conn, NOW).unwrap();
        assert_eq!(b.total_active, 0);
        assert!(b.headline.contains("caught up"));
        assert!(b.top_loops.is_empty());
        // Account exists but was never synced.
        assert_eq!(b.account_count, 1);
        assert!(b.last_synced.is_none());
    }

    #[test]
    fn counts_and_headline_reflect_loops() {
        let mut conn = db_with_account();
        // Kind 1: I emailed John 5 days ago, no reply → waiting_on.
        let waiting = outbound("<w>", "Pricing", "here is the quote", NOW - 5 * DAY);
        // Kind 2: John asked me a question 2 days ago → owe_reply.
        let owe = inbound("<o>", "Question", "can you confirm the date?", NOW - 2 * DAY);
        ingest_messages(&mut conn, 1, &owner(), Folder::Sent, &[waiting], NOW).unwrap();
        ingest_messages(&mut conn, 1, &owner(), Folder::Inbox, &[owe], NOW).unwrap();
        detect_and_store(&mut conn, &Config::default(), NOW).unwrap();

        let b = build(&conn, NOW).unwrap();
        assert_eq!(b.total_active, 2);
        assert_eq!(b.waiting_on, 1);
        assert_eq!(b.owe_reply, 1);
        assert_eq!(b.promised, 0);
        assert_eq!(b.top_loops.len(), 2);
        // Oldest first: the 5-day waiting_on outranks the 2-day owe_reply.
        assert_eq!(b.top_loops[0].kind, LoopKind::WaitingOn);
        assert!(b.headline.contains("2 open loops"));
        assert!(b.headline.contains("1 to reply"));
        assert!(b.headline.contains("1 waiting on others"));
        // Promised is zero, so it must not appear.
        assert!(!b.headline.contains("promised"));
    }

    #[test]
    fn top_loops_capped() {
        let mut conn = db_with_account();
        // Seven distinct waiting_on loops, all past threshold.
        let batch: Vec<IncomingMessage> = (0..7)
            .map(|i| outbound(&format!("<w{i}>"), &format!("Topic {i}"), "quote", NOW - (10 + i) * DAY))
            .collect();
        ingest_messages(&mut conn, 1, &owner(), Folder::Sent, &batch, NOW).unwrap();
        detect_and_store(&mut conn, &Config::default(), NOW).unwrap();

        let b = build(&conn, NOW).unwrap();
        assert_eq!(b.total_active, 7);
        assert_eq!(b.top_loops.len(), TOP_N, "top list is capped");
    }
}
