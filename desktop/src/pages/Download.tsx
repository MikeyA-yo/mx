import { useEffect, useState } from "react";
import { videoDir } from "@tauri-apps/api/path";
import { open } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { download, errorText, fetchInfo, isCancelled, newTaskId, type LinkInfo } from "../api";
import { FileActions, ProgressBar, statusText } from "../components";
import { useTasks } from "../tasks";
import { baseName, clock, join, load, store } from "../util";

const BROWSERS = ["", "firefox", "chrome", "edge", "brave", "opera", "vivaldi"];

export default function DownloadPage({ onEdit }: { onEdit: (path: string) => void }) {
  const [url, setUrl] = useState("");
  const [info, setInfo] = useState<LinkInfo | null>(null);
  const [fetching, setFetching] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const [height, setHeight] = useState<number | null>(load("dl.height", null));
  const [audioOnly, setAudioOnly] = useState(false);
  const [audioFormat, setAudioFormat] = useState(load("dl.audioFormat", "mp3"));
  const [compat, setCompat] = useState(load("dl.compat", true));
  const [folder, setFolder] = useState<string>(load("dl.folder", ""));
  const [browser, setBrowser] = useState(load("dl.browser", ""));

  useEffect(() => {
    if (!folder) videoDir().then((d) => setFolder(join(d, "mx")));
  }, [folder]);
  useEffect(() => store("dl.height", height), [height]);
  useEffect(() => store("dl.audioFormat", audioFormat), [audioFormat]);
  useEffect(() => store("dl.compat", compat), [compat]);
  useEffect(() => store("dl.folder", folder), [folder]);
  useEffect(() => store("dl.browser", browser), [browser]);

  const downloads = useTasks().filter((t) => t.kind === "download");

  const lookUp = async (target = url) => {
    const u = target.trim();
    if (!u) return;
    setFetching(true);
    setError(null);
    setInfo(null);
    try {
      setInfo(await fetchInfo(u, browser || null));
    } catch (e) {
      setError(errorText(e));
    } finally {
      setFetching(false);
    }
  };

  const paste = async () => {
    try {
      const text = (await navigator.clipboard.readText()).trim();
      if (text) {
        setUrl(text);
        lookUp(text);
      }
    } catch {
      // Clipboard access refused; typing still works.
    }
  };

  const start = async () => {
    if (!info) return;
    const opts = {
      url: info.url,
      title: info.title,
      outputDir: folder,
      maxHeight: audioOnly ? null : height,
      audioOnly,
      audioFormat: audioOnly && audioFormat !== "original" ? audioFormat : null,
      compat: !audioOnly && compat,
      playlist: info.isPlaylist,
      cookiesFromBrowser: browser || null,
    };
    setInfo(null);
    setUrl("");
    try {
      await download(newTaskId(), opts);
    } catch (e) {
      // The task list shows the failure; a stop isn't one.
      if (!isCancelled(e)) setError(errorText(e));
    }
  };

  const chooseFolder = async () => {
    const dir = await open({ directory: true, defaultPath: folder });
    if (typeof dir === "string") setFolder(dir);
  };

  const heights = info?.heights.filter((h) => h >= 144) ?? [];

  return (
    <div className="page">
      <h1>Download</h1>
      <p className="lede">Paste a link from YouTube, TikTok, Instagram, X, Reddit, Vimeo and about 1,800 other sites.</p>

      <form
        className="url-bar"
        onSubmit={(e) => {
          e.preventDefault();
          lookUp();
        }}
      >
        <input className="input" placeholder="https://..." value={url} onChange={(e) => setUrl(e.target.value)} autoFocus />
        <button type="button" className="btn" onClick={paste}>
          Paste
        </button>
        <button className="btn primary" disabled={!url.trim() || fetching}>
          {fetching ? <div className="spinner" /> : "Look up"}
        </button>
      </form>
      {error && (
        <p className="error" style={{ maxWidth: 820 }}>
          {error}
        </p>
      )}

      {info && (
        <div className="link-card card">
          {info.thumbnail ? <img className="thumb" src={info.thumbnail} alt="" referrerPolicy="no-referrer" /> : <div className="thumb" />}
          <div>
            <h2>{info.title ?? info.url}</h2>
            <div className="muted" style={{ marginBottom: 14 }}>
              {[info.uploader, info.site, info.duration ? clock(info.duration) : null, info.isPlaylist ? `${info.entries} videos` : null]
                .filter(Boolean)
                .join(" · ")}
              {"  "}
              <a href="#" className="faint" onClick={(e) => (e.preventDefault(), openUrl(info.url))}>
                open page
              </a>
            </div>

            <div className="field">
              <div className="label">What to save</div>
              <div className="chips">
                <button className={`chip${!audioOnly ? " on" : ""}`} onClick={() => setAudioOnly(false)}>
                  Video
                </button>
                <button className={`chip${audioOnly ? " on" : ""}`} onClick={() => setAudioOnly(true)}>
                  Audio only
                </button>
              </div>
            </div>

            {!audioOnly && heights.length > 0 && (
              <div className="field">
                <div className="label">Quality</div>
                <div className="chips">
                  <button className={`chip${height === null ? " on" : ""}`} onClick={() => setHeight(null)}>
                    {/* Sites rarely offer H.264 above 1080p. */}
                    Best ({compat ? `up to ${Math.min(heights[0], 1080)}` : heights[0]}p)
                  </button>
                  {heights.slice(1).map((h) => (
                    <button key={h} className={`chip${height === h ? " on" : ""}`} onClick={() => setHeight(h)}>
                      {h}p
                    </button>
                  ))}
                </div>
              </div>
            )}
            {audioOnly && (
              <div className="field">
                <div className="label">Audio format</div>
                <div className="chips">
                  {["mp3", "m4a", "opus", "original"].map((f) => (
                    <button key={f} className={`chip${audioFormat === f ? " on" : ""}`} onClick={() => setAudioFormat(f)}>
                      {f}
                    </button>
                  ))}
                </div>
              </div>
            )}
            {!audioOnly && (
              <label className="toggle" style={{ marginBottom: 14 }}>
                <input type="checkbox" checked={compat} onChange={(e) => setCompat(e.target.checked)} />
                <span>
                  Editor-friendly <span className="faint">(H.264 + AAC; plays and edits everywhere)</span>
                </span>
              </label>
            )}
            <div className="row">
              <button className="btn primary" onClick={start} disabled={!folder}>
                Download
              </button>
              <span className="faint ellipsis grow" title={folder}>
                to {folder}
              </span>
            </div>
          </div>
        </div>
      )}

      <details style={{ maxWidth: 820, marginTop: 18 }}>
        <summary className="muted" style={{ cursor: "pointer" }}>
          Settings
        </summary>
        <div className="card" style={{ marginTop: 10 }}>
          <div className="field">
            <div className="label">Save to</div>
            <div className="row">
              <input className="input grow" value={folder} onChange={(e) => setFolder(e.target.value)} />
              <button className="btn" onClick={chooseFolder}>
                Choose...
              </button>
            </div>
          </div>
          <div className="field" style={{ marginBottom: 0 }}>
            <div className="label">Use the login from this browser (for Instagram, private videos, age checks)</div>
            <select className="input" value={browser} onChange={(e) => setBrowser(e.target.value)}>
              {BROWSERS.map((b) => (
                <option key={b} value={b}>
                  {b ? b[0].toUpperCase() + b.slice(1) : "None"}
                </option>
              ))}
            </select>
            <span className="faint">Chrome, Edge and Brave lock their cookies while open; Firefox works best.</span>
          </div>
        </div>
      </details>

      {downloads.length > 0 && (
        <>
          <h3 className="section">Downloads</h3>
          <div className="list">
            {downloads.map((t) => (
              <div className="list-item" key={t.id}>
                <div style={{ minWidth: 0 }}>
                  <div className="ellipsis">{t.title}</div>
                  <div className={t.status === "failed" ? "error" : "faint"}>
                    {t.status === "done" && t.output ? baseName(t.output) : statusText(t)}
                  </div>
                </div>
                {t.status === "done" && t.output ? (
                  <div className="row">
                    <button className="btn small primary" onClick={() => onEdit(t.output!)}>
                      Edit
                    </button>
                    <FileActions path={t.output} />
                  </div>
                ) : (
                  <span />
                )}
                {t.status === "running" && <ProgressBar percent={t.percent} />}
              </div>
            ))}
          </div>
        </>
      )}
    </div>
  );
}
