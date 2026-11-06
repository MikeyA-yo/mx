import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import {
  captionCues,
  captionDefaults,
  clearAutosave,
  errorText,
  filmstrip,
  loadProject,
  newTaskId,
  planEdit,
  preparePreview,
  previewTransition,
  probeMedia,
  saveProject,
  waveform,
  type CaptionStyle,
  type EditPlan,
  type Filmstrip,
  type Frame,
  type Highlight,
  type Keyframe,
  type MediaSummary,
  type Music,
  type Range,
  type Overlay,
  type Segment,
  type Sound,
  type Title,
  type Transcript,
  type Transition,
  type TransitionKind,
} from "../api";
import { TaskProgress } from "../components";
import { BackIcon, PauseIcon, PlayIcon, ScissorsIcon } from "../icons";
import { baseName, bytes, clock, dirName, join, load, stem, store } from "../util";
import Stage from "../editor/Stage";
import Timeline from "../editor/Timeline";
import CutPanel from "../editor/CutPanel";
import TextPanel from "../editor/TextPanel";
import OverlaysPanel from "../editor/OverlaysPanel";
import HighlightsPanel from "../editor/HighlightsPanel";
import FramePanel from "../editor/FramePanel";
import AudioPanel from "../editor/AudioPanel";
import ExportPanel from "../editor/ExportPanel";
import { coreProject, projectFile, readProjectFile } from "../editor/exporting";
import {
  motionAt,
  outputSize,
  toOutput,
  type ClipEditorState,
  type ClipSpec,
  type EditorApi,
  type Pauses,
  type PlacedOverlay,
  type PlacedSound,
  type ProjectApi,
} from "../editor/state";

const MEDIA_EXTENSIONS = ["mp4", "mkv", "mov", "webm", "m4v", "avi", "flv", "ts", "mp3", "m4a", "wav", "flac", "ogg", "opus", "aac"];

const chooseMedia = async (multiple: boolean): Promise<string[]> => {
  const picked = await open({ multiple, filters: [{ name: "Video and audio", extensions: MEDIA_EXTENSIONS }] });
  return picked === null ? [] : Array.isArray(picked) ? picked : [picked];
};

const TRANSITIONS: [TransitionKind | "", string][] = [
  ["", "Cut"],
  ["fade", "Crossfade"],
  ["fadeblack", "Through black"],
  ["wipeleft", "Wipe"],
  ["slideleft", "Slide"],
  ["circleopen", "Circle"],
  ["zoomin", "Zoom"],
];

/** How much a transition overlaps its neighbours: what the engine does
 * (at most half of either clip), so the preview's clock agrees. */
const overlapOf = (t: Transition | null, before: number, after: number) => {
  const d = Math.min(t?.duration ?? 0, before / 2, after / 2);
  return d < 0.04 ? 0 : d;
};

/** A clip in the project; `initial` restores a saved editor. */
type Saved = { editor: ClipEditorState; titles: Title[]; motion: Keyframe[]; volume: number; sounds: Sound[]; overlays: Overlay[] };
type ClipEntry = { id: string; path: string; initial?: Saved };

type Props = {
  /** Files to add, from elsewhere in the app; `n` makes repeats count. */
  request: { paths: string[]; n: number } | null;
  /** Opening from this page goes through the app, which remembers it. */
  onOpen: (paths: string[]) => void;
};

