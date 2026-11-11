// The timeline: ruler, filmstrip, waveform, lanes for the layers (text,
// video, sound), and on top of them the cuts, the selection, highlights and
// the playhead. Click to seek, drag to select; drag a layer to move it,
// its edges to trim it.
//
// It draws either one clip in its own time (cut parts shown in red) or the
// whole video in output time, the clips end to end as blocks that can be
// dragged to reorder and trimmed at their edges. The pictures say which.

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import type { Highlight, Range } from "../api";
import { clock } from "../util";
import { layerKey, type ClipBlock, type LayerBar, type LayerKind, type Picture } from "./state";
import { trackHeights, useLayout } from "./layout";

type Edge = "move" | "start" | "end";

type Props = {
  duration: number;
  time: number;
  playing: boolean;
  pictures: Picture[];
  /** Drawn red: what's cut (the clip timeline). */
  cuts: Range[];
  selection: Range | null;
  highlights: Highlight[];
  /** Zoom/pan keyframe times. */
  keyframes: number[];
  layers: LayerBar[];
  selectedKey: string | null;
  /** The clips, on the whole-video timeline. */
  blocks?: ClipBlock[];
  zoom: number;
  onZoom: (zoom: number) => void;
  /** Where it was scrolled is kept under this name while it's away. */
  scrollKey: string;
  onSeek: (t: number) => void;
  onSelect: (r: Range | null) => void;
  onPickLayer: (bar: LayerBar | null) => void;
  /** A layer dragged to new times; `edge` says which part was held. */
  onMoveLayer: (bar: LayerBar, start: number, end: number, edge: Edge) => void;
  onPickBlock?: (id: string, t: number) => void;
  /** A clip dragged to a new place in the order. */
  onMoveBlock?: (id: string, index: number) => void;
  /** A clip's edge dragged by `by` seconds (positive is later). */
  onTrimBlock?: (id: string, edge: "start" | "end", by: number) => void;
};

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
  ["overlay", "Video & image"],
  ["sound", "Sound"],
];
const scrolls = new Map<string, number>();

type Ghost = { key: string; start: number; end: number };

