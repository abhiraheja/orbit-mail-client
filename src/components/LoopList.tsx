import { useRef } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import type { LoopView } from "../lib/ipc";

// Virtualized list of open loops — non-negotiable so large inboxes render without
// dying (spec §4). Render-only: every field arrives display-ready from Rust.

const KIND_LABEL: Record<LoopView["kind"], string> = {
  waiting_on: "Waiting on",
  owe_reply: "Owe reply",
  promised: "Promised",
};

interface Props {
  loops: LoopView[];
  onOpen: (loop: LoopView) => void;
  onSnooze: (loop: LoopView) => void;
  onDismiss: (loop: LoopView) => void;
}

export function LoopList({ loops, onOpen, onSnooze, onDismiss }: Props) {
  const parentRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: loops.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 64,
    overscan: 8,
  });

  if (loops.length === 0) {
    return <p className="empty">No open loops. You're all caught up.</p>;
  }

  return (
    <div ref={parentRef} className="loop-scroll">
      <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
        {virtualizer.getVirtualItems().map((item) => {
          const loop = loops[item.index];
          return (
            <div
              key={loop.id}
              className="loop-row"
              style={{
                position: "absolute",
                top: 0,
                left: 0,
                width: "100%",
                height: item.size,
                transform: `translateY(${item.start}px)`,
              }}
            >
              <span className={`badge badge-${loop.kind}`}>{KIND_LABEL[loop.kind]}</span>
              <div className="loop-main" onClick={() => onOpen(loop)}>
                <div className="loop-contact">{loop.contact_name}</div>
                <div className="loop-subject">{loop.subject}</div>
              </div>
              <span className="loop-age">{loop.age}</span>
              <div className="loop-actions">
                <button type="button" onClick={() => onSnooze(loop)} title="Snooze 3 days">
                  Snooze
                </button>
                <button type="button" onClick={() => onDismiss(loop)} title="Dismiss">
                  Dismiss
                </button>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
