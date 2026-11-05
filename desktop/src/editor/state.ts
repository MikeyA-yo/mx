// The editor's shared state, as the panels see it.

import type { CaptionStyle, EditPlan, Frame, Highlight, MediaSummary, Range, SilenceReport, Transcript } from "../api";

export type Pauses = {
  report: SilenceReport;
  /** Settings the report was made with. */
  minSilence: number;
  thresholdDb: number | null;
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

  highlights: Highlight[] | null;
  setHighlights: (h: Highlight[] | null) => void;

  frame: Frame | null;
  setFrame: (f: Frame | null) => void;

  captionsOn: boolean;
  setCaptionsOn: (on: boolean) => void;
  style: CaptionStyle | null;
  setStyle: (s: CaptionStyle) => void;

  skipCuts: boolean;
  setSkipCuts: (on: boolean) => void;
};

/** Output frame size: the reframe target, else the source. */
export function outputSize(ed: Pick<EditorApi, "media" | "frame">): [number, number] {
  const f = ed.frame;
  if (f) {
    const even = (x: number) => Math.max(1, Math.round(x / 2)) * 2;
    const short = even(f.short_edge);
    return f.aspect.w <= f.aspect.h
      ? [short, even((short * f.aspect.h) / f.aspect.w)]
      : [even((short * f.aspect.w) / f.aspect.h), short];
  }
  return [ed.media.width ?? 1920, ed.media.height ?? 1080];
}
