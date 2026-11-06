// Undo and redo for the whole project. Each edit records what one piece of
// state was and became; undoing puts the old value back through the
// owner's own setter, so it works for any clip, even one removed and
// brought back since. Quick runs of the same edit (a slider drag, a layer
// dragged along the timeline) collapse into one step.

import { useCallback, useRef, useState } from "react";

/** One piece of state: `scope` is a clip's id, or "project". */
export type Change = { scope: string; key: string; before: unknown; after: unknown };
type Entry = { changes: Change[]; at: number };

/** How long a pause ends a run of the same edit. */
const RUN_MS = 900;

/** What each piece of state is, for the Undo button's tooltip. */
const NAMES: Record<string, string> = {
  manualCuts: "cut",
  removed: "transcript edit",
  titles: "text",
  overlays: "video layer",
  sounds: "sound",
  motion: "zoom and pan",
  volume: "clip volume",
  removePauses: "pause removal",
  padding: "breathing room",
  frame: "frame",
  music: "music",
  style: "caption style",
  captionsOn: "captions",
  transitions: "transition",
  clips: "clips",
};

/** Whether a change only adjusts the last one, so the two can be one step:
 * a slider or a drag. Adding or removing something (a list growing or
 * shrinking), clips moving and switches flipping are steps of their own. */
const DISCRETE = new Set(["clips", "transitions", "captionsOn", "removePauses"]);
const adjusts = (previous: unknown, c: Change) =>
  !DISCRETE.has(c.key) && (!Array.isArray(previous) || !Array.isArray(c.after) || previous.length === c.after.length);

export type History = {
  record: (changes: Change[]) => void;
  undo: () => void;
  redo: () => void;
  clear: () => void;
  /** Ends a run: the next edit is its own step, even straight after. */
  seal: () => void;
  /** What Undo and Redo would change, or null when there's nothing. */
  undoName: string | null;
  redoName: string | null;
};

export function useHistory(apply: (scope: string, key: string, value: unknown) => void): History {
  const done = useRef<Entry[]>([]);
  const undone = useRef<Entry[]>([]);
  const [, bump] = useState(0);
  const applyRef = useRef(apply);
  applyRef.current = apply;

  const record = useCallback((changes: Change[]) => {
    const real = changes.filter((c) => c.before !== c.after);
    if (!real.length) return;
    const now = Date.now();
    const top = done.current[done.current.length - 1];
    const same =
      top &&
      now - top.at < RUN_MS &&
      top.changes.length === real.length &&
      top.changes.every((c, i) => c.scope === real[i].scope && c.key === real[i].key && adjusts(c.after, real[i]));
    if (same) {
      top.changes.forEach((c, i) => (c.after = real[i].after));
      top.at = now;
    } else {
      done.current.push({ changes: real, at: now });
      if (done.current.length > 200) done.current.shift();
    }
    undone.current = [];
    bump((n) => n + 1);
  }, []);

  const step = (from: Entry[], to: Entry[], value: (c: Change) => unknown) => {
    const entry = from.pop();
    if (!entry) return;
    entry.changes.forEach((c) => applyRef.current(c.scope, c.key, value(c)));
    // A fresh edit after this never joins the step just moved.
    entry.at = 0;
    to.push(entry);
    bump((n) => n + 1);
  };
  const undo = useCallback(() => step(done.current, undone.current, (c) => c.before), []);
  const redo = useCallback(() => step(undone.current, done.current, (c) => c.after), []);
  const clear = useCallback(() => {
    done.current = [];
    undone.current = [];
    bump((n) => n + 1);
  }, []);

  const seal = useCallback(() => {
    const top = done.current[done.current.length - 1];
    if (top) top.at = 0;
  }, []);

  const name = (e: Entry | undefined) => (e ? (NAMES[e.changes[0].key] ?? "edit") : null);
  return {
    record,
    undo,
    redo,
    clear,
    seal,
    undoName: name(done.current[done.current.length - 1]),
    redoName: name(undone.current[undone.current.length - 1]),
  };
}
