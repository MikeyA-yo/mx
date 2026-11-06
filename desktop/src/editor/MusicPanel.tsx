import { open } from "@tauri-apps/plugin-dialog";
import { baseName, dirName } from "../util";
import type { EditorApi } from "./state";

const AUDIO = ["mp3", "m4a", "aac", "wav", "flac", "ogg", "opus", "mp4", "webm", "mkv"];

export default function MusicPanel({ ed }: { ed: EditorApi }) {
  const music = ed.project.music;
  const set = ed.project.setMusic;

  const choose = async () => {
    const source = await open({ filters: [{ name: "Audio (or a video's soundtrack)", extensions: AUDIO }], defaultPath: dirName(ed.media.path) });
    if (typeof source !== "string") return;
    set({ source, offset: 0, volume: music?.volume ?? 0.25, duck: music?.duck ?? true, fade_out: music?.fade_out ?? 2 });
  };

  return (
    <div className="panel">
      <p className="help">A soundtrack under the whole video. It loops if the video is longer, and fades out at the end.</p>
      {!music && (
        <button className="btn primary wide" onClick={choose}>
          Add music...
        </button>
      )}
      {music && (
        <>
          <div className="card" style={{ padding: 12, marginBottom: 16 }}>
            <div className="row">
              <span className="grow ellipsis" title={music.source}>
                {baseName(music.source)}
              </span>
              <button className="btn small" onClick={choose}>
                Change
              </button>
              <button className="btn small ghost danger" onClick={() => set(null)}>
                Remove
              </button>
            </div>
          </div>
          <div className="field">
            <label>
              <span>Volume</span>
              <span>{Math.round(music.volume * 100)}%</span>
            </label>
            <input type="range" min={0} max={1} step={0.01} value={music.volume} onChange={(e) => set({ ...music, volume: Number(e.target.value) })} />
          </div>
          <div className="field">
            <label>
              <span>Start the song at</span>
              <span>{music.offset.toFixed(1)} s</span>
            </label>
            <input type="range" min={0} max={180} step={0.5} value={music.offset} onChange={(e) => set({ ...music, offset: Number(e.target.value) })} />
          </div>
          <div className="field">
            <label>
              <span>Fade out</span>
              <span>{music.fade_out.toFixed(1)} s</span>
            </label>
            <input type="range" min={0} max={8} step={0.5} value={music.fade_out} onChange={(e) => set({ ...music, fade_out: Number(e.target.value) })} />
          </div>
          <label className="toggle">
            <input type="checkbox" checked={music.duck} onChange={(e) => set({ ...music, duck: e.target.checked })} />
            <span>
              Duck under speech <span className="faint">(music dips while the clips have sound)</span>
            </span>
          </label>
          <p className="faint" style={{ marginTop: 14 }}>
            The preview plays the music at its set volume; ducking happens in the export.
          </p>
        </>
      )}
    </div>
  );
}
