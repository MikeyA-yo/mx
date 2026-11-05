import { useEffect, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { errorText, exportEdit, exportGif, isCancelled, newTaskId, type Exported } from "../api";
import { FileActions, TaskProgress } from "../components";
import { bytes, clock, dirName, join, span, stem, store } from "../util";
import { outputExt, request, savedFormat, type Format } from "./exporting";
import { outputSize, type EditorApi } from "./state";

const SIZES = [10, 25, 50, 100];

export default function ExportPanel({ ed, onEdit }: { ed: EditorApi; onEdit: (path: string) => void }) {
  const [format, setFormat] = useState<Format>(savedFormat);
  const [task, setTask] = useState<string | null>(null);
  const [result, setResult] = useState<Exported | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [gifFps, setGifFps] = useState(15);
  const [gifWidth, setGifWidth] = useState(480);

  useEffect(() => store("export.format", format), [format]);

  const run = async (work: (id: string) => Promise<Exported>) => {
    const id = newTaskId();
    setTask(id);
    setError(null);
    setResult(null);
    try {
      setResult(await work(id));
    } catch (e) {
      if (!isCancelled(e)) setError(errorText(e));
    } finally {
      setTask(null);
    }
  };

  const exportVideo = async () => {
    const ext = outputExt(ed);
    const output = await save({
      defaultPath: join(dirName(ed.media.path), `${stem(ed.media.path)}_edit.${ext}`),
      filters: [{ name: ext.toUpperCase(), extensions: [ext] }],
    });
    if (!output) return;
    const cut = ed.plan.cuts.length > 0;
    run((id) => exportEdit(id, request(ed, output, cut ? ed.plan.keep : [], format)));
  };

  const exportSelectionGif = async () => {
    const sel = ed.selection;
    if (!sel) return;
    const output = await save({
      defaultPath: join(dirName(ed.media.path), `${stem(ed.media.path)}_${Math.floor(sel.start)}s.gif`),
      filters: [{ name: "GIF", extensions: ["gif"] }],
    });
    if (!output) return;
    run((id) => exportGif(id, { input: ed.media.path, output, start: sel.start, end: sel.end, fps: gifFps, width: gifWidth }));
  };

  const [w, h] = outputSize(ed);
  const captions = ed.captionsOn && !!ed.transcript;
  const pieces = ed.plan.cuts.length;

  return (
    <div className="panel">
      <div className="card" style={{ marginBottom: 16 }}>
        <div className="stat-row">
          <span className="muted">Length</span>
          <strong>{clock(ed.plan.outputDuration)}</strong>
        </div>
        <div className="stat-row">
          <span className="muted">Cuts</span>
          <span>{pieces ? `${pieces} (−${span(ed.media.duration - ed.plan.outputDuration)})` : "none"}</span>
        </div>
        {ed.media.hasVideo && (
          <div className="stat-row">
            <span className="muted">Frame</span>
            <span>
              {w}×{h}
              {ed.frame ? ` (${ed.frame.aspect.w}:${ed.frame.aspect.h}, ${ed.frame.fit.kind === "pad" ? "bars" : ed.frame.fit.kind})` : ""}
            </span>
          </div>
        )}
        {ed.media.hasVideo && (
          <div className="stat-row">
            <span className="muted">Captions</span>
            <span>{captions ? "burned in" : "none"}</span>
          </div>
        )}
      </div>

      {ed.media.hasVideo && (
        <>
          <div className="field">
            <div className="label">Video format</div>
            <div className="chips">
              {(
                [
                  ["h264", "H.264", "plays everywhere"],
                  ["hevc", "HEVC", "smaller, newer devices"],
                  ["av1", "AV1", "smallest, slow without a new GPU"],
                ] as const
              ).map(([codec, label, hint]) => (
                <button key={codec} title={hint} className={`chip${format.codec === codec ? " on" : ""}`} onClick={() => setFormat({ ...format, codec })}>
                  {label}
                </button>
              ))}
            </div>
          </div>
          <label className="toggle" style={{ marginBottom: 12 }}>
            <input type="checkbox" checked={format.gpu} onChange={(e) => setFormat({ ...format, gpu: e.target.checked })} />
            <span>Use the GPU when it's faster</span>
          </label>
          <div className="field">
            <label className="toggle">
              <input
                type="checkbox"
                checked={format.targetMb !== null}
                onChange={(e) => setFormat({ ...format, targetMb: e.target.checked ? 25 : null })}
              />
              <span>Fit under a file size</span>
            </label>
            {format.targetMb !== null && (
              <div className="row">
                <div className="chips">
                  {SIZES.map((mb) => (
                    <button key={mb} className={`chip${format.targetMb === mb ? " on" : ""}`} onClick={() => setFormat({ ...format, targetMb: mb })}>
                      {mb} MB
                    </button>
                  ))}
                </div>
                <input
                  className="input"
                  style={{ width: 70 }}
                  type="number"
                  min={1}
                  value={format.targetMb}
                  onChange={(e) => setFormat({ ...format, targetMb: Math.max(1, Number(e.target.value)) })}
                />
              </div>
            )}
            {format.targetMb !== null && <span className="faint">Adds a two-pass encode after the render.</span>}
          </div>
        </>
      )}

      {!task && (
        <button className="btn primary wide" onClick={exportVideo}>
          Export
        </button>
      )}
      <TaskProgress id={task} label="rendering" />
      {error && <p className="error">{error}</p>}
      {result && (
        <div className="card" style={{ marginTop: 12 }}>
          <div className="ellipsis" style={{ marginBottom: 4 }}>
            <strong>Saved</strong> {result.output}
          </div>
          <div className="faint" style={{ marginBottom: 10 }}>
            {bytes(result.size)} · {clock(result.duration)}
            {result.encoder ? ` · ${result.encoder}` : ""}
          </div>
          <div className="row">
            <FileActions path={result.output} />
            {!result.output.endsWith(".gif") && (
              <button className="btn small ghost" onClick={() => onEdit(result.output)}>
                Open in editor
              </button>
            )}
          </div>
        </div>
      )}

      {ed.media.hasVideo && (
        <>
          <h3 className="section">GIF</h3>
          <p className="help">From the selection on the timeline{ed.selection ? ` (${span(ed.selection.end - ed.selection.start)})` : ""}.</p>
          <div className="row" style={{ marginBottom: 10 }}>
            <select className="input grow" value={gifWidth} onChange={(e) => setGifWidth(Number(e.target.value))}>
              {[320, 480, 640, 800].map((px) => (
                <option key={px} value={px}>
                  {px} px wide
                </option>
              ))}
            </select>
            <select className="input grow" value={gifFps} onChange={(e) => setGifFps(Number(e.target.value))}>
              {[10, 15, 20, 25].map((fps) => (
                <option key={fps} value={fps}>
                  {fps} fps
                </option>
              ))}
            </select>
          </div>
          <button className="btn wide" disabled={!ed.selection || !!task} onClick={exportSelectionGif}>
            Make a GIF
          </button>
        </>
      )}
    </div>
  );
}
