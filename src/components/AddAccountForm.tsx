import { useState } from "react";
import { addAccount } from "../lib/ipc";

// Collects IMAP connection details. The password goes straight to Rust, which
// stores it in the OS keychain — it never touches the frontend's state beyond
// this form, and never the database (spec §7).

interface Props {
  onAdded: (accountId: number) => void;
}

export function AddAccountForm({ onAdded }: Props) {
  const [email, setEmail] = useState("");
  const [host, setHost] = useState("");
  const [port, setPort] = useState(993);
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      const acct = await addAccount({
        email,
        display_name: null,
        host,
        port,
        password,
      });
      setPassword("");
      onAdded(acct.id);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <form className="add-account" onSubmit={submit}>
      <h2>Connect an email account</h2>
      <p className="hint">Plain IMAP with an app password. Nothing leaves your machine except the mail fetch.</p>
      <input
        type="email"
        placeholder="you@example.com"
        value={email}
        onChange={(e) => setEmail(e.currentTarget.value)}
        required
      />
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
      <input
        type="password"
        placeholder="App password"
        value={password}
        onChange={(e) => setPassword(e.currentTarget.value)}
        required
      />
      <button type="submit" disabled={busy}>
        {busy ? "Connecting…" : "Connect"}
      </button>
      {error && <p className="error">{error}</p>}
    </form>
  );
}
