// The editor's shared state, as the panels see it.
//
// A project is a row of clips. Each clip has its own editor (cuts,
// transcript, titles, all in that clip's source time); the frame, caption
// style and music belong to the whole project.

import { useEffect } from "react";
import type {
  CaptionStyle,
  EditPlan,
  Frame,
  Highlight,
  Keyframe,
  MediaSummary,
  Music,
  Overlay,
  Pip,
  Range,
  SilenceReport,
  Sound,
  Title,
  Transcript,
  Transition,
} from "../api";

export type Pauses = {
  report: SilenceReport;
  /** Settings the report was made with. */
  minSilence: number;
  thresholdDb: number | null;
};

/** A clip editor's own state: what's saved with a project so the clip
 * reopens as it was left. */
export type ClipEditorState = {
  pauses: Pauses | null;
  removePauses: boolean;
  padding: number;
  manualCuts: Range[];
  /** The whole transcript, deleted lines included. */
  transcript: Transcript | null;
  removed: number[];
  highlights: Highlight[] | null;
  skipCuts: boolean;
};

/** What a clip contributes to the export, kept current by its editor. */
export type ClipSpec = {
  id: string;
  media: MediaSummary;
  /** Kept source ranges; the whole source when nothing is cut. */
  keep: Range[];
  outputDuration: number;
  /** Without the lines deleted in the text editor. */
  transcript: Transcript | null;
  titles: Title[];
  /** How it takes over from the clip before (set in the clip bar). */
  transition: Transition | null;
  motion: Keyframe[];
  /** The clip's own sound level: 0 mutes, 1 as recorded. */
  volume: number;
  sounds: Sound[];
  overlays: Overlay[];
  editor: ClipEditorState;
};

/** A layer on a clip: a title, a video overlay or a sound, by index. */
export type LayerKind = "title" | "overlay" | "sound";
export type LayerRef = { kind: LayerKind; index: number };
export const sameLayer = (a: LayerRef | null, b: LayerRef | null) => !!a && !!b && a.kind === b.kind && a.index === b.index;

/** A layer as the timeline draws it, in the clip's source time. */
export type LayerBar = {
  ref: LayerRef;
  start: number;
  end: number;
  label: string;
  /** How early its start can be dragged: a file can't start before itself. */
  earliest: number;
  /** How late its end can be dragged: the end of the file, or the clip. */
  latest: number;
  /** Has to stay inside the clip (titles); sounds and videos can run on. */
  bounded: boolean;
};

/** Scrolls a panel's card into view when it's picked, on the timeline
 * or the preview. */
export function useOpenCard(card: string | null) {
  useEffect(() => {
    if (!card) return;
    const t = setTimeout(() => {
      const el = Array.from(document.querySelectorAll<HTMLElement>(`[data-card="${card}"]`)).find((e) => e.getClientRects().length);
      el?.scrollIntoView({ block: "nearest", behavior: "smooth" });
    }, 30);
    return () => clearTimeout(t);
  }, [card]);
}

/** An overlay placed on the output, for previewing. */
export type PlacedOverlay = { src: string; start: number; length: number; offset: number; pip: Pip | null; volume: number; fade: number };

/** A sound placed on the output, for previewing. */
export type PlacedSound = { src: string; start: number; length: number; offset: number; volume: number; fadeIn: number; fadeOut: number };

/** Settings shared by every clip. */
export type ProjectApi = {
  clips: ClipSpec[];
  frame: Frame | null;
  setFrame: (f: Frame | null) => void;
  captionsOn: boolean;
  setCaptionsOn: (on: boolean) => void;
  style: CaptionStyle | null;
  setStyle: (s: CaptionStyle) => void;
  music: Music | null;
  setMusic: (m: Music | null) => void;
};

export type EditorApi = {
  media: MediaSummary;
  time: number;
  selection: Range | null;
  setSelection: (r: Range | null) => void;
  seek: (t: number, play?: boolean) => void;

  /** Every tool's cuts combined, from the Rust side. */
  plan: EditPlan;

  pauses: Pauses | null;
  setPauses: (p: Pauses | null) => void;
  removePauses: boolean;
  setRemovePauses: (on: boolean) => void;
  padding: number;
  setPadding: (s: number) => void;

  manualCuts: Range[];
  setManualCuts: (cuts: Range[]) => void;

  transcript: Transcript | null;
  setTranscript: (t: Transcript | null) => void;
  /** Indices of transcript segments deleted in the text editor. */
  removed: number[];
  setRemoved: (r: number[]) => void;

  titles: Title[];
  setTitles: (t: Title[]) => void;
  /** The layer picked on the timeline or in a panel. */
  layer: LayerRef | null;
  setLayer: (l: LayerRef | null) => void;
  /** Index of the title being edited, if any (the layer, when it's a title). */
  selectedTitle: number | null;
  setSelectedTitle: (i: number | null) => void;

  highlights: Highlight[] | null;
  setHighlights: (h: Highlight[] | null) => void;

  motion: Keyframe[];
  setMotion: (m: Keyframe[]) => void;

  volume: number;
  setVolume: (v: number) => void;
  sounds: Sound[];
  setSounds: (s: Sound[]) => void;
  overlays: Overlay[];
  setOverlays: (o: Overlay[]) => void;

  skipCuts: boolean;
  setSkipCuts: (on: boolean) => void;

  project: ProjectApi;
  /** This clip's spec, as the export sees it. */
  spec: ClipSpec;
  /** An export finished, for the quick-start checklist. */
  onExported: () => void;
};

/** Output frame size: the project frame, else the first clip's. */
export function outputSize(frame: Frame | null, media: MediaSummary): [number, number] {
  if (frame) {
    const even = (x: number) => Math.max(1, Math.round(x / 2)) * 2;
    const short = even(frame.short_edge);
    return frame.aspect.w <= frame.aspect.h
      ? [short, even((short * frame.aspect.h) / frame.aspect.w)]
      : [even((short * frame.aspect.w) / frame.aspect.h), short];
  }
  return [media.width ?? 1920, media.height ?? 1080];
}

/** The view at source time `t` on a zoom/pan path: the renderer's half
 * cosine easing between keyframes, held outside them. */
export function motionAt(keys: Keyframe[], t: number): { zoom: number; x: number; y: number } {
  if (keys.length === 0) return { zoom: 1, x: 0.5, y: 0.5 };
  const k = [...keys].sort((a, b) => a.time - b.time);
  if (t <= k[0].time) return k[0];
  for (let i = 0; i + 1 < k.length; i++) {
    const [a, b] = [k[i], k[i + 1]];
    if (t < b.time) {
      const e = 0.5 - 0.5 * Math.cos((Math.PI * (t - a.time)) / Math.max(1e-3, b.time - a.time));
      return { zoom: a.zoom + (b.zoom - a.zoom) * e, x: a.x + (b.x - a.x) * e, y: a.y + (b.y - a.y) * e };
    }
  }
  return k[k.length - 1];
}

/** Where source time `t` plays in the clip's output (cut time snaps to
 * the next kept moment). */
export function toOutput(keep: Range[], t: number): number {
  let out = 0;
  for (const k of keep) {
    if (t < k.start) return out;
    if (t < k.end) return out + t - k.start;
    out += k.end - k.start;
  }
  return out;
}
