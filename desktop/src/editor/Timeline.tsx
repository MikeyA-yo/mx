// The timeline: ruler, filmstrip, waveform, lanes for the layers (text,
// video, sound), and on top of them the cuts, the selection, highlights and
// the playhead. Click to seek, drag to select; drag a layer to move it,
// its edges to trim it. Everything is in source time.

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import type { Filmstrip, Highlight, Range } from "../api";
import { clock } from "../util";
import { sameLayer, type LayerBar, type LayerKind, type LayerRef } from "./state";

type Edge = "move" | "start" | "end";

type Props = {
  duration: number;
  time: number;
  playing: boolean;
  strip: Filmstrip | null;
  wave: number[] | null;
  cuts: Range[];
  selection: Range | null;
  highlights: Highlight[];
  /** Zoom/pan keyframe times. */
  keyframes: number[];
  layers: LayerBar[];
  selectedLayer: LayerRef | null;
  zoom: number;
  onZoom: (zoom: number) => void;
  onSeek: (t: number) => void;
  onSelect: (r: Range | null) => void;
  onPickLayer: (l: LayerRef | null) => void;
  /** A layer dragged to new times; `edge` says which part was held. */
  onMoveLayer: (l: LayerRef, start: number, end: number, edge: Edge) => void;
  /** A drag let go: one drag is one undo step. */
  onDragEnd: () => void;
};

const STRIP_HEIGHT = 48;
const TICK_STEPS = [0.5, 1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 900, 1800, 3600];
// Chromium refuses canvases wider than this.
const MAX_CANVAS = 16384;
const PAD = 14;
const MAX_ZOOM = 40;
/** How close, on screen, an edge has to come to something to snap to it. */
const SNAP_PX = 7;
/** The shortest a layer can be trimmed to. */
const MIN_LENGTH = 0.2;
const LANES: [LayerKind, string][] = [
  ["title", "Text"],
  ["overlay", "Video"],
  ["sound", "Sound"],
];

