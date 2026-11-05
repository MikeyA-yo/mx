// Typed calls into the Rust side (desktop/src-tauri). Field names follow
// what the Rust types serialize to: camelCase for the desktop's own types,
// snake_case for types that come straight from mx-core.

import { invoke } from "@tauri-apps/api/core";

export type Range = { start: number; end: number };

export type MediaSummary = {
  path: string;
  name: string;
  duration: number;
  size: number;
  hasVideo: boolean;
  hasAudio: boolean;
  width: number | null;
  height: number | null;
  fps: number | null;
  videoCodec: string | null;
  audioCodec: string | null;
};

export type Preview = { kind: "direct" | "remux" | "proxy"; src: string };

export type Filmstrip = { path: string; count: number; thumb_width: number; thumb_height: number };

export type SilenceReport = {
  noise_floor_db: number;
  speech_db: number;
  threshold_db: number;
  silences: Range[];
};

export type Breakdown = {
  loudness: number | null;
  activity: number | null;
  motion: number | null;
  pace: number | null;
};
export type Highlight = { range: Range; score: number; breakdown: Breakdown };

export type Segment = { start: number; end: number; text: string };
export type Transcript = { segments: Segment[] };

export type CaptionStyle = {
  max_words: number;
  max_chars: number;
  size: number;
  position: number;
  font: string;
  uppercase: boolean;
  pop: boolean;
};

export type Fit = { kind: "crop"; position: number } | { kind: "blur" } | { kind: "pad" };
export type Frame = { aspect: { w: number; h: number }; fit: Fit; short_edge: number };

export type EditPlan = { cuts: Range[]; keep: Range[]; outputDuration: number };

export type TaskStatus = "running" | "done" | "failed" | "cancelled";
export type TaskUpdate = {
  id: string;
  kind: "download" | "export" | "analyse" | "transcribe" | "preview";
  title: string;
  status: TaskStatus;
  stage: string | null;
  percent: number | null;
  detail: string | null;
  output: string | null;
  error: string | null;
};

export type LinkInfo = {
  url: string;
  title: string | null;
  uploader: string | null;
  site: string | null;
  duration: number | null;
  thumbnail: string | null;
  heights: number[];
  isPlaylist: boolean;
  entries: number;
};

export type ModelInfo = { name: string; mb: number; installed: boolean; englishOnly: boolean };
export type Check = { name: string; found: string | null; fix: string };
export type EncoderInfo = { name: string; codec: "h264" | "hevc" | "av1"; hardware: boolean };
export type Exported = { output: string; size: number; duration: number; encoder: string | null };

let counter = 0;
/** A fresh id for a task, so the UI can cancel it before it returns. */
export const newTaskId = () => `t${Date.now().toString(36)}-${(counter++).toString(36)}`;

export const cancelTask = (id: string) => invoke<void>("cancel_task", { id });

export const probeMedia = (path: string) => invoke<MediaSummary>("probe_media", { path });
export const preparePreview = (taskId: string, path: string) => invoke<Preview>("prepare_preview", { taskId, path });
export const filmstrip = (taskId: string, path: string) => invoke<Filmstrip | null>("filmstrip", { taskId, path });
export const waveform = (taskId: string, path: string, bins: number) => invoke<number[]>("waveform", { taskId, path, bins });

export const detectSilences = (taskId: string, path: string, thresholdDb: number | null, minSilence: number) =>
  invoke<SilenceReport>("detect_silences", { taskId, path, thresholdDb, minSilence });

export const findHighlights = (taskId: string, path: string, count: number, length: number, from: number | null, to: number | null) =>
  invoke<Highlight[]>("highlights", { taskId, path, count, length, from, to });

export type EditPlanRequest = {
  duration: number;
  silences: Range[];
  padding: number;
  cuts: Range[];
  transcript: Transcript | null;
  removed: number[];
};
export const planEdit = (req: EditPlanRequest) => invoke<EditPlan>("plan_edit", { req });

export const captionDefaults = (width: number, height: number) => invoke<CaptionStyle>("caption_defaults", { width, height });
export const captionCues = (transcript: Transcript, style: CaptionStyle, keep: Range[]) =>
  invoke<Segment[]>("caption_cues", { transcript, style, keep });

export const listModels = () => invoke<ModelInfo[]>("list_models");
export const transcribe = (taskId: string, path: string, model: string, language: string, gpu: boolean) =>
  invoke<Transcript>("transcribe", { taskId, path, model, language, gpu });
export const saveSubtitles = (transcript: Transcript, keep: Range[] | null, path: string) =>
  invoke<void>("save_subtitles", { transcript, keep, path });
export const loadSubtitles = (path: string) => invoke<Transcript>("load_subtitles", { path });

export const fetchInfo = (url: string, cookiesFromBrowser: string | null) =>
  invoke<LinkInfo>("fetch_info", { url, cookiesFromBrowser });

export type DownloadOptions = {
  url: string;
  title: string | null;
  outputDir: string;
  maxHeight: number | null;
  audioOnly: boolean;
  audioFormat: string | null;
  compat: boolean;
  playlist: boolean;
  cookiesFromBrowser: string | null;
};
export const download = (taskId: string, opts: DownloadOptions) => invoke<string[]>("download", { taskId, opts });

export type ExportRequest = {
  input: string;
  output: string;
  keep: Range[];
  frame: Frame | null;
  captions: { transcript: Transcript; style: CaptionStyle } | null;
  codec: "h264" | "hevc" | "av1";
  quality: number | null;
  gpu: boolean;
  targetMb: number | null;
};
export const exportEdit = (taskId: string, req: ExportRequest) => invoke<Exported>("export", { taskId, req });

export type GifRequest = { input: string; output: string; start: number; end: number; fps: number; width: number };
export const exportGif = (taskId: string, req: GifRequest) => invoke<Exported>("export_gif", { taskId, req });

export const listEncoders = () => invoke<EncoderInfo[]>("list_encoders");
export const doctor = () => invoke<Check[]>("doctor");
export const startupFile = () => invoke<string | null>("startup_file");

/** Errors from commands arrive as strings; anything else is a bug. */
export const errorText = (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : String(e));
export const isCancelled = (e: unknown) => errorText(e) === "cancelled";
