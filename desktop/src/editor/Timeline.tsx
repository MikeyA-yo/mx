// The timeline: ruler, filmstrip, waveform, and on top of them the cuts,
// the selection, highlights and the playhead. Click to seek, drag to
// select. Everything is in source time.

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import type { Filmstrip, Highlight, Range, Title } from "../api";
import { clock } from "../util";

type Props = {
  duration: number;
  time: number;
  playing: boolean;
  strip: Filmstrip | null;
  wave: number[] | null;
  cuts: Range[];
  selection: Range | null;
  highlights: Highlight[];
  titles: Title[];
  /** Zoom/pan keyframe times. */
  keyframes: number[];
  zoom: number;
  onSeek: (t: number) => void;
  onSelect: (r: Range | null) => void;
  onPickTitle: (i: number) => void;
};

const STRIP_HEIGHT = 48;
const TICK_STEPS = [0.5, 1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 900, 1800, 3600];
// Chromium refuses canvases wider than this.
const MAX_CANVAS = 16384;

export default function Timeline(p: Props) {
  const scroller = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const [viewWidth, setViewWidth] = useState(800);

  useLayoutEffect(() => {
    const el = scroller.current!;
    const ro = new ResizeObserver(() => setViewWidth(el.clientWidth - 28));
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

  const timeAt = (clientX: number, el: HTMLElement) => {
    const rect = el.getBoundingClientRect();
    return Math.min(p.duration, Math.max(0, (clientX - rect.left) / pps));
  };

  const onMouseDown = (e: React.MouseEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    const el = e.currentTarget;
    const startX = e.clientX;
    const t0 = timeAt(e.clientX, el);
    let dragging = false;
    const move = (ev: MouseEvent) => {
      if (!dragging && Math.abs(ev.clientX - startX) < 4) return;
      dragging = true;
      const t1 = timeAt(ev.clientX, el);
      p.onSelect({ start: Math.min(t0, t1), end: Math.max(t0, t1) });
    };
    const up = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
      if (!dragging) {
        p.onSelect(null);
        p.onSeek(t0);
      }
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  };

  return (
    <div className="timeline-scroll" ref={scroller}>
      <div className="tracks" style={{ width }} onMouseDown={onMouseDown}>
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
        <div className="marks">
          {p.cuts.map((r) => (
            <div key={`${r.start}`} className="mark-cut" style={{ left: x(r.start), width: Math.max(1, x(r.end - r.start)) }} />
          ))}
          {p.selection && (
            <div className="mark-sel" style={{ left: x(p.selection.start), width: Math.max(2, x(p.selection.end - p.selection.start)) }} />
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
          {p.titles.map((t, i) => (
            <div
              key={`t${i}`}
              className="mark-title"
              title={t.text}
              style={{ left: x(t.start), width: Math.max(6, x(t.end - t.start)) }}
              onMouseDown={(e) => {
                e.stopPropagation();
                p.onPickTitle(i);
                p.onSeek(t.start);
              }}
            >
              {t.text}
            </div>
          ))}
        </div>
        <div className="playhead" style={{ left: x(p.time) }} />
      </div>
    </div>
  );
}
