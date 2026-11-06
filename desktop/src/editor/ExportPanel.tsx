import { useEffect, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { errorText, exportEdit, exportGif, isCancelled, newTaskId, type Exported } from "../api";
import { FileActions, TaskProgress } from "../components";
import { baseName, bytes, clock, dirName, join, span, stem, store } from "../util";
import { outputExt, projectRequest, savedFormat, type Format } from "./exporting";
import { outputSize, type EditorApi } from "./state";

const SIZES = [10, 25, 50, 100];

export default function ExportPanel({ ed }: { ed: EditorApi }) {
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

  const project = ed.project;
  const first = project.clips[0]?.media ?? ed.media;
  const exportVideo = async () => {
    const ext = outputExt(project.clips);
    const output = await save({
      defaultPath: join(dirName(first.path), `${stem(first.path)}_edit.${ext}`),
      filters: [{ name: ext.toUpperCase(), extensions: [ext] }],
    });
    if (!output) return;
    run((id) => exportEdit(id, projectRequest(project, output, format)));
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

  const [w, h] = outputSize(project.frame, first);
  const hasVideo = project.clips.some((c) => c.media.hasVideo);
  const length = project.clips.reduce((sum, c) => sum + c.outputDuration, 0);
  const original = project.clips.reduce((sum, c) => sum + c.media.duration, 0);
  const captioned = project.captionsOn && project.clips.some((c) => c.transcript);
  const titles = project.clips.reduce((n, c) => n + c.titles.length, 0);

  return (
    <div className="panel">
      <div className="card" style={{ marginBottom: 16 }}>
        <div className="stat-row">
          <span className="muted">Length</span>
          <strong>{clock(length)}</strong>
        </div>
        <div className="stat-row">
          <span className="muted">Clips</span>
          <span>
            {project.clips.length}
            {original - length > 0.05 ? ` (−${span(original - length)} cut)` : ""}
          </span>
        </div>
        {hasVideo && (
          <div className="stat-row">
            <span className="muted">Frame</span>
            <span>
              {w}×{h}
              {project.frame ? ` (${project.frame.aspect.w}:${project.frame.aspect.h}, ${project.frame.fit.kind === "pad" ? "bars" : project.frame.fit.kind})` : ""}
            </span>
          </div>
        )}
        {hasVideo && (
          <div className="stat-row">
            <span className="muted">Captions · titles</span>
            <span>
              {captioned ? "burned in" : "none"} · {titles}
            </span>
          </div>
        )}
        <div className="stat-row">
          <span className="muted">Music</span>
          <span className="ellipsis" style={{ maxWidth: 170 }}>
            {project.music ? baseName(project.music.source) : "none"}
          </span>
        </div>
      </div>

      {hasVideo && (
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
          </div>
        </div>
      )}

      {ed.media.hasVideo && (
        <>
          <h3 className="section">GIF</h3>
          <p className="help">From the selection on this clip's timeline{ed.selection ? ` (${span(ed.selection.end - ed.selection.start)})` : ""}.</p>
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
