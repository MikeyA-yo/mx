// Background noise reduction for the clip's own sound, with a few seconds
// before and after to hear what it does.

import { useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { errorText, isCancelled, newTaskId, previewDenoise, type Denoise } from "../api";
import { TaskProgress } from "../components";
import { clock } from "../util";
import type { EditorApi } from "./state";

const LEVELS: [Denoise, string, string][] = [
  ["off", "Off", "The sound as recorded."],
  ["light", "Light", "Learns your room's hum from a pause and takes some of it away. Your voice stays natural."],
  ["strong", "Strong", "A much quieter background: fans, traffic, hiss. Made for talking, so music or other sounds in this clip go too."],
];

type Pair = { before: string; after: string; at: number; denoise: Denoise };

export default function NoisePanel({ ed }: { ed: EditorApi }) {
  const [task, setTask] = useState<string | null>(null);
  const [pair, setPair] = useState<Pair | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [playing, setPlaying] = useState<"before" | "after" | null>(null);
  const players = { before: useRef<HTMLAudioElement>(null), after: useRef<HTMLAudioElement>(null) };

  const listen = async () => {
    const id = newTaskId();
    setTask(id);
    setError(null);
    stop();
    try {
      const p = await previewDenoise(id, ed.media.path, ed.time, ed.denoise);
      setPair({ before: convertFileSrc(p.before), after: convertFileSrc(p.after), at: ed.time, denoise: ed.denoise });
    } catch (e) {
      if (!isCancelled(e)) setError(errorText(e));
    } finally {
      setTask(null);
    }
  };
  // Switching between the two keeps the place, so the difference is
  // heard on the same words.
  const play = (which: "before" | "after") => {
    const other = which === "before" ? "after" : "before";
    const a = players[which].current;
    const b = players[other].current;
    if (!a) return;
    if (playing === which) return stop();
    if (b && playing === other) {
      a.currentTime = b.currentTime;
      b.pause();
    } else a.currentTime = 0;
    a.play().catch(() => {});
    setPlaying(which);
  };
  const stop = () => {
    players.before.current?.pause();
    players.after.current?.pause();
    setPlaying(null);
  };

  const current = LEVELS.find(([d]) => d === ed.denoise)!;
  const stale = pair && pair.denoise !== ed.denoise;

  return (
    <>
      <h3 className="section">Background noise</h3>
      <div className="chips" style={{ marginBottom: 8 }}>
        {LEVELS.map(([d, label, hint]) => (
          <button key={d} className={`chip${ed.denoise === d ? " on" : ""}`} title={hint} onClick={() => ed.setDenoise(d)}>
            {label}
          </button>
        ))}
      </div>
      <p className="help" style={{ marginBottom: 10 }}>
        {current[2]}
      </p>
      {ed.denoise !== "off" && (
        <>
          {!task && (
            <button className="btn wide" onClick={listen}>
              {pair && !stale ? "Listen again" : "Listen"} to {clock(ed.time)}–{clock(ed.time + 6)}, before and after
            </button>
          )}
          <TaskProgress id={task} label="cleaning the sound" />
          {error && <p className="error">{error}</p>}
          {pair && !stale && !task && (
            <div className="row" style={{ marginTop: 8 }}>
              <button className={`btn small grow${playing === "before" ? " primary" : ""}`} onClick={() => play("before")}>
                {playing === "before" ? "❚❚" : "▶"} As recorded
              </button>
              <button className={`btn small grow${playing === "after" ? " primary" : ""}`} onClick={() => play("after")}>
                {playing === "after" ? "❚❚" : "▶"} Cleaned
              </button>
            </div>
          )}
          {pair && (
            <>
              <audio ref={players.before} src={pair.before} preload="auto" onEnded={() => setPlaying(null)} />
              <audio ref={players.after} src={pair.after} preload="auto" onEnded={() => setPlaying(null)} />
            </>
          )}
          <p className="faint" style={{ margin: "8px 0 0" }}>
            Playing in the editor stays as recorded; the export is cleaned. Use earphones to hear the difference.
          </p>
        </>
      )}
    </>
  );
}
