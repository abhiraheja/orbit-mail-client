// Streaming AI draft-reply modal. The draft is produced entirely in Rust (which
// also writes the audit row before sending); this component only collects an
// instruction, kicks off `draftReply`, and renders tokens as they arrive.

import { useEffect, useRef, useState } from "react";
import { draftReply, onAiToken, onAiDone, type LoopView } from "../lib/ipc";

interface Props {
  loop: LoopView;
  onClose: () => void;
}

export function DraftReply({ loop, onClose }: Props) {
  const [instructions, setInstructions] = useState("");
  const [draft, setDraft] = useState("");
  const [streaming, setStreaming] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // The request id we're currently listening for, so stale streams are ignored.
  const requestId = useRef<string | null>(null);

  // Subscribe once; filter events by the active request id.
  useEffect(() => {
    const unsubs = [
      onAiToken((p) => {
        if (p.request_id === requestId.current) setDraft((d) => d + p.token);
      }),
      onAiDone((p) => {
        if (p.request_id === requestId.current) setStreaming(false);
      }),
    ];
    return () => unsubs.forEach((u) => u.then((fn) => fn()));
  }, []);

  async function handleGenerate() {
    setError(null);
    setDraft("");
    setStreaming(true);
    try {
      requestId.current = await draftReply(loop.thread_id, instructions.trim());
    } catch (err) {
      setError(String(err));
      setStreaming(false);
    }
  }

  return (
    <div className="palette-overlay" onMouseDown={onClose}>
      <div className="draft" onMouseDown={(e) => e.stopPropagation()}>
        <header className="settings-head">
          <h2>Draft reply</h2>
          <button type="button" onClick={onClose}>
            Close
          </button>
        </header>

        <p className="hint">
          {loop.subject} · {loop.contact_name}
        </p>

        <label className="draft-field">
          Instructions (optional)
          <input
            value={instructions}
            placeholder="e.g. politely decline, suggest next week"
            onChange={(e) => setInstructions(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !streaming) handleGenerate();
            }}
          />
        </label>

        <button type="button" onClick={handleGenerate} disabled={streaming}>
          {streaming ? "Generating…" : draft ? "Regenerate" : "Generate"}
        </button>

        {error && <p className="error">{error}</p>}

        {draft && (
          <textarea
            className="draft-output"
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            rows={10}
          />
        )}
      </div>
    </div>
  );
}
