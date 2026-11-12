import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { ask, open, save } from "@tauri-apps/plugin-dialog";
import {
  captionCues,
  captionDefaults,
  discardSession,
  projectsDir,
  errorText,
  filmstrip,
  hurryTask,
  loadProject,
  newTaskId,
  planEdit,
  preparePreview,
  previewTransition,
  probeMedia,
  saveProject,
  waveform,
  type CaptionStyle,
  type Denoise,
  type EditPlan,
  type Filmstrip,
  type Frame,
  type Highlight,
  type Keyframe,
  type MediaSummary,
  type Music,
  type Range,
  type Overlay,
  type Pip,
  type Segment,
  type Sound,
  type Title,
  type Transcript,
  type Transition,
  type TransitionKind,
} from "../api";
import { TaskProgress } from "../components";
import { BackIcon, PauseIcon, PlayIcon, RedoIcon, ScissorsIcon, UndoIcon } from "../icons";
import { offerTour, startTour } from "../tour";
import { IMAGE_EXTENSIONS, baseName, bytes, clock, dirName, isImage, join, load, rememberProject, stem, store } from "../util";
import RecentProjects from "../editor/RecentProjects";
import Stage from "../editor/Stage";
import Timeline from "../editor/Timeline";
import { DEFAULT_LAYOUT, setLayout, Splitter, useLayout } from "../editor/layout";
import Arrange from "../editor/Arrange";
import CutPanel from "../editor/CutPanel";
import TextPanel from "../editor/TextPanel";
import OverlaysPanel from "../editor/OverlaysPanel";
import HighlightsPanel from "../editor/HighlightsPanel";
import FramePanel from "../editor/FramePanel";
import AudioPanel from "../editor/AudioPanel";
import ExportPanel from "../editor/ExportPanel";
import QuickStart from "../editor/QuickStart";
import { useHistory, type Change, type History } from "../editor/history";
import { useLengths } from "../editor/lengths";
import { coreProject, projectFile, readProjectFile } from "../editor/exporting";
import {
  IMAGE_SECONDS,
  fromOutput,
  imageLayer,
  layerKey,
  motionAt,
  outputSize,
  toOutput,
  type ClipEditorState,
  type ClipSpec,
  type ClipBlock,
  type EditorApi,
  type LayerBar,
  type LayerRef,
  type Picture,
  type Pauses,
  type HeldTitle,
  type PlacedOverlay,
  type PlacedSound,
  type ProjectApi,
  addRange,
  heldFor,
  placeRange,
  removeRange,
} from "../editor/state";
import { rememberLength } from "../editor/lengths";

// What a selection's layer and sound buttons offer.
const LAYER_EXTENSIONS = [...IMAGE_EXTENSIONS, "mp4", "mov", "mkv", "webm", "m4v", "avi", "gif"];
const SOUND_EXTENSIONS = ["mp3", "m4a", "aac", "wav", "flac", "ogg", "opus", "mp4", "mov", "webm", "mkv"];

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

/** A clip's filmstrip and waveform, shared for the whole-video timeline. */
type Assets = { strip: Filmstrip | null; wave: number[] | null };
type Edge = "move" | "start" | "end";

/** The whole-video timeline: what it shows, in output time, and the
 * edits it makes across clips. The clip on screen draws it. */
type VideoTimeline = {
  mode: "video" | "clip";
  setMode: (m: "video" | "clip") => void;
  duration: number;
  pictures: Picture[];
  blocks: ClipBlock[];
  layers: LayerBar[];
  selection: Range | null;
  setSelection: (r: Range | null) => void;
  zoom: number;
  setZoom: (z: number) => void;
  /** Jumps to a moment of the output, in whichever clip plays it. */
  seek: (t: number, play?: boolean) => void;
  pickLayer: (bar: LayerBar | null) => void;
  moveLayer: (bar: LayerBar, start: number, end: number, edge: Edge) => void;
  moveBlock: (id: string, index: number) => void;
  trimBlock: (id: string, edge: "start" | "end", by: number) => void;
  /** Cuts a stretch of the output, from every clip it covers. */
  cut: (r: Range) => void;
  /** The clips' muted stretches, in output time. */
  mutes: Range[];
  /** Mutes (or unmutes) the clips' own sound over a stretch of the output. */
  mute: (r: Range, on: boolean) => void;
  /** Text over a stretch of the output, running on across clips. */
  addText: (r: Range) => void;
  /** A file as a layer or sound over a stretch of the output. */
  addFile: (r: Range, kind: "overlay" | "sound", path: string) => Promise<void>;
  /** How many clips a clip makes when split at its cuts. */
  pieces: (id: string) => number;
  splitAtCuts: (ids: string[]) => void;
  /** Every clip's own sound level, as one step. */
  volumeForAll: (v: number) => void;
};

/** A clip in the project; `initial` restores a saved editor. */
type Saved = {
  editor: ClipEditorState;
  titles: Title[];
  motion: Keyframe[];
  volume: number;
  mutes: Range[];
  denoise: Denoise;
  sounds: Sound[];
  overlays: Overlay[];
};
type ClipEntry = { id: string; path: string; initial?: Saved };

type Props = {
  /** Files to add, from elsewhere in the app; `n` makes repeats count. */
  request: { paths: string[]; n: number } | null;
  /** Opening from this page goes through the app, which remembers it. */
  onOpen: (paths: string[]) => void;
  /** How many clips are open, for Help. */
  onClips: (n: number) => void;
};

