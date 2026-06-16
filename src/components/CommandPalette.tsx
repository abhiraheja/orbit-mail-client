// Ctrl+K command palette. Render + navigation only: keyword search is delegated
// to Rust (`search`), and the action callbacks are supplied by the host. This
// component holds no business logic — just input, debounce, and keyboard nav.

import { useCallback, useEffect, useRef, useState } from "react";
import { search, type SearchResult } from "../lib/ipc";

/** A static, app-supplied command (navigation, sync, …). */
export interface PaletteAction {
  id: string;
  label: string;
  hint?: string;
  run: () => void;
}

interface Props {
  open: boolean;
  onClose: () => void;
  actions: PaletteAction[];
  onOpenThread: (threadId: number) => void;
  onOpenContact: (email: string) => void;
}

/** One selectable row, normalized across actions and search hits. */
interface Item {
  key: string;
  label: string;
  hint: string;
  activate: () => void;
}

export function CommandPalette({ open, onClose, actions, onOpenThread, onOpenContact }: Props) {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SearchResult[]>([]);
  const [selected, setSelected] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  // Reset and focus whenever the palette opens.
  useEffect(() => {
    if (open) {
      setQuery("");
      setResults([]);
      setSelected(0);
      // Focus after the element is mounted/visible.
      requestAnimationFrame(() => inputRef.current?.focus());
    }
  }, [open]);

  // Debounced search against Rust. Blank query clears results.
  useEffect(() => {
    if (!open) return;
    const q = query.trim();
    if (q === "") {
      setResults([]);
      return;
    }
    const handle = setTimeout(() => {
      search(q)
        .then(setResults)
        .catch(() => setResults([]));
    }, 120);
    return () => clearTimeout(handle);
  }, [query, open]);

  // Build the flat, ordered item list: matching actions first, then search hits.
  const lower = query.trim().toLowerCase();
  const actionItems: Item[] = actions
    .filter((a) => lower === "" || a.label.toLowerCase().includes(lower))
    .map((a) => ({
      key: `action:${a.id}`,
      label: a.label,
      hint: a.hint ?? "Action",
      activate: () => {
        a.run();
        onClose();
      },
    }));
  const resultItems: Item[] = results.map((r, i) => ({
    key: `result:${r.kind}:${i}`,
    label: r.title,
    hint: r.subtitle,
    activate: () => {
      if (r.kind === "thread" && r.thread_id != null) onOpenThread(r.thread_id);
      else if (r.kind === "contact" && r.contact_email != null) onOpenContact(r.contact_email);
      onClose();
    },
  }));
  const items = [...actionItems, ...resultItems];

  // Keep the selected index in range as the list changes.
  const clampedSelected = items.length === 0 ? 0 : Math.min(selected, items.length - 1);

  const onKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      } else if (e.key === "ArrowDown") {
        e.preventDefault();
        setSelected((s) => (items.length === 0 ? 0 : (s + 1) % items.length));
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        setSelected((s) => (items.length === 0 ? 0 : (s - 1 + items.length) % items.length));
      } else if (e.key === "Enter") {
        e.preventDefault();
        items[clampedSelected]?.activate();
      }
    },
    [items, clampedSelected, onClose],
  );

  if (!open) return null;

  return (
    <div className="palette-overlay" onMouseDown={onClose}>
      <div className="palette" onMouseDown={(e) => e.stopPropagation()}>
        <input
          ref={inputRef}
          className="palette-input"
          placeholder="Search threads, contacts, or run a command…"
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
            setSelected(0);
          }}
          onKeyDown={onKeyDown}
        />
        <ul className="palette-list">
          {items.length === 0 && <li className="palette-empty">No matches</li>}
          {items.map((item, i) => (
            <li key={item.key}>
              <button
                type="button"
                className={i === clampedSelected ? "palette-item selected" : "palette-item"}
                onMouseEnter={() => setSelected(i)}
                onClick={item.activate}
              >
                <span className="palette-item-label">{item.label}</span>
                <span className="palette-item-hint">{item.hint}</span>
              </button>
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}
