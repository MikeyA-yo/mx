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
  type Music,
  type Range,
  type Segment,
  type Title,
  type Transcript,
} from "../api";
import { TaskProgress } from "../components";
import { BackIcon, PauseIcon, PlayIcon, ScissorsIcon } from "../icons";
import { baseName, bytes, clock, load } from "../util";
import Stage from "../editor/Stage";
import Timeline from "../editor/Timeline";
import CutPanel from "../editor/CutPanel";
import TextPanel from "../editor/TextPanel";
import TitlesPanel from "../editor/TitlesPanel";
import HighlightsPanel from "../editor/HighlightsPanel";
import FramePanel from "../editor/FramePanel";
import MusicPanel from "../editor/MusicPanel";
import ExportPanel from "../editor/ExportPanel";
import { outputSize, toOutput, type ClipSpec, type EditorApi, type Pauses, type ProjectApi } from "../editor/state";

const MEDIA_EXTENSIONS = ["mp4", "mkv", "mov", "webm", "m4v", "avi", "flv", "ts", "mp3", "m4a", "wav", "flac", "ogg", "opus", "aac"];

const chooseMedia = async (multiple: boolean): Promise<string[]> => {
  const picked = await open({ multiple, filters: [{ name: "Video and audio", extensions: MEDIA_EXTENSIONS }] });
  return picked === null ? [] : Array.isArray(picked) ? picked : [picked];
};

type Props = {
  /** Files to add, from elsewhere in the app; `n` makes repeats count. */
  request: { paths: string[]; n: number } | null;
  /** Opening from this page goes through the app, which remembers it. */
  onOpen: (paths: string[]) => void;
};

/** The project: clips in order, and what they share. */
export default function EditorPage({ request, onOpen }: Props) {
  const [clips, setClips] = useState<{ id: string; path: string }[]>([]);
  const [active, setActive] = useState<string | null>(null);
  const [specs, setSpecs] = useState<Record<string, ClipSpec>>({});
  const [frame, setFrame] = useState<Frame | null>(null);
  const [captionsOn, setCaptionsOn] = useState(false);
  const [style, setStyle] = useState<CaptionStyle | null>(null);
  const [music, setMusic] = useState<Music | null>(null);
  // Playing through: the clip to start next, and a counter so the same
  // clip can be asked twice.
  const [playNext, setPlayNext] = useState<{ id: string; n: number } | null>(null);

  const add = useCallback((paths: string[]) => {
    if (paths.length === 0) return;
    const added = paths.map((path) => ({ id: newTaskId(), path }));
    setClips((c) => [...c, ...added]);
    setActive(added[0].id);
  }, []);

  useEffect(() => {
    if (request) add(request.paths);
  }, [request, add]);

  const ordered = clips.map((c) => specs[c.id]).filter((s): s is ClipSpec => !!s);
  const first = ordered[0]?.media;

  // Caption placement follows the output frame's orientation.
  const [outW, outH] = first ? outputSize(frame, first) : [1920, 1080];
  const portrait = outH > outW;
  useEffect(() => {
    if (!first) return;
    captionDefaults(outW, outH).then((d) => setStyle((s) => (s ? { ...s, position: d.position } : d)));
  }, [!!first, portrait]);

  const project: ProjectApi = { clips: ordered, frame, setFrame, captionsOn, setCaptionsOn, style, setStyle, music, setMusic };

  const close = () => {
    setClips([]);
    setSpecs({});
    setActive(null);
    setFrame(null);
    setMusic(null);
    setCaptionsOn(false);
  };
  const remove = (id: string) => {
    const i = clips.findIndex((c) => c.id === id);
    const rest = clips.filter((c) => c.id !== id);
    setClips(rest);
    setSpecs(({ [id]: _, ...others }) => others);
    if (active === id) setActive(rest[Math.min(i, rest.length - 1)]?.id ?? null);
  };
  const move = (id: string, by: number) => {
    const i = clips.findIndex((c) => c.id === id);
    const j = i + by;
    if (j < 0 || j >= clips.length) return;
    const next = [...clips];
    [next[i], next[j]] = [next[j], next[i]];
    setClips(next);
  };
  const onEnded = (id: string) => {
    const i = clips.findIndex((c) => c.id === id);
    const next = clips[i + 1];
    if (!next) return;
    setActive(next.id);
    setPlayNext((p) => ({ id: next.id, n: (p?.n ?? 0) + 1 }));
  };
  const onSpec = useCallback((spec: ClipSpec) => setSpecs((s) => ({ ...s, [spec.id]: spec })), []);

  if (clips.length === 0) return <Empty onOpen={onOpen} />;

  let offset = 0;
  const offsets: Record<string, number> = {};
  for (const c of clips) {
    offsets[c.id] = offset;
    offset += specs[c.id]?.outputDuration ?? 0;
  }

  const sequence = (
    <div className="sequence">
      {clips.map((c, i) => {
        const spec = specs[c.id];
        return (
          <div key={c.id} className={`seq-clip${c.id === active ? " active" : ""}`} onClick={() => setActive(c.id)} title={c.path}>
            <span className="n">{i + 1}</span>
            <span className="ellipsis">{baseName(c.path)}</span>
            <span className="faint">{spec ? clock(spec.outputDuration) : "…"}</span>
            {clips.length > 1 && (
              <>
                <button title="Move earlier" onClick={(e) => (e.stopPropagation(), move(c.id, -1))} disabled={i === 0}>
                  ‹
                </button>
                <button title="Move later" onClick={(e) => (e.stopPropagation(), move(c.id, 1))} disabled={i === clips.length - 1}>
                  ›
                </button>
              </>
            )}
            <button title="Remove from the project" onClick={(e) => (e.stopPropagation(), remove(c.id))}>
              ✕
            </button>
          </div>
        );
      })}
      <button className="btn small" onClick={async () => onOpen(await chooseMedia(true))}>
        + Add clip
      </button>
      <span className="faint" style={{ marginLeft: "auto", whiteSpace: "nowrap" }}>
        {clips.length > 1 ? `${clips.length} clips · ${clock(offset)}` : ""}
      </span>
    </div>
  );

  return (
    <>
      {clips.map((c) => (
        <div key={c.id} style={{ display: c.id === active ? "contents" : "none" }}>
          <Workspace
            id={c.id}
            path={c.path}
            active={c.id === active}
            project={project}
            outputOffset={offsets[c.id] ?? 0}
            playRequest={playNext?.id === c.id ? playNext.n : 0}
            sequence={sequence}
            onSpec={onSpec}
            onEnded={() => onEnded(c.id)}
            onClose={close}
          />
        </div>
      ))}
    </>
  );
}

