// Sound: the clip's own level, sounds started on this clip, and the
// project's music.

import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { errorText, probeMedia, type Sound } from "../api";
import { baseName, clock, dirName } from "../util";
import MusicPanel from "./MusicPanel";
import NoisePanel from "./NoisePanel";
import { rememberLength } from "./lengths";
import { useOpenCard, type EditorApi } from "./state";

const AUDIO = ["mp3", "m4a", "aac", "wav", "flac", "ogg", "opus", "mp4", "mov", "webm", "mkv"];

export default function AudioPanel({ ed }: { ed: EditorApi }) {
  const [lengths, setLengths] = useState<Record<string, number>>({});
  const selected = ed.layer?.kind === "sound" ? ed.layer.index : null;
  const setSelected = (index: number | null) => ed.setLayer(index === null ? null : { kind: "sound", index });
  useOpenCard(selected === null ? null : `sound-${selected}`);
  const [error, setError] = useState<string | null>(null);

  // File lengths, for the sliders' ranges.
  useEffect(() => {
    for (const s of ed.sounds) {
      if (lengths[s.source] === undefined) {
        probeMedia(s.source)
          .then((m) => setLengths((l) => ({ ...l, [s.source]: m.duration })))
          .catch(() => {});
      }
    }
  }, [ed.sounds]);

  const add = async () => {
    const source = await open({ filters: [{ name: "Audio (or a video's sound)", extensions: AUDIO }], defaultPath: dirName(ed.media.path) });
    if (typeof source !== "string") return;
    try {
      const m = await probeMedia(source);
      if (!m.hasAudio) throw new Error(`${baseName(source)} has no sound`);
      setLengths((l) => ({ ...l, [source]: m.duration }));
      ed.setSounds([...ed.sounds, { source, at: ed.time, offset: 0, duration: m.duration, volume: 1, fade_in: 0, fade_out: 0.3 }]);
      rememberLength(source, m.duration);
      setSelected(ed.sounds.length);
      setError(null);
    } catch (e) {
      setError(errorText(e));
    }
  };
  const update = (i: number, change: Partial<Sound>) => ed.setSounds(ed.sounds.map((s, j) => (j === i ? { ...s, ...change } : s)));
  const pct = (v: number) => `${Math.round(v * 100)}%`;

  return (
    <div className="panel">
      {ed.media.hasAudio && (
        <>
          <h3 className="section" style={{ marginTop: 0 }}>
            This clip's sound
          </h3>
          <div className="field">
            <label>
              <span>Volume</span>
              <span>{ed.volume === 0 ? "muted" : pct(ed.volume)}</span>
            </label>
            <div className="row">
              <input type="range" min={0} max={2} step={0.01} value={ed.volume} onChange={(e) => ed.setVolume(Number(e.target.value))} />
              <button className="btn small" onClick={() => ed.setVolume(ed.volume === 0 ? 1 : 0)}>
                {ed.volume === 0 ? "Unmute" : "Mute"}
              </button>
            </div>
            <span className="faint">Turn it down under a voiceover, or mute it for b-roll.</span>
          </div>
          <NoisePanel ed={ed} />
        </>
      )}

      <h3 className="section" style={{ marginTop: ed.media.hasAudio ? undefined : 0 }}>
        Sounds · start on this clip
      </h3>
      <p className="help">A voiceover or effect. It starts here and plays to its end, over the clips after this one too.</p>
      <button className="btn primary wide" onClick={add}>
        Add a sound at {clock(ed.time, true)}...
      </button>
      {error && <p className="error">{error}</p>}
      {ed.sounds.map((s, i) => {
        const fileLength = lengths[s.source] ?? s.offset + (s.duration ?? 0);
        const length = s.duration ?? fileLength - s.offset;
        const open = selected === i;
        return (
          <div key={i} data-card={`sound-${i}`} className="hl-item" style={{ marginTop: 8, cursor: "default", borderColor: open ? "var(--accent)" : undefined }}>
            <div className="row" style={{ cursor: "pointer" }} onClick={() => setSelected(open ? null : i)}>
              <span className="grow ellipsis" title={s.source}>
                ♪ {baseName(s.source)}
              </span>
              <a
                href="#"
                className="faint mono"
                onClick={(e) => {
                  e.preventDefault();
                  e.stopPropagation();
                  ed.seek(s.at, true);
                }}
              >
                {clock(s.at, true)} · {clock(length)}
              </a>
            </div>
            {open && (
              <div style={{ marginTop: 10 }}>
                <div className="field" style={{ marginBottom: 8 }}>
                  <label>
                    <span>Volume</span>
                    <span>{pct(s.volume)}</span>
                  </label>
                  <input type="range" min={0} max={2} step={0.01} value={s.volume} onChange={(e) => update(i, { volume: Number(e.target.value) })} />
                </div>
                <div className="field" style={{ marginBottom: 8 }}>
                  <label>
                    <span>Start the file at</span>
                    <span>{s.offset.toFixed(1)} s</span>
                  </label>
                  <input
                    type="range"
                    min={0}
                    max={Math.max(0, fileLength - 0.5)}
                    step={0.1}
                    value={s.offset}
                    onChange={(e) => {
                      const offset = Number(e.target.value);
                      update(i, { offset, duration: Math.min(length, fileLength - offset) });
                    }}
                  />
                </div>
                <div className="field" style={{ marginBottom: 8 }}>
                  <label>
                    <span>Play for</span>
                    <span>{length.toFixed(1)} s</span>
                  </label>
                  <input
                    type="range"
                    min={0.5}
                    max={Math.max(0.5, fileLength - s.offset)}
                    step={0.1}
                    value={length}
                    onChange={(e) => update(i, { duration: Number(e.target.value) })}
                  />
                </div>
                <div className="row" style={{ marginBottom: 8 }}>
                  {(
                    [
                      ["Fade in", "fade_in"],
                      ["Fade out", "fade_out"],
                    ] as const
                  ).map(([label, field]) => (
                    <div className="field grow" key={field} style={{ marginBottom: 0 }}>
                      <label>
                        <span>{label}</span>
                        <span>{s[field].toFixed(1)} s</span>
                      </label>
                      <input type="range" min={0} max={3} step={0.1} value={s[field]} onChange={(e) => update(i, { [field]: Number(e.target.value) })} />
                    </div>
                  ))}
                </div>
                <div className="row">
                  <button className="btn small" onClick={() => update(i, { at: ed.time })}>
                    Start at playhead
                  </button>
                  <button
                    className="btn small ghost danger"
                    onClick={() => {
                      ed.setSounds(ed.sounds.filter((_, j) => j !== i));
                      setSelected(null);
                    }}
                  >
                    Delete
                  </button>
                  <span className="grow" />
                  <button className="btn small primary" onClick={() => setSelected(null)} title="Close these settings (Esc)">
                    Done
                  </button>
                </div>
              </div>
            )}
          </div>
        );
      })}

      <MusicPanel ed={ed} />
    </div>
  );
}
