import { useState } from "react";
import { detectSilences, errorText, isCancelled, newTaskId } from "../api";
import { TaskProgress } from "../components";
import { clock, span } from "../util";
import type { EditorApi } from "./state";

export default function CutPanel({ ed }: { ed: EditorApi }) {
  const [task, setTask] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [minSilence, setMinSilence] = useState(ed.pauses?.minSilence ?? 0.4);
  const [thresholdDb, setThresholdDb] = useState<number | null>(ed.pauses?.thresholdDb ?? null);

  const find = async (min = minSilence, threshold = thresholdDb) => {
    const id = newTaskId();
    setTask(id);
    setError(null);
    try {
      const report = await detectSilences(id, ed.media.path, threshold, min);
      ed.setPauses({ report, minSilence: min, thresholdDb: threshold });
      ed.setRemovePauses(true);
    } catch (e) {
      if (!isCancelled(e)) setError(errorText(e));
    } finally {
      setTask(null);
    }
  };

  const d = ed.media.duration;
  const out = ed.plan.outputDuration;
  const removed = d - out;
  const report = ed.pauses?.report;

  return (
    <div className="panel">
      <div className="card" style={{ marginBottom: 18 }}>
        <div className="stat-row">
          <span className="muted">Original</span>
          <span>{clock(d)}</span>
        </div>
        <div className="stat-row">
          <span className="muted">After cuts</span>
          <strong>{clock(out)}</strong>
        </div>
        <div className="stat-row">
          <span className="muted">Removed</span>
          <span>
            {span(removed)} {d > 0 && removed > 0.05 ? `(${((removed / d) * 100).toFixed(0)}%)` : ""}
          </span>
        </div>
      </div>

      <h3 className="section" style={{ marginTop: 0 }}>
        Pauses
      </h3>
      <p className="help">Find the silences between sentences and cut them out: a jump cut.</p>
      {!ed.media.hasAudio && <p className="faint">This file has no audio, so there are no pauses to find.</p>}
      {ed.media.hasAudio && !report && !task && (
        <button className="btn primary wide" onClick={() => find()}>
          Find pauses
        </button>
      )}
      <TaskProgress id={task} label="measuring loudness" />
      {error && <p className="error">{error}</p>}
      {report && (
        <>
          <label className="toggle" style={{ margin: "4px 0 14px" }}>
            <input type="checkbox" checked={ed.removePauses} onChange={(e) => ed.setRemovePauses(e.target.checked)} />
            <span>
              Remove {report.silences.length} pauses <span className="faint">({span(report.silences.reduce((s, r) => s + r.end - r.start, 0))})</span>
            </span>
          </label>
          <div className="field">
            <label>
              <span>Shortest pause to cut</span>
              <span>{minSilence.toFixed(2)} s</span>
            </label>
            <input
              type="range"
              min={0.15}
              max={2}
              step={0.05}
              value={minSilence}
              onChange={(e) => setMinSilence(Number(e.target.value))}
              onPointerUp={() => find(minSilence, thresholdDb)}
            />
          </div>
          <div className="field">
            <label>
              <span>Breathing room kept at each cut</span>
              <span>{ed.padding.toFixed(2)} s</span>
            </label>
            <input type="range" min={0} max={0.4} step={0.01} value={ed.padding} onChange={(e) => ed.setPadding(Number(e.target.value))} />
          </div>
          <div className="field">
            <label>
              <span>Silence is quieter than</span>
              <span>{thresholdDb === null ? `auto (${report.threshold_db.toFixed(0)} dB)` : `${thresholdDb} dB`}</span>
            </label>
            <input
              type="range"
              min={-70}
              max={-10}
              step={1}
              value={thresholdDb ?? Math.round(report.threshold_db)}
              onChange={(e) => setThresholdDb(Number(e.target.value))}
              onPointerUp={() => find(minSilence, thresholdDb)}
            />
            <span className="faint">
              Noise floor {report.noise_floor_db.toFixed(0)} dB, speech {report.speech_db.toFixed(0)} dB.{" "}
              {thresholdDb !== null && (
                <a
                  href="#"
                  onClick={(e) => {
                    e.preventDefault();
                    setThresholdDb(null);
                    find(minSilence, null);
                  }}
                >
                  Back to auto
                </a>
              )}
            </span>
          </div>
        </>
      )}

      <h3 className="section">Your cuts</h3>
      <p className="help">
        Drag across the timeline to select (drag its edges to adjust), then press <kbd>X</kbd>. <kbd>I</kbd> and <kbd>O</kbd> set the start
        and end at the playhead. Double-click the timeline or press <kbd>Esc</kbd> to clear the selection. Made a mistake? <kbd>Ctrl+Z</kbd>
        undoes it.
      </p>
      <button className="btn wide" disabled={!ed.cutSelection} onClick={() => ed.cutSelection?.()}>
        Cut the selection {ed.cutSelection ? `(${span(ed.selectionLength)})` : ""}
      </button>
      <div className="row" style={{ marginTop: 8 }}>
        <button
          className="btn small grow"
          disabled={ed.time < 0.05}
          title="Cut from the start up to the playhead (Q)"
          onClick={() => ed.setManualCuts([...ed.manualCuts, { start: 0, end: ed.time }])}
        >
          Cut before playhead <kbd>Q</kbd>
        </button>
        <button
          className="btn small grow"
          disabled={ed.time > d - 0.05}
          title="Cut from the playhead to the end (W)"
          onClick={() => ed.setManualCuts([...ed.manualCuts, { start: ed.time, end: d }])}
        >
          Cut after playhead <kbd>W</kbd>
        </button>
      </div>
      <button
        className="btn small wide"
        style={{ marginTop: 8 }}
        disabled={!ed.split}
        title="Make two clips of this one at the playhead (S), to use its parts in any order"
        onClick={() => ed.split?.()}
      >
        Split at playhead <kbd>S</kbd>
      </button>
      <p className="faint" style={{ margin: "6px 0 0" }}>
        Splitting makes two clips of the same video: drag them apart on the whole-video timeline, or split again for a third part.
      </p>
      <div style={{ marginTop: 10 }}>
        {ed.manualCuts.map((c, i) => (
          <div className="stat-row" key={i}>
            <a href="#" onClick={(e) => (e.preventDefault(), ed.seek(Math.max(0, c.start - 2), true))}>
              {clock(c.start, true)} – {clock(c.end, true)}
            </a>
            <button className="btn small ghost danger" onClick={() => ed.setManualCuts(ed.manualCuts.filter((_, j) => j !== i))}>
              Restore
            </button>
          </div>
        ))}
      </div>

      <label className="toggle" style={{ marginTop: 18 }}>
        <input type="checkbox" checked={ed.skipCuts} onChange={(e) => ed.setSkipCuts(e.target.checked)} />
        <span>Skip cut parts while previewing</span>
      </label>
    </div>
  );
}
