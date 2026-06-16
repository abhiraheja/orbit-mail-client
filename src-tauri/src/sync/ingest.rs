//! Turn fetched mail into normalized rows: resolve contacts, group into threads,
//! dedup, and persist. This is the bridge between `sync` and what loop detection
//! reads. It contains the logic; all SQL lives in `db::queries`.

use std::collections::HashSet;

use rusqlite::Connection;

use crate::db::queries;
use crate::error::Result;
use crate::sync::{Folder, IncomingMessage};

/// Normalize an email address for storage and comparison: trimmed, lowercased.
pub fn normalize_email(raw: &str) -> String {
    raw.trim().trim_matches(|c| c == '<' || c == '>').to_lowercase()
}

/// Strip leading reply/forward prefixes so replies group with their original.
pub fn normalize_subject(subject: &str) -> String {
    let mut s = subject.trim();
    loop {
        let lower = s.to_lowercase();
        let trimmed = ["re:", "fwd:", "fw:", "re :", "fwd :"]
            .iter()
            .find_map(|p| lower.strip_prefix(p).map(|_| s[p.len()..].trim_start()));
        match trimmed {
            Some(rest) => s = rest,
            None => break,
        }
    }
    s.to_lowercase()
}

/// Heuristic: does this look like automated/bulk mail that should never produce
/// an `owe_reply` loop? (spec §10 automated-sender heuristic)
pub fn is_automated(from_email: &str, bulk_headers: bool) -> bool {
    if bulk_headers {
        return true;
    }
    let local = from_email.split('@').next().unwrap_or("");
    const MARKERS: &[&str] = &[
        "no-reply", "noreply", "no_reply", "donotreply", "do-not-reply",
        "newsletter", "notifications", "notification", "mailer-daemon",
        "postmaster", "bounce", "mailer", "automated", "alerts",
    ];
    MARKERS.iter().any(|m| local.contains(m))
}

/// First ~160 chars of the body on a single line, for list display.
fn make_snippet(body: Option<&str>) -> Option<String> {
    body.map(|b| {
        let collapsed = b.split_whitespace().collect::<Vec<_>>().join(" ");
        collapsed.chars().take(160).collect()
    })
}

/// Resolve the thread a message belongs to, creating one if needed.
/// Preference order: direct reply → referenced chain → same normalized subject.
fn resolve_thread(
    conn: &Connection,
    account_id: i64,
    msg: &IncomingMessage,
    norm_subject: &str,
) -> Result<i64> {
    if let Some(irt) = &msg.in_reply_to {
        if let Some(tid) = queries::thread_of_message_id(conn, account_id, irt)? {
            return Ok(tid);
        }
    }
    for r in msg.references.iter().rev() {
        if let Some(tid) = queries::thread_of_message_id(conn, account_id, r)? {
            return Ok(tid);
        }
    }
    if let Some(tid) = queries::thread_by_norm_subject(conn, account_id, norm_subject)? {
        return Ok(tid);
    }
    queries::create_thread(
        conn,
        account_id,
        msg.subject.as_deref(),
        norm_subject,
        msg.sent_at,
    )
}

