import type { Title } from "../api";
import { clock } from "../util";
import { useOpenCard, type EditorApi } from "./state";

const FONTS = ["Arial", "Impact", "Segoe UI", "Verdana", "Georgia", "Trebuchet MS", "Comic Sans MS"];
const COLORS = ["#ffffff", "#ffd400", "#ff4d6d", "#4dd9ff", "#7cff6b", "#111111"];

export default function TitlesPanel({ ed }: { ed: EditorApi }) {
  useOpenCard(ed.selectedTitle === null ? null : `title-${ed.selectedTitle}`);
  if (!ed.media.hasVideo) return null;
  const titles = ed.titles;
  const update = (i: number, change: Partial<Title>) => ed.setTitles(titles.map((t, j) => (j === i ? { ...t, ...change } : t)));
  const add = () => {
    const start = ed.time;
    ed.setTitles([
      ...titles,
      { text: "Your text", start, end: Math.min(ed.media.duration, start + 3), x: 0.5, y: 0.2, size: 0.07, color: "#ffffff", font: "Arial", boxed: false },
    ]);
    ed.setSelectedTitle(titles.length);
  };

  return (
    <div>
      <h3 className="section">Text</h3>
      <p className="help">A hook, a label, a call to action. Drag it on the preview to place it.</p>
      <button className="btn primary wide" onClick={add}>
        Add text at {clock(ed.time)}
      </button>

      {titles.map((t, i) => {
        const open = ed.selectedTitle === i;
        return (
          <div key={i} data-card={`title-${i}`} className="hl-item" style={{ marginTop: 10, cursor: "default", borderColor: open ? "var(--accent)" : undefined }}>
            <div className="row" onClick={() => ed.setSelectedTitle(open ? null : i)} style={{ cursor: "pointer" }}>
              <strong className="grow ellipsis">{t.text || "(empty)"}</strong>
              <a
                href="#"
                className="faint mono"
                onClick={(e) => {
                  e.preventDefault();
                  e.stopPropagation();
                  ed.seek(t.start);
                }}
              >
                {clock(t.start, true)}–{clock(t.end, true)}
              </a>
            </div>
            {open && (
              <div style={{ marginTop: 10 }}>
                <textarea
                  className="input"
                  rows={2}
                  style={{ width: "100%", resize: "vertical", marginBottom: 10 }}
                  value={t.text}
                  onChange={(e) => update(i, { text: e.target.value })}
                />
                <div className="field">
                  <label>
                    <span>Show for</span>
                    <span>
                      {(t.end - t.start).toFixed(1)} s, until {clock(t.end, true)}
                      {(t.hold ?? 0) > 0 && `, then ${t.hold!.toFixed(1)} s into the next clips`}
                    </span>
                  </label>
                  <input
                    type="range"
                    min={0.3}
                    max={Math.max(0.3, Math.min(30, ed.media.duration - t.start))}
                    step={0.1}
                    value={t.end - t.start}
                    onChange={(e) => update(i, { end: t.start + Number(e.target.value), hold: 0 })}
                  />
                </div>
                <div className="row" style={{ marginBottom: 4 }}>
                  <button className="btn small grow" onClick={() => update(i, { start: Math.min(ed.time, t.end - 0.1) })} title="It appears where the playhead is">
                    Start at playhead
                  </button>
                  <button className="btn small grow" onClick={() => update(i, { end: Math.max(ed.time, t.start + 0.1), hold: 0 })} title="It disappears where the playhead is">
                    End at playhead
                  </button>
                </div>
                <p className="faint" style={{ margin: "0 0 10px" }}>
                  Move the playhead, then press a button to make the text appear or disappear there.
                </p>
                <div className="field">
                  <label>
                    <span>Size</span>
                    <span>{(t.size * 100).toFixed(0)}%</span>
                  </label>
                  <input type="range" min={0.03} max={0.2} step={0.005} value={t.size} onChange={(e) => update(i, { size: Number(e.target.value) })} />
                </div>
                <div className="row wrap" style={{ marginBottom: 10 }}>
                  {COLORS.map((c) => (
                    <button
                      key={c}
                      title={c}
                      onClick={() => update(i, { color: c })}
                      style={{
                        width: 22,
                        height: 22,
                        borderRadius: "50%",
                        background: c,
                        cursor: "pointer",
                        border: t.color === c ? "2px solid var(--accent)" : "1px solid var(--line)",
                      }}
                    />
                  ))}
                  <input type="color" value={t.color} onChange={(e) => update(i, { color: e.target.value })} style={{ width: 28, height: 24, border: 0, background: "none" }} />
                </div>
                <div className="row" style={{ marginBottom: 10 }}>
                  <select className="input grow" value={t.font} onChange={(e) => update(i, { font: e.target.value })}>
                    {FONTS.map((f) => (
                      <option key={f} value={f}>
                        {f}
                      </option>
                    ))}
                  </select>
                  <label className="toggle">
                    <input type="checkbox" checked={t.boxed} onChange={(e) => update(i, { boxed: e.target.checked })} />
                    <span>Box</span>
                  </label>
                </div>
                <button
                  className="btn small ghost danger"
                  onClick={() => {
                    ed.setTitles(titles.filter((_, j) => j !== i));
                    ed.setSelectedTitle(null);
                  }}
                >
                  Delete
                </button>
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}
