import { useState } from "react";
import {
  addAccount,
  detectAccount,
  startOAuthLogin,
  type ProviderHint,
} from "../lib/ipc";

// Email-first onboarding. You type your address; Rust detects the provider and we
// either launch OAuth automatically, ask only for an app password (host pre-filled),
// or fall back to a manual IMAP form. Passwords go straight to the Rust keychain.

interface Props {
  onAdded: (accountId: number) => void;
}

type Step = "email" | "password" | "manual";

/** Per-provider help for the app-password step (until OAuth client IDs ship). */
const APP_PASSWORD_HELP: Record<string, string> = {
  "imap.mail.yahoo.com": "Create an app password in Yahoo Account Security.",
  "imap.mail.me.com": "Create an app-specific password at appleid.apple.com.",
  "imap.gmail.com": "Enable 2-Step Verification, then create an App Password in your Google account.",
};

export function AddAccountForm({ onAdded }: Props) {
  const [step, setStep] = useState<Step>("email");
  const [email, setEmail] = useState("");
  const [hint, setHint] = useState<ProviderHint | null>(null);

  const [host, setHost] = useState("");
  const [port, setPort] = useState(993);
  const [password, setPassword] = useState("");

  const [busy, setBusy] = useState(false);
  const [statusText, setStatusText] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Step 1 → detect provider and branch.
  async function handleContinue(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    setBusy(true);
    try {
      const h = await detectAccount(email.trim());
      setHint(h);

      if (h.method === "oauth") {
        // Try the slick path: launch consent automatically.
        setStatusText(`Opening ${h.label} sign-in…`);
        try {
          const acct = await startOAuthLogin(h.provider);
          onAdded(acct.id);
          return;
        } catch (oauthErr) {
          // No client ID configured yet (or consent failed): fall back to an app
          // password if we know the host, otherwise manual.
          setError(`${h.label} sign-in unavailable: ${oauthErr}`);
          setStatusText(null);
          // Gmail still allows IMAP app passwords; offer that as the live path.
          if (h.provider === "gmail") {
            setHost("imap.gmail.com");
            setStep("password");
          } else {
            setStep("manual");
          }
          return;
        }
      }

      if (h.method === "password") {
        setHost(h.imap_host);
        setPort(h.imap_port);
        setStep("password");
        return;
      }

      setStep("manual");
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  }

  // Step 2/3 → create an IMAP account with an app password.
  async function handleConnect(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    setBusy(true);
    try {
      const acct = await addAccount({ email: email.trim(), display_name: null, host, port, password });
      setPassword("");
      onAdded(acct.id);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  }

  if (step === "email") {
    return (
      <form className="add-account" onSubmit={handleContinue}>
        <h2>Add your email</h2>
        <p className="hint">We'll detect your provider and sign you in the right way. Nothing leaves your machine except the mail fetch.</p>
        <input
          type="email"
          placeholder="you@example.com"
          value={email}
          onChange={(e) => setEmail(e.currentTarget.value)}
          autoFocus
          required
        />
        <button type="submit" disabled={busy}>
          {busy ? statusText ?? "Checking…" : "Continue"}
        </button>
        {error && <p className="error">{error}</p>}
      </form>
    );
  }

  // Password step (known provider) or manual step (unknown).
  const help = APP_PASSWORD_HELP[host];
  return (
    <form className="add-account" onSubmit={handleConnect}>
      <h2>{hint?.label ?? "IMAP"} · {email}</h2>
      {step === "manual" ? (
        <p className="hint">We couldn't auto-detect this provider — enter its IMAP server.</p>
      ) : (
        <p className="hint">{help ?? "Sign in with an app password."}</p>
      )}

      {step === "manual" && (
        <div className="row">
          <input
            type="text"
            placeholder="imap.example.com"
            value={host}
            onChange={(e) => setHost(e.currentTarget.value)}
            required
          />
          <input
            type="number"
            value={port}
            onChange={(e) => setPort(Number(e.currentTarget.value))}
            style={{ width: 90 }}
          />
        </div>
      )}

      <input
        type="password"
        placeholder="App password"
        value={password}
        onChange={(e) => setPassword(e.currentTarget.value)}
        autoFocus
        required
      />
      <button type="submit" disabled={busy}>
        {busy ? "Connecting…" : "Connect"}
      </button>
      <button type="button" className="link-button" onClick={() => { setStep("email"); setError(null); }}>
        ← Use a different email
      </button>
      {error && <p className="error">{error}</p>}
    </form>
  );
}
