import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import {
  captionCues,
  captionDefaults,
  errorText,
  filmstrip,
  newTaskId,
  planEdit,
  preparePreview,
  probeMedia,
  waveform,
  type CaptionStyle,
  type EditPlan,
  type Filmstrip,
  type Frame,
  type Highlight,
  type MediaSummary,
  type Range,
  type Segment,
  type Transcript,
} from "../api";
import { TaskProgress } from "../components";
import { BackIcon, PauseIcon, PlayIcon, ScissorsIcon } from "../icons";
import { baseName, bytes, clock, load } from "../util";
import Stage from "../editor/Stage";
import Timeline from "../editor/Timeline";
import CutPanel from "../editor/CutPanel";
import TextPanel from "../editor/TextPanel";
import HighlightsPanel from "../editor/HighlightsPanel";
import FramePanel from "../editor/FramePanel";
import ExportPanel from "../editor/ExportPanel";
import { outputSize, type EditorApi, type Pauses } from "../editor/state";

const VIDEO_EXTENSIONS = ["mp4", "mkv", "mov", "webm", "m4v", "avi", "flv", "ts", "mp3", "m4a", "wav", "flac", "ogg", "opus", "aac"];

type Props = { file: string | null; onOpen: (path: string) => void; onClose: () => void };

export default function EditorPage({ file, onOpen, onClose }: Props) {
  if (!file) return <Empty onOpen={onOpen} />;
  return <Workspace key={file} path={file} onOpen={onOpen} onClose={onClose} />;
}