/** The project: clips in order, and what they share. */
export default function EditorPage({ request, onOpen, onClips }: Props) {
  const [clips, setClips] = useState<ClipEntry[]>([]);
  const [transitions, setTransitions] = useState<Record<string, Transition | null>>({});
  const [projectPath, setProjectPath] = useState<string | null>(null);
  const [saved, setSaved] = useState<string | null>(null);
  // This project's autosave: one file per project, so one never
  // overwrites another's.
  const [session, setSession] = useState(() => newTaskId());
  // A project just opened, or just started, counts as unchanged once its
  // clips have loaded and settled: only edits after that are unsaved.
  const cleanWhenReady = useRef(false);
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
  // The whole-video timeline: which view, its zoom and selection, and a
  // clip asked to jump to a moment.
  const [assets, setAssets] = useState<Record<string, Assets>>({});
  const onAssets = useCallback((id: string, a: Assets) => setAssets((all) => ({ ...all, [id]: a })), []);
  const [mode, setMode] = useState<"video" | "clip">(load("timeline.mode", "video"));
  // The clip on screen stays in view in the clip bar.
  useEffect(() => {
    const t = setTimeout(() => {
      const el = Array.from(document.querySelectorAll<HTMLElement>(".seq-clip.active")).find((e) => e.getClientRects().length);
      el?.scrollIntoView({ block: "nearest", inline: "nearest", behavior: "smooth" });
    }, 50);
    return () => clearTimeout(t);
  }, [active]);
  useEffect(() => store("timeline.mode", mode), [mode]);
  const [videoZoom, setVideoZoom] = useState(1);
  const [videoSelection, setVideoSelection] = useState<Range | null>(null);
  const [seekTo, setSeekTo] = useState<{ id: string; t: number; play: boolean; n: number } | null>(null);
  const [arranging, setArranging] = useState(false);

  // Undo: each clip registers its setters by id, so a step can be put back
  // even into a clip that was removed and has come back.
  const clipSetters = useRef<Record<string, Record<string, (v: never) => void>>>({});
  const register = useCallback((id: string, setters: Record<string, (v: never) => void>) => {
    clipSetters.current[id] = setters;
    return () => {
      if (clipSetters.current[id] === setters) delete clipSetters.current[id];
    };
  }, []);
  // Each clip's state as last seen, to bring a removed clip back as it was.
  const lastSaved = useRef<Record<string, Saved>>({});
  useEffect(() => {
    for (const s of Object.values(specs)) {
      lastSaved.current[s.id] = {
        editor: s.editor,
        titles: s.titles,
        motion: s.motion,
        volume: s.volume,
        mutes: s.mutes,
        denoise: s.denoise,
        sounds: s.sounds,
        overlays: s.overlays,
      };
    }
  }, [specs]);
  const restoreClips = (list: ClipEntry[]) => {
    setClips(list.map((c) => ({ ...c, initial: lastSaved.current[c.id] ?? c.initial })));
    setSpecs((all) => Object.fromEntries(Object.entries(all).filter(([id]) => list.some((c) => c.id === id))));
    setActive((a) => (list.some((c) => c.id === a) ? a : (list[0]?.id ?? null)));
  };
  const history = useHistory((scope, key, value) => {
    if (scope !== "project") return clipSetters.current[scope]?.[key]?.(value as never);
    const v = value as never;
    if (key === "frame") setFrame(v);
    else if (key === "music") setMusic(v);
    else if (key === "style") setStyle(v);
    else if (key === "captionsOn") setCaptionsOn(v);
    else if (key === "transitions") setTransitions(v);
    else if (key === "clips") restoreClips(v);
  });
  // Any edit ends the settling after a project opens: from then on it's
  // the user's change, to be autosaved and shown as unsaved.
  const recordEdit = history.record;
  const record = useCallback(
    (changes: Parameters<typeof recordEdit>[0]) => {
      cleanWhenReady.current = false;
      recordEdit(changes);
    },
    [recordEdit],
  );
  const editHistory: History = { ...history, record };
  /** A project setter that can be undone. */
  const tracked =
    <T,>(key: string, value: T, set: (v: T) => void) =>
    (next: T) => {
      record([{ scope: "project", key, before: value, after: next }]);
      set(next);
    };

  const handled = useRef(0);
  const clipsRef = useRef(clips);
  clipsRef.current = clips;
  const activeRef = useRef(active);
  activeRef.current = active;
  const add = useCallback(
    (all: string[]) => {
      // Images go over the clip on screen, as layers at its playhead.
      const images = all.filter(isImage);
      const paths = all.filter((p) => !isImage(p));
      if (images.length) {
        const addImages = clipSetters.current[activeRef.current ?? ""]?.addImages;
        if (addImages) addImages(images as never);
        else setFileError("Images go over a video: once a video is open, drop the image onto it (or use Layers, Add image).");
      }
      if (paths.length === 0) return;
      const added = paths.map((path) => ({ id: newTaskId(), path }));
      const fresh = clipsRef.current.length === 0;
      const next = [...clipsRef.current, ...added];
      record([{ scope: "project", key: "clips", before: clipsRef.current, after: next }]);
      // A new project: nothing to save until it's edited.
      if (fresh) cleanWhenReady.current = true;
      clipsRef.current = next;
      setClips(next);
      setActive(added[0].id);
    },
    [record],
  );

  useEffect(() => {
    // Each request once, however often this runs.
    if (!request || request.n === handled.current) return;
    handled.current = request.n;
    add(request.paths);
  }, [request, add]);
  useEffect(() => onClips(clips.length), [clips.length, onClips]);

  const ordered = clips
    .map((c, i): ClipSpec | undefined => specs[c.id] && { ...specs[c.id], transition: i === 0 ? null : (transitions[c.id] ?? null) })
    .filter((s): s is ClipSpec => !!s);
  const first = ordered[0]?.media;

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
  const lengths = useLengths(ordered.flatMap((c) => [...c.overlays.map((o) => o.source), ...c.sounds.map((s) => s.source)]).filter((p) => !isImage(p)));

  // Caption placement follows the output frame's orientation.
  const [outW, outH] = first ? outputSize(frame, first) : [1920, 1080];
  const portrait = outH > outW;
  useEffect(() => {
    if (!first) return;
    captionDefaults(outW, outH).then((d) => setStyle((s) => (s ? { ...s, position: d.position } : d)));
  }, [!!first, portrait]);

  const project: ProjectApi = {
    clips: ordered,
    frame,
    setFrame: tracked("frame", frame, setFrame),
    captionsOn,
    setCaptionsOn: tracked("captionsOn", captionsOn, setCaptionsOn),
    style,
    setStyle: tracked("style", style, setStyle),
    music,
    setMusic: tracked("music", music, setMusic),
  };
  const setTransitionsTracked = tracked("transitions", transitions, setTransitions);

  const close = async () => {
    if (dirty) {
      const sure = await ask(
        projectPath ? "Close this project? Changes since you last saved will be lost." : "Close this project? It hasn't been saved, so your edits will be lost.",
        { title: "Close project", kind: "warning", okLabel: "Close", cancelLabel: "Keep editing" },
      );
      if (!sure) return;
    }
    setClips([]);
    setSpecs({});
    setTransitions({});
    setActive(null);
    setFrame(null);
    setMusic(null);
    setCaptionsOn(false);
    setProjectPath(null);
    setSaved(null);
    history.clear();
    // Nothing unsaved: its autosave has nothing to offer back. (Closed
    // with unsaved changes, it stays in Recent projects, just in case.)
    if (!dirty) discardSession(session).catch(() => {});
    setSession(newTaskId());
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
      // Projects go to one place, Videos\mx\Projects, so they're easy to find again.
      const first = ordered[0].media.path;
      const dir = await projectsDir().catch(() => dirName(first));
      target = await save({ defaultPath: join(dir, `${stem(first)}.mxp`), filters: [{ name: "mx project", extensions: ["mxp"] }] });
      if (!target) return;
    }
    try {
      await saveProject(target, JSON.parse(json));
      setProjectPath(target);
      setSaved(json);
      rememberProject(target);
      // Saved in full: the autosave isn't needed until the next change.
      discardSession(session).catch(() => {});
    } catch (e) {
      setFileError(errorText(e));
    }
  };
  /** Opens a project file, or (with `restore`) an autosaved session, which
   * carries on autosaving where it was. */
  const openFile = async (path: string | null, restore?: { session: string; savedTo: string | null }) => {
    try {
      const raw = await loadProject(path);
      if (raw === null) return;
      const f = readProjectFile(raw);
      const entries = f.clips.map((c) => ({
        id: newTaskId(),
        path: c.source,
        initial: {
          editor: c.editor,
          titles: c.titles ?? [],
          motion: c.motion ?? [],
          volume: c.volume ?? 1,
          mutes: c.mutes ?? [],
          denoise: c.denoise ?? "off",
          sounds: c.sounds ?? [],
          overlays: c.overlays ?? [],
        },
      }));
      setClips(entries);
      setSpecs({});
      setTransitions(Object.fromEntries(entries.map((e, i) => [e.id, f.clips[i].transition ?? null])));
      setActive(entries[0]?.id ?? null);
      setFrame(f.frame ?? null);
      setMusic(f.music ?? null);
      setCaptionsOn(f.editor?.captionsOn ?? !!f.captions);
      setStyle(f.editor?.style ?? f.captions ?? null);
      setProjectPath(restore ? restore.savedTo : path);
      setSession(restore?.session ?? newTaskId());
      setSaved(null);
      // A file opens as saved; a session's changes are still unsaved.
      cleanWhenReady.current = !restore;
      if (path && !restore) rememberProject(path);
      setFileError(null);
      history.clear();
    } catch (e) {
      setFileError(errorText(e));
    }
  };

  useEffect(() => {
    if (!json || !cleanWhenReady.current) return;
    // Loading settles in a few steps (cuts planned, caption defaults), so
    // it's the state once that's quiet.
    const t = setTimeout(() => {
      cleanWhenReady.current = false;
      setSaved(json);
    }, 1000);
    return () => clearTimeout(t);
  }, [json]);

  // Autosave a moment after each unsaved change, so closing the window
  // loses nothing; the start screen offers it back. It notes the file it
  // belongs to, if there is one.
  useEffect(() => {
    if (!json || !dirty || cleanWhenReady.current) return;
    const t = setTimeout(() => {
      const f = JSON.parse(json);
      saveProject(null, { ...f, editor: { ...f.editor, savedTo: projectPath } }, session).catch(() => {});
    }, 1500);
    return () => clearTimeout(t);
  }, [json, dirty, session, projectPath]);

  // Ctrl+S saves, Ctrl+Shift+S saves as. Ctrl+Z undoes, Ctrl+Shift+Z or
  // Ctrl+Y redoes, except in a text box, which has its own.
  const saveKeys = useRef<(e: KeyboardEvent) => void>(() => {});
  saveKeys.current = (e: KeyboardEvent) => {
    if (!(e.ctrlKey || e.metaKey)) return;
    const k = e.key.toLowerCase();
    if (k === "s" && clips.length) {
      e.preventDefault();
      saveTo(e.shiftKey ? null : projectPath);
    } else if (k === "z" || k === "y") {
      const target = e.target as HTMLInputElement;
      if (target.tagName === "TEXTAREA" || (target.tagName === "INPUT" && !["range", "checkbox"].includes(target.type))) return;
      e.preventDefault();
      if (k === "y" || e.shiftKey) history.redo();
      else history.undo();
    }
  };
  useEffect(() => {
    const handler = (e: KeyboardEvent) => saveKeys.current(e);
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, []);
  const remove = (ids: string[]) => {
    const gone = new Set(ids);
    const i = clips.findIndex((c) => c.id === active);
    const rest = clips.filter((c) => !gone.has(c.id));
    const otherTransitions = Object.fromEntries(Object.entries(transitions).filter(([id]) => !gone.has(id)));
    record([
      { scope: "project", key: "clips", before: clips, after: rest },
      { scope: "project", key: "transitions", before: transitions, after: otherTransitions },
    ]);
    setClips(rest);
    setSpecs((all) => Object.fromEntries(Object.entries(all).filter(([id]) => !gone.has(id))));
    setTransitions(otherTransitions);
    // The clip on screen, if it went, gives way to the next one left.
    if (active && gone.has(active)) setActive((clips.slice(i).find((c) => !gone.has(c.id)) ?? rest[rest.length - 1])?.id ?? null);
  };
  /** The clips in a new order, by id. */
  const reorder = (order: string[]) => {
    const next = order.map((id) => clips.find((c) => c.id === id)).filter((c): c is ClipEntry => !!c);
    if (next.length !== clips.length || next.every((c, i) => c === clips[i])) return;
    record([{ scope: "project", key: "clips", before: clips, after: next }]);
    setClips(next);
  };
  /** Puts `entry` into the project right after clip `id`. */
  const insertAfter = (id: string, entry: ClipEntry) => {
    const i = clips.findIndex((c) => c.id === id);
    const next = [...clips.slice(0, i + 1), entry, ...clips.slice(i + 1)];
    record([{ scope: "project", key: "clips", before: clips, after: next }]);
    setClips(next);
    setActive(entry.id);
    history.seal();
  };
  // Splitting a clip at moments of its file makes clips of the same file,
  // one for each stretch between the moments, and each can go anywhere.
  // Layers go with the part they start in, and text running over a split
  // holds on into the parts after; all of it is timed on the file, so
  // nothing moves.
  const piecesOf = (s: Saved, spec: ClipSpec, points: number[]): Saved[] => {
    const d = spec.media.duration;
    const bounds = [0, ...points, d];
    const last = points.length;
    return bounds.slice(0, -1).map((from, j) => {
      const to = bounds[j + 1];
      const inside = (t: number) => (j === 0 || t >= from) && (j === last || t < to);
      const into = toOutput(spec.keep, to);
      const titles = s.titles
        .filter((t) => inside(t.start))
        .map((t) => (j < last && t.end > to ? { ...t, hold: toOutput(spec.keep, t.end) - into + heldFor(t, spec.keep, spec.outputDuration) } : t));
      const manualCuts = [...s.editor.manualCuts];
      if (j > 0) manualCuts.push({ start: 0, end: from });
      if (j < last) manualCuts.push({ start: to, end: d });
      return {
        ...s,
        editor: { ...s.editor, manualCuts },
        titles,
        sounds: s.sounds.filter((l) => inside(l.at)),
        overlays: s.overlays.filter((l) => inside(l.at)),
      };
    });
  };
  /** Splits clips at moments of their files, as one undo step. Each clip
   * stays as its first part, with the rest right after it. */
  const splitClips = (at: Record<string, number[]>) => {
    const also: Change[] = [];
    const next = clips.flatMap((c) => {
      const s = lastSaved.current[c.id];
      const spec = specs[c.id];
      const points = at[c.id];
      if (!s || !spec || !points?.length) return [c];
      const [first, ...rest] = piecesOf(s, spec, points);
      for (const key of ["titles", "sounds", "overlays"] as const) also.push({ scope: c.id, key, before: s[key], after: first[key] });
      also.push({ scope: c.id, key: "manualCuts", before: s.editor.manualCuts, after: first.editor.manualCuts });
      return [c, ...rest.map((initial) => ({ id: newTaskId(), path: c.path, initial }))];
    });
    if (!also.length) return next;
    record([{ scope: "project", key: "clips", before: clips, after: next }, ...also]);
    also.forEach((c) => clipSetters.current[c.scope]?.[c.key]?.(c.after as never));
    setClips(next);
    history.seal();
    return next;
  };
  const split = (id: string, at: number) => {
    const spec = specs[id];
    if (!spec) return;
    const into = toOutput(spec.keep, at);
    if (into < 0.1 || into > spec.outputDuration - 0.1) return;
    const next = splitClips({ [id]: [at] });
    // On to the second part, where the playhead is.
    setActive(next[next.findIndex((c) => c.id === id) + 1]?.id ?? id);
  };
  /** Where a clip splits into the parts kept between its own cuts: at the
   * start of each cut with something kept on both sides. */
  const cutPoints = (spec: ClipSpec) => {
    const cuts = [...spec.editor.manualCuts].sort((a, b) => a.start - b.start);
    const points: number[] = [];
    let reach = -1;
    let kept = 0;
    for (const r of cuts) {
      // Overlapping cuts are one cut.
      if (r.start <= reach) {
        reach = Math.max(reach, r.end);
        continue;
      }
      reach = r.end;
      const into = toOutput(spec.keep, r.start);
      if (into >= kept + 0.1 && into <= spec.outputDuration - 0.1) {
        points.push(r.start);
        kept = into;
      }
    }
    return points;
  };
  const splitAtCuts = (ids: string[]) =>
    splitClips(Object.fromEntries(ids.flatMap((id) => (specs[id] ? [[id, cutPoints(specs[id])]] : []))));
  // A box layer dragged on the preview of a later clip than its own.
  const moveOverlay = (clip: string, index: number, pip: Pip) => {
    const list = specs[clip]?.overlays;
    if (!list?.[index]) return;
    const next = list.map((o, i) => (i === index ? { ...o, pip } : o));
    record([{ scope: clip, key: "overlays", before: list, after: next }]);
    clipSetters.current[clip]?.overlays?.(next as never);
  };
  /** The same clip again, edits and all, right after it. */
  const duplicate = (id: string) => {
    const c = clips.find((e) => e.id === id);
    const s = lastSaved.current[id];
    if (c && s) insertAfter(id, { id: newTaskId(), path: c.path, initial: s });
  };
  // A clip pointed at its file's new place keeps everything done to it.
  const relink = (id: string, path: string) => {
    const next = clips.map((c) => (c.id === id ? { ...c, path } : c));
    record([{ scope: "project", key: "clips", before: clips, after: next }]);
    setClips(next);
  };
  const move = (id: string, by: number) => {
    const i = clips.findIndex((c) => c.id === id);
    const j = i + by;
    if (j < 0 || j >= clips.length) return;
    const next = [...clips];
    [next[i], next[j]] = [next[j], next[i]];
    record([{ scope: "project", key: "clips", before: clips, after: next }]);
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

  if (clips.length === 0) {
    // Removing the last clip can be undone from here too.
    const undo = history.undoName === "clips" ? history.undo : null;
    return <Empty onOpen={onOpen} onOpenProject={(path, restore) => openFile(path, restore)} error={fileError} onUndo={undo} />;
  }

  // The whole-video timeline. Everything on it is in output time; edits
  // go back into each clip's own time through its kept ranges.
  const startOf = (c: ClipSpec) => offsets[c.id] ?? 0;
  /** The clip playing at output time `t`: the later one where two overlap. */
  const clipAt = (t: number) => {
    let found: ClipSpec | undefined = ordered[0];
    for (const c of ordered) if (t >= startOf(c) - 1e-6) found = c;
    return found;
  };
  const sourceAt = (c: ClipSpec, t: number) => Math.min(c.media.duration, fromOutput(c.keep, Math.max(0, t - startOf(c))));
  const seekVideo = (t: number, play = false) => {
    const c = clipAt(t);
    if (!c) return;
    setActive(c.id);
    setSeekTo((q) => ({ id: c.id, t: sourceAt(c, t), play, n: (q?.n ?? 0) + 1 }));
  };
  /** Applies edits to clips' state as one undo step. */
  const commit = (changes: { id: string; key: string; before: unknown; after: unknown }[]) => {
    record(changes.map((c) => ({ scope: c.id, key: c.key, before: c.before, after: c.after })));
    changes.forEach((c) => clipSetters.current[c.id]?.[c.key]?.(c.after as never));
    history.seal();
  };
  const LISTS = { title: "titles", overlay: "overlays", sound: "sounds" } as const;
  const videoLayers: LayerBar[] = ordered.flatMap((c) => {
    const at = (t: number) => startOf(c) + toOutput(c.keep, t);
    const files = (kind: "overlay" | "sound", list: (Overlay | Sound)[]) =>
      list.map((l, index): LayerBar => {
        // An image has no length of its own: it shows as long as it's asked.
        const still = isImage(l.source);
        const fileLength = still ? Infinity : (lengths.get(l.source) ?? l.offset + (l.duration ?? 3));
        const start = at(l.at);
        return {
          clip: c.id,
          ref: { kind, index },
          start,
          end: start + (l.duration ?? (still ? IMAGE_SECONDS : fileLength - l.offset)),
          label: baseName(l.source),
          earliest: still ? 0 : start - l.offset,
          latest: start - l.offset + fileLength,
          bounded: false,
        };
      });
    return [
      ...c.titles.map(
        (t, index): LayerBar => ({
          clip: c.id,
          ref: { kind: "title", index },
          start: at(t.start),
          end: at(t.end) + heldFor(t, c.keep, c.outputDuration),
          label: t.text.trim() || "(empty text)",
          earliest: startOf(c),
          latest: offset,
          bounded: true,
        }),
      ),
      ...files("overlay", c.overlays),
      ...files("sound", c.sounds),
    ];
  });
  const video: VideoTimeline = {
    mode,
    setMode,
    duration: Math.max(0.1, offset),
    pictures: ordered.map((c) => ({
      start: startOf(c),
      end: startOf(c) + c.outputDuration,
      strip: assets[c.id]?.strip ?? null,
      wave: assets[c.id]?.wave ?? null,
      sourceDuration: c.media.duration,
      toSource: (t: number) => sourceAt(c, t),
    })),
    blocks: ordered.map((c) => {
      const i = clips.findIndex((e) => e.id === c.id);
      const t = i > 0 ? transitions[c.id] : null;
      return {
        id: c.id,
        n: i + 1,
        label: c.media.name,
        start: startOf(c),
        end: startOf(c) + c.outputDuration,
        active: c.id === active,
        transition: t ? `${TRANSITIONS.find(([k]) => k === t.kind)?.[1] ?? t.kind} from clip ${i}` : null,
      };
    }),
    layers: videoLayers,
    selection: videoSelection,
    setSelection: setVideoSelection,
    zoom: videoZoom,
    setZoom: setVideoZoom,
    seek: seekVideo,
    pickLayer: (bar) => {
      if (!bar) return clipSetters.current[active ?? ""]?.pickLayer?.(null as never);
      if (bar.clip !== active) seekVideo(bar.start);
      clipSetters.current[bar.clip]?.pickLayer?.(bar.ref as never);
    },
    // A layer dragged into another clip moves to it, keeping its length.
    moveLayer: (bar, start, end, edge) => {
      const c = ordered.find((s) => s.id === bar.clip);
      if (!c) return;
      const key = LISTS[bar.ref.kind];
      const list = c[key] as (Title | Overlay | Sound)[];
      const item = list[bar.ref.index];
      if (!item) return;
      const target = edge === "move" ? (clipAt(start + 1e-3) ?? c) : c;
      let next: Title | Overlay | Sound;
      if (bar.ref.kind === "title") {
        const t = item as Title;
        // Text that ends past its clip's end runs on over the clips after
        // it: it ends with the clip, then holds for the rest.
        const until = (into: ClipSpec, e: number) => {
          const last = startOf(into) + into.outputDuration;
          return e > last + 0.05 ? { end: sourceAt(into, last), hold: e - last } : { end: sourceAt(into, e), hold: 0 };
        };
        if (edge === "move") {
          const s = Math.max(start, startOf(target));
          next = { ...t, start: sourceAt(target, s), ...until(target, s + (end - start)) };
        } else if (edge === "start") next = { ...t, start: sourceAt(c, Math.min(start, startOf(c) + c.outputDuration - 0.1)) };
        else next = { ...t, ...until(c, end) };
      } else {
        const f = item as Overlay | Sound;
        next =
          edge === "move"
            ? { ...f, at: sourceAt(target, start) }
            : edge === "start"
              ? { ...f, at: sourceAt(c, start), offset: Math.max(0, f.offset + start - bar.start), duration: end - start }
              : { ...f, duration: end - start };
      }
      if (target.id === c.id) {
        commit([{ id: c.id, key, before: list, after: list.map((l, i) => (i === bar.ref.index ? next : l)) }]);
        clipSetters.current[c.id]?.pickLayer?.(bar.ref as never);
        return;
      }
      const into = target[key] as (Title | Overlay | Sound)[];
      commit([
        { id: c.id, key, before: list, after: list.filter((_, i) => i !== bar.ref.index) },
        { id: target.id, key, before: into, after: [...into, next] },
      ]);
      clipSetters.current[c.id]?.pickLayer?.(null as never);
      clipSetters.current[target.id]?.pickLayer?.({ kind: bar.ref.kind, index: into.length } as never);
      if (target.id !== active) seekVideo(start);
    },
    moveBlock: (id, index) => {
      const from = clips.findIndex((c) => c.id === id);
      if (from < 0 || from === index) return;
      const next = clips.filter((c) => c.id !== id);
      next.splice(index, 0, clips[from]);
      record([{ scope: "project", key: "clips", before: clips, after: next }]);
      setClips(next);
    },
    // Trimming a clip's edge is a cut from its start, or to its end: the
    // one made by Q or W, moved.
    trimBlock: (id, edge, by) => {
      const c = ordered.find((s) => s.id === id);
      if (!c) return;
      const d = c.media.duration;
      const cuts = c.editor.manualCuts;
      let next: Range[];
      if (edge === "start") {
        const s = by >= 0 ? fromOutput(c.keep, by) : Math.max(0, (c.keep[0]?.start ?? 0) + by);
        next = cuts.filter((r) => r.start > 1e-3);
        if (s > 0.05) next.push({ start: 0, end: Math.min(s, d - 0.1) });
      } else {
        const s = by <= 0 ? fromOutput(c.keep, c.outputDuration + by) : Math.min(d, (c.keep[c.keep.length - 1]?.end ?? d) + by);
        next = cuts.filter((r) => r.end < d - 1e-3);
        if (s < d - 0.05) next.push({ start: Math.max(s, 0.1), end: d });
      }
      commit([{ id, key: "manualCuts", before: cuts, after: next }]);
    },
    cut: (r) => {
      const changes = ordered.flatMap((c) => {
        const a = Math.max(r.start, startOf(c));
        const b = Math.min(r.end, startOf(c) + c.outputDuration);
        if (b - a < 0.02) return [];
        const cut = { start: sourceAt(c, a), end: sourceAt(c, b) };
        const cuts = c.editor.manualCuts;
        return [{ id: c.id, key: "manualCuts", before: cuts, after: [...cuts, cut] }];
      });
      if (changes.length) commit(changes);
    },
    mutes: ordered.flatMap((c) => c.mutes.flatMap((m) => placeRange(c.keep, m).map((x) => ({ start: x.start + startOf(c), end: x.end + startOf(c) })))),
    mute: (r, on) => {
      const changes = ordered.flatMap((c) => {
        const a = Math.max(r.start, startOf(c));
        const b = Math.min(r.end, startOf(c) + c.outputDuration);
        if (b - a < 0.02) return [];
        const s = { start: sourceAt(c, a), end: sourceAt(c, b) };
        return [{ id: c.id, key: "mutes", before: c.mutes, after: on ? addRange(c.mutes, s) : removeRange(c.mutes, s) }];
      });
      if (changes.length) commit(changes);
    },
    // Added in the clip the stretch starts in, and opened there.
    addText: (r) => {
      const c = clipAt(r.start + 1e-3);
      if (!c) return;
      const last = startOf(c) + c.outputDuration;
      const runsOn = r.end > last + 0.05;
      const title: Title = {
        text: "Your text",
        start: sourceAt(c, r.start),
        end: sourceAt(c, runsOn ? last : r.end),
        hold: runsOn ? r.end - last : 0,
        x: 0.5,
        y: 0.2,
        size: 0.07,
        color: "#ffffff",
        font: "Arial",
        boxed: false,
      };
      commit([{ id: c.id, key: "titles", before: c.titles, after: [...c.titles, title] }]);
      seekVideo(r.start);
      clipSetters.current[c.id]?.pickLayer?.({ kind: "title", index: c.titles.length } as never);
    },
    addFile: async (r, kind, path) => {
      const c = clipAt(r.start + 1e-3);
      if (!c) return;
      const at = sourceAt(c, r.start);
      const still = isImage(path);
      const fileLength = still ? Infinity : (await probeMedia(path)).duration;
      if (!still) rememberLength(path, fileLength);
      const duration = Math.max(0.1, Math.min(r.end - r.start, fileLength));
      const key = kind === "sound" ? "sounds" : "overlays";
      const list = c[key] as (Overlay | Sound)[];
      const item: Overlay | Sound =
        kind === "sound"
          ? { source: path, at, offset: 0, duration, volume: 1, fade_in: 0, fade_out: 0.3 }
          : still
            ? { ...imageLayer(path, at), duration }
            : { source: path, at, offset: 0, duration, pip: null, volume: 0, fade: 0.2 };
      commit([{ id: c.id, key, before: list, after: [...list, item] }]);
      seekVideo(r.start);
      clipSetters.current[c.id]?.pickLayer?.({ kind: kind === "sound" ? "sound" : "overlay", index: list.length } as never);
    },
    pieces: (id) => (specs[id] ? cutPoints(specs[id]).length + 1 : 1),
    splitAtCuts,
    volumeForAll: (volume) => {
      const changes = ordered.filter((c) => c.volume !== volume).map((c) => ({ id: c.id, key: "volume", before: c.volume, after: volume }));
      if (changes.length) commit(changes);
    },
  };
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
  // Text running on past its clip, for the preview of the clips after.
  const heldTitles: HeldTitle[] = ordered.flatMap((spec) => {
    const end = (offsets[spec.id] ?? 0) + spec.outputDuration;
    return spec.titles.flatMap((title) => {
      const hold = heldFor(title, spec.keep, spec.outputDuration);
      return hold > 0 ? [{ clip: spec.id, start: end, end: end + hold, title }] : [];
    });
  });
  const placedOverlays: PlacedOverlay[] = ordered.flatMap((spec) =>
    spec.overlays.map((o, index) => ({
      clip: spec.id,
      index,
      src: o.source,
      start: (offsets[spec.id] ?? 0) + toOutput(spec.keep, o.at),
      length: o.duration ?? (isImage(o.source) ? IMAGE_SECONDS : 0),
      offset: o.offset,
      pip: o.pip,
      volume: o.volume,
      fade: o.fade,
    })),
  );

  // The clip bar: the clips scroll; the view switch stays in reach however many there are.
  const sequence = (
    <div className="sequence-bar">
    <div className="sequence" data-tour="sequence">
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
                setTransitionsTracked({ ...transitions, [c.id]: kind ? { kind, duration: t?.duration ?? 0.6 } : null });
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
            <button title="Duplicate: the same clip again, right after it" onClick={(e) => (e.stopPropagation(), duplicate(c.id))}>
              ⧉
            </button>
            <button title="Remove from the project" onClick={(e) => (e.stopPropagation(), remove([c.id]))}>
              ✕
            </button>
          </div>
          </div>
        );
      })}
      <button className="btn small" onClick={async () => onOpen(await chooseMedia(true))} title="Add more videos after these">
        + Add clip
      </button>
    </div>
      {clips.length > 1 && (
        <button className="btn small arrange-open" onClick={() => setArranging(true)} title="See every clip at once and put them in order">
          Arrange
        </button>
      )}
      <div className="chips view-switch" data-tour="view-switch">
        <button className={`chip${mode === "video" ? " on" : ""}`} onClick={() => setMode("video")} title="All the clips end to end, as the export plays them">
          Whole video
        </button>
        <button
          className={`chip${mode === "clip" ? " on" : ""}`}
          onClick={() => setMode("clip")}
          title="Just the clip on screen, spread across the timeline, with what's cut shown in red (or double-click a clip on the whole-video timeline)"
        >
          This clip
        </button>
      </div>
      <span className="faint" style={{ whiteSpace: "nowrap" }}>
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
      <button
        data-tour="undo"
        className="btn small ghost icon-only"
        onClick={history.undo}
        disabled={!history.undoName}
        title={history.undoName ? `Undo ${history.undoName} (Ctrl+Z)` : "Nothing to undo"}
      >
        <UndoIcon />
      </button>
      <button
        className="btn small ghost icon-only"
        onClick={history.redo}
        disabled={!history.redoName}
        title={history.redoName ? `Redo ${history.redoName} (Ctrl+Shift+Z)` : "Nothing to redo"}
      >
        <RedoIcon />
      </button>
      <button className="btn small" onClick={() => saveTo(projectPath)} title="Save the project to edit later (Ctrl+S)" disabled={!ready} data-tour="save">
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
      {arranging && (
        <Arrange
          items={clips.map((c) => {
            const spec = specs[c.id];
            return {
              id: c.id,
              name: baseName(c.path),
              length: spec?.outputDuration ?? null,
              strip: assets[c.id]?.strip ?? null,
              at: spec?.keep[0]?.start ?? 0,
              sourceDuration: spec?.media.duration ?? 1,
              pieces: spec ? cutPoints(spec).length + 1 : 1,
            };
          })}
          active={active}
          total={offset}
          onOrder={reorder}
          onSplit={splitAtCuts}
          onRemove={remove}
          onOpen={(id) => {
            setActive(id);
            setMode("clip");
            setArranging(false);
          }}
          onClose={() => setArranging(false)}
        />
      )}
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
            heldTitles={heldTitles}
            playRequest={playNext?.id === c.id ? playNext.n : 0}
            sequence={sequence}
            toolbar={toolbar}
            initial={c.initial}
            history={editHistory}
            register={register}
            video={video}
            seekTo={seekTo?.id === c.id ? seekTo : null}
            onAssets={onAssets}
            onSpec={onSpec}
            onEnded={() => onEnded(c.id)}
            onRelink={(found) => relink(c.id, found)}
            onSplit={(at) => split(c.id, at)}
            onMoveOverlay={moveOverlay}
            onRemove={() => remove([c.id])}
            onClose={close}
          />
        </div>
      ))}
    </>
  );
}

