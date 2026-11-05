// Building export requests from the editor state.

import { exportEdit, newTaskId, type ExportRequest, type Highlight, type Range } from "../api";
import { join, load, stem } from "../util";
import type { EditorApi } from "./state";

export type Format = { codec: ExportRequest["codec"]; gpu: boolean; targetMb: number | null };

export const savedFormat = (): Format => load("export.format", { codec: "h264", gpu: true, targetMb: null });

/** Output extension: video stays mp4; audio-only keeps a common format. */
export function outputExt(ed: EditorApi): string {
  if (ed.media.hasVideo) return "mp4";
  const ext = ed.media.path.split(".").pop()?.toLowerCase() ?? "";
  return ["mp3", "m4a", "wav", "flac"].includes(ext) ? ext : "m4a";
}

/** The request for rendering `keep` with the editor's frame and captions. */
export function request(ed: EditorApi, output: string, keep: Range[], format: Format): ExportRequest {
  const captions = ed.captionsOn && ed.transcript && ed.style && ed.media.hasVideo;
  return {
    input: ed.media.path,
    output,
    keep,
    frame: ed.media.hasVideo ? ed.frame : null,
    // Cut lines aren't captioned: they're not in the video.
    captions: captions
      ? { transcript: { segments: ed.transcript!.segments.filter((_, i) => !ed.removed.includes(i)) }, style: ed.style! }
      : null,
    codec: format.codec,
    quality: null,
    gpu: format.gpu,
    targetMb: format.targetMb,
  };
}

/** Export each highlight as its own clip into `dir`, one after another. */
export async function exportClips(ed: EditorApi, picks: Highlight[], dir: string, onState: (s: string) => void) {
  const format = savedFormat();
  for (const [i, h] of picks.entries()) {
    const at = Math.floor(h.range.start);
    const name = `${stem(ed.media.path)}_${Math.floor(at / 60)}m${String(at % 60).padStart(2, "0")}s.${outputExt(ed)}`;
    onState(`Exporting clip ${i + 1} of ${picks.length}...`);
    // Within a clip, the edit's other cuts still apply.
    const keep = ed.plan.keep
      .map((k) => ({ start: Math.max(k.start, h.range.start), end: Math.min(k.end, h.range.end) }))
      .filter((k) => k.end - k.start > 0.05);
    await exportEdit(newTaskId(), request(ed, join(dir, name), keep.length ? keep : [h.range], format));
  }
}
