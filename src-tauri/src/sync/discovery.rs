//! Provider auto-detection from an email address (spec §8 — "enter your email,
//! we figure out the rest"). The frontend asks for nothing but the address; this
//! maps the domain to either an OAuth provider (Gmail / M365), a known IMAP host
//! with app-password auth, or a manual fallback.
//!
//! Domain-table first because it's instant, offline, and covers the long tail of
//! consumer mail. Network autodiscover (Thunderbird autoconfig / RFC 6186 SRV) is
//! a future enhancement layered behind the same `detect` entry point.

use serde::Serialize;

/// What the onboarding UI should do next for a given address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum AuthMethod {
    /// Launch the provider's OAuth consent flow automatically. No password.
    #[serde(rename = "oauth")]
    OAuth { provider: String },
    /// Known IMAP host; prompt only for an app password.
    Password { imap_host: String, imap_port: u16 },
    /// Unknown domain — ask for full IMAP details.
    Manual,
}

/// Display-ready detection result handed to the frontend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProviderHint {
    /// Human label, e.g. "Gmail", "Outlook / Microsoft 365", "Fastmail".
    pub label: String,
    #[serde(flatten)]
    pub method: AuthMethod,
}

/// Detect how to connect, from the address alone.
pub fn detect(email: &str) -> ProviderHint {
    let domain = email
        .rsplit_once('@')
        .map(|(_, d)| d.trim().to_lowercase())
        .unwrap_or_default();

    // OAuth providers: no password ever touches the app.
    match domain.as_str() {
        "gmail.com" | "googlemail.com" => {
            return ProviderHint { label: "Gmail".into(), method: AuthMethod::OAuth { provider: "gmail".into() } }
        }
        "outlook.com" | "hotmail.com" | "live.com" | "msn.com" | "passport.com" => {
            return ProviderHint {
                label: "Outlook / Microsoft 365".into(),
                method: AuthMethod::OAuth { provider: "m365".into() },
            }
        }
        _ => {}
    }

    // Known IMAP hosts that use app passwords.
    let known: Option<(&str, &str)> = match domain.as_str() {
        "yahoo.com" | "ymail.com" => Some(("Yahoo Mail", "imap.mail.yahoo.com")),
        "icloud.com" | "me.com" | "mac.com" => Some(("iCloud Mail", "imap.mail.me.com")),
        "fastmail.com" | "fastmail.fm" => Some(("Fastmail", "imap.fastmail.com")),
        "aol.com" => Some(("AOL Mail", "imap.aol.com")),
        "gmx.com" | "gmx.net" => Some(("GMX", "imap.gmx.com")),
        "zoho.com" => Some(("Zoho Mail", "imap.zoho.com")),
        "proton.me" | "protonmail.com" => Some(("Proton Mail (Bridge)", "127.0.0.1")),
        _ => None,
    };
    if let Some((label, host)) = known {
        return ProviderHint {
            label: label.into(),
            method: AuthMethod::Password { imap_host: host.into(), imap_port: 993 },
        };
    }

    ProviderHint { label: "IMAP".into(), method: AuthMethod::Manual }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gmail_uses_oauth() {
        assert_eq!(
            detect("a@gmail.com").method,
            AuthMethod::OAuth { provider: "gmail".into() }
        );
        // Case + the googlemail alias both resolve to Gmail OAuth.
        assert_eq!(
            detect("A@GoogleMail.com").method,
            AuthMethod::OAuth { provider: "gmail".into() }
        );
    }

    #[test]
    fn outlook_family_uses_m365_oauth() {
        for d in ["outlook.com", "hotmail.com", "live.com", "msn.com"] {
            assert_eq!(
                detect(&format!("a@{d}")).method,
                AuthMethod::OAuth { provider: "m365".into() },
                "{d} should be M365 OAuth"
            );
        }
    }

    #[test]
    fn known_imap_hosts_prefill() {
        assert_eq!(
            detect("a@fastmail.com").method,
            AuthMethod::Password { imap_host: "imap.fastmail.com".into(), imap_port: 993 }
        );
        assert_eq!(
            detect("a@icloud.com").method,
            AuthMethod::Password { imap_host: "imap.mail.me.com".into(), imap_port: 993 }
        );
    }

    #[test]
    fn unknown_domain_falls_back_to_manual() {
        assert_eq!(detect("a@acme-internal.example").method, AuthMethod::Manual);
    }
}
