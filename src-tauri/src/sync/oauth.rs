//! OAuth 2.0 (Authorization Code + PKCE) for Gmail and Microsoft 365, plus the
//! XOAUTH2 SASL wiring so the IMAP layer can authenticate with a bearer token
//! instead of a password (spec §8).
//!
//! Flow: spin up a loopback redirect listener → open the provider's consent page
//! in the browser → capture the `code` → exchange it (with the PKCE verifier) for
//! access + refresh tokens → fetch the account email. The refresh token is stored
//! in the OS keychain; access tokens are short-lived and refreshed per sync.
//!
//! UNVERIFIED: this needs the user's own OAuth client_id (Google/Azure app
//! registration) and has not been run end-to-end. The pure pieces (PKCE, the auth
//! URL, the XOAUTH2 string) are unit-tested; the live exchange is not.

use std::collections::HashMap;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::Rng;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::error::{AppError, Result};

/// A provider's OAuth + IMAP endpoints.
#[derive(Debug, Clone)]
pub struct OAuthProviderDef {
    pub kind: &'static str,
    pub auth_url: &'static str,
    pub token_url: &'static str,
    /// Space-separated scopes, including offline access and the IMAP scope.
    pub scopes: &'static str,
    /// Endpoint returning the account's email for the bearer token.
    pub userinfo_url: &'static str,
    pub imap_host: &'static str,
    pub imap_port: u16,
}

pub fn provider_def(kind: &str) -> Option<OAuthProviderDef> {
    match kind {
        "gmail" => Some(OAuthProviderDef {
            kind: "gmail",
            auth_url: "https://accounts.google.com/o/oauth2/v2/auth",
            token_url: "https://oauth2.googleapis.com/token",
            scopes: "https://mail.google.com/ email openid",
            userinfo_url: "https://www.googleapis.com/oauth2/v2/userinfo",
            imap_host: "imap.gmail.com",
            imap_port: 993,
        }),
        "m365" => Some(OAuthProviderDef {
            kind: "m365",
            auth_url: "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
            token_url: "https://login.microsoftonline.com/common/oauth2/v2.0/token",
            scopes: "https://outlook.office365.com/IMAP.AccessAsUser.All offline_access openid email",
            userinfo_url: "https://graph.microsoft.com/v1.0/me",
            imap_host: "outlook.office365.com",
            imap_port: 993,
        }),
        _ => None,
    }
}

/// PKCE pair (RFC 7636). The verifier is kept secret; the S256 challenge is sent
/// in the authorization request.
#[derive(Debug, Clone)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl Pkce {
    pub fn generate() -> Self {
        let verifier = random_token(64);
        let digest = Sha256::digest(verifier.as_bytes());
        let challenge = URL_SAFE_NO_PAD.encode(digest);
        Self { verifier, challenge }
    }
}

/// A URL-safe random token (used for the PKCE verifier and the CSRF `state`).
pub fn random_token(len: usize) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
    let mut rng = rand::thread_rng();
    (0..len).map(|_| CHARS[rng.gen_range(0..CHARS.len())] as char).collect()
}

/// Build the authorization URL the user's browser is sent to.
pub fn authorization_url(
    def: &OAuthProviderDef,
    client_id: &str,
    redirect_uri: &str,
    state: &str,
    pkce: &Pkce,
) -> String {
    let q = |s: &str| urlencoding(s);
    format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}\
         &code_challenge={}&code_challenge_method=S256&access_type=offline&prompt=consent",
        def.auth_url,
        q(client_id),
        q(redirect_uri),
        q(def.scopes),
        q(state),
        q(&pkce.challenge),
    )
}

/// Tokens returned by the token endpoint.
#[derive(Debug, Clone, Deserialize)]
pub struct OAuthTokens {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_in: Option<i64>,
}

/// Exchange an authorization code for tokens (PKCE; client_secret optional for
/// public clients).
pub async fn exchange_code(
    def: &OAuthProviderDef,
    client_id: &str,
    client_secret: Option<&str>,
    code: &str,
    redirect_uri: &str,
    verifier: &str,
) -> Result<OAuthTokens> {
    let mut form = vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("client_id", client_id),
        ("code_verifier", verifier),
    ];
    if let Some(secret) = client_secret {
        form.push(("client_secret", secret));
    }
    post_token(def.token_url, &form).await
}

/// Trade a refresh token for a fresh access token (called before each sync).
pub async fn refresh_access_token(
    def: &OAuthProviderDef,
    client_id: &str,
    client_secret: Option<&str>,
    refresh_token: &str,
) -> Result<OAuthTokens> {
    let mut form = vec![
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", client_id),
    ];
    if let Some(secret) = client_secret {
        form.push(("client_secret", secret));
    }
    post_token(def.token_url, &form).await
}

async fn post_token(token_url: &str, form: &[(&str, &str)]) -> Result<OAuthTokens> {
    let resp = reqwest::Client::new()
        .post(token_url)
        .form(form)
        .send()
        .await
        .map_err(|e| AppError::Sync(format!("token request: {e}")))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(AppError::Sync(format!("token endpoint {status}: {body}")));
    }
    resp.json::<OAuthTokens>()
        .await
        .map_err(|e| AppError::Sync(format!("token parse: {e}")))
}

