// The editor's shared state, as the panels see it.
//
// A project is a row of clips. Each clip has its own editor (cuts,
// transcript, titles, all in that clip's source time); the frame, caption
// style and music belong to the whole project.

import type {
  CaptionStyle,
  EditPlan,
  Frame,
  Highlight,
  MediaSummary,
  Music,
  Range,
  SilenceReport,
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
  editor: ClipEditorState;
};

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
  /** Index of the title being edited, if any. */
  selectedTitle: number | null;
  setSelectedTitle: (i: number | null) => void;

  highlights: Highlight[] | null;
  setHighlights: (h: Highlight[] | null) => void;

  skipCuts: boolean;
  setSkipCuts: (on: boolean) => void;

  project: ProjectApi;
  /** This clip's spec, as the export sees it. */
  spec: ClipSpec;
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
