// Every clip at a glance, to put them in order: drag one (or a few
// picked together) to where it goes, or send them to the start, the end
// or a numbered place. Clips split at their cuts here too, ready to be
// spread out. A player beside the grid watches a clip as it'll be cut,
// and plays on into the clips after it, to check the order.

import { useEffect, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import type { Filmstrip, Range } from "../api";
import { clock } from "../util";
import { fromOutput, toOutput } from "./state";

export type ArrangeItem = {
  id: string;
  name: string;
  /** Its length after cuts, once known. */
  length: number | null;
  strip: Filmstrip | null;
  /** What's kept of its file, in the file's time. */
  keep: Range[];
  sourceDuration: number;
  /** How many clips it makes split at its cuts. */
  pieces: number;
  /** Its playable preview, once made. */
  src: string | null;
  volume: number;
  mutes: Range[];
};

type Props = {
  items: ArrangeItem[];
  active: string | null;
  total: number;
  onOrder: (ids: string[]) => void;
  onSplit: (ids: string[]) => void;
  onRemove: (ids: string[]) => void;
  /** Goes to a clip to edit it. */
  onOpen: (id: string) => void;
  onClose: () => void;
};

const CARD = 150;
const THUMB = 84;

export default function Arrange(p: Props) {
  const [picked, setPicked] = useState<Set<string>>(() => new Set(p.active ? [p.active] : []));
  const anchor = useRef<string | null>(p.active);
  const [drop, setDrop] = useState<number | null>(null);
  const [dragging, setDragging] = useState<string[] | null>(null);
  const [place, setPlace] = useState("");
  // The clip in the player, whether it's playing, and the picture skimmed.
  const [shown, setShown] = useState<string | null>(p.active ?? p.items[0]?.id ?? null);
  const [playing, setPlaying] = useState(false);
  const [skim, setSkim] = useState<{ id: string; at: number } | null>(null);
  const grid = useRef<HTMLDivElement>(null);
  const moved = useRef(false);
  const ids = p.items.map((i) => i.id);
  // Picked clips in their order; clips gone (removed, undone) drop out.
  const chosen = ids.filter((id) => picked.has(id));
  const showing = p.items.find((i) => i.id === shown) ?? null;
  const showingAt = showing ? ids.indexOf(showing.id) : -1;

  /** Puts `moving` at `k` among the other clips. */
  const moveTo = (moving: string[], k: number) => {
    const rest = ids.filter((id) => !moving.includes(id));
    const at = Math.max(0, Math.min(rest.length, k));
    p.onOrder([...rest.slice(0, at), ...moving, ...rest.slice(at)]);
  };
  /** The clip `by` places from the one in the player; past the last, it stops. */
  const step = (by: number) => {
    const next = ids[showingAt + by];
    if (next) setShown(next);
    else if (by > 0) setPlaying(false);
  };

  // The clip in the player stays in view as it plays on.
  useEffect(() => {
    grid.current?.querySelector(`[data-i="${showingAt}"]`)?.scrollIntoView({ block: "nearest" });
  }, [showingAt]);

  const live = useRef({ chosen, onClose: p.onClose, onRemove: p.onRemove, ids, step });
  live.current = { chosen, onClose: p.onClose, onRemove: p.onRemove, ids, step };
  // The editor's own keys stay out while this is open; undo still works.
  useEffect(() => {
    const keys = (e: KeyboardEvent) => {
      if (e.ctrlKey || e.metaKey) {
        if (e.key.toLowerCase() === "a" && (e.target as HTMLElement).tagName !== "INPUT") {
          e.preventDefault();
          e.stopPropagation();
          setPicked(new Set(live.current.ids));
        }
        return;
      }
      // A box here has its own keys (and the editor leaves boxes alone).
      if ((e.target as HTMLElement).tagName === "INPUT") return;
      e.stopPropagation();
      const { chosen } = live.current;
      if (e.key === "Escape") live.current.onClose();
      else if (e.key === " ") {
        e.preventDefault();
        setPlaying((on) => !on);
      } else if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
        e.preventDefault();
        live.current.step(e.key === "ArrowLeft" ? -1 : 1);
      } else if ((e.key === "Delete" || e.key === "Backspace") && chosen.length) {
        e.preventDefault();
        live.current.onRemove(chosen);
      }
    };
    window.addEventListener("keydown", keys, true);
    return () => window.removeEventListener("keydown", keys, true);
  }, []);

  const pick = (e: React.MouseEvent, id: string) => {
    if (e.ctrlKey || e.metaKey) {
      setPicked((was) => {
        const next = new Set(was);
        if (next.has(id)) next.delete(id);
        else next.add(id);
        return next;
      });
      anchor.current = id;
    } else if (e.shiftKey && anchor.current && ids.includes(anchor.current)) {
      const [a, b] = [ids.indexOf(anchor.current), ids.indexOf(id)].sort((x, y) => x - y);
      setPicked(new Set(ids.slice(a, b + 1)));
    } else {
      setPicked(new Set([id]));
      anchor.current = id;
      setShown(id);
    }
  };

  // Dragging: where the clips would land is drawn as a bar between cards,
  // and the grid scrolls when the pointer nears its top or bottom.
  const startDrag = (e: React.MouseEvent, id: string) => {
    moved.current = false;
    if (e.button !== 0 || e.shiftKey || e.ctrlKey || e.metaKey) return;
    const [x0, y0] = [e.clientX, e.clientY];
    const moving = picked.has(id) ? chosen : [id];
    let at: number | null = null;
    let y = y0;
    let x = x0;
    let started = false;
    let frame = 0;
    const landing = () => {
      const card = (document.elementFromPoint(x, y) as HTMLElement | null)?.closest<HTMLElement>("[data-i]");
      if (!card) return;
      const r = card.getBoundingClientRect();
      at = Number(card.dataset.i) + (x > r.left + r.width / 2 ? 1 : 0);
      setDrop(at);
    };
    const scroll = () => {
      const g = grid.current;
      if (g) {
        const r = g.getBoundingClientRect();
        const by = y < r.top + 60 ? -(r.top + 60 - y) / 3 : y > r.bottom - 60 ? (y - (r.bottom - 60)) / 3 : 0;
        if (by) {
          g.scrollTop += Math.max(-24, Math.min(24, by));
          landing();
        }
      }
      frame = requestAnimationFrame(scroll);
    };
    const move = (m: MouseEvent) => {
      [x, y] = [m.clientX, m.clientY];
      if (!started && Math.hypot(x - x0, y - y0) > 5) {
        started = true;
        moved.current = true;
        setDragging(moving);
        setSkim(null);
        frame = requestAnimationFrame(scroll);
      }
      if (started) landing();
    };
    const up = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
      cancelAnimationFrame(frame);
      setDrop(null);
      setDragging(null);
      if (started && at !== null) moveTo(moving, ids.slice(0, at).filter((i) => !moving.includes(i)).length);
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  };

  const splittable = chosen.filter((id) => (p.items.find((i) => i.id === id)?.pieces ?? 1) > 1);
  const made = splittable.reduce((n, id) => n + (p.items.find((i) => i.id === id)?.pieces ?? 1), 0);
  const placeAt = Number(place);
  const placeOk = chosen.length > 0 && Number.isInteger(placeAt) && placeAt >= 1 && placeAt <= ids.length;

  return (
    <div className="modal arrange-modal" onMouseDown={(e) => e.target === e.currentTarget && p.onClose()}>
      <div className="card arrange">
        <div className="row" style={{ marginBottom: 6 }}>
          <strong className="grow">
            Arrange clips <span className="faint">· {ids.length} clips · {clock(p.total)}</span>
          </strong>
          <button className="btn small primary" onClick={p.onClose} title="Back to editing (Esc)">
            Done
          </button>
        </div>
        <p className="faint" style={{ margin: "0 0 10px" }}>
          Drag a clip to where it goes. Click to pick one (and watch it, as it's cut), Ctrl+click to pick more, Shift+click for a run.
          Point at a picture to skim it. <kbd>Space</kbd> plays on through the clips after it, <kbd>←</kbd> <kbd>→</kbd> go a clip back or
          on. Double-click a clip to edit it. Ctrl+Z undoes.
        </p>
        <div className="row arrange-actions">
          <span className="mono faint">{chosen.length ? `${chosen.length} picked` : "Nothing picked"}</span>
          <button className="btn small" disabled={!chosen.length} onClick={() => moveTo(chosen, 0)}>
            Move to start
          </button>
          <button className="btn small" disabled={!chosen.length} onClick={() => moveTo(chosen, ids.length)}>
            Move to end
          </button>
          <span className="row" style={{ gap: 4, flexWrap: "nowrap" }}>
            <input
              className="arrange-place"
              type="number"
              min={1}
              max={ids.length}
              placeholder="#"
              value={place}
              onChange={(e) => setPlace(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && placeOk && moveTo(chosen, placeAt - 1)}
              title="The place to move the picked clips to: 1 is first"
            />
            <button className="btn small" disabled={!placeOk} onClick={() => moveTo(chosen, placeAt - 1)}>
              Move to place
            </button>
          </span>
          <button
            className="btn small"
            disabled={!splittable.length}
            title="Make a clip of each part kept between the cuts of the picked clips"
            onClick={() => p.onSplit(splittable)}
          >
            {splittable.length ? `Split at cuts (${made} clips)` : "Split at cuts"}
          </button>
          <button className="btn small ghost danger" disabled={!chosen.length} onClick={() => p.onRemove(chosen)} title="Take out of the project (Delete)">
            Remove
          </button>
          <button className="btn small ghost" disabled={!chosen.length} onClick={() => setPicked(new Set())}>
            Clear
          </button>
        </div>
        <div className="arrange-body">
          <div className="arrange-grid" ref={grid}>
            {p.items.map((item, i) => {
              const s = item.strip;
              let thumb = null;
              if (s && s.count > 0) {
                let [w, h] = [(THUMB * s.thumb_width) / s.thumb_height, THUMB];
                if (w > CARD) [w, h] = [CARD, (CARD * s.thumb_height) / s.thumb_width];
                const at = skim?.id === item.id ? skim.at : (item.keep[0]?.start ?? 0);
                const tile = Math.min(s.count - 1, Math.floor((at / Math.max(0.01, item.sourceDuration)) * s.count));
                thumb = (
                  <div
                    className="arrange-tile"
                    style={{
                      width: w,
                      height: h,
                      backgroundImage: `url("${convertFileSrc(s.path)}")`,
                      backgroundSize: `${s.count * w}px ${h}px`,
                      backgroundPosition: `${-tile * w}px 0`,
                    }}
                  />
                );
              }
              const classes = [
                "arrange-card",
                picked.has(item.id) && "picked",
                item.id === p.active && "active",
                dragging?.includes(item.id) && "lifted",
                drop === i && "drop-before",
                drop === i + 1 && i === p.items.length - 1 && "drop-after",
              ];
              // Skimming runs through what's kept, left to right.
              const skimAt = (e: React.MouseEvent) => {
                if (dragging || item.length === null) return;
                const r = e.currentTarget.getBoundingClientRect();
                const f = Math.max(0, Math.min(1, (e.clientX - r.left) / r.width));
                setSkim({ id: item.id, at: fromOutput(item.keep, f * item.length) });
              };
              return (
                <div
                  key={item.id}
                  data-i={i}
                  className={classes.filter(Boolean).join(" ")}
                  title={item.name}
                  onMouseDown={(e) => startDrag(e, item.id)}
                  onClick={(e) => !moved.current && pick(e, item.id)}
                  onDoubleClick={() => p.onOpen(item.id)}
                >
                  <div className="arrange-thumb" style={{ height: THUMB }} onMouseMove={skimAt} onMouseLeave={() => setSkim(null)}>
                    {thumb}
                    <span className={`arrange-n${item.id === shown ? " on" : ""}`}>
                      {item.id === shown && playing ? "▶ " : ""}
                      {i + 1}
                    </span>
                    {item.pieces > 1 && <span className="arrange-parts" title={`${item.pieces} parts between its cuts`}>✂ {item.pieces}</span>}
                    {skim?.id === item.id && item.length !== null && (
                      <span className="arrange-skim" style={{ left: `${(toOutput(item.keep, skim.at) / Math.max(0.01, item.length)) * 100}%` }} />
                    )}
                  </div>
                  <div className="row" style={{ gap: 6, padding: "4px 6px" }}>
                    <span className="grow ellipsis">{item.name}</span>
                    <span className="faint mono">{item.length === null ? "…" : clock(item.length)}</span>
                  </div>
                </div>
              );
            })}
          </div>
          {showing && (
            <Player
              item={showing}
              n={showingAt + 1}
              of={ids.length}
              playing={playing}
              setPlaying={setPlaying}
              onEnded={() => step(1)}
              onStep={step}
            />
          )}
        </div>
      </div>
    </div>
  );
}

type PlayerProps = {
  item: ArrangeItem;
  n: number;
  of: number;
  playing: boolean;
  setPlaying: (on: boolean) => void;
  /** The clip's last kept moment has played. */
  onEnded: () => void;
  onStep: (by: number) => void;
};

/** One clip as it'll be cut: its kept parts back to back, with its own
 * sound as set (muted parts silent). Only for watching. */
function Player({ item, n, of, playing, setPlaying, onEnded, onStep }: PlayerProps) {
  const video = useRef<HTMLVideoElement>(null);
  const [time, setTime] = useState(0);
  // Until a newly shown clip has gone to its first kept moment, the
  // video's time is still the last clip's.
  const settled = useRef(false);
  const live = useRef({ item, onEnded, playing });
  live.current = { item, onEnded, playing };

  /** Back to the clip's first kept moment. */
  const toStart = (v: HTMLVideoElement) => {
    settled.current = false;
    setTime(0);
    v.currentTime = item.keep[0]?.start ?? 0;
    const done = () => (settled.current = true);
    v.addEventListener("seeked", done, { once: true });
    setTimeout(done, 400);
  };

  // A new clip starts at its first kept moment.
  useEffect(() => {
    const v = video.current;
    if (!v || !item.src) return;
    const go = () => toStart(v);
    if (v.readyState >= 1) go();
    else v.addEventListener("loadedmetadata", go, { once: true });
    return () => v.removeEventListener("loadedmetadata", go);
  }, [item.id, item.src]);

  // Playing again after the end starts over.
  useEffect(() => {
    const v = video.current;
    if (!v) return;
    v.volume = Math.max(0, Math.min(1, item.volume));
    if (playing && item.src) {
      const last = item.keep[item.keep.length - 1];
      if (v.readyState >= 1 && (v.ended || (last && v.currentTime >= last.end - 0.05))) toStart(v);
      v.play().catch(() => {});
    } else v.pause();
  }, [playing, item.id, item.src, item.volume]);

  // Playing skips what's cut and silences what's muted.
  useEffect(() => {
    let frame = 0;
    const tick = () => {
      const v = video.current;
      const { item, onEnded, playing } = live.current;
      if (v && settled.current) {
        const t = v.currentTime;
        const part = item.keep.find((r) => t < r.end - 0.03);
        if (!part || v.ended) {
          if (playing) {
            settled.current = false;
            v.pause();
            onEnded();
          }
        } else {
          if (t < part.start - 0.03) v.currentTime = part.start;
          v.muted = item.mutes.some((m) => t >= m.start && t < m.end);
          setTime(toOutput(item.keep, t));
        }
      }
      frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, []);

  const length = item.length ?? 0;
  const seek = (e: React.MouseEvent<HTMLDivElement>) => {
    const v = video.current;
    if (!v || !length) return;
    const r = e.currentTarget.getBoundingClientRect();
    const f = Math.max(0, Math.min(1, (e.clientX - r.left) / r.width));
    v.currentTime = fromOutput(item.keep, f * length);
    setTime(f * length);
  };

  return (
    <div className="arrange-watch">
      <div className="arrange-screen">
        {item.src ? (
          <video ref={video} src={item.src} playsInline onClick={() => setPlaying(!playing)} />
        ) : (
          <span className="faint">Getting this clip ready…</span>
        )}
      </div>
      <div className="arrange-bar" onClick={seek} title="Click to jump">
        <div style={{ width: `${length ? (time / length) * 100 : 0}%` }} />
      </div>
      <div className="row" style={{ gap: 6 }}>
        <button className="btn small" onClick={() => onStep(-1)} disabled={n <= 1} title="The clip before (←)">
          ◀◀
        </button>
        <button className="btn small primary" onClick={() => setPlaying(!playing)} disabled={!item.src} title="Play or pause (Space)">
          {playing ? "Pause" : "Play"}
        </button>
        <button className="btn small" onClick={() => onStep(1)} disabled={n >= of} title="The clip after (→)">
          ▶▶
        </button>
        <span className="grow" />
        <span className="mono faint">
          {clock(time, true)} / {clock(length, true)}
        </span>
      </div>
      <div className="ellipsis">
        <span className="faint mono">{n} · </span>
        {item.name}
      </div>
      <p className="faint" style={{ margin: 0 }}>
        As it'll be cut, with its own sound. Music, text and layers aren't shown here. Playing runs on into the clips after it.
      </p>
    </div>
  );
}
