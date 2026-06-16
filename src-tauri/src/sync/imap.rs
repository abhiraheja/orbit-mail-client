//! Concrete `MailSource` backed by `async-imap` (app-password auth first, spec §8).
//!
//! Connects over TLS, logs in, SELECTs the folder, and UID-FETCHes messages newer
//! than the stored cursor, parsing each with `mail-parser` into [`IncomingMessage`].
//! Runs on the app's tokio runtime (see the runtime features in Cargo.toml).
//!
//! Not yet exercised against a live server — there's no test account. The shape is
//! complete; first real use should verify incremental pulls, the Sent-folder name,
//! and UIDVALIDITY changes (see TODOs). Sync *orchestration* in `runner` is already
//! proven via a mock source.

use async_imap::Client;
use futures::StreamExt;
use mail_parser::{Address, HeaderValue, MessageParser};
use tokio::net::TcpStream;

use crate::error::{AppError, Result};
use crate::sync::oauth::XOAuth2;
use crate::sync::{FetchBatch, Folder, IncomingMessage, MailSource};

/// Cap the first (cursorless) pull so initial sync stays bounded on large
/// mailboxes. Subsequent syncs are incremental from the stored UID.
const INITIAL_WINDOW: i64 = 500;

/// How the IMAP session authenticates: app password (LOGIN) or an OAuth bearer
/// token (SASL XOAUTH2, for Gmail / M365).
pub enum ImapAuth {
    Password(String),
    OAuth { access_token: String },
}

/// Connection parameters for an IMAP account.
pub struct ImapConfig {
    pub host: String,
    pub port: u16,
    pub email: String,
    pub auth: ImapAuth,
}

pub struct ImapSource {
    cfg: ImapConfig,
}

impl ImapSource {
    pub fn new(cfg: ImapConfig) -> Self {
        Self { cfg }
    }
}

#[async_trait::async_trait]
impl MailSource for ImapSource {
    async fn fetch(&mut self, folder: Folder, since_uid: Option<i64>) -> Result<FetchBatch> {
        // 1. TCP + TLS to the IMAP endpoint.
        let tcp = TcpStream::connect((self.cfg.host.as_str(), self.cfg.port))
            .await
            .map_err(|e| AppError::Sync(format!("connect {}:{}: {e}", self.cfg.host, self.cfg.port)))?;
        let tls = async_native_tls::TlsConnector::new()
            .connect(self.cfg.host.as_str(), tcp)
            .await
            .map_err(|e| AppError::Sync(format!("TLS handshake: {e}")))?;

        // 2. Authenticate: app-password LOGIN, or SASL XOAUTH2 for OAuth accounts.
        //    On failure async-imap hands back the client alongside the error.
        let client = Client::new(tls);
        let mut session = match &self.cfg.auth {
            ImapAuth::Password(password) => client
                .login(&self.cfg.email, password)
                .await
                .map_err(|(e, _client)| AppError::Sync(format!("IMAP login: {e}")))?,
            ImapAuth::OAuth { access_token } => {
                let auth = XOAuth2 { user: self.cfg.email.clone(), access_token: access_token.clone() };
                client
                    .authenticate("XOAUTH2", &auth)
                    .await
                    .map_err(|(e, _client)| AppError::Sync(format!("IMAP XOAUTH2: {e}")))?
            }
        };

        // 3. SELECT the folder and read its UID metadata.
        // TODO(folders): the Sent folder name varies by server ("Sent Items",
        // "[Gmail]/Sent Mail"); make it configurable / discover via LIST.
        let mailbox = session
            .select(folder.as_str())
            .await
            .map_err(|e| AppError::Sync(format!("SELECT {}: {e}", folder.as_str())))?;
        let uid_validity = mailbox.uid_validity.map(|v| v as i64);
        // TODO(uidvalidity): if uid_validity changed vs the stored cursor, the old
        // UIDs are invalid and we should re-pull from 1. The runner persists it;
        // reconciliation is a follow-up once we can test against a real server.

        // Empty mailbox: nothing to do.
        if mailbox.exists == 0 {
            let _ = session.logout().await;
            return Ok(FetchBatch { folder, messages: vec![], last_uid: since_uid, uid_validity });
        }

        // 4. Build the UID range. Incremental from the cursor, or a bounded tail
        //    on the first pull.
        let range = match since_uid {
            Some(u) => format!("{}:*", u + 1),
            None => {
                let floor = mailbox
                    .uid_next
                    .map(|n| (n as i64 - INITIAL_WINDOW).max(1))
                    .unwrap_or(1);
                format!("{floor}:*")
            }
        };

        // 5. UID-FETCH headers, body, and the server timestamp in one pass.
        let mut stream = session
            .uid_fetch(&range, "(UID INTERNALDATE BODY.PEEK[])")
            .await
            .map_err(|e| AppError::Sync(format!("UID FETCH {range}: {e}")))?;

        let parser = MessageParser::default();
        let mut messages = Vec::new();
        let mut max_uid = since_uid.unwrap_or(0);

        while let Some(item) = stream.next().await {
            let fetch = item.map_err(|e| AppError::Sync(format!("fetch stream: {e}")))?;
            if let Some(uid) = fetch.uid {
                max_uid = max_uid.max(uid as i64);
            }
            let Some(raw) = fetch.body() else { continue };
            let internal = fetch.internal_date().map(|d| d.timestamp());
            if let Some(msg) = parse_incoming(&parser, raw, internal) {
                messages.push(msg);
            }
        }

        // Drop the stream's borrow on the session before logging out.
        drop(stream);
        let _ = session.logout().await;

        Ok(FetchBatch {
            folder,
            messages,
            last_uid: Some(max_uid),
            uid_validity,
        })
    }
}

