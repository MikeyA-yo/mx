// Every clip at a glance, to put them in order: drag one (or a few
// picked together) to where it goes, or send them to the start, the end
// or a numbered place. Clips split at their cuts here too, ready to be
// spread out.

import { useEffect, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import type { Filmstrip } from "../api";
import { clock } from "../util";

export type ArrangeItem = {
  id: string;
  name: string;
  /** Its length after cuts, once known. */
  length: number | null;
  strip: Filmstrip | null;
  /** The moment of its file its picture shows: the first kept. */
  at: number;
  sourceDuration: number;
  /** How many clips it makes split at its cuts. */
  pieces: number;
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
  const grid = useRef<HTMLDivElement>(null);
  const moved = useRef(false);
  const ids = p.items.map((i) => i.id);
  // Picked clips in their order; clips gone (removed, undone) drop out.
  const chosen = ids.filter((id) => picked.has(id));

  /** Puts `moving` at `k` among the other clips. */
  const moveTo = (moving: string[], k: number) => {
    const rest = ids.filter((id) => !moving.includes(id));
    const at = Math.max(0, Math.min(rest.length, k));
    p.onOrder([...rest.slice(0, at), ...moving, ...rest.slice(at)]);
  };

  const live = useRef({ chosen, onClose: p.onClose, onRemove: p.onRemove, ids });
  live.current = { chosen, onClose: p.onClose, onRemove: p.onRemove, ids };
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
      else if ((e.key === "Delete" || e.key === "Backspace") && chosen.length) {
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
          Drag a clip to where it goes. Click to pick one, Ctrl+click to pick more, Shift+click for a run, then drag them together or use
          the buttons. Double-click a clip to edit it. Ctrl+Z undoes.
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
        <div className="arrange-grid" ref={grid}>
          {p.items.map((item, i) => {
            const s = item.strip;
            let thumb = null;
            if (s && s.count > 0) {
              let [w, h] = [(THUMB * s.thumb_width) / s.thumb_height, THUMB];
              if (w > CARD) [w, h] = [CARD, (CARD * s.thumb_height) / s.thumb_width];
              const tile = Math.min(s.count - 1, Math.floor((item.at / Math.max(0.01, item.sourceDuration)) * s.count));
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
                <div className="arrange-thumb" style={{ height: THUMB }}>
                  {thumb}
                  <span className="arrange-n">{i + 1}</span>
                  {item.pieces > 1 && <span className="arrange-parts" title={`${item.pieces} parts between its cuts`}>✂ {item.pieces}</span>}
                </div>
                <div className="row" style={{ gap: 6, padding: "4px 6px" }}>
                  <span className="grow ellipsis">{item.name}</span>
                  <span className="faint mono">{item.length === null ? "…" : clock(item.length)}</span>
                </div>
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}
