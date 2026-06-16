import { useEffect, useState } from "react";
import { ping, emitTestEvent, onLoopsUpdated } from "./lib/ipc";
import "./App.css";

// Phase 0 bridge smoke-test screen: proves a command returns a value and a
// Rust-pushed event arrives. The real loops screen replaces this in Phase 1d.
function App() {
  const [schemaVersion, setSchemaVersion] = useState<number | null>(null);
  const [eventCount, setEventCount] = useState(0);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    ping()
      .then(setSchemaVersion)
      .catch((e) => setError(String(e)));

    const unlisten = onLoopsUpdated(() => setEventCount((n) => n + 1));
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  return (
    <main className="container">
      <h1>Orbit</h1>
      <p>Local-first Communication OS — Phase 0 bridge check</p>

      {error && <p style={{ color: "crimson" }}>Error: {error}</p>}

      <p>
        DB schema version:{" "}
        <strong>{schemaVersion === null ? "…" : schemaVersion}</strong>{" "}
        (command round-trip)
      </p>

      <p>
        Events received: <strong>{eventCount}</strong>
      </p>

      <button type="button" onClick={() => emitTestEvent()}>
        Trigger Rust event
      </button>
    </main>
  );
}

export default App;
