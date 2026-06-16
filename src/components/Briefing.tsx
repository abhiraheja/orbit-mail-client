// The daily briefing card. Render-only: every string and count comes pre-computed
// from Rust (`get_daily_briefing`); this component does zero business logic.

import type { BriefingView, LoopView } from "../lib/ipc";

const KIND_LABEL: Record<LoopView["kind"], string> = {
  waiting_on: "Waiting on",
  owe_reply: "Owe reply",
  promised: "Promised",
};

interface Props {
  briefing: BriefingView;
  onOpen: (loop: LoopView) => void;
}

export function Briefing({ briefing, onOpen }: Props) {
  return (
    <section className="briefing">
      <div className="briefing-head">
        <p className="briefing-headline">{briefing.headline}</p>
        <span className="briefing-synced">
          {briefing.last_synced ? `Synced ${briefing.last_synced}` : "Not yet synced"}
        </span>
      </div>

      <div className="briefing-counts">
        <Count label="Owe reply" value={briefing.owe_reply} tone="owe_reply" />
        <Count label="Waiting on" value={briefing.waiting_on} tone="waiting_on" />
        <Count label="Promised" value={briefing.promised} tone="promised" />
      </div>

      {briefing.top_loops.length > 0 && (
        <ol className="briefing-top">
          {briefing.top_loops.map((loop) => (
            <li key={loop.id}>
              <button type="button" className="briefing-top-item" onClick={() => onOpen(loop)}>
                <span className={`badge badge-${loop.kind}`}>{KIND_LABEL[loop.kind]}</span>
                <span className="briefing-top-subject">{loop.subject}</span>
                <span className="briefing-top-contact">{loop.contact_name}</span>
                <span className="briefing-top-age">{loop.age}</span>
              </button>
            </li>
          ))}
        </ol>
      )}
    </section>
  );
}

function Count({ label, value, tone }: { label: string; value: number; tone: string }) {
  return (
    <div className={`briefing-count tone-${tone}`}>
      <span className="briefing-count-value">{value}</span>
      <span className="briefing-count-label">{label}</span>
    </div>
  );
}
