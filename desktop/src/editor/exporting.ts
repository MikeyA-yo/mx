// Building export requests and project files from the editor state.

import { exportEdit, newTaskId, type ExportRequest, type Highlight, type Project, type ProjectClip, type Range } from "../api";
import { join, load, stem } from "../util";
import type { ClipEditorState, ClipSpec, EditorApi, ProjectApi } from "./state";

export type Format = { codec: ExportRequest["codec"]; gpu: boolean; targetMb: number | null };

export const savedFormat = (): Format => load("export.format", { codec: "h264", gpu: true, targetMb: null });

/** Output extension: video stays mp4; audio-only keeps a common format. */
export function outputExt(clips: ClipSpec[]): string {
  if (clips.some((c) => c.media.hasVideo)) return "mp4";
  const ext = clips[0]?.media.path.split(".").pop()?.toLowerCase() ?? "";
  return ["mp3", "m4a", "wav", "flac"].includes(ext) ? ext : "m4a";
}

function projectClip(spec: ClipSpec, keep: Range[] = spec.keep): ProjectClip {
  return {
    source: spec.media.path,
    keep,
    volume: spec.volume,
    titles: spec.titles,
    transcript: spec.transcript,
    transition: spec.transition,
    motion: spec.motion,
    sounds: spec.sounds,
  };
}

/** The project as mx-core renders it. */
export function coreProject(project: ProjectApi): Project {
  const video = project.clips.some((c) => c.media.hasVideo);
  return {
    clips: project.clips.map((c) => projectClip(c)),
    frame: video ? project.frame : null,
    music: project.music,
    captions: video && project.captionsOn ? project.style : null,
  };
}

export function projectRequest(project: ProjectApi, output: string, format: Format): ExportRequest {
  return { project: coreProject(project), output, codec: format.codec, quality: null, gpu: format.gpu, targetMb: format.targetMb };
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
    const req = projectRequest({ ...ed.project, music: null }, join(dir, name), { ...format, targetMb: null });
    req.project.clips = [{ ...projectClip(ed.spec, keep.length ? keep : [h.range]), transition: null }];
    await exportEdit(newTaskId(), req);
  }
}

/** A saved project: what `mx render` reads, plus each editor's state. */
export type ProjectFile = Omit<Project, "clips"> & {
  mx: 1;
  clips: (ProjectClip & { editor: ClipEditorState })[];
  editor: { captionsOn: boolean; style: ProjectApi["style"] };
};

export function projectFile(project: ProjectApi): ProjectFile {
  const core = coreProject(project);
  return {
    mx: 1,
    ...core,
    clips: core.clips.map((c, i) => ({ ...c, editor: project.clips[i].editor })),
    editor: { captionsOn: project.captionsOn, style: project.style },
  };
}

/** What reopening a project needs, or an error for files that aren't one. */
export function readProjectFile(json: unknown): ProjectFile {
  const f = json as Partial<ProjectFile> | null;
  if (!f || typeof f !== "object" || !Array.isArray(f.clips) || f.clips.some((c) => typeof c?.source !== "string")) {
    throw new Error("This isn't an mx project file.");
  }
  return f as ProjectFile;
}
