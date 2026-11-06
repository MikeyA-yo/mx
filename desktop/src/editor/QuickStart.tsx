// The quick-start checklist: the usual steps of an edit, ticked off as
// they're done, each opening the tab that does it. Help hides and shows it.

import { load, store } from "../util";
import { showChecklist, useGuide } from "../tour";
import { useState } from "react";

export type Step = { tab: string; label: string; hint: string; done: boolean };

export default function QuickStart({ steps, tab, onTab }: { steps: Step[]; tab: string; onTab: (tab: string) => void }) {
  const { checklist } = useGuide();
  const [open, setOpen] = useState(load("guide.checklistOpen", true));
  if (!checklist) return null;
  const done = steps.filter((s) => s.done).length;
  const next = steps.find((s) => !s.done);
  const toggle = () => {
    store("guide.checklistOpen", !open);
    setOpen(!open);
  };
  return (
    <div className="quick-start" data-tour="checklist">
      <div className="row qs-head" onClick={toggle}>
        <strong className={open || !next ? "grow" : undefined}>{next ? "Quick start" : "All done. Nice work!"}</strong>
        {open && next && <span className="faint">every step is optional ·</span>}
        {!open && next && (
          <a
            href="#"
            className="grow ellipsis"
            onClick={(e) => {
              e.preventDefault();
              e.stopPropagation();
              onTab(next.tab);
            }}
          >
            next: {next.label.toLowerCase()}
          </a>
        )}
        <span className="faint">
          {done} of {steps.length}
        </span>
        <span className="faint">{open ? "▾" : "▸"}</span>
        <button
          className="qs-close"
          title="Hide the quick start (Help brings it back)"
          onClick={(e) => {
            e.stopPropagation();
            showChecklist(false);
          }}
        >
          ✕
        </button>
      </div>
      {open &&
        steps.map((s, i) => (
          <button key={s.tab} className={`qs-step${s.done ? " done" : ""}${tab === s.tab ? " here" : ""}`} onClick={() => onTab(s.tab)}>
            <span className="qs-mark">{s.done ? "✓" : i + 1}</span>
            <span className="grow">
              {s.label}
              <span className="faint"> · {s.hint}</span>
            </span>
          </button>
        ))}
    </div>
  );
}
