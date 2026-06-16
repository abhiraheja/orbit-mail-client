import { useCallback, useEffect, useState } from "react";
import {
  listAccounts,
  listLoops,
  getDailyBriefing,
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
  type BriefingView,
} from "./lib/ipc";
import { LoopList } from "./components/LoopList";
import { Briefing } from "./components/Briefing";
import { CommandPalette, type PaletteAction } from "./components/CommandPalette";
import { SettingsPanel } from "./components/SettingsPanel";
import { DraftReply } from "./components/DraftReply";
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
  const [briefing, setBriefing] = useState<BriefingView | null>(null);
  const [filter, setFilter] = useState<Filter>("all");
  const [syncStatus, setSyncStatus] = useState<string | null>(null);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [draftLoop, setDraftLoop] = useState<LoopView | null>(null);

  const refreshLoops = useCallback((f: Filter) => {
    listLoops(f === "all" ? undefined : f)
      .then(setLoops)
      .catch((e) => setSyncStatus(`Error loading loops: ${e}`));
    getDailyBriefing().then(setBriefing).catch(() => {});
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

  // Global Ctrl/Cmd+K toggles the command palette.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setPaletteOpen((o) => !o);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

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

  function handleOpenThread(threadId: number) {
    // Thread context view lands next; for now surface the selection.
    setSyncStatus(`Thread #${threadId}`);
  }

  function handleOpen(loop: LoopView) {
    handleOpenThread(loop.thread_id);
  }

  function handleOpenContact(email: string) {
    setSyncStatus(`Contact: ${email}`);
  }

  // Static, app-owned palette commands (navigation + sync). Pure UI intent.
  const paletteActions: PaletteAction[] = [
    { id: "sync", label: "Sync all accounts", hint: "Command", run: () => void handleSyncAll() },
    { id: "settings", label: "Open settings (AI & privacy)", hint: "Command", run: () => setSettingsOpen(true) },
    { id: "all", label: "Show: All loops", hint: "View", run: () => setFilter("all") },
    { id: "owe", label: "Show: Owe reply", hint: "View", run: () => setFilter("owe_reply") },
    { id: "waiting", label: "Show: Waiting on", hint: "View", run: () => setFilter("waiting_on") },
    { id: "promised", label: "Show: Promised", hint: "View", run: () => setFilter("promised") },
  ];

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
          <button type="button" className="palette-hint" onClick={() => setPaletteOpen(true)}>
            Search <kbd>Ctrl K</kbd>
          </button>
          <button type="button" onClick={() => setSettingsOpen(true)}>
            Settings
          </button>
          <button type="button" onClick={handleSyncAll}>
            Sync
          </button>
        </div>
      </header>

      {briefing && <Briefing briefing={briefing} onOpen={handleOpen} />}

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
        onDraft={(loop) => setDraftLoop(loop)}
      />

      <CommandPalette
        open={paletteOpen}
        onClose={() => setPaletteOpen(false)}
        actions={paletteActions}
        onOpenThread={handleOpenThread}
        onOpenContact={handleOpenContact}
      />

      {settingsOpen && (
        <SettingsPanel
          onClose={() => setSettingsOpen(false)}
          onAccountsChanged={() => {
            listAccounts().then(setAccounts).catch(() => {});
            refreshLoops(filter);
          }}
        />
      )}
      {draftLoop && <DraftReply loop={draftLoop} onClose={() => setDraftLoop(null)} />}
    </main>
  );
}

export default App;
