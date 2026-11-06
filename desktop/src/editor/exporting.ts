// Building export requests from the editor state.

import { exportEdit, newTaskId, type ClipRequest, type ExportRequest, type Highlight, type Range } from "../api";
import { join, load, stem } from "../util";
import type { ClipSpec, EditorApi, ProjectApi } from "./state";

export type Format = { codec: ExportRequest["codec"]; gpu: boolean; targetMb: number | null };

export const savedFormat = (): Format => load("export.format", { codec: "h264", gpu: true, targetMb: null });

/** Output extension: video stays mp4; audio-only keeps a common format. */
export function outputExt(clips: ClipSpec[]): string {
  if (clips.some((c) => c.media.hasVideo)) return "mp4";
  const ext = clips[0]?.media.path.split(".").pop()?.toLowerCase() ?? "";
  return ["mp3", "m4a", "wav", "flac"].includes(ext) ? ext : "m4a";
}

function clipRequest(spec: ClipSpec, keep: Range[] = spec.keep): ClipRequest {
  return { source: spec.media.path, keep, volume: 1, titles: spec.titles, transcript: spec.transcript };
}

/** The whole project, as one file. */
export function projectRequest(project: ProjectApi, output: string, format: Format): ExportRequest {
  const video = project.clips.some((c) => c.media.hasVideo);
  return {
    clips: project.clips.map((c) => clipRequest(c)),
    frame: video ? project.frame : null,
    music: project.music,
    captions: video && project.captionsOn ? project.style : null,
    output,
    codec: format.codec,
    quality: null,
    gpu: format.gpu,
    targetMb: format.targetMb,
  };
}

/** Export each highlight of the current clip as its own file in `dir`,
 * one after another, with the project's frame and captions. */
export async function exportClips(ed: EditorApi, picks: Highlight[], dir: string, onState: (s: string) => void) {
  const format = savedFormat();
  const ext = outputExt([ed.spec]);
  for (const [i, h] of picks.entries()) {
    const at = Math.floor(h.range.start);
    const name = `${stem(ed.media.path)}_${Math.floor(at / 60)}m${String(at % 60).padStart(2, "0")}s.${ext}`;
    onState(`Exporting clip ${i + 1} of ${picks.length}...`);
    // Within a highlight, the clip's other cuts still apply.
    const keep = ed.spec.keep
      .map((k) => ({ start: Math.max(k.start, h.range.start), end: Math.min(k.end, h.range.end) }))
      .filter((k) => k.end - k.start > 0.05);
    const req = projectRequest({ ...ed.project, clips: [ed.spec], music: null }, join(dir, name), { ...format, targetMb: null });
    req.clips = [clipRequest(ed.spec, keep.length ? keep : [h.range])];
    await exportEdit(newTaskId(), req);
  }
}