/// Strip the angle brackets and surrounding whitespace from a Message-ID so all
/// ids (message_id, in_reply_to, references) normalize identically — ingest's
/// threading matches them by exact string.
fn norm_mid(s: &str) -> String {
    s.trim().trim_start_matches('<').trim_end_matches('>').trim().to_string()
}

/// Lowercased, trimmed email from a parsed address.
fn addr_email(addr: &mail_parser::Addr) -> Option<String> {
    addr.address.as_ref().map(|a| a.trim().to_lowercase()).filter(|a| !a.is_empty())
}

/// Collect every address in a To:/Cc: header value.
fn address_emails(value: Option<&Address>) -> Vec<String> {
    match value {
        Some(addr) => addr.iter().filter_map(addr_email).collect(),
        None => vec![],
    }
}

/// Extract Message-IDs from an In-Reply-To / References header value, which may
/// be a single id or a list.
fn header_mids(value: &HeaderValue) -> Vec<String> {
    match value {
        HeaderValue::Text(t) => vec![norm_mid(t)],
        HeaderValue::TextList(list) => list.iter().map(|t| norm_mid(t)).collect(),
        _ => vec![],
    }
}

/// Parse a raw RFC822 message into our normalized `IncomingMessage`. Returns None
/// only if the bytes don't parse as a message at all.
fn parse_incoming(parser: &MessageParser, raw: &[u8], internal: Option<i64>) -> Option<IncomingMessage> {
    let msg = parser.parse(raw)?;

    let (from_email, from_name) = match msg.from().and_then(|a| a.first()) {
        Some(addr) => (
            addr_email(addr).unwrap_or_default(),
            addr.name.as_ref().map(|n| n.trim().to_string()).filter(|n| !n.is_empty()),
        ),
        None => (String::new(), None),
    };

    let in_reply_to = msg
        .in_reply_to()
        .as_text()
        .map(norm_mid)
        .filter(|s| !s.is_empty());
    let references = header_mids(msg.references());

    // Date header preferred; fall back to the server's INTERNALDATE, then 0.
    let sent_at = msg
        .date()
        .map(|d| d.to_timestamp())
        .or(internal)
        .unwrap_or(0);

    Some(IncomingMessage {
        message_id: msg.message_id().map(norm_mid).filter(|s| !s.is_empty()),
        in_reply_to,
        references,
        from_email,
        from_name,
        to_emails: address_emails(msg.to()),
        cc_emails: address_emails(msg.cc()),
        subject: msg.subject().map(|s| s.to_string()),
        body_text: msg.body_text(0).map(|b| b.into_owned()),
        sent_at,
        bulk_headers: has_bulk_headers(&msg),
    })
}

/// Bulk/automated markers (spec §10): mailing-list and auto-precedence headers.
/// Combined later with the no-reply address heuristic during ingest.
fn has_bulk_headers(msg: &mail_parser::Message) -> bool {
    if msg.header_raw("List-Id").is_some()
        || msg.header_raw("List-Unsubscribe").is_some()
        || msg.header_raw("Auto-Submitted").is_some()
    {
        return true;
    }
    matches!(
        msg.header_raw("Precedence").map(|p| p.trim().to_lowercase()),
        Some(p) if p == "bulk" || p == "list" || p == "junk"
    )
}