type EmptyProps = {
  onOpen: (paths: string[]) => void;
  onOpenProject: (path: string, restore?: { session: string; savedTo: string | null }) => void;
  error: string | null;
  onUndo: (() => void) | null;
};

function Empty(p: EmptyProps) {
  const { onOpen } = p;
  const recent = load<string[]>("recent", []);
  return (
    <div className="drop">
      <div>
        <div className="box">
          <h1 style={{ margin: "0 0 6px" }}>Open a video</h1>
          <p className="muted" style={{ margin: "0 0 18px" }}>
            Drop files anywhere on this window, or pick some. Several files become clips in one video.
          </p>
          <div className="row" style={{ justifyContent: "center" }} data-tour="open">
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
          {p.onUndo && (
            <div>
              <button className="btn ghost" style={{ marginTop: 14 }} onClick={p.onUndo}>
                Undo removing the clip
              </button>
            </div>
          )}
          {p.error && <p className="error">{p.error}</p>}
          <ol className="how">
            <li>
              <strong>Open</strong> your videos
            </li>
            <li>
              <strong>Edit</strong>: cut, caption, add music
            </li>
            <li>
              <strong>Export</strong> the finished video
            </li>
          </ol>
          <div>
            <a
              href="#"
              onClick={(e) => {
                e.preventDefault();
                startTour("welcome");
              }}
            >
              New to mx? Take the one-minute tour
            </a>
          </div>
        </div>
        <RecentProjects onOpen={p.onOpenProject} />
        {recent.length > 0 && (
          <div className="recent">
            <h3 className="section">Recent videos</h3>
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
  /** Text from other clips running on past their ends. */
  heldTitles: HeldTitle[];
  /** Changes when the project wants this clip to start playing. */
  playRequest: number;
  sequence: React.ReactNode;
  /** Project file controls, for the top bar. */
  toolbar: React.ReactNode;
  initial?: Saved;
  /** The project's undo history, which this clip's edits go into. */
  history: History;
  video: VideoTimeline;
  /** A moment of this clip to jump to, from the whole-video timeline. */
  seekTo: { t: number; play: boolean; n: number } | null;
  onAssets: (id: string, a: Assets) => void;
  register: (id: string, setters: Record<string, (v: never) => void>) => () => void;
  onSpec: (spec: ClipSpec) => void;
  onEnded: () => void;
  /** Split this clip in two at a moment of its file. */
  onSplit: (at: number) => void;
  /** The clip's file was found somewhere else. */
  onRelink: (path: string) => void;
  /** Moves or resizes another clip's box layer (one running on over this clip). */
  onMoveOverlay: (clip: string, index: number, pip: Pip) => void;
  onRemove: () => void;
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
  const [layer, setLayer] = useState<LayerRef | null>(null);
  const [highlights, setHighlights] = useState<Highlight[] | null>(saved?.highlights ?? null);
  const [skipCuts, setSkipCuts] = useState(saved?.skipCuts ?? true);
  const [motion, setMotion] = useState<Keyframe[]>(p.initial?.motion ?? []);
  const [volume, setVolume] = useState(p.initial?.volume ?? 1);
  const [mutes, setMutes] = useState<Range[]>(p.initial?.mutes ?? []);
  const [denoise, setDenoise] = useState<Denoise>(p.initial?.denoise ?? "off");
  const [sounds, setSounds] = useState<Sound[]>(p.initial?.sounds ?? []);
  const soundEls = useRef<(HTMLAudioElement | null)[]>([]);
  const [overlays, setOverlays] = useState<Overlay[]>(p.initial?.overlays ?? []);
  const layerEls = useRef<(HTMLDivElement | null)[]>([]);
  const [plan, setPlan] = useState<EditPlan>({ cuts: [], keep: [], outputDuration: 0 });
  const [cues, setCues] = useState<Segment[]>([]);
  const [exported, setExported] = useState(false);

  // Edits go through these, which record them in the project's history;
  // undoing puts values back through the plain setters registered here.
  const { record } = p.history;
  const track =
    <T,>(key: string, value: T, set: (v: T) => void) =>
    (next: T) => {
      record([{ scope: p.id, key, before: value, after: next }]);
      set(next);
    };
  const edit = {
    manualCuts: track("manualCuts", manualCuts, setManualCuts),
    removed: track("removed", removed, setRemoved),
    titles: track("titles", titles, setTitles),
    overlays: track("overlays", overlays, setOverlays),
    sounds: track("sounds", sounds, setSounds),
    motion: track("motion", motion, setMotion),
    volume: track("volume", volume, setVolume),
    mutes: track("mutes", mutes, setMutes),
    denoise: track("denoise", denoise, setDenoise),
    removePauses: track("removePauses", removePauses, setRemovePauses),
    padding: track("padding", padding, setPadding),
  };
  const { register } = p;
  useEffect(
    () =>
      register(p.id, {
        manualCuts: setManualCuts,
        removed: setRemoved,
        titles: setTitles,
        overlays: setOverlays,
        sounds: setSounds,
        motion: setMotion,
        volume: setVolume,
        mutes: setMutes,
        denoise: setDenoise,
        removePauses: setRemovePauses,
        padding: setPadding,
        // Not an edit: lets the whole-video timeline pick this clip's layers.
        pickLayer: (l: LayerRef | null) => {
          setLayer(l);
          if (l) setTab(l.kind === "sound" ? "audio" : "titles");
        },
        addImages: (paths: string[]) => addImagesRef.current(paths),
      }),
    [p.id, register],
  );

  // Images dropped on the window, from the project: layers at the playhead.
  const addImagesRef = useRef<(paths: string[]) => void>(() => {});
  addImagesRef.current = (paths) => {
    if (!media?.hasVideo) return;
    const at = video.current?.currentTime ?? time;
    edit.overlays([...overlays, ...paths.map((source) => imageLayer(source, at))]);
    pickLayer({ kind: "overlay", index: overlays.length });
  };

  // The layers on the timeline. A video or sound can't be stretched past
  // its own file, so their lengths are probed.
  const lengths = useLengths([...overlays.map((o) => o.source), ...sounds.map((s) => s.source)].filter((s) => !isImage(s)));
  const fileBar = (kind: "overlay" | "sound", index: number, l: Overlay | Sound): LayerBar => {
    // An image has no length of its own: it shows as long as it's asked.
    const still = isImage(l.source);
    const fileLength = still ? Infinity : (lengths.get(l.source) ?? l.offset + (l.duration ?? 3));
    const length = l.duration ?? (still ? IMAGE_SECONDS : fileLength - l.offset);
    return {
      clip: p.id,
      ref: { kind, index },
      start: l.at,
      end: l.at + length,
      label: baseName(l.source),
      earliest: still ? 0 : l.at - l.offset,
      latest: l.at - l.offset + fileLength,
      bounded: false,
    };
  };
  const layerBars: LayerBar[] = [
    ...titles.map((t, index) => ({
      clip: p.id,
      ref: { kind: "title" as const, index },
      start: t.start,
      end: t.end,
      label: t.text.trim() || "(empty text)",
      earliest: 0,
      latest: media?.duration ?? t.end,
      bounded: true,
    })),
    ...overlays.map((o, i) => fileBar("overlay", i, o)),
    ...sounds.map((s, i) => fileBar("sound", i, s)),
  ];
  // The clip timeline's picture: this clip, whole. Its filmstrip and
  // waveform also go up to the whole-video timeline.
  const clipPictures: Picture[] = useMemo(
    () => (media ? [{ start: 0, end: media.duration, strip, wave, sourceDuration: media.duration, toSource: (t: number) => t }] : []),
    [media, strip, wave],
  );
  const { onAssets } = p;
  useEffect(() => onAssets(p.id, { strip, wave }), [strip, wave, onAssets, p.id]);

  // Dragged on the timeline: the middle moves it; the start edge trims
  // the head, starting the file later; the end edge sets its length.
  const moveLayer = (l: LayerRef, start: number, end: number, edge: "move" | "start" | "end") => {
    setLayer(l);
    if (l.kind === "title") {
      edit.titles(titles.map((t, i) => (i === l.index ? { ...t, start, end, hold: edge === "end" ? 0 : t.hold } : t)));
      return;
    }
    const change = <T extends Overlay | Sound>(item: T): T =>
      edge === "move"
        ? { ...item, at: start }
        : edge === "start"
          ? { ...item, at: start, offset: Math.max(0, item.offset + start - item.at), duration: end - start }
          : { ...item, duration: end - start };
    if (l.kind === "overlay") edit.overlays(overlays.map((o, i) => (i === l.index ? change(o) : o)));
    else edit.sounds(sounds.map((s, i) => (i === l.index ? change(s) : s)));
  };
  const deleteLayer = (l: LayerRef) => {
    if (l.kind === "title") edit.titles(titles.filter((_, i) => i !== l.index));
    else if (l.kind === "overlay") edit.overlays(overlays.filter((_, i) => i !== l.index));
    else edit.sounds(sounds.filter((_, i) => i !== l.index));
    setLayer(null);
  };
  const pickLayer = (l: LayerRef | null) => {
    setLayer(l);
    if (l) setTab(l.kind === "sound" ? "audio" : "titles");
  };

  // The editor's tour, the first time a video is ready to play.
  useEffect(() => {
    if (src && p.active) setTimeout(() => offerTour("editor"), 600);
  }, [!!src, p.active]);

  // What this clip is loading. Every clip loads at once, taking turns;
  // the one on screen goes first, including when it's switched to.
  const loading = useRef<string[]>([]);
  const activeNow = useRef(p.active);
  activeNow.current = p.active;
  useEffect(() => {
    if (p.active) loading.current.forEach((id) => hurryTask(id).catch(() => {}));
  }, [p.active]);

  // Load: probe first, then preview, filmstrip and waveform side by side.
  useEffect(() => {
    let live = true;
    setLoadError(null);
    probeMedia(path)
      .then((m) => {
        if (!live) return;
        setMedia(m);
        setPlan({ cuts: [], keep: [{ start: 0, end: m.duration }], outputDuration: m.duration });
        const pt = newTaskId();
        const [ft, wt] = [newTaskId(), newTaskId()];
        loading.current = [pt, ft, wt];
        if (activeNow.current) loading.current.forEach((id) => hurryTask(id).catch(() => {}));
        setPreviewTask(pt);
        preparePreview(pt, path)
          .then((pv) => live && setSrc(convertFileSrc(pv.src)))
          .catch((e) => live && setLoadError(`Couldn't make a preview: ${errorText(e)}`))
          .finally(() => live && setPreviewTask(null));
        if (m.hasVideo) filmstrip(ft, path).then((s) => live && setStrip(s)).catch(() => {});
        waveform(wt, path, 3000).then((w) => live && setWave(w)).catch(() => {});
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
        mutes,
        denoise,
        sounds,
        overlays,
        editor: { pauses, removePauses, padding, manualCuts, transcript, removed, highlights, skipCuts },
      },
    [media, plan, keptTranscript, titles, motion, volume, mutes, denoise, sounds, overlays, p.id, pauses, removePauses, padding, manualCuts, transcript, removed, highlights, skipCuts],
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
  const live = useRef({ plan, skipCuts, active: p.active, onEnded: p.onEnded, music: project.music, offset: p.outputOffset, sounds: p.placedSounds, volume, mutes, layers: p.placedOverlays });
  live.current = { plan, skipCuts, active: p.active, onEnded: p.onEnded, music: project.music, offset: p.outputOffset, sounds: p.placedSounds, volume, mutes, layers: p.placedOverlays };
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
      const { plan, skipCuts, music, offset, sounds, volume, mutes, layers } = live.current;
      // The web view can't play louder than as recorded; the export can.
      v.volume = mutes.some((m) => t >= m.start && t < m.end) ? 0 : Math.min(1, Math.max(0, volume));
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
        box.style.pointerEvents = inside && o.pip ? "auto" : "none";
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
  // Asked to jump here by the whole-video timeline.
  useEffect(() => {
    const el = video.current;
    const q = p.seekTo;
    if (!q || !el) return;
    el.currentTime = q.t;
    setTime(q.t);
    if (q.play) el.play().catch(() => {});
  }, [p.seekTo?.n]);

  // Panel sizes, shared by every clip.
  const layout = useLayout();
  const sideWidth = Math.min(layout.side, Math.max(260, window.innerWidth - 480));

  // Which timeline is showing; on the whole-video one, times are the output's.
  const v = p.video;
  const wholeVideo = v.mode === "video";
  const now = p.outputOffset + toOutput(plan.keep, time);
  // A split needs something kept on both sides of the playhead.
  const canSplit = toOutput(plan.keep, time) >= 0.1 && toOutput(plan.keep, time) <= plan.outputDuration - 0.1;
  const shownSelection = wholeVideo ? v.selection : selection;
  const clearSelection = () => (wholeVideo ? v.setSelection(null) : setSelection(null));
  // The selection in output time, which the whole-video edits work in.
  const selectionOut = wholeVideo
    ? v.selection
    : selection && { start: p.outputOffset + toOutput(plan.keep, selection.start), end: p.outputOffset + toOutput(plan.keep, selection.end) };
  const selectionMuted = !!shownSelection && (wholeVideo ? v.mutes : mutes).some((m) => m.start < shownSelection.end - 0.01 && m.end > shownSelection.start + 0.01);
  const cutSelection = wholeVideo
    ? v.selection && (() => (v.cut(v.selection!), v.setSelection(null)))
    : selection && (() => (edit.manualCuts([...manualCuts, selection]), setSelection(null)));

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
    if ((target as HTMLInputElement).type === "range" && e.key.startsWith("Arrow")) return;
    const step = media.fps ? 1 / media.fps : 1 / 30;
    const k = e.key.toLowerCase();
    // The whole-video timeline selects and cuts across clips.
    if (wholeVideo) {
      const done = () => e.preventDefault();
      if (k === "i") return done(), v.setSelection({ start: now, end: Math.max(now + 0.1, v.selection?.end ?? v.duration) });
      if (k === "o") return done(), v.setSelection({ start: Math.min(v.selection?.start ?? 0, now - 0.1), end: now });
      if ((k === "x" || k === "delete" || k === "backspace") && v.selection) return done(), v.cut(v.selection), v.setSelection(null);
      if (k === "escape") v.setSelection(null);
    }
    if (k === " ") togglePlay();
    else if (k === "arrowleft") seek(time - (e.shiftKey ? 5 : 1));
    else if (k === "arrowright") seek(time + (e.shiftKey ? 5 : 1));
    else if (k === ",") seek(time - step);
    else if (k === ".") seek(time + step);
    else if (k === "i") setSelection({ start: time, end: Math.max(time + 0.1, selection?.end ?? media.duration) });
    else if (k === "o") setSelection({ start: Math.min(selection?.start ?? 0, time - 0.1), end: time });
    else if ((k === "x" || k === "delete" || k === "backspace") && selection) {
      edit.manualCuts([...manualCuts, selection]);
      setSelection(null);
    } else if ((k === "delete" || k === "backspace") && layer) deleteLayer(layer);
    else if (k === "q" && time > 0.05) edit.manualCuts([...manualCuts, { start: 0, end: time }]);
    else if (k === "w" && time < media.duration - 0.05) edit.manualCuts([...manualCuts, { start: time, end: media.duration }]);
    else if (k === "s" && !e.ctrlKey && canSplit) p.onSplit(time);
    else if (k === "m" && !e.ctrlKey && selectionOut) v.mute(selectionOut, !selectionMuted);
    else if (k === "escape") {
      setSelection(null);
      setLayer(null);
    } else return;
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
          setRemovePauses: edit.removePauses,
          padding,
          setPadding: edit.padding,
          manualCuts,
          setManualCuts: edit.manualCuts,
          transcript,
          setTranscript,
          removed,
          setRemoved: edit.removed,
          titles,
          setTitles: edit.titles,
          layer,
          setLayer,
          selectedTitle: layer?.kind === "title" ? layer.index : null,
          setSelectedTitle: (i: number | null) => setLayer(i === null ? null : { kind: "title", index: i }),
          highlights,
          setHighlights,
          motion,
          setMotion: edit.motion,
          volume,
          setVolume: edit.volume,
          mutes,
          setMutes: edit.mutes,
          denoise,
          setDenoise: edit.denoise,
          sounds,
          setSounds: edit.sounds,
          overlays,
          setOverlays: edit.overlays,
          skipCuts,
          setSkipCuts,
          project,
          spec,
          onExported: () => setExported(true),
          split: canSplit ? () => p.onSplit(time) : null,
          splitAtCuts: v.pieces(p.id) > 1 ? () => v.splitAtCuts([p.id]) : null,
          pieces: v.pieces(p.id),
          volumeForAll: v.blocks.length > 1 ? v.volumeForAll : null,
          cutSelection,
          selectionLength: shownSelection ? shownSelection.end - shownSelection.start : 0,
        }
      : null;

  if (loadError && !media) {
    // Usually the file was moved, renamed or deleted since the project
    // was saved: point it at the file's new place, and its edits carry on.
    const gone = /no such file|cannot find|not found/i.test(loadError);
    return (
      <div className="drop">
        <div className="box">
          <h1 style={{ margin: "0 0 8px" }}>{gone ? `${baseName(path)} isn't where it was` : `Can't open ${baseName(path)}`}</h1>
          {gone ? (
            <p className="muted">
              It was in <span className="mono">{dirName(path)}</span>. If you moved or renamed it, show mx where it is now; this clip's
              edits stay as they were.
            </p>
          ) : (
            <p className="error">{loadError}</p>
          )}
          <div className="row" style={{ justifyContent: "center" }}>
            <button
              className="btn primary"
              onClick={async () => {
                const found = await chooseMedia(false);
                if (found[0]) p.onRelink(found[0]);
              }}
            >
              Find the file...
            </button>
            <button className="btn" onClick={p.onRemove}>
              Remove this clip
            </button>
          </div>
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

  const tabs: [Tab, string, boolean, string][] = [
    ["cut", "Cut", plan.cuts.length > 0, "Remove pauses and parts you don't want"],
    ["text", "Text", !!transcript, "Captions from what's said"],
    ["titles", "Layers", titles.length > 0 || overlays.length > 0, "Text and videos over the picture"],
    ["highlights", "Picks", !!highlights?.length, "Find the liveliest moments"],
    ["frame", "Frame", !!project.frame || motion.length > 0, "Shape (9:16, square...) and zoom"],
    ["audio", "Audio", !!project.music || sounds.length > 0 || volume !== 1 || denoise !== "off", "Music, sounds, volume and noise"],
    ["export", "Export", false, "Save the finished video"],
  ];
  const anyClip = (has: (c: ClipSpec) => boolean) => project.clips.some(has);
  const quickSteps = [
    { tab: "cut", label: "Trim it", hint: "cut pauses or parts", done: anyClip((c) => c.outputDuration < c.media.duration - 0.05) },
    { tab: "text", label: "Add captions", hint: "or skip it", done: anyClip((c) => !!c.transcript) },
    { tab: "titles", label: "Add text or b-roll", hint: "over the picture", done: anyClip((c) => c.titles.length > 0 || c.overlays.length > 0) },
    { tab: "frame", label: "Pick a shape", hint: "9:16 for TikTok", done: !!project.frame },
    { tab: "audio", label: "Add music", hint: "or sound effects", done: !!project.music || anyClip((c) => c.sounds.length > 0) },
    { tab: "export", label: "Export", hint: "save the video", done: exported },
  ];

  return (
    <div className="editor" style={{ gridTemplateColumns: `minmax(0, 1fr) ${sideWidth}px` }}>
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

      <div className="stage" data-tour="stage">
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
            carried={p.heldTitles.filter((h) => h.clip !== p.id && now >= h.start && now < h.end).map((h) => h.title)}
            selectedTitle={tab === "titles" && layer?.kind === "title" ? layer.index : null}
            onSelectTitle={(i) => pickLayer(i === null ? null : { kind: "title", index: i })}
            onMoveTitle={(i, x, y) => edit.titles(titles.map((t, j) => (j === i ? { ...t, x, y } : t)))}
            view={motionAt(motion, time)}
            layers={p.placedOverlays}
            layerRefs={layerEls}
            selectedLayer={(() => {
              const k = p.placedOverlays.findIndex((o) => o.clip === p.id && layer?.kind === "overlay" && o.index === layer.index);
              return k >= 0 ? k : null;
            })()}
            onPickLayer={(k) => {
              const o = p.placedOverlays[k];
              if (o?.clip === p.id) pickLayer({ kind: "overlay", index: o.index });
            }}
            onMoveLayer={(k, pip) => {
              const o = p.placedOverlays[k];
              if (!o) return;
              if (o.clip === p.id) edit.overlays(overlays.map((l, i) => (i === o.index ? { ...l, pip } : l)));
              else p.onMoveOverlay(o.clip, o.index, pip);
            }}
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
          <button className="icon-btn big" onClick={togglePlay} title="Play/pause (Space)" data-tour="play">
            {playing ? <PauseIcon /> : <PlayIcon />}
          </button>
          <span className="time" title={wholeVideo ? `${clock(time, true)} into ${media.name}` : undefined}>
            {clock(wholeVideo ? now : time, true)} <span className="faint">/ {clock(wholeVideo ? v.duration : media.duration)}</span>
          </span>
          <span className="muted">
            {plan.cuts.length > 0 && (
              <>
                <ScissorsIcon className="" /> {clock(plan.outputDuration)} after cuts
              </>
            )}
          </span>
          <span className="grow" />
          {shownSelection && selectionOut ? (
            <div className="sel-actions">
              <span className="muted mono">
                {clock(shownSelection.start, true)}–{clock(shownSelection.end, true)}
              </span>
              <button className="btn small" onClick={() => cutSelection?.()} title="Cut it out (X)">
                Cut
              </button>
              <button
                className="btn small"
                onClick={() => v.mute(selectionOut, !selectionMuted)}
                title={selectionMuted ? "Bring back its sound (M)" : "Silence the clip's own sound here; the picture, music and sounds carry on (M)"}
              >
                {selectionMuted ? "Unmute" : "Mute"}
              </button>
              <button className="btn small" onClick={() => (v.addText(selectionOut), clearSelection())} title="Text shown for exactly this stretch">
                + Text
              </button>
              <button
                className="btn small"
                onClick={async () => {
                  const path = await open({ filters: [{ name: "Image or video", extensions: LAYER_EXTENSIONS }], defaultPath: dirName(media.path) });
                  if (typeof path === "string") await v.addFile(selectionOut, "overlay", path).then(clearSelection, (e) => setLoadError(errorText(e)));
                }}
                title="An image or video over the picture for exactly this stretch"
              >
                + Image/video
              </button>
              <button
                className="btn small"
                onClick={async () => {
                  const path = await open({ filters: [{ name: "Audio (or a video's sound)", extensions: SOUND_EXTENSIONS }], defaultPath: dirName(media.path) });
                  if (typeof path === "string") await v.addFile(selectionOut, "sound", path).then(clearSelection, (e) => setLoadError(errorText(e)));
                }}
                title="A sound or voiceover for this stretch (as long as the file allows)"
              >
                + Sound
              </button>
              <button className="btn small ghost" onClick={clearSelection} title="Clear the selection (Esc, or double-click the timeline)">
                Clear
              </button>
            </div>
          ) : (
            <span className="faint ellipsis">Click the timeline to jump · drag across it to select</span>
          )}
          <span className="faint" style={{ whiteSpace: "nowrap" }}>
            Timeline zoom
          </span>
          <input
            type="range"
            min={1}
            max={40}
            step={0.5}
            value={wholeVideo ? v.zoom : zoom}
            onChange={(e) => (wholeVideo ? v.setZoom : setZoom)(Number(e.target.value))}
            style={{ width: 110 }}
            title="Stretch the timeline to cut more precisely"
          />
        </div>
      </div>

      <aside className="side">
        <Splitter
          dir="x"
          title="Drag to resize the panel; double-click to reset"
          value={sideWidth}
          onDrag={(start, moved) => setLayout({ side: Math.round(Math.min(window.innerWidth - 480, Math.max(260, start - moved))) })}
          onReset={() => setLayout({ side: DEFAULT_LAYOUT.side })}
        />
        <QuickStart steps={quickSteps} tab={tab} onTab={(t) => setTab(t as Tab)} />
        <div className="tabs">
          {tabs.map(([id, label, on, hint]) => (
            <button key={id} className={tab === id ? "active" : ""} onClick={() => setTab(id)} title={hint} data-tour={`tab-${id}`}>
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
        <Splitter
          dir="y"
          title="Drag to resize the timeline; double-click to reset"
          value={layout.timeline}
          onDrag={(start, moved) => setLayout({ timeline: Math.round(Math.min(window.innerHeight * 0.45, Math.max(-40, start - moved))) })}
          onReset={() => setLayout({ timeline: DEFAULT_LAYOUT.timeline })}
        />
        {p.sequence}
        <div data-tour="timeline">
          {!p.active ? null : wholeVideo ? (
            <Timeline
              duration={v.duration}
              time={now}
              playing={playing}
              pictures={v.pictures}
              cuts={[]}
              mutes={v.mutes}
              selection={v.selection}
              highlights={[]}
              keyframes={[]}
              layers={v.layers}
              selectedKey={layer ? layerKey(p.id, layer) : null}
              blocks={v.blocks}
              zoom={v.zoom}
              onZoom={v.setZoom}
              scrollKey="video"
              onSeek={(t) => v.seek(t)}
              onSelect={v.setSelection}
              onPickLayer={v.pickLayer}
              onMoveLayer={v.moveLayer}
              onPickBlock={(_, t) => v.seek(t)}
              onOpenBlock={() => v.setMode("clip")}
              onMoveBlock={v.moveBlock}
              onTrimBlock={v.trimBlock}
            />
          ) : (
            <Timeline
              duration={media.duration}
              time={time}
              playing={playing}
              pictures={clipPictures}
              cuts={plan.cuts}
              mutes={mutes}
              selection={selection}
              highlights={highlights ?? []}
              keyframes={motion.map((k) => k.time)}
              layers={layerBars}
              selectedKey={layer ? layerKey(p.id, layer) : null}
              zoom={zoom}
              onZoom={setZoom}
              scrollKey={p.id}
              onSeek={seek}
              onSelect={setSelection}
              onPickLayer={(bar) => pickLayer(bar?.ref ?? null)}
              onMoveLayer={(bar, start, end, edge) => {
                moveLayer(bar.ref, start, end, edge);
                p.history.seal();
              }}
            />
          )}
        </div>
      </div>
    </div>
  );
}
