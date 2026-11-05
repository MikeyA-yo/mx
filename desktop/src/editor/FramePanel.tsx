import type { Fit, Frame } from "../api";
import { outputSize, type EditorApi } from "./state";

const ASPECTS: [string, number, number, string][] = [
  ["9:16", 9, 16, "Shorts, TikTok, Reels"],
  ["1:1", 1, 1, "Square"],
  ["4:5", 4, 5, "Instagram feed"],
  ["16:9", 16, 9, "YouTube"],
];

export default function FramePanel({ ed }: { ed: EditorApi }) {
  if (!ed.media.hasVideo) {
    return (
      <div className="panel">
        <p className="faint">This file is audio only.</p>
      </div>
    );
  }
  const source = Math.min(ed.media.width ?? 1080, ed.media.height ?? 1080);
  const f = ed.frame;
  const set = (next: Partial<Frame>) =>
    ed.setFrame({ aspect: { w: 9, h: 16 }, fit: { kind: "blur" }, short_edge: Math.min(source, 1080), ...f, ...next });
  const fits: [Fit["kind"], string, string][] = [
    ["blur", "Blur", "the whole picture over a blurred copy"],
    ["crop", "Crop", "fill the frame, cut off the sides"],
    ["pad", "Bars", "the whole picture with black bars"],
  ];
  const [w, h] = outputSize(ed);
  const landscapeSource = (ed.media.width ?? 16) >= (ed.media.height ?? 9);

  return (
    <div className="panel">
      <p className="help">Change the shape of the video. The preview shows what the export will look like.</p>
      <div className="field">
        <div className="label">Shape</div>
        <div className="chips">
          <button className={`chip${!f ? " on" : ""}`} onClick={() => ed.setFrame(null)}>
            Original
          </button>
          {ASPECTS.map(([label, aw, ah, hint]) => (
            <button
              key={label}
              title={hint}
              className={`chip${f && f.aspect.w === aw && f.aspect.h === ah ? " on" : ""}`}
              onClick={() => set({ aspect: { w: aw, h: ah } })}
            >
              {label}
            </button>
          ))}
        </div>
      </div>
      {f && (
        <>
          <div className="field">
            <div className="label">Fit</div>
            <div className="chips">
              {fits.map(([kind, label, hint]) => (
                <button
                  key={kind}
                  title={hint}
                  className={`chip${f.fit.kind === kind ? " on" : ""}`}
                  onClick={() => set({ fit: kind === "crop" ? { kind, position: 0.5 } : { kind } })}
                >
                  {label}
                </button>
              ))}
            </div>
            <span className="faint">{fits.find(([k]) => k === f.fit.kind)?.[2]}</span>
          </div>
          {f.fit.kind === "crop" && (
            <div className="field">
              <label>
                <span>{landscapeSource ? "Left" : "Top"}</span>
                <span>{landscapeSource ? "Right" : "Bottom"}</span>
              </label>
              <input
                type="range"
                min={0}
                max={1}
                step={0.01}
                value={f.fit.position}
                onChange={(e) => set({ fit: { kind: "crop", position: Number(e.target.value) } })}
              />
            </div>
          )}
          <div className="field">
            <div className="label">Size</div>
            <div className="chips">
              {[720, 1080].map((edge) => (
                <button key={edge} className={`chip${f.short_edge === edge ? " on" : ""}`} onClick={() => set({ short_edge: edge })}>
                  {edge}p
                </button>
              ))}
            </div>
            <span className="faint">
              Output {w}×{h}
              {f.short_edge > source ? " (upscaled from the source)" : ""}
            </span>
          </div>
        </>
      )}
    </div>
  );
}
