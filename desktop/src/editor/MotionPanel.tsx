// Zoom and pan for the clip on screen: presets, and keyframes to fine-tune.

import { useState } from "react";
import type { Keyframe } from "../api";
import { clock } from "../util";
import { motionAt, type EditorApi } from "./state";

export default function MotionPanel({ ed }: { ed: EditorApi }) {
  const [selected, setSelected] = useState<number | null>(null);
  if (!ed.media.hasVideo) return null;

  const keys = [...ed.motion].sort((a, b) => a.time - b.time);
  const keep = ed.spec.keep;
  const start = keep[0]?.start ?? 0;
  const end = keep[keep.length - 1]?.end ?? ed.media.duration;
  const set = (next: Keyframe[]) => ed.setMotion([...next].sort((a, b) => a.time - b.time));
  const k = (time: number, zoom: number, x = 0.5, y = 0.5): Keyframe => ({ time, zoom, x, y });
  const presets: [string, Keyframe[]][] = [
    ["Still", []],
    ["Zoom in", [k(start, 1), k(end, 1.25)]],
    ["Zoom out", [k(start, 1.25), k(end, 1)]],
    ["Punch in", [k(start, 1.4)]],
    ["Pan across", [k(start, 1.3, 0.3), k(end, 1.3, 0.7)]],
  ];
  const update = (i: number, change: Partial<Keyframe>) => set(keys.map((key, j) => (j === i ? { ...key, ...change } : key)));

  return (
    <div style={{ marginTop: 8 }}>
      <h3 className="section">Zoom & pan · this clip</h3>
      <div className="chips" style={{ marginBottom: 12 }}>
        {presets.map(([label, next]) => (
          <button
            key={label}
            className="chip"
            onClick={() => {
              set(next);
              setSelected(null);
            }}
          >
            {label}
          </button>
        ))}
      </div>
      <button
        className="btn wide"
        onClick={() => {
          // A keyframe where the view is now, so adding one changes nothing
          // until it's edited.
          const now = motionAt(keys, ed.time);
          const next = [...keys.filter((key) => Math.abs(key.time - ed.time) > 0.05), k(ed.time, now.zoom, now.x, now.y)].sort((a, b) => a.time - b.time);
          set(next);
          setSelected(next.findIndex((key) => key.time === ed.time));
        }}
      >
        Add a keyframe at {clock(ed.time, true)}
      </button>
      {keys.map((key, i) => (
        <div key={`${key.time}-${i}`} className="hl-item" style={{ marginTop: 8, cursor: "default", borderColor: selected === i ? "var(--accent)" : undefined }}>
          <div className="row" style={{ cursor: "pointer" }} onClick={() => setSelected(selected === i ? null : i)}>
            <span className="mono">◆</span>
            <a
              href="#"
              className="grow"
              onClick={(e) => {
                e.preventDefault();
                e.stopPropagation();
                ed.seek(key.time);
                setSelected(i);
              }}
            >
              {clock(key.time, true)}
            </a>
            <span className="faint">
              {key.zoom.toFixed(2)}× at {Math.round(key.x * 100)}%, {Math.round(key.y * 100)}%
            </span>
          </div>
          {selected === i && (
            <div style={{ marginTop: 10 }}>
              {(
                [
                  ["Zoom", "zoom", 1, 3, 0.01],
                  ["Across", "x", 0, 1, 0.01],
                  ["Down", "y", 0, 1, 0.01],
                ] as const
              ).map(([label, field, min, max, step]) => (
                <div className="field" key={field} style={{ marginBottom: 8 }}>
                  <label>
                    <span>{label}</span>
                    <span>{field === "zoom" ? `${key.zoom.toFixed(2)}×` : `${Math.round(key[field] * 100)}%`}</span>
                  </label>
                  <input
                    type="range"
                    min={min}
                    max={max}
                    step={step}
                    value={key[field]}
                    onChange={(e) => {
                      update(i, { [field]: Number(e.target.value) });
                      ed.seek(key.time);
                    }}
                  />
                </div>
              ))}
              <div className="row">
                <button className="btn small" onClick={() => update(i, { time: ed.time })}>
                  Move to playhead
                </button>
                <button
                  className="btn small ghost danger"
                  onClick={() => {
                    set(keys.filter((_, j) => j !== i));
                    setSelected(null);
                  }}
                >
                  Delete
                </button>
              </div>
            </div>
          )}
        </div>
      ))}
      {keys.length > 0 && <p className="faint" style={{ marginTop: 10 }}>The view eases between keyframes and holds before the first and after the last.</p>}
    </div>
  );
}