export default function Timeline(p: Props) {
  const scroller = useRef<HTMLDivElement>(null);
  const tracks = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const heights = trackHeights(useLayout().timeline);
  const [viewWidth, setViewWidth] = useState(800);
  // What's being dragged, drawn where it would land; nothing changes
  // until it's let go.
  const [ghost, setGhost] = useState<Ghost | null>(null);

  useLayoutEffect(() => {
    const el = scroller.current!;
    const ro = new ResizeObserver(() => setViewWidth(el.clientWidth - 2 * PAD));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const width = Math.max(200, viewWidth * p.zoom);
  const pps = width / Math.max(p.duration, 0.001);
  const x = (t: number) => t * pps;

  // Back where it was scrolled, once it's as wide as it was.
  const restored = useRef(false);
  useLayoutEffect(() => {
    if (restored.current || viewWidth === 800) return;
    restored.current = true;
    scroller.current!.scrollLeft = scrolls.get(p.scrollKey) ?? 0;
  }, [viewWidth]);

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

  const pictureAt = (t: number) => {
    for (let i = p.pictures.length - 1; i >= 0; i--) if (t >= p.pictures[i].start && t < p.pictures[i].end) return p.pictures[i];
    return null;
  };

  // Waveform: bars mirrored around the middle, red where they're cut;
  // each column shows the loudest moment of the source it covers.
  useEffect(() => {
    const c = canvas.current;
    if (!c) return;
    const dpr = window.devicePixelRatio || 1;
    const w = Math.min(MAX_CANVAS, Math.round(width * dpr));
    const h = Math.round(heights.wave * dpr);
    c.width = w;
    c.height = h;
    const ctx = c.getContext("2d")!;
    ctx.clearRect(0, 0, w, h);
    const step = Math.max(1, Math.round(2 * dpr));
    const span = (step / w) * p.duration;
    const isCut = (t: number) => p.cuts.some((r) => t >= r.start && t < r.end);
    ctx.globalAlpha = 0.85;
    for (let px = 0; px < w; px += step) {
      const t = (px / w) * p.duration;
      const pic = pictureAt(t);
      if (!pic?.wave?.length) continue;
      const bins = pic.wave.length / Math.max(pic.sourceDuration, 0.001);
      const from = Math.floor(pic.toSource(t) * bins);
      const to = Math.max(from + 1, Math.min(from + Math.ceil(span * bins * 2), Math.floor(pic.toSource(Math.min(pic.end, t + span)) * bins)));
      let v = 0;
      for (let i = Math.max(0, from); i < to && i < pic.wave.length; i++) v = Math.max(v, pic.wave[i]);
      const bar = Math.max(1, v * (h - 4));
      ctx.fillStyle = isCut(t + span / 2) ? "#ff5470" : "#8a8fff";
      ctx.fillRect(px, (h - bar) / 2, Math.max(1, step - dpr), bar);
    }
  }, [p.pictures, p.cuts, width, p.duration, heights.wave]);

  // Ruler ticks at a step that leaves room for their labels.
  const tickStep = TICK_STEPS.find((s) => s * pps >= 72) ?? 3600;
  const ticks: number[] = [];
  for (let t = 0; t <= p.duration; t += tickStep) ticks.push(t);

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

  // Empty timeline: click to seek, drag to select. A click keeps the
  // selection (there's more to do with it than cut); a double-click
  // clears it, leaving the playhead where it was before.
  const lastClick = useRef({ at: 0, time: 0 });
  const onMouseDown = (e: React.MouseEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    const t0 = timeAt(e.clientX);
    const now = performance.now();
    if (now - lastClick.current.at > 500) lastClick.current.time = p.time;
    lastClick.current.at = now;
    follow(
      e,
      (_, ev) => {
        const t1 = timeAt(ev.clientX);
        p.onSelect({ start: Math.min(t0, t1), end: Math.max(t0, t1) });
      },
      (moved) => {
        if (moved) return;
        p.onPickLayer(null);
        p.onSeek(t0);
      },
    );
  };
  const onDoubleClick = (e: React.MouseEvent<HTMLDivElement>) => {
    if (!p.selection || (e.target as HTMLElement).closest(".layer-bar, .clip-block, .mark-hl")) return;
    p.onSelect(null);
    p.onSeek(lastClick.current.time);
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

  /** The nearest thing to snap to, within reach on screen; Alt drags freely. */
  const snapper = (points: number[]) => (t: number, free: boolean) => {
    if (free) return null;
    let best: number | null = null;
    for (const q of points) if (Math.abs(q - t) * pps <= SNAP_PX && (best === null || Math.abs(q - t) < Math.abs(best - t))) best = q;
    return best;
  };

  // Layers: the middle moves, the edges trim. Edges snap to the playhead,
  // the ends of clips, cuts and other layers.
  const dragLayer = (e: React.MouseEvent, bar: LayerBar, edge: Edge) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    e.preventDefault();
    const key = layerKey(bar.clip, bar.ref);
    const { start: s0, end: e0 } = bar;
    const length = e0 - s0;
    const snap = snapper([
      0,
      p.duration,
      p.time,
      ...p.cuts.flatMap((c) => [c.start, c.end]),
      ...(p.blocks ?? []).flatMap((b) => [b.start, b.end]),
      ...p.layers.filter((b) => layerKey(b.clip, b.ref) !== key).flatMap((b) => [b.start, b.end]),
    ]);
    let landed = { start: s0, end: e0 };
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
        landed = { start, end };
        setGhost({ key, start, end });
      },
      (moved) => {
        setGhost(null);
        if (moved) p.onMoveLayer(bar, landed.start, landed.end, edge);
        p.onPickLayer(bar);
        // A click shows it: jump there unless the playhead's already inside.
        if (!moved && !(p.time >= s0 && p.time < e0)) p.onSeek(s0);
      },
    );
  };

  // Clips: the middle reorders, the edges trim the clip's start or end.
  const dragBlock = (e: React.MouseEvent, block: ClipBlock, edge: Edge) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    e.preventDefault();
    const key = `clip:${block.id}`;
    const { start: s0, end: e0 } = block;
    const others = (p.blocks ?? []).filter((b) => b.id !== block.id);
    const snap = snapper([p.time, ...others.flatMap((b) => [b.start, b.end])]);
    let by = 0;
    const t0 = timeAt(e.clientX);
    follow(
      e,
      (dt, ev) => {
        if (edge === "move") {
          by = dt;
          setGhost({ key, start: s0 + dt, end: e0 + dt });
        } else if (edge === "start") {
          const start = Math.max(0, Math.min(e0 - 0.3, snap(s0 + dt, ev.altKey) ?? s0 + dt));
          by = start - s0;
          setGhost({ key, start, end: e0 });
        } else {
          const end = Math.max(s0 + 0.3, snap(e0 + dt, ev.altKey) ?? e0 + dt);
          by = end - e0;
          setGhost({ key, start: s0, end });
        }
      },
      (moved) => {
        setGhost(null);
        if (!moved) return p.onPickBlock?.(block.id, t0);
        if (edge === "move") {
          const mid = (s0 + e0) / 2 + by;
          p.onMoveBlock?.(block.id, others.filter((b) => (b.start + b.end) / 2 < mid).length);
        } else if (Math.abs(by) > 0.02) p.onTrimBlock?.(block.id, edge, by);
      },
    );
  };

  const placed = <T extends { start: number; end: number }>(key: string, item: T) =>
    ghost?.key === key ? { ...item, start: ghost.start, end: ghost.end } : item;

  return (
    <div className="timeline-scroll" ref={scroller} onScroll={(e) => scrolls.set(p.scrollKey, e.currentTarget.scrollLeft)}>
      <div className="tracks" style={{ width }} onMouseDown={onMouseDown} onDoubleClick={onDoubleClick} ref={tracks}>
        <div className="ruler">
          {ticks.map((t) => (
            <span key={t} style={{ left: x(t) - 3 }}>
              {clock(t, tickStep < 1)}
            </span>
          ))}
        </div>
        {p.blocks && (
          <div className="clip-row">
            {p.blocks.map((raw) => {
              const key = `clip:${raw.id}`;
              const b = placed(key, raw);
              return (
                <div
                  key={raw.id}
                  className={`clip-block${raw.active ? " active" : ""}${ghost?.key === key ? " dragging" : ""}`}
                  style={{ left: x(b.start), width: Math.max(10, x(b.end - b.start)) }}
                  title={`Clip ${raw.n}: ${raw.label}, ${clock(raw.end - raw.start, true)}. Click to edit it, drag to reorder, drag an edge to trim.`}
                  onMouseDown={(e) => dragBlock(e, raw, "move")}
                >
                  {raw.transition && <span className="clip-join" title={raw.transition} />}
                  <span className="layer-name">
                    <b>{raw.n}</b> {raw.label}
                  </span>
                  <div className="grip start" onMouseDown={(e) => dragBlock(e, raw, "start")} />
                  <div className="grip end" onMouseDown={(e) => dragBlock(e, raw, "end")} />
                  {ghost?.key === key && (
                    <span className="drag-tip">
                      {clock(b.start, true)}–{clock(b.end, true)} · {(b.end - b.start).toFixed(1)} s
                    </span>
                  )}
                </div>
              );
            })}
          </div>
        )}
        <div className="strip" style={{ height: heights.strip }}>
          {p.pictures.map((pic, i) => {
            const left = x(pic.start);
            const w = x(pic.end - pic.start);
            const tiles: { left: number; index: number }[] = [];
            const tileWidth = pic.strip ? (heights.strip * pic.strip.thumb_width) / pic.strip.thumb_height : 0;
            if (pic.strip && tileWidth > 0) {
              for (let l = 0; l < w; l += tileWidth) {
                const s = pic.toSource(pic.start + Math.min(w, l + tileWidth / 2) / pps);
                tiles.push({ left: l, index: Math.min(pic.strip.count - 1, Math.floor((s / pic.sourceDuration) * pic.strip.count)) });
              }
            }
            const url = pic.strip ? convertFileSrc(pic.strip.path) : "";
            return (
              <div key={i} className="pic" style={{ left, width: w }}>
                {tiles.map((tile) => (
                  <div
                    key={tile.left}
                    style={{
                      left: tile.left,
                      width: tileWidth,
                      backgroundImage: `url("${url}")`,
                      backgroundSize: `${pic.strip!.count * tileWidth}px ${heights.strip}px`,
                      backgroundPosition: `${-tile.index * tileWidth}px 0`,
                    }}
                  />
                ))}
              </div>
            );
          })}
        </div>
        <canvas className="wave" ref={canvas} style={{ width, height: heights.wave }} />
        <div className="lanes">
          {LANES.map(([kind, label]) => (
            <div key={kind} className={`lane lane-${kind}`}>
              <span className="lane-label">{label}</span>
              {p.layers
                .filter((raw) => raw.ref.kind === kind)
                .map((raw) => {
                  const key = layerKey(raw.clip, raw.ref);
                  const b = placed(key, raw);
                  const left = x(b.start);
                  // Sounds and videos can run past the end; draw them to it.
                  const w = Math.max(8, Math.min(width - left, x(b.end - b.start)));
                  const runsOn = b.end > p.duration + 0.05;
                  // A title inside a cut plays for no time at all.
                  const gone = b.end - b.start < 0.05;
                  const note = gone ? " (inside a cut, so it doesn't show)" : runsOn ? " (carries on past the end)" : "";
                  return (
                    <div
                      key={key}
                      className={`layer-bar${key === p.selectedKey ? " on" : ""}${runsOn ? " runs-on" : ""}${gone ? " gone" : ""}`}
                      style={{ left, width: w }}
                      title={`${b.label}: ${clock(b.start, true)}–${clock(b.end, true)}${note}. Drag to move, drag an edge to trim.`}
                      onMouseDown={(e) => dragLayer(e, raw, "move")}
                    >
                      <span className="layer-name">{b.label}</span>
                      <div className="grip start" onMouseDown={(e) => dragLayer(e, raw, "start")} />
                      <div className="grip end" onMouseDown={(e) => dragLayer(e, raw, "end")} />
                      {ghost?.key === key && (
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
              <button
                className="sel-clear"
                title="Clear the selection (Esc, or double-click the timeline)"
                onMouseDown={(e) => e.stopPropagation()}
                onClick={() => p.onSelect(null)}
              >
                ✕
              </button>
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
