// The editor's panel sizes: dragged at their edges like VS Code's, the
// same for every clip, and kept between runs.

import { useSyncExternalStore } from "react";
import { load, store } from "../util";

export type Layout = {
  /** Width of the side panel, px. */
  side: number;
  /** Extra height for the timeline's filmstrip and waveform, px (less than 0 shrinks them). */
  timeline: number;
};

export const DEFAULT_LAYOUT: Layout = { side: 340, timeline: 0 };

let current: Layout = { ...DEFAULT_LAYOUT, ...load<Partial<Layout>>("layout", {}) };
const listeners = new Set<() => void>();

export function setLayout(next: Partial<Layout>) {
  current = { ...current, ...next };
  listeners.forEach((l) => l());
}

/** Remembers the sizes, once a drag ends. */
export const saveLayout = () => store("layout", current);

export const useLayout = () =>
  useSyncExternalStore(
    (changed) => (listeners.add(changed), () => void listeners.delete(changed)),
    () => current,
  );

/** The timeline's filmstrip and waveform heights for its extra height. */
export const trackHeights = (extra: number) => ({
  strip: Math.round(Math.min(200, Math.max(28, 48 + extra / 2))),
  wave: Math.round(Math.min(200, Math.max(24, 44 + extra / 2))),
});

/** A divider between two panels, for a size `value`: dragging it reports
 * the value it began at and how far it has moved; a double-click puts it back. */
export function Splitter(p: {
  dir: "x" | "y";
  value: number;
  onDrag: (start: number, moved: number) => void;
  onReset: () => void;
  title: string;
}) {
  return (
    <div
      className={`splitter splitter-${p.dir}`}
      title={p.title}
      onDoubleClick={() => (p.onReset(), saveLayout())}
      onPointerDown={(e) => {
        if (e.button !== 0) return;
        e.preventDefault();
        const from = p.dir === "x" ? e.clientX : e.clientY;
        const start = p.value;
        document.body.classList.add(`resizing-${p.dir}`);
        const move = (ev: PointerEvent) => p.onDrag(start, (p.dir === "x" ? ev.clientX : ev.clientY) - from);
        const up = () => {
          window.removeEventListener("pointermove", move);
          window.removeEventListener("pointerup", up);
          document.body.classList.remove(`resizing-${p.dir}`);
          saveLayout();
        };
        window.addEventListener("pointermove", move);
        window.addEventListener("pointerup", up);
      }}
    />
  );
}
