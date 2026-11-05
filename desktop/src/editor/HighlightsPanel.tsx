import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { errorText, findHighlights, isCancelled, newTaskId, type Highlight } from "../api";
import { TaskProgress } from "../components";
import { clock, dirName, span } from "../util";
import type { EditorApi } from "./state";
import { exportClips } from "./exporting";

const METERS: [keyof Highlight["breakdown"], string][] = [
  ["loudness", "loud"],
  ["activity", "lively"],
  ["motion", "motion"],
  ["pace", "cuts"],
];

export default function HighlightsPanel({ ed }: { ed: EditorApi }) {
  const [count, setCount] = useState(3);
  const [length, setLength] = useState(30);
  const [inSelection, setInSelection] = useState(false);
  const [task, setTask] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [exportState, setExportState] = useState<string | null>(null);

  const find = async () => {
    const id = newTaskId();
    setTask(id);
    setError(null);
    const within = inSelection && ed.selection ? ed.selection : null;
    try {
      ed.setHighlights(await findHighlights(id, ed.media.path, count, length, within?.start ?? null, within?.end ?? null));
    } catch (e) {
      if (!isCancelled(e)) setError(errorText(e));
    } finally {
      setTask(null);
    }
  };

  const exportAll = async (picks: Highlight[]) => {
    const dir = await open({ directory: true, defaultPath: dirName(ed.media.path) });
    if (typeof dir !== "string") return;
    setError(null);
    try {
      await exportClips(ed, picks, dir, setExportState);
    } catch (e) {
      if (!isCancelled(e)) setError(errorText(e));
    } finally {
      setExportState(null);
    }
  };

  return (
    <div className="panel">
      <p className="help">
        Finds the liveliest stretches: loud, busy, moving, fast-cut. It measures activity, not meaning, so check the picks. Exports use the
        Frame and caption settings.
      </p>
      <div className="row" style={{ marginBottom: 12 }}>
        <div className="field grow" style={{ marginBottom: 0 }}>
          <label>How many</label>
          <input className="input" type="number" min={1} max={20} value={count} onChange={(e) => setCount(Number(e.target.value))} />
        </div>
        <div className="field grow" style={{ marginBottom: 0 }}>
          <label>Seconds each</label>
          <input className="input" type="number" min={3} max={300} value={length} onChange={(e) => setLength(Number(e.target.value))} />
        </div>
      </div>
      <label className="toggle" style={{ marginBottom: 14, opacity: ed.selection ? 1 : 0.5 }}>
        <input type="checkbox" disabled={!ed.selection} checked={inSelection && !!ed.selection} onChange={(e) => setInSelection(e.target.checked)} />
        <span>Only search the selection {ed.selection ? `(${clock(ed.selection.start)}–${clock(ed.selection.end)})` : ""}</span>
      </label>
      {!task && (
        <button className="btn primary wide" onClick={find}>
          {ed.highlights ? "Find again" : "Find highlights"}
        </button>
      )}
      <TaskProgress id={task} label="measuring" />
      {!task && !ed.highlights && <p className="faint">Measuring takes about a minute per 10 minutes of video, once; after that it's instant.</p>}
      {error && <p className="error">{error}</p>}

      {ed.highlights && ed.highlights.length > 0 && (
        <>
          <div className="row" style={{ margin: "18px 0 10px" }}>
            <span className="grow muted">{ed.highlights.length} picks</span>
            <button className="btn small" disabled={!!exportState} onClick={() => exportAll(ed.highlights!)}>
              Export all as clips
            </button>
          </div>
          {exportState && (
            <div className="task-inline">
              <div className="row">
                <div className="spinner" />
                {exportState}
              </div>
            </div>
          )}
          {ed.highlights.map((h, i) => (
            <div
              key={i}
              className="hl-item"
              onClick={() => {
                ed.setSelection(h.range);
                ed.seek(h.range.start, true);
              }}
            >
              <div className="row">
                <strong>#{i + 1}</strong>
                <span className="grow">
                  {clock(h.range.start)} – {clock(h.range.end)} <span className="faint">({span(h.range.end - h.range.start)})</span>
                </span>
                <span className="muted">{(h.score * 100).toFixed(0)}</span>
                <button
                  className="btn small"
                  disabled={!!exportState}
                  onClick={(e) => {
                    e.stopPropagation();
                    exportAll([h]);
                  }}
                >
                  Export
                </button>
              </div>
              <div className="meters">
                {METERS.map(([key, label]) => {
                  const v = h.breakdown[key];
                  return (
                    <div key={key} style={{ opacity: v === null ? 0.35 : 1 }}>
                      <div className="bar" style={{ marginBottom: 3 }}>
                        <div style={{ width: `${(v ?? 0) * 100}%` }} />
                      </div>
                      {label}
                    </div>
                  );
                })}
              </div>
            </div>
          ))}
        </>
      )}
    </div>
  );
}
