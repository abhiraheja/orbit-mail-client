// AI settings + the "what left my machine" transparency view. Render + form only:
// provider selection, key entry, and the audit log are all handled by Rust
// commands; this component holds no business logic.

import { useEffect, useState } from "react";
import {
  getAiStatus,
  setAiProvider,
  clearAiProvider,
  getAiAuditLog,
  type AiStatus,
  type AuditEntry,
} from "../lib/ipc";

interface Props {
  onClose: () => void;
}

/** Provider kinds and whether they run locally / need a key. Mirrors the Rust
 *  `AiConfig::defaults_for`; the actual endpoints are resolved in Rust. */
const KINDS: { value: string; label: string; local: boolean; custom?: boolean }[] = [
  { value: "openai", label: "OpenAI", local: false },
  { value: "openrouter", label: "OpenRouter", local: false },
  { value: "deepseek", label: "DeepSeek", local: false },
  { value: "ollama", label: "Ollama (local)", local: true },
  { value: "lmstudio", label: "LM Studio (local)", local: true },
  { value: "custom", label: "Custom (OpenAI-compatible)", local: false, custom: true },
];

function fmtTime(unix: number): string {
  return new Date(unix * 1000).toLocaleString();
}

export function SettingsPanel({ onClose }: Props) {
  const [status, setStatus] = useState<AiStatus | null>(null);
  const [audit, setAudit] = useState<AuditEntry[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  // Form state
  const [kind, setKind] = useState("openai");
  const [model, setModel] = useState("");
  const [baseUrl, setBaseUrl] = useState("");
  const [apiKey, setApiKey] = useState("");

  const selected = KINDS.find((k) => k.value === kind)!;

  function refresh() {
    getAiStatus().then(setStatus).catch(() => {});
    getAiAuditLog().then(setAudit).catch(() => {});
  }

  useEffect(refresh, []);

  async function handleSave(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    setSaving(true);
    try {
      const next = await setAiProvider({
        kind,
        model: model.trim(),
        base_url: baseUrl.trim() === "" ? null : baseUrl.trim(),
        api_key: apiKey.trim() === "" ? null : apiKey.trim(),
      });
      setStatus(next);
      setApiKey("");
    } catch (err) {
      setError(String(err));
    } finally {
      setSaving(false);
    }
  }

  async function handleClear() {
    setError(null);
    try {
      setStatus(await clearAiProvider());
    } catch (err) {
      setError(String(err));
    }
  }

  return (
    <div className="palette-overlay" onMouseDown={onClose}>
      <div className="settings" onMouseDown={(e) => e.stopPropagation()}>
        <header className="settings-head">
          <h2>Settings</h2>
          <button type="button" onClick={onClose}>
            Close
          </button>
        </header>

        <section className="settings-section">
          <h3>AI provider</h3>
          <p className="hint">
            Optional. Orbit works fully without AI — heuristics need no provider. Local
            providers (Ollama, LM Studio) keep everything on your machine.
          </p>

          <div className="ai-status">
            {status?.configured ? (
              <span>
                Active: <strong>{status.kind}</strong> · {status.model}{" "}
                {status.local ? (
                  <span className="badge badge-promised">local</span>
                ) : (
                  <span className="badge badge-owe_reply">cloud</span>
                )}
              </span>
            ) : (
              <span className="muted">No provider configured</span>
            )}
            {status?.configured && (
              <button type="button" onClick={handleClear}>
                Remove
              </button>
            )}
          </div>

          <form className="ai-form" onSubmit={handleSave}>
            <label>
              Provider
              <select value={kind} onChange={(e) => setKind(e.target.value)}>
                {KINDS.map((k) => (
                  <option key={k.value} value={k.value}>
                    {k.label}
                  </option>
                ))}
              </select>
            </label>

            <label>
              Model
              <input
                value={model}
                placeholder={selected.local ? "e.g. llama3.1" : "e.g. gpt-4o-mini"}
                onChange={(e) => setModel(e.target.value)}
              />
            </label>

            {(selected.custom || !selected.local) && (
              <label>
                Base URL {selected.custom ? "(required)" : "(optional override)"}
                <input
                  value={baseUrl}
                  placeholder="https://api.example.com/v1"
                  onChange={(e) => setBaseUrl(e.target.value)}
                />
              </label>
            )}

            {!selected.local && (
              <label>
                API key
                <input
                  type="password"
                  value={apiKey}
                  placeholder="stored in your OS keychain"
                  onChange={(e) => setApiKey(e.target.value)}
                />
              </label>
            )}

            {error && <p className="error">{error}</p>}
            <button type="submit" disabled={saving}>
              {saving ? "Saving…" : "Save provider"}
            </button>
          </form>
        </section>

        <section className="settings-section">
          <h3>What left my machine</h3>
          <p className="hint">
            Every AI request is logged here before it is sent — including failed ones.
          </p>
          {audit.length === 0 ? (
            <p className="muted">Nothing has been sent to an AI provider.</p>
          ) : (
            <ul className="audit-list">
              {audit.map((a) => (
                <li key={a.id} className="audit-row">
                  <div className="audit-meta">
                    <span className="audit-provider">{a.provider}</span>
                    {a.was_local ? (
                      <span className="badge badge-promised">local</span>
                    ) : (
                      <span className="badge badge-owe_reply">cloud</span>
                    )}
                    <span className="muted">{a.purpose}</span>
                    <span className="audit-time">{fmtTime(a.timestamp)}</span>
                  </div>
                  <div className="audit-summary">{a.data_summary}</div>
                </li>
              ))}
            </ul>
          )}
        </section>
      </div>
    </div>
  );
}
