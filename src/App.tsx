import { useCallback, useEffect, useState } from "react";
import {
  listAccounts,
  listLoops,
  syncAccount,
  snoozeLoop,
  dismissLoop,
  onLoopsUpdated,
  onSyncProgress,
  onSyncComplete,
  onSyncError,
  type Account,
  type LoopView,
  type LoopKind,
} from "./lib/ipc";
import { LoopList } from "./components/LoopList";
import { AddAccountForm } from "./components/AddAccountForm";
import "./App.css";

type Filter = "all" | LoopKind;
const FILTERS: { key: Filter; label: string }[] = [
  { key: "all", label: "All" },
  { key: "waiting_on", label: "Waiting on" },
  { key: "owe_reply", label: "Owe reply" },
  { key: "promised", label: "Promised" },
];

const THREE_DAYS = 3 * 24 * 60 * 60;

function App() {
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [loops, setLoops] = useState<LoopView[]>([]);
  const [filter, setFilter] = useState<Filter>("all");
  const [syncStatus, setSyncStatus] = useState<string | null>(null);

  const refreshLoops = useCallback((f: Filter) => {
    listLoops(f === "all" ? undefined : f)
      .then(setLoops)
      .catch((e) => setSyncStatus(`Error loading loops: ${e}`));
  }, []);

  // Initial load + live event wiring (Rust pushes; the UI only re-reads).
  useEffect(() => {
    listAccounts().then(setAccounts).catch(() => {});
    refreshLoops(filter);

    const unlisteners = [
      onLoopsUpdated(() => refreshLoops(filter)),
      onSyncProgress((p) => setSyncStatus(`Syncing… ${p.done} / ${p.total}`)),
      onSyncComplete((p) => {
        setSyncStatus(`Synced ${p.new_messages} new messages`);
        listAccounts().then(setAccounts).catch(() => {});
      }),
      onSyncError((p) => setSyncStatus(`Sync error: ${p.message}`)),
    ];
    return () => {
      unlisteners.forEach((u) => u.then((fn) => fn()));
    };
  }, [filter, refreshLoops]);

  async function handleSyncAll() {
    setSyncStatus("Starting sync…");
    for (const a of accounts) await syncAccount(a.id);
  }

  async function handleSnooze(loop: LoopView) {
    await snoozeLoop(loop.id, Math.floor(Date.now() / 1000) + THREE_DAYS);
    refreshLoops(filter);
  }

  async function handleDismiss(loop: LoopView) {
    await dismissLoop(loop.id);
    refreshLoops(filter);
  }

  function handleOpen(loop: LoopView) {
    // Thread context view lands next; for now surface the selection.
    setSyncStatus(`Thread: ${loop.subject}`);
  }

  if (accounts.length === 0) {
    return (
      <main className="container onboarding">
        <h1>Orbit</h1>
        <AddAccountForm
          onAdded={(id) => {
            listAccounts().then(setAccounts).catch(() => {});
            syncAccount(id);
          }}
        />
      </main>
    );
  }

  return (
    <main className="container">
      <header className="topbar">
        <h1>Orbit</h1>
        <div className="topbar-right">
          {syncStatus && <span className="status">{syncStatus}</span>}
          <button type="button" onClick={handleSyncAll}>
            Sync
          </button>
        </div>
      </header>

      <nav className="tabs">
        {FILTERS.map((f) => (
          <button
            key={f.key}
            className={filter === f.key ? "tab active" : "tab"}
            onClick={() => setFilter(f.key)}
          >
            {f.label}
          </button>
        ))}
      </nav>

      <LoopList
        loops={loops}
        onOpen={handleOpen}
        onSnooze={handleSnooze}
        onDismiss={handleDismiss}
      />
    </main>
  );
}

export default App;
