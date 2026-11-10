// Formatting, paths and small persistent preferences.

/** 75.4 -> "1:15", 3725 -> "1:02:05"; `precise` adds tenths: "1:15.4". */
export function clock(secs: number, precise = false): string {
  const s = Math.max(0, secs);
  const whole = Math.floor(s);
  const h = Math.floor(whole / 3600);
  const m = Math.floor((whole % 3600) / 60);
  const sec = whole % 60;
  const tenths = precise ? `.${Math.floor((s - whole) * 10)}` : "";
  const mm = h > 0 ? String(m).padStart(2, "0") : String(m);
  return `${h > 0 ? `${h}:` : ""}${mm}:${String(sec).padStart(2, "0")}${tenths}`;
}

/** Longer spans in words: "25 min 9 s", "42 s". */
export function span(secs: number): string {
  const s = Math.round(secs);
  if (s < 60) return `${s} s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m} min${s % 60 ? ` ${s % 60} s` : ""}`;
  return `${Math.floor(m / 60)} h ${m % 60} min`;
}

export function bytes(n: number): string {
  if (n < 1e3) return `${n} B`;
  if (n < 1e6) return `${(n / 1e3).toFixed(0)} KB`;
  if (n < 1e9) return `${(n / 1e6).toFixed(1)} MB`;
  return `${(n / 1e9).toFixed(2)} GB`;
}

export const baseName = (path: string) => path.split(/[\\/]/).pop() ?? path;

/** Still images: they go over a video as layers, never as clips. (A GIF
 * moves, so it's a video.) */
export const IMAGE_EXTENSIONS = ["png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff"];
export const isImage = (path: string) => IMAGE_EXTENSIONS.includes(path.split(".").pop()?.toLowerCase() ?? "");
export const dirName = (path: string) => path.replace(/[\\/][^\\/]*$/, "");
export const stem = (path: string) => baseName(path).replace(/\.[^.]+$/, "");
export const sep = (path: string) => (path.includes("\\") ? "\\" : "/");
export const join = (dir: string, name: string) => `${dir}${sep(dir)}${name}`;

/** Preferences that survive restarts. Storage can be unavailable; then
 * defaults apply and nothing is remembered. */
export function load<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(`mx.${key}`);
    return raw === null ? fallback : (JSON.parse(raw) as T);
  } catch {
    return fallback;
  }
}

export function store<T>(key: string, value: T) {
  try {
    localStorage.setItem(`mx.${key}`, JSON.stringify(value));
  } catch {
    // Not remembered this time; nothing depends on it.
  }
}

/** Project files saved or opened, newest first. Older versions kept
 * only the last one, as `lastProject`. */
export function recentProjectPaths(): string[] {
  const list = load<string[]>("projects", []);
  const last = load<string | null>("lastProject", null);
  return last && !list.includes(last) ? [...list, last] : list;
}

export function rememberProject(path: string) {
  store("projects", [path, ...recentProjectPaths().filter((p) => p !== path)].slice(0, 20));
}

export function forgetProject(path: string) {
  store("projects", recentProjectPaths().filter((p) => p !== path));
  if (load<string | null>("lastProject", null) === path) store("lastProject", null);
}

/** How long ago, roughly: "just now", "12 min ago", "3 h ago", "yesterday", "4 days ago", or the date. */
export function ago(ms: number): string {
  const s = (Date.now() - ms) / 1000;
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.floor(s / 60)} min ago`;
  if (s < 86400) return `${Math.floor(s / 3600)} h ago`;
  if (s < 2 * 86400) return "yesterday";
  if (s < 14 * 86400) return `${Math.floor(s / 86400)} days ago`;
  return new Date(ms).toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" });
}

/** The most recently opened files, newest first. */
export function rememberRecent(path: string) {
  const recent = load<string[]>("recent", []).filter((p) => p !== path);
  store("recent", [path, ...recent].slice(0, 8));
}
