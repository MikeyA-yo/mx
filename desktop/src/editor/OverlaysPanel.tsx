// What goes over the picture: videos and images (b-roll, picture-in-
// picture, a logo or a sticker) and text.

import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { errorText, probeMedia, type Overlay } from "../api";
import { IMAGE_EXTENSIONS, baseName, clock, dirName, isImage } from "../util";
import TitlesPanel from "./TitlesPanel";
import { rememberLength } from "./lengths";
import { IMAGE_SECONDS, imageLayer, useOpenCard, type EditorApi } from "./state";

const VIDEO = ["mp4", "mov", "mkv", "webm", "m4v", "avi", "gif"];

export default function OverlaysPanel({ ed }: { ed: EditorApi }) {
  const [lengths, setLengths] = useState<Record<string, number>>({});
  const selected = ed.layer?.kind === "overlay" ? ed.layer.index : null;
  const setSelected = (index: number | null) => ed.setLayer(index === null ? null : { kind: "overlay", index });
  useOpenCard(selected === null ? null : `overlay-${selected}`);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    for (const o of ed.overlays) {
      if (lengths[o.source] === undefined && !isImage(o.source)) {
        probeMedia(o.source)
          .then((m) => setLengths((l) => ({ ...l, [o.source]: m.duration })))
          .catch(() => {});
      }
    }
  }, [ed.overlays]);

  if (!ed.media.hasVideo) {
    return (
      <div className="panel">
        <p className="faint">This clip is audio only.</p>
      </div>
    );
  }

  const add = async () => {
    const source = await open({ filters: [{ name: "Video", extensions: VIDEO }], defaultPath: dirName(ed.media.path) });
    if (typeof source !== "string") return;
    try {
      const m = await probeMedia(source);
      if (!m.hasVideo) throw new Error(`${baseName(source)} has no video`);
      setLengths((l) => ({ ...l, [source]: m.duration }));
      // Three seconds of b-roll is the usual cutaway; it's easy to stretch.
      ed.setOverlays([...ed.overlays, { source, at: ed.time, offset: 0, duration: Math.min(3, m.duration), pip: null, volume: 0, fade: 0.2 }]);
      rememberLength(source, m.duration);
      setSelected(ed.overlays.length);
      setError(null);
    } catch (e) {
      setError(errorText(e));
    }
  };
  const addImage = async () => {
    const source = await open({ filters: [{ name: "Image", extensions: IMAGE_EXTENSIONS }], defaultPath: dirName(ed.media.path) });
    if (typeof source !== "string") return;
    ed.setOverlays([...ed.overlays, imageLayer(source, ed.time)]);
    setSelected(ed.overlays.length);
    setError(null);
  };
  const update = (i: number, change: Partial<Overlay>) => ed.setOverlays(ed.overlays.map((o, j) => (j === i ? { ...o, ...change } : o)));

  return (
    <div className="panel">
      <h3 className="section" style={{ marginTop: 0 }}>
        Videos and images over this clip
      </h3>
      <p className="help">
        B-roll while you keep talking, a picture-in-picture, a photo, a logo or a sticker. Your clip's sound carries on underneath. A PNG
        with a see-through background stays see-through.
      </p>
      <div className="row">
        <button className="btn primary grow" onClick={add}>
          Add video...
        </button>
        <button className="btn primary grow" onClick={addImage}>
          Add image...
        </button>
      </div>
      <p className="faint" style={{ margin: "6px 0 0" }}>
        It starts at the playhead ({clock(ed.time, true)}). You can also drop images onto the window.
      </p>
      {error && <p className="error">{error}</p>}
      {ed.overlays.map((o, i) => {
        // An image holds still for as long as it's asked to.
        const still = isImage(o.source);
        const fileLength = still ? Infinity : (lengths[o.source] ?? o.offset + (o.duration ?? 0));
        const length = o.duration ?? (still ? IMAGE_SECONDS : fileLength - o.offset);
        const isOpen = selected === i;
        const slider = (label: string, value: number, shown: string, min: number, max: number, step: number, set: (v: number) => void) => (
          <div className="field" style={{ marginBottom: 8 }}>
            <label>
              <span>{label}</span>
              <span>{shown}</span>
            </label>
            <input type="range" min={min} max={max} step={step} value={value} onChange={(e) => set(Number(e.target.value))} />
          </div>
        );
        return (
          <div key={i} data-card={`overlay-${i}`} className="hl-item" style={{ marginTop: 8, cursor: "default", borderColor: isOpen ? "var(--accent)" : undefined }}>
            <div className="row" style={{ cursor: "pointer" }} onClick={() => setSelected(isOpen ? null : i)}>
              <span className="grow ellipsis" title={o.source}>
                {still ? "🖼" : "▣"} {baseName(o.source)}
              </span>
              <a
                href="#"
                className="faint mono"
                onClick={(e) => {
                  e.preventDefault();
                  e.stopPropagation();
                  ed.seek(o.at, true);
                }}
              >
                {clock(o.at, true)} · {clock(length)}
              </a>
            </div>
            {isOpen && (
              <div style={{ marginTop: 10 }}>
                <div className="chips" style={{ marginBottom: 10 }}>
                  <button className={`chip${!o.pip ? " on" : ""}`} onClick={() => update(i, { pip: null })}>
                    Full frame
                  </button>
                  <button className={`chip${o.pip ? " on" : ""}`} onClick={() => update(i, { pip: o.pip ?? { x: 0.75, y: 0.22, size: 0.35 } })}>
                    Box
                  </button>
                </div>
                {o.pip && <p className="faint" style={{ margin: "0 0 8px" }}>Drag the box on the preview to place it, and its corner to resize it.</p>}
                {o.pip && (
                  <>
                    {slider("Box size", o.pip.size, `${Math.round(o.pip.size * 100)}%`, 0.1, 0.9, 0.01, (size) => update(i, { pip: { ...o.pip!, size } }))}
                    {slider("Across", o.pip.x, `${Math.round(o.pip.x * 100)}%`, 0, 1, 0.01, (x) => update(i, { pip: { ...o.pip!, x } }))}
                    {slider("Down", o.pip.y, `${Math.round(o.pip.y * 100)}%`, 0, 1, 0.01, (y) => update(i, { pip: { ...o.pip!, y } }))}
                  </>
                )}
                {slider("Show for", length, `${length.toFixed(1)} s`, 0.5, still ? Math.max(30, length) : Math.max(0.5, fileLength - o.offset), 0.1, (duration) =>
                  update(i, { duration }),
                )}
                {!still && slider("Start the file at", o.offset, `${o.offset.toFixed(1)} s`, 0, Math.max(0, fileLength - 0.5), 0.1, (offset) =>
                  update(i, { offset, duration: Math.min(length, fileLength - offset) }),
                )}
                {slider("Fade", o.fade, `${o.fade.toFixed(1)} s`, 0, 1.5, 0.05, (fade) => update(i, { fade }))}
                {!still && slider("Its own sound", o.volume, o.volume === 0 ? "muted" : `${Math.round(o.volume * 100)}%`, 0, 1.5, 0.01, (volume) => update(i, { volume }))}
                <div className="row">
                  <button className="btn small" onClick={() => update(i, { at: ed.time })}>
                    Start at playhead
                  </button>
                  <button
                    className="btn small ghost danger"
                    onClick={() => {
                      ed.setOverlays(ed.overlays.filter((_, j) => j !== i));
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

      <TitlesPanel ed={ed} />
    </div>
  );
}