/// Fetch the account email for a bearer token (needed to form the XOAUTH2 string).
pub async fn fetch_email(def: &OAuthProviderDef, access_token: &str) -> Result<String> {
    let v: serde_json::Value = reqwest::Client::new()
        .get(def.userinfo_url)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| AppError::Sync(format!("userinfo: {e}")))?
        .json()
        .await
        .map_err(|e| AppError::Sync(format!("userinfo parse: {e}")))?;
    // Google returns `email`; Graph returns `mail` or `userPrincipalName`.
    let email = v["email"]
        .as_str()
        .or_else(|| v["mail"].as_str())
        .or_else(|| v["userPrincipalName"].as_str())
        .map(|s| s.to_lowercase());
    email.ok_or_else(|| AppError::Sync("could not determine account email".into()))
}

/// The XOAUTH2 SASL initial response: `user=<email>^Aauth=Bearer <token>^A^A`,
/// where `^A` is 0x01. async-imap base64-encodes the bytes we return.
pub fn xoauth2_string(email: &str, access_token: &str) -> String {
    format!("user={email}\x01auth=Bearer {access_token}\x01\x01")
}

/// async-imap [`Authenticator`](async_imap::Authenticator) for XOAUTH2. The
/// server's initial challenge is empty; we answer with the SASL string.
pub struct XOAuth2 {
    pub user: String,
    pub access_token: String,
}

impl async_imap::Authenticator for &XOAuth2 {
    type Response = String;
    fn process(&mut self, _challenge: &[u8]) -> Self::Response {
        xoauth2_string(&self.user, &self.access_token)
    }
}

/// Bind a loopback listener on an ephemeral port and return it with the matching
/// `redirect_uri`. The provider redirects the browser here after consent.
pub async fn start_loopback() -> Result<(TcpListener, String)> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| AppError::Sync(format!("loopback bind: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| AppError::Sync(e.to_string()))?
        .port();
    Ok((listener, format!("http://127.0.0.1:{port}")))
}

/// Wait for the single OAuth redirect, parse its query params, and answer the
/// browser with a "you can close this" page.
pub async fn await_redirect(listener: TcpListener) -> Result<HashMap<String, String>> {
    let (mut socket, _) = listener
        .accept()
        .await
        .map_err(|e| AppError::Sync(format!("loopback accept: {e}")))?;

    let mut buf = vec![0u8; 8192];
    let n = socket
        .read(&mut buf)
        .await
        .map_err(|e| AppError::Sync(format!("loopback read: {e}")))?;
    let req = String::from_utf8_lossy(&buf[..n]);

    // Request line: "GET /?code=...&state=... HTTP/1.1"
    let target = req.lines().next().and_then(|l| l.split_whitespace().nth(1)).unwrap_or("");
    let query = target.split_once('?').map(|(_, q)| q).unwrap_or("");
    let params = parse_query(query);

    let body = "<!doctype html><html><body style=\"font-family:system-ui;background:#0f1115;\
        color:#e6e8ec;text-align:center;padding-top:18vh\"><h2>Orbit</h2>\
        <p>Signed in. You can close this window and return to Orbit.</p></body></html>";
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = socket.write_all(resp.as_bytes()).await;
    Ok(params)
}

fn parse_query(q: &str) -> HashMap<String, String> {
    q.split('&')
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| (k.to_string(), urldecode(v)))
        .collect()
}

/// Decode `%XX` and `+` in a query value.
fn urldecode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    out.push(b);
                    i += 3;
                    continue;
                }
                out.push(b'%');
                i += 1;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Minimal percent-encoding for query-string values (RFC 3986 unreserved set is
/// left as-is; everything else is `%XX`).
fn urlencoding(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_is_s256_base64url() {
        let pkce = Pkce::generate();
        // Recompute the expected challenge from the verifier.
        let expected = URL_SAFE_NO_PAD.encode(Sha256::digest(pkce.verifier.as_bytes()));
        assert_eq!(pkce.challenge, expected);
        // base64url-no-pad: no '+', '/', or '='.
        assert!(!pkce.challenge.contains(['+', '/', '=']));
        assert!(pkce.verifier.len() >= 43, "RFC 7636 minimum verifier length");
    }

    #[test]
    fn random_tokens_are_unique() {
        assert_ne!(random_token(32), random_token(32));
    }

    #[test]
    fn xoauth2_has_control_separators() {
        let s = xoauth2_string("me@gmail.com", "ya29.tok");
        assert_eq!(s, "user=me@gmail.com\x01auth=Bearer ya29.tok\x01\x01");
    }

    #[test]
    fn auth_url_encodes_params() {
        let def = provider_def("gmail").unwrap();
        let pkce = Pkce { verifier: "v".into(), challenge: "chal".into() };
        let url = authorization_url(&def, "client.123", "http://127.0.0.1:9000", "st&ate", &pkce);
        assert!(url.contains("client_id=client.123"));
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A9000"));
        assert!(url.contains("code_challenge=chal"));
        assert!(url.contains("code_challenge_method=S256"));
        // Scope's spaces and the IMAP scope's slashes/colon are encoded.
        assert!(url.contains("mail.google.com"));
        assert!(url.contains("state=st%26ate"));
    }

    #[test]
    fn unknown_provider_is_none() {
        assert!(provider_def("yahoo").is_none());
    }
}