function Empty({ onOpen }: { onOpen: (paths: string[]) => void }) {
  const recent = load<string[]>("recent", []);
  return (
    <div className="drop">
      <div>
        <div className="box">
          <h1 style={{ margin: "0 0 6px" }}>Open a video</h1>
          <p className="muted" style={{ margin: "0 0 18px" }}>
            Drop files anywhere on this window, or pick some. Several files become clips in one video.
          </p>
          <button className="btn primary" onClick={async () => onOpen(await chooseMedia(true))}>
            Choose files...
          </button>
        </div>
        {recent.length > 0 && (
          <div className="recent">
            <h3 className="section">Recent</h3>
            {recent.map((p) => (
              <button key={p} onClick={() => onOpen([p])} title={p} className="ellipsis">
                {baseName(p)}
              </button>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

type Tab = "cut" | "text" | "titles" | "highlights" | "frame" | "music" | "export";

type WorkspaceProps = {
  id: string;
  path: string;
  active: boolean;
  project: ProjectApi;
  /** Where this clip starts in the output, for music sync. */
  outputOffset: number;
  /** Changes when the project wants this clip to start playing. */
  playRequest: number;
  sequence: React.ReactNode;
  onSpec: (spec: ClipSpec) => void;
  onEnded: () => void;
  onClose: () => void;
};

/** One clip's editor. Stays mounted while other clips are shown, so its
 * work survives switching. */
function Workspace(p: WorkspaceProps) {
  const { path, project } = p;
  const [media, setMedia] = useState<MediaSummary | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [previewTask, setPreviewTask] = useState<string | null>(null);
  const [src, setSrc] = useState<string | null>(null);
  const [strip, setStrip] = useState<Filmstrip | null>(null);
  const [wave, setWave] = useState<number[] | null>(null);

  const video = useRef<HTMLVideoElement>(null);
  const audio = useRef<HTMLAudioElement>(null);
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
  const [titles, setTitles] = useState<Title[]>([]);
  const [selectedTitle, setSelectedTitle] = useState<number | null>(null);
  const [highlights, setHighlights] = useState<Highlight[] | null>(null);
  const [skipCuts, setSkipCuts] = useState(true);
  const [plan, setPlan] = useState<EditPlan>({ cuts: [], keep: [], outputDuration: 0 });
  const [cues, setCues] = useState<Segment[]>([]);

  // Undo covers the edits made by hand: cuts, deleted lines and titles.
  type Snapshot = { manualCuts: Range[]; removed: number[]; titles: Title[] };
  const history = useRef<Snapshot[]>([]);
  const remember = () => {
    history.current.push({ manualCuts, removed, titles });
    if (history.current.length > 100) history.current.shift();
  };
  const undo = () => {
    const last = history.current.pop();
    if (last) {
      setManualCuts(last.manualCuts);
      setRemoved(last.removed);
      setTitles(last.titles);
    }
  };

  // Load: probe first, then preview, filmstrip and waveform side by side.
  useEffect(() => {
    let live = true;
    probeMedia(path)
      .then((m) => {
        if (!live) return;
        setMedia(m);
        setPlan({ cuts: [], keep: [{ start: 0, end: m.duration }], outputDuration: m.duration });
        const pt = newTaskId();
        setPreviewTask(pt);
        preparePreview(pt, path)
          .then((pv) => live && setSrc(convertFileSrc(pv.src)))
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

  // What this clip gives the export.
  const keptTranscript = useMemo(
    () => (transcript ? { segments: transcript.segments.filter((_, i) => !removed.includes(i)) } : null),
    [transcript, removed],
  );
  const spec: ClipSpec | null = useMemo(
    () =>
      media && {
        id: p.id,
        media,
        keep: plan.keep.length ? plan.keep : [{ start: 0, end: media.duration }],
        outputDuration: plan.keep.length ? plan.outputDuration : media.duration,
        transcript: keptTranscript,
        titles,
      },
    [media, plan, keptTranscript, titles, p.id],
  );
  const onSpec = p.onSpec;
  useEffect(() => {
    if (spec) onSpec(spec);
  }, [spec, onSpec]);

  // Caption phrases on the source timeline, for the preview overlay.
  const style = project.style;
  useEffect(() => {
    if (!keptTranscript || !style) return setCues([]);
    const t = setTimeout(() => captionCues(keptTranscript, style, []).then(setCues), 120);
    return () => clearTimeout(t);
  }, [keptTranscript, style]);

  // Playback clock, skipping what's cut, handing over to the next clip at
  // the end, and keeping the music in step.
  const live = useRef({ plan, skipCuts, active: p.active, onEnded: p.onEnded, music: project.music, offset: p.outputOffset });
  live.current = { plan, skipCuts, active: p.active, onEnded: p.onEnded, music: project.music, offset: p.outputOffset };
  useEffect(() => {
    const v = video.current;
    if (!v) return;
    let frameId = 0;
    let shown = -1;
    const end = () => {
      v.pause();
      live.current.onEnded();
    };
    const tick = () => {
      const t = v.currentTime;
      const { plan, skipCuts, music, offset } = live.current;
      if (skipCuts && !v.paused) {
        const cut = plan.cuts.find((c) => t >= c.start && t < c.end - 0.02);
        if (cut) {
          if (cut.end >= v.duration - 0.05) end();
          else v.currentTime = cut.end;
        }
      }
      const a = audio.current;
      if (a && music && a.duration > 0) {
        // Where the music should be: the song offset plus this moment's
        // place in the whole output, wrapped like the export's loop.
        const want = (music.offset + offset + toOutput(plan.keep, t)) % a.duration;
        if (!v.paused && a.paused) a.play().catch(() => {});
        if (v.paused && !a.paused) a.pause();
        if (Math.abs(a.currentTime - want) > 0.3) a.currentTime = want;
        a.volume = Math.min(1, music.volume);
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
    const onEnd = () => live.current.onEnded();
    v.addEventListener("play", onPlay);
    v.addEventListener("pause", onPause);
    v.addEventListener("ended", onEnd);
    frameId = requestAnimationFrame(tick);
    return () => {
      cancelAnimationFrame(frameId);
      v.removeEventListener("play", onPlay);
      v.removeEventListener("pause", onPause);
      v.removeEventListener("ended", onEnd);
    };
  }, [src]);

  // Hidden clips stop; a clip asked to play starts at its first kept moment.
  useEffect(() => {
    if (!p.active) {
      video.current?.pause();
      audio.current?.pause();
    }
  }, [p.active]);
  useEffect(() => {
    const v = video.current;
    if (!p.playRequest || !v) return;
    v.currentTime = plan.keep[0]?.start ?? 0;
    v.play().catch(() => {});
  }, [p.playRequest]);

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

  // Keyboard: the usual editor keys, for the clip on screen, ignored while
  // typing.
  const keys = useRef<(e: KeyboardEvent) => void>(() => {});
  keys.current = (e: KeyboardEvent) => {
    if (!p.active || !media) return;
    const target = e.target as HTMLElement;
    if (["INPUT", "SELECT", "TEXTAREA"].includes(target.tagName) && (target as HTMLInputElement).type !== "range") return;
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
    } else if (k === "escape") {
      setSelection(null);
      setSelectedTitle(null);
    } else if (k === "z" && (e.ctrlKey || e.metaKey)) undo();
    else return;
    e.preventDefault();
  };
  useEffect(() => {
    const handler = (e: KeyboardEvent) => keys.current(e);
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, []);

  const ed: EditorApi | null =
    media && spec
      ? {
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
          titles,
          setTitles: (t: Title[]) => {
            remember();
            setTitles(t);
          },
          selectedTitle,
          setSelectedTitle,
          highlights,
          setHighlights,
          skipCuts,
          setSkipCuts,
          project,
          spec,
        }
      : null;

  if (loadError && !media) {
    return (
      <div className="drop">
        <div className="box">
          <h1 style={{ margin: "0 0 8px" }}>Can't open {baseName(path)}</h1>
          <p className="error">{loadError}</p>
          <button className="btn" onClick={p.onClose}>
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
    ["titles", "Titles", titles.length > 0],
    ["highlights", "Picks", !!highlights?.length],
    ["frame", "Frame", !!project.frame],
    ["music", "Music", !!project.music],
    ["export", "Export", false],
  ];

  return (
    <div className="editor">
      <div className="editor-top">
        <button className="btn small ghost" onClick={p.onClose} title="Close the project">
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
            frame={project.frame}
            cues={cues}
            style={style}
            showCaptions={project.captionsOn}
            time={time}
            videoRef={video}
            onLoaded={() => setTime(video.current?.currentTime ?? 0)}
            titles={titles}
            selectedTitle={tab === "titles" ? selectedTitle : null}
            onSelectTitle={(i) => {
              setSelectedTitle(i);
              setTab("titles");
            }}
            onMoveTitle={(i, x, y) => setTitles((all) => all.map((t, j) => (j === i ? { ...t, x, y } : t)))}
          />
        ) : (
          <div className="stage-view">
            <div style={{ width: 360 }}>
              <TaskProgress id={previewTask} label="preparing the preview" />
              {!previewTask && !loadError && <div className="spinner" />}
            </div>
          </div>
        )}
        {project.music && <audio ref={audio} src={convertFileSrc(project.music.source)} preload="auto" loop />}
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
          {tabs.map(([id, label, on]) => (
            <button key={id} className={tab === id ? "active" : ""} onClick={() => setTab(id)}>
              {label}
              {on && <span className="dot" />}
            </button>
          ))}
        </div>
        {tab === "cut" && <CutPanel ed={ed} />}
        {tab === "text" && <TextPanel ed={ed} />}
        {tab === "titles" && <TitlesPanel ed={ed} />}
        {tab === "highlights" && <HighlightsPanel ed={ed} />}
        {tab === "frame" && <FramePanel ed={ed} />}
        {tab === "music" && <MusicPanel ed={ed} />}
        {tab === "export" && <ExportPanel ed={ed} />}
      </aside>

      <div className="timeline">
        {p.sequence}
        <Timeline
          duration={media.duration}
          time={time}
          playing={playing}
          strip={strip}
          wave={wave}
          cuts={plan.cuts}
          selection={selection}
          highlights={highlights ?? []}
          titles={titles}
          zoom={zoom}
          onSeek={seek}
          onSelect={setSelection}
          onPickTitle={(i) => {
            setSelectedTitle(i);
            setTab("titles");
          }}
        />
      </div>
    </div>
  );
}