/** The project: clips in order, and what they share. */
export default function EditorPage({ request, onOpen }: Props) {
  const [clips, setClips] = useState<ClipEntry[]>([]);
  const [transitions, setTransitions] = useState<Record<string, Transition | null>>({});
  const [projectPath, setProjectPath] = useState<string | null>(null);
  const [saved, setSaved] = useState<string | null>(null);
  const [fileError, setFileError] = useState<string | null>(null);
  // The transition preview being made or shown.
  const [preview, setPreview] = useState<{ task: string; index: number; src: string | null; error: string | null } | null>(null);
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

  const ordered = clips
    .map((c, i): ClipSpec | undefined => specs[c.id] && { ...specs[c.id], transition: i === 0 ? null : (transitions[c.id] ?? null) })
    .filter((s): s is ClipSpec => !!s);
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
    setTransitions({});
    setActive(null);
    setFrame(null);
    setMusic(null);
    setCaptionsOn(false);
    setProjectPath(null);
    setSaved(null);
    clearAutosave();
  };

  // Saving. The file is rebuilt from the clips' latest state each time;
  // `saved` is that JSON as last written, so changes show as unsaved.
  const ready = ordered.length === clips.length && clips.length > 0;
  const json = ready ? JSON.stringify(projectFile(project)) : null;
  const dirty = json !== null && json !== saved;
  const saveTo = async (path: string | null) => {
    if (!json) return;
    let target = path;
    if (!target) {
      const first = ordered[0].media.path;
      target = await save({ defaultPath: join(dirName(first), `${stem(first)}.mxp`), filters: [{ name: "mx project", extensions: ["mxp"] }] });
      if (!target) return;
    }
    try {
      await saveProject(target, JSON.parse(json));
      setProjectPath(target);
      setSaved(json);
      store("lastProject", target);
    } catch (e) {
      setFileError(errorText(e));
    }
  };
  const openFile = async (path: string | null) => {
    try {
      const raw = await loadProject(path);
      if (raw === null) return;
      const f = readProjectFile(raw);
      const entries = f.clips.map((c) => ({
        id: newTaskId(),
        path: c.source,
        initial: { editor: c.editor, titles: c.titles ?? [], motion: c.motion ?? [], volume: c.volume ?? 1, sounds: c.sounds ?? [], overlays: c.overlays ?? [] },
      }));
      setClips(entries);
      setSpecs({});
      setTransitions(Object.fromEntries(entries.map((e, i) => [e.id, f.clips[i].transition ?? null])));
      setActive(entries[0]?.id ?? null);
      setFrame(f.frame ?? null);
      setMusic(f.music ?? null);
      setCaptionsOn(f.editor?.captionsOn ?? !!f.captions);
      setStyle(f.editor?.style ?? f.captions ?? null);
      setProjectPath(path);
      setSaved(null);
      setFileError(null);
    } catch (e) {
      setFileError(errorText(e));
    }
  };

  // Autosave a moment after each change, so closing the window loses
  // nothing; the start screen offers it back.
  useEffect(() => {
    if (!json) return;
    const t = setTimeout(() => saveProject(null, JSON.parse(json)).catch(() => {}), 1500);
    return () => clearTimeout(t);
  }, [json]);

  // Ctrl+S saves, Ctrl+Shift+S saves as.
  const saveKeys = useRef<(e: KeyboardEvent) => void>(() => {});
  saveKeys.current = (e: KeyboardEvent) => {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s" && clips.length) {
      e.preventDefault();
      saveTo(e.shiftKey ? null : projectPath);
    }
  };
  useEffect(() => {
    const handler = (e: KeyboardEvent) => saveKeys.current(e);
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, []);
  const remove = (id: string) => {
    const i = clips.findIndex((c) => c.id === id);
    const rest = clips.filter((c) => c.id !== id);
    setClips(rest);
    setSpecs(({ [id]: _, ...others }) => others);
    setTransitions(({ [id]: _, ...others }) => others);
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

  if (clips.length === 0) return <Empty onOpen={onOpen} onOpenProject={openFile} error={fileError} />;

  // Each clip's start on the output, overlapping by its transition.
  let offset = 0;
  let previous = 0;
  const offsets: Record<string, number> = {};
  clips.forEach((c, i) => {
    const d = specs[c.id]?.outputDuration ?? 0;
    const o = i === 0 ? 0 : overlapOf(transitions[c.id] ?? null, previous, d);
    offset -= o;
    offsets[c.id] = offset;
    offset += d;
    previous = d;
  });
  // Every sound on the output, for the preview: each starts where its
  // clip's moment plays and runs its length, past the clip if it's long.
  const placedSounds: PlacedSound[] = ordered.flatMap((spec) =>
    spec.sounds.map((s) => ({
      src: s.source,
      start: (offsets[spec.id] ?? 0) + toOutput(spec.keep, s.at),
      length: s.duration ?? 0,
      offset: s.offset,
      volume: s.volume,
      fadeIn: s.fade_in,
      fadeOut: s.fade_out,
    })),
  );
  const placedOverlays: PlacedOverlay[] = ordered.flatMap((spec) =>
    spec.overlays.map((o) => ({
      src: o.source,
      start: (offsets[spec.id] ?? 0) + toOutput(spec.keep, o.at),
      length: o.duration ?? 0,
      offset: o.offset,
      pip: o.pip,
      volume: o.volume,
      fade: o.fade,
    })),
  );

  const sequence = (
    <div className="sequence">
      {clips.map((c, i) => {
        const spec = specs[c.id];
        const t = transitions[c.id] ?? null;
        return (
          <div key={c.id} className="row" style={{ gap: 6, flex: "none" }}>
          {i > 0 && (
            <select
              className="seq-transition"
              title="How this clip takes over from the one before"
              value={t?.kind ?? ""}
              onChange={(e) => {
                const kind = e.target.value as TransitionKind | "";
                setTransitions((all) => ({ ...all, [c.id]: kind ? { kind, duration: t?.duration ?? 0.6 } : null }));
              }}
            >
              {TRANSITIONS.map(([kind, label]) => (
                <option key={kind} value={kind}>
                  {label}
                </option>
              ))}
            </select>
          )}
          {i > 0 && t && (
            <button
              className="btn small ghost"
              title="Preview this transition"
              disabled={!ready || preview?.src === null}
              onClick={async () => {
                const task = newTaskId();
                setPreview({ task, index: i, src: null, error: null });
                try {
                  const src = await previewTransition(task, coreProject(project), i);
                  setPreview((pv) => (pv?.task === task ? { ...pv, src: convertFileSrc(src) } : pv));
                } catch (e) {
                  setPreview((pv) => (pv?.task === task ? { ...pv, error: errorText(e) } : pv));
                }
              }}
            >
              ▶
            </button>
          )}
          <div className={`seq-clip${c.id === active ? " active" : ""}`} onClick={() => setActive(c.id)} title={c.path}>
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

  const toolbar = (
    <>
      <span className="faint ellipsis" title={projectPath ?? undefined}>
        {projectPath ? baseName(projectPath) : "Unsaved project"}
        {dirty ? " •" : ""}
      </span>
      {fileError && <span className="error ellipsis">{fileError}</span>}
      <button className="btn small" onClick={() => saveTo(projectPath)} title="Save (Ctrl+S)" disabled={!ready}>
        Save
      </button>
      <button className="btn small ghost" onClick={() => saveTo(null)} title="Save as (Ctrl+Shift+S)" disabled={!ready}>
        Save as
      </button>
      <button
        className="btn small ghost"
        onClick={async () => {
          const path = await open({ filters: [{ name: "mx project", extensions: ["mxp"] }] });
          if (typeof path === "string") openFile(path);
        }}
      >
        Open
      </button>
    </>
  );

  return (
    <>
      {preview && (
        <div className="modal" onClick={() => setPreview(null)}>
          <div className="card" onClick={(e) => e.stopPropagation()}>
            <div className="row" style={{ marginBottom: 10 }}>
              <strong className="grow">Transition into clip {preview.index + 1}</strong>
              <button className="btn small ghost" onClick={() => setPreview(null)}>
                Close
              </button>
            </div>
            {preview.src ? (
              <video src={preview.src} autoPlay loop controls />
            ) : preview.error ? (
              <p className="error">{preview.error}</p>
            ) : (
              <div style={{ width: 360 }}>
                <TaskProgress id={preview.task} label="rendering the preview" />
              </div>
            )}
            <p className="faint" style={{ margin: "8px 0 0" }}>
              Rendered by the export's own filters, at 360p.
            </p>
          </div>
        </div>
      )}
      {clips.map((c) => (
        <div key={c.id} style={{ display: c.id === active ? "contents" : "none" }}>
          <Workspace
            id={c.id}
            path={c.path}
            active={c.id === active}
            project={project}
            outputOffset={offsets[c.id] ?? 0}
            placedSounds={placedSounds}
            placedOverlays={placedOverlays}
            playRequest={playNext?.id === c.id ? playNext.n : 0}
            sequence={sequence}
            toolbar={toolbar}
            initial={c.initial}
            onSpec={onSpec}
            onEnded={() => onEnded(c.id)}
            onClose={close}
          />
        </div>
      ))}
    </>
  );
}

function Empty(p: { onOpen: (paths: string[]) => void; onOpenProject: (path: string | null) => void; error: string | null }) {
  const { onOpen } = p;
  const recent = load<string[]>("recent", []);
  const [autosave, setAutosave] = useState(false);
  useEffect(() => {
    loadProject(null).then((a) => setAutosave(a !== null)).catch(() => {});
  }, []);
  return (
    <div className="drop">
      <div>
        <div className="box">
          <h1 style={{ margin: "0 0 6px" }}>Open a video</h1>
          <p className="muted" style={{ margin: "0 0 18px" }}>
            Drop files anywhere on this window, or pick some. Several files become clips in one video.
          </p>
          <div className="row" style={{ justifyContent: "center" }}>
            <button className="btn primary" onClick={async () => onOpen(await chooseMedia(true))}>
              Choose files...
            </button>
            <button
              className="btn"
              onClick={async () => {
                const path = await open({ filters: [{ name: "mx project", extensions: ["mxp"] }] });
                if (typeof path === "string") p.onOpenProject(path);
              }}
            >
              Open project...
            </button>
          </div>
          {autosave && (
            <button className="btn ghost" style={{ marginTop: 14 }} onClick={() => p.onOpenProject(null)}>
              Restore last session
            </button>
          )}
          {p.error && <p className="error">{p.error}</p>}
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

type Tab = "cut" | "text" | "titles" | "highlights" | "frame" | "audio" | "export";

type WorkspaceProps = {
  id: string;
  path: string;
  active: boolean;
  project: ProjectApi;
  /** Where this clip starts in the output, for music sync. */
  outputOffset: number;
  /** The project's sounds on the output, to play in step. */
  placedSounds: PlacedSound[];
  placedOverlays: PlacedOverlay[];
  /** Changes when the project wants this clip to start playing. */
  playRequest: number;
  sequence: React.ReactNode;
  /** Project file controls, for the top bar. */
  toolbar: React.ReactNode;
  initial?: Saved;
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

  const saved = p.initial?.editor;
  const [pauses, setPauses] = useState<Pauses | null>(saved?.pauses ?? null);
  const [removePauses, setRemovePauses] = useState(saved?.removePauses ?? true);
  const [padding, setPadding] = useState(saved?.padding ?? 0.12);
  const [manualCuts, setManualCuts] = useState<Range[]>(saved?.manualCuts ?? []);
  const [transcript, setTranscript] = useState<Transcript | null>(saved?.transcript ?? null);
  const [removed, setRemoved] = useState<number[]>(saved?.removed ?? []);
  const [titles, setTitles] = useState<Title[]>(p.initial?.titles ?? []);
  const [selectedTitle, setSelectedTitle] = useState<number | null>(null);
  const [highlights, setHighlights] = useState<Highlight[] | null>(saved?.highlights ?? null);
  const [skipCuts, setSkipCuts] = useState(saved?.skipCuts ?? true);
  const [motion, setMotion] = useState<Keyframe[]>(p.initial?.motion ?? []);
  const [volume, setVolume] = useState(p.initial?.volume ?? 1);
  const [sounds, setSounds] = useState<Sound[]>(p.initial?.sounds ?? []);
  const soundEls = useRef<(HTMLAudioElement | null)[]>([]);
  const [overlays, setOverlays] = useState<Overlay[]>(p.initial?.overlays ?? []);
  const layerEls = useRef<(HTMLDivElement | null)[]>([]);
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
        transition: null,
        motion,
        volume,
        sounds,
        overlays,
        editor: { pauses, removePauses, padding, manualCuts, transcript, removed, highlights, skipCuts },
      },
    [media, plan, keptTranscript, titles, motion, volume, sounds, overlays, p.id, pauses, removePauses, padding, manualCuts, transcript, removed, highlights, skipCuts],
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
  const live = useRef({ plan, skipCuts, active: p.active, onEnded: p.onEnded, music: project.music, offset: p.outputOffset, sounds: p.placedSounds, volume, layers: p.placedOverlays });
  live.current = { plan, skipCuts, active: p.active, onEnded: p.onEnded, music: project.music, offset: p.outputOffset, sounds: p.placedSounds, volume, layers: p.placedOverlays };
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
      // Whatever goes wrong in one frame, the clock keeps running.
      try {
        step();
      } finally {
        frameId = requestAnimationFrame(tick);
      }
    };
    const step = () => {
      const t = v.currentTime;
      const { plan, skipCuts, music, offset, sounds, volume, layers } = live.current;
      // The web view can't play louder than as recorded; the export can.
      v.volume = Math.min(1, Math.max(0, volume));
      // Sounds: each plays while the output clock is inside it, faded.
      const now = offset + toOutput(plan.keep, t);
      sounds.forEach((s, k) => {
        const el = soundEls.current[k];
        if (!el) return;
        const inside = now >= s.start && now < s.start + s.length;
        if (!inside || v.paused) {
          if (!el.paused) el.pause();
          return;
        }
        const into = now - s.start;
        const want = s.offset + into;
        if (el.paused) {
          el.currentTime = want;
          el.play().catch(() => {});
        } else if (Math.abs(el.currentTime - want) > 0.3) el.currentTime = want;
        const fade = Math.min(1, s.fadeIn > 0 ? into / s.fadeIn : 1, s.fadeOut > 0 ? (s.length - into) / s.fadeOut : 1);
        el.volume = Math.max(0, Math.min(1, s.volume * fade));
      });
      // Overlays: shown (faded through opacity) while the output clock is
      // inside them; paused or playing with the clip.
      layers.forEach((o, k) => {
        const box = layerEls.current[k];
        if (!box) return;
        const into = now - o.start;
        const inside = into >= 0 && into < o.length;
        box.style.opacity = inside ? String(Math.min(1, o.fade > 0 ? Math.min(into, o.length - into) / o.fade : 1)) : "0";
        box.querySelectorAll("video").forEach((el) => {
          if (!inside) {
            if (!el.paused) el.pause();
            return;
          }
          const want = o.offset + into;
          if (v.paused) {
            if (!el.paused) el.pause();
            if (Math.abs(el.currentTime - want) > 0.05) el.currentTime = want;
          } else if (el.paused) {
            el.currentTime = want;
            el.play().catch(() => {});
          } else if (Math.abs(el.currentTime - want) > 0.3) el.currentTime = want;
          if (!el.muted) el.volume = Math.min(1, o.volume);
        });
      });
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
      soundEls.current.forEach((el) => el?.pause());
      layerEls.current.forEach((box) => box?.querySelectorAll("video").forEach((el) => el.pause()));
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
          motion,
          setMotion,
          volume,
          setVolume,
          sounds,
          setSounds,
          overlays,
          setOverlays,
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

  // With no frame chosen, the output takes the first clip's shape and
  // other clips are fitted into it with a blurred background: preview
  // them that way too.
  const first = project.clips[0]?.media ?? media;
  const stageFrame: Frame | null =
    project.frame ??
    (first.width && first.height && media.width && media.height && first.width * media.height !== first.height * media.width
      ? { aspect: { w: first.width, h: first.height }, fit: { kind: "blur" }, short_edge: Math.min(first.width, first.height) }
      : null);

  const tabs: [Tab, string, boolean][] = [
    ["cut", "Cut", plan.cuts.length > 0],
    ["text", "Text", !!transcript],
    ["titles", "Layers", titles.length > 0 || overlays.length > 0],
    ["highlights", "Picks", !!highlights?.length],
    ["frame", "Frame", !!project.frame || motion.length > 0],
    ["audio", "Audio", !!project.music || sounds.length > 0 || volume !== 1],
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
        {p.toolbar}
      </div>

      <div className="stage">
        {src ? (
          <Stage
            src={src}
            media={media}
            frame={stageFrame}
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
            view={motionAt(motion, time)}
            layers={p.placedOverlays}
            layerRefs={layerEls}
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
        {p.placedSounds.map((s, k) => (
          <audio key={`${k}-${s.src}`} ref={(el) => void (soundEls.current[k] = el)} src={convertFileSrc(s.src)} preload="auto" />
        ))}
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
        {tab === "titles" && <OverlaysPanel ed={ed} />}
        {tab === "highlights" && <HighlightsPanel ed={ed} />}
        {tab === "frame" && <FramePanel ed={ed} />}
        {tab === "audio" && <AudioPanel ed={ed} />}
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
          keyframes={motion.map((k) => k.time)}
          sounds={sounds.map((s) => ({ start: s.at, end: s.at + (s.duration ?? 0) }))}
          overlays={overlays.map((o) => ({ start: o.at, end: o.at + (o.duration ?? 0) }))}
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
