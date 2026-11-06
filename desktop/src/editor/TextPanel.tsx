import { useEffect, useRef, useState } from "react";
import { open, save } from "@tauri-apps/plugin-dialog";
import { errorText, isCancelled, listModels, loadSubtitles, newTaskId, saveSubtitles, transcribe, type ModelInfo } from "../api";
import { TaskProgress } from "../components";
import { clock, dirName, join, load, stem, store } from "../util";
import type { EditorApi } from "./state";

const LANGUAGES: [string, string][] = [
  ["auto", "Detect"],
  ["en", "English"],
  ["es", "Spanish"],
  ["fr", "French"],
  ["de", "German"],
  ["pt", "Portuguese"],
  ["it", "Italian"],
  ["nl", "Dutch"],
  ["yo", "Yoruba"],
  ["ar", "Arabic"],
  ["hi", "Hindi"],
  ["ja", "Japanese"],
  ["ko", "Korean"],
  ["zh", "Chinese"],
];
const FONTS = ["Arial", "Segoe UI", "Impact", "Verdana", "Trebuchet MS", "Georgia", "Comic Sans MS"];

export default function TextPanel({ ed }: { ed: EditorApi }) {
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [language, setLanguage] = useState(load("speech.language", "en"));
  const [model, setModel] = useState(load("speech.model", "base.en"));
  const [gpu, setGpu] = useState(load("speech.gpu", true));
  const [task, setTask] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const list = useRef<HTMLDivElement>(null);

  useEffect(() => {
    listModels().then(setModels);
  }, []);
  useEffect(() => store("speech.language", language), [language]);
  useEffect(() => store("speech.model", model), [model]);
  useEffect(() => store("speech.gpu", gpu), [gpu]);

  // English-only models can't do other languages.
  const usable = models.filter((m) => language === "en" || !m.englishOnly);
  useEffect(() => {
    if (usable.length && !usable.some((m) => m.name === model)) setModel(language === "en" ? "base.en" : "base");
  }, [language, models]);

  const run = async () => {
    const id = newTaskId();
    setTask(id);
    setError(null);
    try {
      ed.setTranscript(await transcribe(id, ed.media.path, model, language, gpu));
      ed.setRemoved([]);
      listModels().then(setModels);
    } catch (e) {
      if (!isCancelled(e)) setError(errorText(e));
    } finally {
      setTask(null);
    }
  };

  const loadFile = async () => {
    const path = await open({ filters: [{ name: "Subtitles", extensions: ["srt", "vtt"] }], defaultPath: dirName(ed.media.path) });
    if (typeof path !== "string") return;
    try {
      ed.setTranscript(await loadSubtitles(path));
      ed.setRemoved([]);
    } catch (e) {
      setError(errorText(e));
    }
  };

  const saveFile = async () => {
    if (!ed.transcript) return;
    const path = await save({
      defaultPath: join(dirName(ed.media.path), `${stem(ed.media.path)}.srt`),
      filters: [{ name: "Subtitles", extensions: ["srt", "vtt", "txt"] }],
    });
    if (!path) return;
    // Timed like the export: cut lines gone, the rest moved up.
    const cut = ed.plan.cuts.length > 0;
    await saveSubtitles(ed.transcript, cut ? ed.plan.keep : null, path).catch((e) => setError(errorText(e)));
  };

  const segments = ed.transcript?.segments ?? [];
  const now = segments.findIndex((s) => ed.time >= s.start && ed.time < s.end);
  // Follow the playback in the list.
  useEffect(() => {
    if (now < 0 || !list.current) return;
    const el = list.current.children[now] as HTMLElement | undefined;
    el?.scrollIntoView({ block: "nearest" });
  }, [now]);

  const toggle = (i: number) => ed.setRemoved(ed.removed.includes(i) ? ed.removed.filter((j) => j !== i) : [...ed.removed, i]);
  const style = ed.project.style;

  return (
    <div className="panel">
      {!ed.transcript && (
        <>
          <p className="help">Turn the speech into text. Then edit the video by deleting lines, and burn the text in as captions.</p>
          <div className="field">
            <label>Language</label>
            <select className="input" value={language} onChange={(e) => setLanguage(e.target.value)}>
              {LANGUAGES.map(([code, name]) => (
                <option key={code} value={code}>
                  {name}
                </option>
              ))}
            </select>
          </div>
          <div className="field">
            <label>Model</label>
            <select className="input" value={model} onChange={(e) => setModel(e.target.value)}>
              {usable.map((m) => (
                <option key={m.name} value={m.name}>
                  {m.name} · {m.mb} MB{m.installed ? "" : " · downloads first"}
                </option>
              ))}
            </select>
            <span className="faint">base is quick. small and medium hear technical words better but take 3–10× longer.</span>
          </div>
          <label className="toggle" style={{ marginBottom: 14 }}>
            <input type="checkbox" checked={gpu} onChange={(e) => setGpu(e.target.checked)} />
            <span>Use the GPU (about 4× faster)</span>
          </label>
          {!task && (
            <div className="row">
              <button className="btn primary grow" onClick={run} disabled={!ed.media.hasAudio}>
                Transcribe
              </button>
              <button className="btn" onClick={loadFile}>
                Load .srt
              </button>
            </div>
          )}
        </>
      )}
      <TaskProgress id={task} label="transcribing" />
      {error && <p className="error">{error}</p>}

      {ed.transcript && style && (
        <>
          <label className="toggle" style={{ marginBottom: 12 }}>
            <input type="checkbox" checked={ed.project.captionsOn} onChange={(e) => ed.project.setCaptionsOn(e.target.checked)} />
            <strong>Burn captions into the video</strong>
          </label>
          {ed.project.captionsOn && (
            <div className="card" style={{ marginBottom: 16, padding: 12 }}>
              <div className="field">
                <label>
                  <span>Words at a time</span>
                  <span>{style.max_words}</span>
                </label>
                <input type="range" min={1} max={8} value={style.max_words} onChange={(e) => ed.project.setStyle({ ...style, max_words: Number(e.target.value) })} />
              </div>
              <div className="field">
                <label>
                  <span>Size</span>
                  <span>{(style.size * 100).toFixed(1)}%</span>
                </label>
                <input
                  type="range"
                  min={0.03}
                  max={0.14}
                  step={0.005}
                  value={style.size}
                  onChange={(e) => ed.project.setStyle({ ...style, size: Number(e.target.value) })}
                />
              </div>
              <div className="field">
                <label>
                  <span>Height on screen</span>
                  <span>{(style.position * 100).toFixed(0)}%</span>
                </label>
                <input
                  type="range"
                  min={0.2}
                  max={0.97}
                  step={0.01}
                  value={style.position}
                  onChange={(e) => ed.project.setStyle({ ...style, position: Number(e.target.value) })}
                />
              </div>
              <div className="row" style={{ marginBottom: 10 }}>
                <select className="input grow" value={style.font} onChange={(e) => ed.project.setStyle({ ...style, font: e.target.value })}>
                  {FONTS.map((f) => (
                    <option key={f} value={f}>
                      {f}
                    </option>
                  ))}
                </select>
              </div>
              <div className="row">
                <label className="toggle">
                  <input type="checkbox" checked={style.uppercase} onChange={(e) => ed.project.setStyle({ ...style, uppercase: e.target.checked })} />
                  <span>CAPITALS</span>
                </label>
                <label className="toggle" style={{ marginLeft: 14 }}>
                  <input type="checkbox" checked={style.pop} onChange={(e) => ed.project.setStyle({ ...style, pop: e.target.checked })} />
                  <span>Pop in</span>
                </label>
              </div>
            </div>
          )}

          <div className="row" style={{ marginBottom: 8 }}>
            <span className="grow muted">
              {segments.length} lines{ed.removed.length > 0 ? ` · ${ed.removed.length} cut` : ""}
            </span>
            <button className="btn small" onClick={saveFile}>
              Save .srt
            </button>
            <button
              className="btn small ghost"
              onClick={() => {
                ed.setTranscript(null);
                ed.setRemoved([]);
                ed.project.setCaptionsOn(false);
              }}
            >
              Redo
            </button>
          </div>
          <p className="help">
            Click a line to jump to it. <strong>✕</strong> cuts it from the video.
          </p>
          <div className="transcript" ref={list}>
            {segments.map((s, i) => {
              const cut = ed.removed.includes(i);
              return (
                <div key={i} className={`line${i === now ? " now" : ""}${cut ? " removed" : ""}`} onClick={() => ed.seek(s.start)}>
                  <span className="t">{clock(s.start)}</span>
                  <span className="text">{s.text}</span>
                  <button
                    className="x"
                    title={cut ? "Put back" : "Cut this line"}
                    onClick={(e) => {
                      e.stopPropagation();
                      toggle(i);
                    }}
                  >
                    {cut ? "↺" : "✕"}
                  </button>
                </div>
              );
            })}
          </div>
        </>
      )}
    </div>
  );
}