function Empty({ onOpen }: { onOpen: (path: string) => void }) {
  const recent = load<string[]>("recent", []);
  const choose = async () => {
    const path = await open({ filters: [{ name: "Video and audio", extensions: VIDEO_EXTENSIONS }] });
    if (typeof path === "string") onOpen(path);
  };
  return (
    <div className="drop">
      <div>
        <div className="box">
          <h1 style={{ margin: "0 0 6px" }}>Open a video</h1>
          <p className="muted" style={{ margin: "0 0 18px" }}>
            Drop a file anywhere on this window, or pick one.
          </p>
          <button className="btn primary" onClick={choose}>
            Choose a file...
          </button>
        </div>
        {recent.length > 0 && (
          <div className="recent">
            <h3 className="section">Recent</h3>
            {recent.map((p) => (
              <button key={p} onClick={() => onOpen(p)} title={p} className="ellipsis">
                {baseName(p)}
              </button>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

type Tab = "cut" | "text" | "highlights" | "frame" | "export";

function Workspace({ path, onOpen, onClose }: { path: string; onOpen: (path: string) => void; onClose: () => void }) {
  const [media, setMedia] = useState<MediaSummary | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [previewTask, setPreviewTask] = useState<string | null>(null);
  const [src, setSrc] = useState<string | null>(null);
  const [strip, setStrip] = useState<Filmstrip | null>(null);
  const [wave, setWave] = useState<number[] | null>(null);

  const video = useRef<HTMLVideoElement>(null);
  const [time, setTime] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [selection, setSelection] = useState<Range | null>(null);
  const [zoom, setZoom] = useState(1);
  const [tab, setTab] = useState<Tab>("cut");

  const [pauses, setPauses] = useState<Pauses | null>(null);
  const [removePauses, setRemovePauses] = useState(true);
  const [padding, setPadding] = useState(0.12);
  const [manualCuts, setManualCuts] = useState<Range[]>([]);
  const [transcript, setTranscript] = useState<Transcript | null>(null);
  const [removed, setRemoved] = useState<number[]>([]);
  const [highlights, setHighlights] = useState<Highlight[] | null>(null);
  const [frame, setFrame] = useState<Frame | null>(null);
  const [captionsOn, setCaptionsOn] = useState(false);
  const [style, setStyle] = useState<CaptionStyle | null>(null);
  const [skipCuts, setSkipCuts] = useState(true);
  const [plan, setPlan] = useState<EditPlan>({ cuts: [], keep: [], outputDuration: 0 });
  const [cues, setCues] = useState<Segment[]>([]);

  // Undo covers the edits made by hand: cuts and deleted lines.
  const history = useRef<{ manualCuts: Range[]; removed: number[] }[]>([]);
  const remember = () => {
    history.current.push({ manualCuts, removed });
    if (history.current.length > 100) history.current.shift();
  };
  const undo = () => {
    const last = history.current.pop();
    if (last) {
      setManualCuts(last.manualCuts);
      setRemoved(last.removed);
    }
  };

  // Load: probe first, then preview, filmstrip and waveform side by side.
  useEffect(() => {
    let live = true;
    probeMedia(path)
      .then((m) => {
        if (!live) return;
        setMedia(m);
        const pt = newTaskId();
        setPreviewTask(pt);
        preparePreview(pt, path)
          .then((p) => live && setSrc(convertFileSrc(p.src)))
          .catch((e) => live && setLoadError(`Couldn't make a preview: ${errorText(e)}`))
          .finally(() => live && setPreviewTask(null));
        if (m.hasVideo) filmstrip(newTaskId(), path).then((s) => live && setStrip(s)).catch(() => {});
        waveform(newTaskId(), path, 3000).then((w) => live && setWave(w)).catch(() => {});
      })
      .catch((e) => live && setLoadError(errorText(e)));
    return () => {
      live = false;
    };
  }, [path]);

  // Caption defaults follow the output frame's orientation.
  const [outW, outH] = media ? outputSize({ media, frame }) : [1920, 1080];
  const portrait = outH > outW;
  useEffect(() => {
    if (!media) return;
    captionDefaults(outW, outH).then((d) => setStyle((s) => (s ? { ...s, position: d.position } : d)));
  }, [media, portrait]);

  // Every tool's cuts, combined on the Rust side.
  useEffect(() => {
    if (!media) return;
    planEdit({
      duration: media.duration,
      silences: removePauses && pauses ? pauses.report.silences : [],
      padding,
      cuts: manualCuts,
      transcript,
      removed,
    }).then(setPlan);
  }, [media, pauses, removePauses, padding, manualCuts, transcript, removed]);

  // Caption phrases on the source timeline, for the preview overlay.
  useEffect(() => {
    if (!transcript || !style) return setCues([]);
    const kept = { segments: transcript.segments.filter((_, i) => !removed.includes(i)) };
    const t = setTimeout(() => captionCues(kept, style, []).then(setCues), 120);
    return () => clearTimeout(t);
  }, [transcript, removed, style]);

  // Playback clock, and skipping what's cut.
  const planRef = useRef(plan);
  planRef.current = plan;
  const skipRef = useRef(skipCuts);
  skipRef.current = skipCuts;
  useEffect(() => {
    const v = video.current;
    if (!v) return;
    let frameId = 0;
    let shown = -1;
    const tick = () => {
      const t = v.currentTime;
      if (skipRef.current && !v.paused) {
        const cut = planRef.current.cuts.find((c) => t >= c.start && t < c.end - 0.02);
        if (cut) {
          if (cut.end >= v.duration - 0.05) v.pause();
          else v.currentTime = cut.end;
        }
      }
      // About 30 updates a second: smooth, without re-rendering the
      // whole editor on every animation frame.
      if (Math.abs(t - shown) >= 0.033 || (v.paused && t !== shown)) {
        shown = t;
        setTime(t);
      }
      frameId = requestAnimationFrame(tick);
    };
    const onPlay = () => setPlaying(true);
    const onPause = () => setPlaying(false);
    v.addEventListener("play", onPlay);
    v.addEventListener("pause", onPause);
    frameId = requestAnimationFrame(tick);
    return () => {
      cancelAnimationFrame(frameId);
      v.removeEventListener("play", onPlay);
      v.removeEventListener("pause", onPause);
    };
  }, [src]);

  const seek = useCallback((t: number, play = false) => {
    const v = video.current;
    if (!v) return;
    v.currentTime = Math.max(0, t);
    setTime(v.currentTime);
    if (play) v.play();
  }, []);
  const togglePlay = () => {
    const v = video.current;
    if (!v) return;
    if (v.paused) v.play();
    else v.pause();
  };

  // Keyboard: the usual editor keys, ignored while typing.
  const keys = useRef<(e: KeyboardEvent) => void>(() => {});
  keys.current = (e: KeyboardEvent) => {
    const target = e.target as HTMLElement;
    if (["INPUT", "SELECT", "TEXTAREA"].includes(target.tagName) && (target as HTMLInputElement).type !== "range") return;
    if (!media) return;
    const step = media.fps ? 1 / media.fps : 1 / 30;
    const k = e.key.toLowerCase();
    if (k === " ") togglePlay();
    else if (k === "arrowleft") seek(time - (e.shiftKey ? 5 : 1));
    else if (k === "arrowright") seek(time + (e.shiftKey ? 5 : 1));
    else if (k === ",") seek(time - step);
    else if (k === ".") seek(time + step);
    else if (k === "i") setSelection({ start: time, end: Math.max(time + 0.1, selection?.end ?? media.duration) });
    else if (k === "o") setSelection({ start: Math.min(selection?.start ?? 0, time - 0.1), end: time });
    else if ((k === "x" || k === "delete" || k === "backspace") && selection) {
      remember();
      setManualCuts([...manualCuts, selection]);
      setSelection(null);
    } else if (k === "escape") setSelection(null);
    else if (k === "z" && (e.ctrlKey || e.metaKey)) undo();
    else return;
    e.preventDefault();
  };
  useEffect(() => {
    const handler = (e: KeyboardEvent) => keys.current(e);
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, []);

  const ed: EditorApi | null = useMemo(
    () =>
      media && {
        media,
        time,
        selection,
        setSelection,
        seek,
        plan,
        pauses,
        setPauses,
        removePauses,
        setRemovePauses,
        padding,
        setPadding,
        manualCuts,
        setManualCuts: (c: Range[]) => {
          remember();
          setManualCuts(c);
        },
        transcript,
        setTranscript,
        removed,
        setRemoved: (r: number[]) => {
          remember();
          setRemoved(r);
        },
        highlights,
        setHighlights,
        frame,
        setFrame,
        captionsOn,
        setCaptionsOn,
        style,
        setStyle,
        skipCuts,
        setSkipCuts,
      },
    [media, time, selection, plan, pauses, removePauses, padding, manualCuts, transcript, removed, highlights, frame, captionsOn, style, skipCuts],
  );

  if (loadError && !media) {
    return (
      <div className="drop">
        <div className="box">
          <h1 style={{ margin: "0 0 8px" }}>Can't open {baseName(path)}</h1>
          <p className="error">{loadError}</p>
          <button className="btn" onClick={onClose}>
            Back
          </button>
        </div>
      </div>
    );
  }
  if (!media || !ed) {
    return (
      <div className="drop">
        <div className="row muted">
          <div className="spinner" /> Reading {baseName(path)}...
        </div>
      </div>
    );
  }

  const tabs: [Tab, string, boolean][] = [
    ["cut", "Cut", plan.cuts.length > 0],
    ["text", "Text", !!transcript],
    ["highlights", "Highlights", !!highlights?.length],
    ["frame", "Frame", !!frame],
    ["export", "Export", false],
  ];

  return (
    <div className="editor">
      <div className="editor-top">
        <button className="btn small ghost" onClick={onClose} title="Close">
          <BackIcon className="" />
        </button>
        <span className="title ellipsis" title={media.path}>
          {media.name}
        </span>
        <span className="faint ellipsis">
          {[
            media.width && media.height ? `${media.width}×${media.height}` : null,
            media.fps ? `${Math.round(media.fps * 100) / 100} fps` : null,
            media.videoCodec,
            media.audioCodec,
            clock(media.duration),
            bytes(media.size),
          ]
            .filter(Boolean)
            .join(" · ")}
        </span>
        <span className="grow" />
        {loadError && <span className="error ellipsis">{loadError}</span>}
      </div>

      <div className="stage">
        {src ? (
          <Stage
            src={src}
            media={media}
            frame={frame}
            cues={cues}
            style={style}
            showCaptions={captionsOn}
            time={time}
            videoRef={video}
            onLoaded={() => setTime(video.current?.currentTime ?? 0)}
          />
        ) : (
          <div className="stage-view">
            <div style={{ width: 360 }}>
              <TaskProgress id={previewTask} label="preparing the preview" />
              {!previewTask && !loadError && <div className="spinner" />}
            </div>
          </div>
        )}
        <div className="transport">
          <button className="icon-btn big" onClick={togglePlay} title="Play/pause (Space)">
            {playing ? <PauseIcon /> : <PlayIcon />}
          </button>
          <span className="time">
            {clock(time, true)} <span className="faint">/ {clock(media.duration)}</span>
          </span>
          <span className="muted">
            {plan.cuts.length > 0 && (
              <>
                <ScissorsIcon className="" /> {clock(plan.outputDuration)} after cuts
              </>
            )}
          </span>
          <span className="grow" />
          {selection && (
            <span className="muted mono">
              {clock(selection.start, true)}–{clock(selection.end, true)}
            </span>
          )}
          <span className="faint">Zoom</span>
          <input type="range" min={1} max={40} step={0.5} value={zoom} onChange={(e) => setZoom(Number(e.target.value))} style={{ width: 110 }} />
        </div>
      </div>

      <aside className="side">
        <div className="tabs">
          {tabs.map(([id, label, active]) => (
            <button key={id} className={tab === id ? "active" : ""} onClick={() => setTab(id)}>
              {label}
              {active && <span className="dot" />}
            </button>
          ))}
        </div>
        {tab === "cut" && <CutPanel ed={ed} />}
        {tab === "text" && <TextPanel ed={ed} />}
        {tab === "highlights" && <HighlightsPanel ed={ed} />}
        {tab === "frame" && <FramePanel ed={ed} />}
        {tab === "export" && <ExportPanel ed={ed} onEdit={onOpen} />}
      </aside>

      <div className="timeline">
        <Timeline
          duration={media.duration}
          time={time}
          playing={playing}
          strip={strip}
          wave={wave}
          cuts={plan.cuts}
          selection={selection}
          highlights={highlights ?? []}
          zoom={zoom}
          onSeek={seek}
          onSelect={setSelection}
        />
      </div>
    </div>
  );
}
