//! Keyword search powering the Ctrl+K palette (spec §6: search is Rust's job).
//!
//! Two concerns the frontend must never touch live here: turning raw user input
//! into a safe FTS5 MATCH expression / LIKE pattern, and merging the message and
//! contact hits into one display-ready, relevance-ordered list.

use rusqlite::Connection;

use crate::db::queries;
use crate::error::Result;
use crate::models::{SearchKind, SearchResult};

/// Max hits returned per source (messages, contacts). Keeps the palette snappy.
const PER_SOURCE: i64 = 8;

/// Build an FTS5 MATCH expression from raw input. Each alphanumeric token becomes
/// a quoted prefix term (`"tok"*`) so partial words match and FTS5 operators in
/// the input can't change the query's meaning. Returns None when nothing usable
/// remains (e.g. punctuation-only input).
pub fn build_match_query(raw: &str) -> Option<String> {
    let terms: Vec<String> = raw
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{}\"*", t.to_lowercase()))
        .collect();
    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" "))
    }
}

/// Escape a string for use inside a `LIKE … ESCAPE '\'` pattern, then wrap it in
/// `%…%` for a contains match.
fn like_contains(raw: &str) -> String {
    let mut escaped = String::with_capacity(raw.len() + 2);
    escaped.push('%');
    for ch in raw.chars() {
        if matches!(ch, '\\' | '%' | '_') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped.push('%');
    escaped
}

/// Run a keyword search across threads (FTS) and contacts (substring). Contacts
/// come first — a name match is usually the more precise intent — then the most
/// relevant thread hits. Empty/blank input yields no results.
pub fn search(conn: &Connection, raw: &str) -> Result<Vec<SearchResult>> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(Vec::new());
    }

    let mut out = Vec::new();

    for c in queries::search_contacts(conn, &like_contains(raw), PER_SOURCE)? {
        out.push(SearchResult {
            kind: SearchKind::Contact,
            title: c.display,
            subtitle: c.email.clone(),
            thread_id: None,
            contact_email: Some(c.email),
        });
    }

    if let Some(fts) = build_match_query(raw) {
        for m in queries::search_messages(conn, &fts, PER_SOURCE)? {
            let subtitle = if m.snippet.is_empty() {
                m.from_email
            } else {
                m.snippet
            };
            out.push(SearchResult {
                kind: SearchKind::Thread,
                title: m.subject,
                subtitle,
                thread_id: Some(m.thread_id),
                contact_email: None,
            });
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::ingest::ingest_messages;
    use crate::sync::{Folder, IncomingMessage};
    use std::collections::HashSet;

    const NOW: i64 = 1_000_000_000;

    fn db() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::migrations::run(&conn).unwrap();
        queries::insert_account(&conn, "me@acme.com", None, "imap", "password", None, 0).unwrap();
        let owner: HashSet<String> = ["me@acme.com".to_string()].into_iter().collect();
        let msg = IncomingMessage {
            message_id: Some("<a>".into()),
            in_reply_to: None,
            references: vec![],
            from_email: "alice@globex.com".into(),
            from_name: Some("Alice Globex".into()),
            to_emails: vec!["me@acme.com".into()],
            cc_emails: vec![],
            subject: Some("Quarterly budget review".into()),
            body_text: Some("Can we lock the budget numbers this week?".into()),
            sent_at: NOW,
            bulk_headers: false,
        };
        ingest_messages(&mut conn, 1, &owner, Folder::Inbox, &[msg], NOW).unwrap();
        conn
    }

    #[test]
    fn match_query_builds_prefix_terms() {
        assert_eq!(build_match_query("budget rev").unwrap(), "\"budget\"* \"rev\"*");
        // Operator characters can't break out — they're stripped as separators.
        assert_eq!(build_match_query("a OR* b").unwrap(), "\"a\"* \"or\"* \"b\"*");
        assert!(build_match_query("   ").is_none());
        assert!(build_match_query("!!!").is_none());
    }

    #[test]
    fn finds_thread_by_subject_prefix() {
        let conn = db();
        let hits = search(&conn, "budg").unwrap();
        assert!(hits.iter().any(|h| h.kind == SearchKind::Thread
            && h.title == "Quarterly budget review"
            && h.thread_id.is_some()));
    }

    #[test]
    fn finds_thread_by_body_term() {
        let conn = db();
        let hits = search(&conn, "numbers").unwrap();
        assert!(hits.iter().any(|h| h.kind == SearchKind::Thread));
    }

    #[test]
    fn finds_contact_by_name_substring() {
        let conn = db();
        let hits = search(&conn, "globex").unwrap();
        let contact = hits.iter().find(|h| h.kind == SearchKind::Contact).unwrap();
        assert_eq!(contact.contact_email.as_deref(), Some("alice@globex.com"));
    }

    #[test]
    fn blank_query_returns_nothing() {
        let conn = db();
        assert!(search(&conn, "   ").unwrap().is_empty());
    }

    #[test]
    fn like_special_chars_are_escaped() {
        // A literal % must not act as a wildcard; this contact has no such name.
        let conn = db();
        assert!(search(&conn, "%").unwrap().is_empty());
    }
}