export default function Timeline(p: Props) {
  const scroller = useRef<HTMLDivElement>(null);
  const tracks = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const [viewWidth, setViewWidth] = useState(800);
  // The layer being dragged, for its time label.
  const [dragging, setDragging] = useState<LayerRef | null>(null);

  useLayoutEffect(() => {
    const el = scroller.current!;
    const ro = new ResizeObserver(() => setViewWidth(el.clientWidth - 2 * PAD));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const width = Math.max(200, viewWidth * p.zoom);
  const pps = width / Math.max(p.duration, 0.001);
  const x = (t: number) => t * pps;

  // Keep the playhead in view while playing zoomed in.
  useEffect(() => {
    const el = scroller.current;
    if (!el || !p.playing || p.zoom <= 1) return;
    const px = x(p.time);
    if (px < el.scrollLeft || px > el.scrollLeft + el.clientWidth - 60) el.scrollLeft = px - 60;
  });

  // Ctrl+wheel zooms around the pointer; the wheel alone scrolls along a
  // zoomed timeline.
  const live = useRef({ pps, zoom: p.zoom, onZoom: p.onZoom });
  live.current = { pps, zoom: p.zoom, onZoom: p.onZoom };
  const anchor = useRef<{ t: number; cx: number } | null>(null);
  useEffect(() => {
    const el = scroller.current!;
    const onWheel = (e: WheelEvent) => {
      const { pps, zoom, onZoom } = live.current;
      if (e.ctrlKey) {
        e.preventDefault();
        const cx = e.clientX - el.getBoundingClientRect().left;
        const next = Math.min(MAX_ZOOM, Math.max(1, zoom * (e.deltaY < 0 ? 1.25 : 0.8)));
        if (next === zoom) return;
        anchor.current = { t: (el.scrollLeft + cx - PAD) / pps, cx };
        onZoom(next);
      } else if (zoom > 1 && Math.abs(e.deltaY) > Math.abs(e.deltaX)) {
        e.preventDefault();
        el.scrollLeft += e.deltaY;
      }
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, []);
  useLayoutEffect(() => {
    const a = anchor.current;
    const el = scroller.current;
    if (!a || !el) return;
    anchor.current = null;
    el.scrollLeft = PAD + a.t * pps - a.cx;
  }, [width]);

  // Waveform: bars mirrored around the middle, red where they're cut.
  useEffect(() => {
    const c = canvas.current;
    if (!c) return;
    const dpr = window.devicePixelRatio || 1;
    const w = Math.min(MAX_CANVAS, Math.round(width * dpr));
    const h = Math.round(44 * dpr);
    c.width = w;
    c.height = h;
    const ctx = c.getContext("2d")!;
    ctx.clearRect(0, 0, w, h);
    if (!p.wave || p.wave.length === 0) return;
    const step = Math.max(1, Math.round(2 * dpr));
    const isCut = (t: number) => p.cuts.some((r) => t >= r.start && t < r.end);
    for (let px = 0; px < w; px += step) {
      const from = Math.floor((px / w) * p.wave.length);
      const to = Math.max(from + 1, Math.floor(((px + step) / w) * p.wave.length));
      let v = 0;
      for (let i = from; i < to && i < p.wave.length; i++) v = Math.max(v, p.wave[i]);
      const bar = Math.max(1, v * (h - 4));
      ctx.fillStyle = isCut(((px + step / 2) / w) * p.duration) ? "#ff5470" : "#8a8fff";
      ctx.globalAlpha = 0.85;
      ctx.fillRect(px, (h - bar) / 2, Math.max(1, step - dpr), bar);
    }
  }, [p.wave, p.cuts, width, p.duration]);

  // Ruler ticks at a step that leaves room for their labels.
  const tickStep = TICK_STEPS.find((s) => s * pps >= 72) ?? 3600;
  const ticks: number[] = [];
  for (let t = 0; t <= p.duration; t += tickStep) ticks.push(t);

  // Filmstrip tiles: as many as fit, each showing the thumbnail nearest
  // its own time.
  const tiles: { left: number; index: number }[] = [];
  let tileWidth = 0;
  if (p.strip) {
    tileWidth = (STRIP_HEIGHT * p.strip.thumb_width) / p.strip.thumb_height;
    for (let left = 0; left < width; left += tileWidth) {
      const t = ((left + tileWidth / 2) / width) * p.duration;
      tiles.push({ left, index: Math.min(p.strip.count - 1, Math.floor((t / p.duration) * p.strip.count)) });
    }
  }
  const stripUrl = p.strip ? convertFileSrc(p.strip.path) : "";

  const timeAt = (clientX: number) => {
    const rect = tracks.current!.getBoundingClientRect();
    return Math.min(p.duration, Math.max(0, (clientX - rect.left) / pps));
  };

  /** Follows the mouse until it's released; `move` gets the distance moved, in seconds. */
  const follow = (e: React.MouseEvent, move: (dt: number, ev: MouseEvent) => void, done: (moved: boolean) => void) => {
    const startX = e.clientX;
    let moved = false;
    const onMove = (ev: MouseEvent) => {
      if (!moved && Math.abs(ev.clientX - startX) < 4) return;
      moved = true;
      move((ev.clientX - startX) / pps, ev);
    };
    const onUp = () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
      done(moved);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  };

  // Empty timeline: click to seek, drag to select.
  const onMouseDown = (e: React.MouseEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    const t0 = timeAt(e.clientX);
    follow(
      e,
      (_, ev) => {
        const t1 = timeAt(ev.clientX);
        p.onSelect({ start: Math.min(t0, t1), end: Math.max(t0, t1) });
      },
      (moved) => {
        if (moved) return;
        p.onSelect(null);
        p.onPickLayer(null);
        p.onSeek(t0);
      },
    );
  };

  // The selection's edges drag on their own.
  const dragSelection = (e: React.MouseEvent, side: "start" | "end") => {
    if (e.button !== 0 || !p.selection) return;
    e.stopPropagation();
    const fixed = side === "start" ? p.selection.end : p.selection.start;
    follow(
      e,
      (_, ev) => {
        const t = timeAt(ev.clientX);
        p.onSelect({ start: Math.min(t, fixed), end: Math.max(t, fixed) });
      },
      () => {},
    );
  };

  // Layers: the middle moves, the edges trim. Edges snap to the playhead,
  // the clip's ends, cuts and other layers; Alt drags freely.
  const dragLayer = (e: React.MouseEvent, bar: LayerBar, edge: Edge) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    e.preventDefault();
    p.onPickLayer(bar.ref);
    const { start: s0, end: e0 } = bar;
    const length = e0 - s0;
    const points = [
      0,
      p.duration,
      p.time,
      ...p.cuts.flatMap((c) => [c.start, c.end]),
      ...p.layers.filter((b) => !sameLayer(b.ref, bar.ref)).flatMap((b) => [b.start, b.end]),
    ];
    const snap = (t: number, free: boolean): number | null => {
      if (free) return null;
      let best: number | null = null;
      for (const q of points) if (Math.abs(q - t) * pps <= SNAP_PX && (best === null || Math.abs(q - t) < Math.abs(best - t))) best = q;
      return best;
    };
    setDragging(bar.ref);
    follow(
      e,
      (dt, ev) => {
        let start = s0;
        let end = e0;
        if (edge === "move") {
          start = s0 + dt;
          const a = snap(start, ev.altKey);
          const b = snap(start + length, ev.altKey);
          if (a !== null && (b === null || Math.abs(a - start) <= Math.abs(b - start - length))) start = a;
          else if (b !== null) start = b - length;
          const last = bar.bounded ? p.duration - length : p.duration - MIN_LENGTH;
          start = Math.max(0, Math.min(last, start));
          end = start + length;
        } else if (edge === "start") {
          start = s0 + dt;
          start = snap(start, ev.altKey) ?? start;
          start = Math.max(Math.max(0, bar.earliest), Math.min(e0 - MIN_LENGTH, start));
        } else {
          end = e0 + dt;
          end = snap(end, ev.altKey) ?? end;
          end = Math.min(bar.latest, Math.max(s0 + MIN_LENGTH, end));
        }
        p.onMoveLayer(bar.ref, start, end, edge);
      },
      (moved) => {
        setDragging(null);
        p.onDragEnd();
        // A click shows it: jump there unless the playhead's already inside.
        if (!moved && !(p.time >= s0 && p.time < e0)) p.onSeek(s0);
      },
    );
  };

  return (
    <div className="timeline-scroll" ref={scroller}>
      <div className="tracks" style={{ width }} onMouseDown={onMouseDown} ref={tracks}>
        <div className="ruler">
          {ticks.map((t) => (
            <span key={t} style={{ left: x(t) - 3 }}>
              {clock(t, tickStep < 1)}
            </span>
          ))}
        </div>
        <div className="strip">
          {tiles.map((tile) => (
            <div
              key={tile.left}
              style={{
                width: tileWidth,
                backgroundImage: `url("${stripUrl}")`,
                backgroundSize: `${p.strip!.count * tileWidth}px ${STRIP_HEIGHT}px`,
                backgroundPosition: `${-tile.index * tileWidth}px 0`,
              }}
            />
          ))}
        </div>
        <canvas className="wave" ref={canvas} style={{ width }} />
        <div className="lanes">
          {LANES.map(([kind, label]) => (
            <div key={kind} className={`lane lane-${kind}`}>
              <span className="lane-label">{label}</span>
              {p.layers
                .filter((b) => b.ref.kind === kind)
                .map((b) => {
                  const left = x(b.start);
                  // Sounds and videos can run past the clip; draw them to its end.
                  const w = Math.max(8, Math.min(width - left, x(b.end - b.start)));
                  const on = sameLayer(b.ref, p.selectedLayer);
                  const runsOn = b.end > p.duration + 0.05;
                  return (
                    <div
                      key={b.ref.index}
                      className={`layer-bar${on ? " on" : ""}${runsOn ? " runs-on" : ""}`}
                      style={{ left, width: w }}
                      title={`${b.label}: ${clock(b.start, true)}–${clock(b.end, true)}${runsOn ? " (carries on into the next clips)" : ""}. Drag to move, drag an edge to trim.`}
                      onMouseDown={(e) => dragLayer(e, b, "move")}
                    >
                      <span className="layer-name">{b.label}</span>
                      <div className="grip start" onMouseDown={(e) => dragLayer(e, b, "start")} />
                      <div className="grip end" onMouseDown={(e) => dragLayer(e, b, "end")} />
                      {sameLayer(b.ref, dragging) && (
                        <span className="drag-tip">
                          {clock(b.start, true)}–{clock(b.end, true)} · {(b.end - b.start).toFixed(1)} s
                        </span>
                      )}
                    </div>
                  );
                })}
            </div>
          ))}
        </div>
        <div className="marks">
          {p.cuts.map((r) => (
            <div key={`${r.start}`} className="mark-cut" style={{ left: x(r.start), width: Math.max(1, x(r.end - r.start)) }} />
          ))}
          {p.selection && (
            <div className="mark-sel" style={{ left: x(p.selection.start), width: Math.max(2, x(p.selection.end - p.selection.start)) }}>
              <div className="sel-grip start" onMouseDown={(e) => dragSelection(e, "start")} title="Drag to change the start" />
              <div className="sel-grip end" onMouseDown={(e) => dragSelection(e, "end")} title="Drag to change the end" />
            </div>
          )}
          {p.highlights.map((h, i) => (
            <div
              key={i}
              className="mark-hl"
              title={`Highlight #${i + 1}: ${clock(h.range.start)} - ${clock(h.range.end)}`}
              style={{ left: x(h.range.start), width: Math.max(14, x(h.range.end - h.range.start)) }}
              onMouseDown={(e) => {
                e.stopPropagation();
                p.onSelect(h.range);
                p.onSeek(h.range.start);
              }}
            >
              #{i + 1}
            </div>
          ))}
          {p.keyframes.map((t, i) => (
            <div key={`k${i}`} className="mark-key" style={{ left: x(t) }} title={`Zoom/pan keyframe at ${clock(t, true)}`} />
          ))}
        </div>
        <div className="playhead" style={{ left: x(p.time) }} />
      </div>
    </div>
  );
}