/// Ingest a batch of fetched messages. Returns the number of NEW messages stored
/// (already-seen Message-IDs are skipped). `owner_emails` are this user's own
/// addresses, used to set `is_from_me`.
pub fn ingest_messages(
    conn: &mut Connection,
    account_id: i64,
    owner_emails: &HashSet<String>,
    folder: Folder,
    messages: &[IncomingMessage],
    now: i64,
) -> Result<usize> {
    let tx = conn.transaction()?;
    let mut inserted = 0usize;

    for msg in messages {
        let from_email = normalize_email(&msg.from_email);

        // Dedup on Message-ID (when present).
        if let Some(mid) = &msg.message_id {
            if queries::message_exists(&tx, account_id, mid)? {
                continue;
            }
        }

        let norm_subject = msg.subject.as_deref().map(normalize_subject).unwrap_or_default();
        let thread_id = resolve_thread(&tx, account_id, msg, &norm_subject)?;

        // Sent-folder mail is mine regardless of From; otherwise compare addresses.
        let is_from_me = folder == Folder::Sent || owner_emails.contains(&from_email);
        let automated = !is_from_me && is_automated(&from_email, msg.bulk_headers);

        let to_normalized: Vec<String> = msg.to_emails.iter().map(|e| normalize_email(e)).collect();
        let to_json = serde_json::to_string(&to_normalized).unwrap_or_else(|_| "[]".into());
        let snippet = make_snippet(msg.body_text.as_deref());

        queries::insert_message(
            &tx,
            account_id,
            thread_id,
            msg.message_id.as_deref(),
            &from_email,
            &to_json,
            msg.subject.as_deref(),
            snippet.as_deref(),
            msg.body_text.as_deref(),
            msg.sent_at,
            is_from_me,
            automated,
        )?;
        queries::bump_thread_last_message(&tx, thread_id, msg.sent_at)?;

        // Contact resolution: everyone who isn't me becomes/updates a contact.
        let from_name = msg.from_name.as_deref();
        if !is_from_me {
            queries::upsert_contact(&tx, &from_email, from_name, msg.sent_at, now)?;
        }
        for to in &to_normalized {
            if !owner_emails.contains(to) {
                queries::upsert_contact(&tx, to, None, msg.sent_at, now)?;
            }
        }
        for cc in &msg.cc_emails {
            let cc = normalize_email(cc);
            if !owner_emails.contains(&cc) {
                queries::upsert_contact(&tx, &cc, None, msg.sent_at, now)?;
            }
        }

        inserted += 1;
    }

    tx.commit()?;
    Ok(inserted)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(id: &str, from: &str, to: &[&str], subject: &str, sent_at: i64) -> IncomingMessage {
        IncomingMessage {
            message_id: Some(id.to_string()),
            in_reply_to: None,
            references: vec![],
            from_email: from.to_string(),
            from_name: None,
            to_emails: to.iter().map(|s| s.to_string()).collect(),
            cc_emails: vec![],
            subject: Some(subject.to_string()),
            body_text: Some("hello".into()),
            sent_at,
            bulk_headers: false,
        }
    }

    fn fresh_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::migrations::run(&conn).unwrap();
        // Messages FK-reference accounts (rusqlite's bundled SQLite enforces FKs).
        crate::db::queries::insert_account(&conn, "me@acme.com", None, "imap", "password", None, 0)
            .unwrap();
        conn
    }

    #[test]
    fn normalize_subject_strips_prefixes() {
        assert_eq!(normalize_subject("Re: Re: Hello"), "hello");
        assert_eq!(normalize_subject("FWD: Project"), "project");
        assert_eq!(normalize_subject("Plain"), "plain");
    }

    #[test]
    fn automated_detection() {
        assert!(is_automated("no-reply@stripe.com", false));
        assert!(is_automated("newsletter@news.com", false));
        assert!(is_automated("anyone@x.com", true));
        assert!(!is_automated("john@acme.com", false));
    }

    #[test]
    fn dedup_skips_seen_message_id() {
        let mut conn = fresh_db();
        let owner: HashSet<String> = ["me@acme.com".into()].into_iter().collect();
        let batch = vec![msg("<a>", "john@x.com", &["me@acme.com"], "Hi", 100)];
        let n1 = ingest_messages(&mut conn, 1, &owner, Folder::Inbox, &batch, 100).unwrap();
        let n2 = ingest_messages(&mut conn, 1, &owner, Folder::Inbox, &batch, 100).unwrap();
        assert_eq!(n1, 1);
        assert_eq!(n2, 0, "second ingest of same Message-ID is a no-op");
    }

    #[test]
    fn replies_group_into_one_thread_by_subject() {
        let mut conn = fresh_db();
        let owner: HashSet<String> = ["me@acme.com".into()].into_iter().collect();
        let batch = vec![
            msg("<a>", "john@x.com", &["me@acme.com"], "Pricing", 100),
            msg("<b>", "me@acme.com", &["john@x.com"], "Re: Pricing", 200),
        ];
        ingest_messages(&mut conn, 1, &owner, Folder::Inbox, &batch, 200).unwrap();
        let threads: i64 = conn
            .query_row("SELECT count(*) FROM threads", [], |r| r.get(0))
            .unwrap();
        assert_eq!(threads, 1, "reply should join the original thread");
    }

    #[test]
    fn in_reply_to_groups_even_without_subject_match() {
        let mut conn = fresh_db();
        let owner: HashSet<String> = ["me@acme.com".into()].into_iter().collect();
        let mut reply = msg("<b>", "me@acme.com", &["john@x.com"], "totally different", 200);
        reply.in_reply_to = Some("<a>".into());
        let batch = vec![
            msg("<a>", "john@x.com", &["me@acme.com"], "Pricing", 100),
            reply,
        ];
        ingest_messages(&mut conn, 1, &owner, Folder::Inbox, &batch, 200).unwrap();
        let threads: i64 = conn
            .query_row("SELECT count(*) FROM threads", [], |r| r.get(0))
            .unwrap();
        assert_eq!(threads, 1);
    }

    #[test]
    fn contacts_exclude_me() {
        let mut conn = fresh_db();
        let owner: HashSet<String> = ["me@acme.com".into()].into_iter().collect();
        let batch = vec![msg("<a>", "john@x.com", &["me@acme.com"], "Hi", 100)];
        ingest_messages(&mut conn, 1, &owner, Folder::Inbox, &batch, 100).unwrap();
        let contacts: Vec<String> = {
            let mut stmt = conn.prepare("SELECT email FROM contacts").unwrap();
            let rows = stmt.query_map([], |r| r.get::<_, String>(0)).unwrap();
            rows.map(|r| r.unwrap()).collect()
        };
        assert_eq!(contacts, vec!["john@x.com".to_string()]);
    }
}
