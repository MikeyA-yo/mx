// First-run guides: a spotlight on one part of the window at a time, with
// a card saying what it's for. Each guide shows by itself once; Help
// replays it. Also the keyboard shortcuts sheet and the quick-start
// checklist's on/off switch, which Help reaches too.

import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import { load, store } from "./util";

export type TourName = "welcome" | "editor";

type Step = {
  /** A `data-tour` name; the first visible match is lit. None centres the card. */
  target?: string;
  title: string;
  body: React.ReactNode;
  /** Where the card goes, in order of preference. */
  place?: Side[];
  /** Click the target too: tabs open to show their tools. */
  press?: boolean;
};
type Side = "right" | "left" | "below" | "above";

const TOURS: Record<TourName, Step[]> = {
  welcome: [
    {
      title: "Welcome to mx",
      body: "A video editor for talking videos, shorts and edits. This tour takes a minute; press Help on the left to see it again any time.",
    },
    {
      target: "nav-download",
      title: "Download",
      body: "Paste a link from YouTube, TikTok, Instagram and others to save the video, then edit it here.",
      place: ["right"],
    },
    {
      target: "nav-editor",
      title: "Editor",
      body: "Where you cut, caption and export. Your work stays here while you visit the other pages.",
      place: ["right"],
    },
    {
      target: "nav-tasks",
      title: "Tasks",
      body: "Downloads and exports keep running in the background. Watch them, stop them, or open what they made.",
      place: ["right"],
    },
    {
      target: "open",
      title: "Start with your videos",
      body: "Drop files anywhere on this window, or choose them here. Pick several and they play one after another in a single video.",
      place: ["below", "right", "above"],
    },
    {
      target: "help",
      title: "Help is here",
      body: "Replay these tours, see the keyboard shortcuts, or bring back the quick-start checklist.",
      place: ["right"],
    },
  ],
  editor: [
    {
      title: "Your editor",
      body: "Here's what each part does. You can't break anything: your work saves itself as you go.",
    },
    {
      target: "stage",
      title: "Preview",
      body: "Shows the finished video: cuts skipped, captions, text and the new shape. Drag text on it to move it.",
      place: ["right", "below"],
    },
    {
      target: "play",
      title: "Play",
      body: (
        <>
          Press <kbd>Space</kbd> to play or pause. The arrow keys jump a second.
        </>
      ),
      place: ["above", "right"],
    },
    {
      target: "timeline",
      title: "Timeline",
      body: (
        <>
          Your whole video, every clip end to end. Click to jump to a moment; drag across it to select a part, then press <kbd>X</kbd> to cut
          it out. Drag a clip to reorder it, or its edges to trim it. Below, the text, videos and sounds you add: drag one to move it (even into
          another clip), or its edges to make it shorter or longer.
        </>
      ),
      place: ["above"],
    },
    {
      target: "sequence",
      title: "Your clips",
      body: "Every file you open is a clip, played in this order. Add more, reorder them, and choose how one changes to the next.",
      place: ["above", "below"],
    },
    {
      target: "view-switch",
      title: "Whole video or one clip",
      body: "Whole video shows everything as it will play. This clip shows just the clip on screen, with what's cut still there in red, for precise trimming.",
      place: ["above", "left"],
    },
    {
      target: "checklist",
      title: "Quick start",
      body: "Not sure what to do first? Follow these steps. Click one to open its tools.",
      place: ["left", "below"],
    },
    {
      target: "tab-cut",
      title: "Cut",
      body: "Remove the pauses between sentences in one click, or cut out parts you select on the timeline.",
      place: ["left", "below"],
      press: true,
    },
    {
      target: "tab-text",
      title: "Text",
      body: "Turn speech into captions. Delete a line to cut it from the video too.",
      place: ["left", "below"],
      press: true,
    },
    {
      target: "tab-titles",
      title: "Layers",
      body: "Put text and other videos over your clip: a hook at the start, b-roll, a picture-in-picture.",
      place: ["left", "below"],
      press: true,
    },
    {
      target: "tab-highlights",
      title: "Picks",
      body: "Finds the liveliest moments in a long video, so you can turn them into short clips.",
      place: ["left", "below"],
      press: true,
    },
    {
      target: "tab-frame",
      title: "Frame",
      body: "Change the shape for TikTok, Shorts or Instagram, and zoom in on moments.",
      place: ["left", "below"],
      press: true,
    },
    {
      target: "tab-audio",
      title: "Audio",
      body: "Add music and sound effects, or turn your clip's own sound down.",
      place: ["left", "below"],
      press: true,
    },
    {
      target: "tab-export",
      title: "Export",
      body: "When it looks right, save the finished video here. You can aim for a file size too.",
      place: ["left", "below"],
      press: true,
    },
    {
      target: "undo",
      title: "Undo anything",
      body: (
        <>
          Every edit can be undone with <kbd>Ctrl+Z</kbd>, and redone with <kbd>Ctrl+Shift+Z</kbd>. Hover the arrows to see what they'll change.
        </>
      ),
      place: ["below", "left"],
    },
    {
      target: "save",
      title: "Save your project",
      body: "Keeps everything editable in a project file, so you can come back and change it later. Ctrl+S works too.",
      place: ["below", "left"],
    },
    {
      target: "help",
      title: "That's it",
      body: "Press Help to replay this tour or see the shortcuts. Have fun editing!",
      place: ["right"],
    },
  ],
};

// A small shared store: which tour is running, and whether the shortcuts
// sheet and checklist are showing.
type State = { tour: TourName | null; shortcuts: boolean; checklist: boolean };
let state: State = { tour: null, shortcuts: false, checklist: load("guide.checklist", true) };
const listeners = new Set<() => void>();
const set = (change: Partial<State>) => {
  state = { ...state, ...change };
  listeners.forEach((l) => l());
};
const subscribe = (l: () => void) => {
  listeners.add(l);
  return () => void listeners.delete(l);
};
export const useGuide = () => useSyncExternalStore(subscribe, () => state);

export const startTour = (tour: TourName) => set({ tour, shortcuts: false });
/** Starts a tour the first time only, and never over another one. */
export function offerTour(tour: TourName) {
  if (state.tour || load<string[]>("guide.seen", []).includes(tour)) return;
  startTour(tour);
}
export const showShortcuts = (shortcuts: boolean) => set({ shortcuts });
export function showChecklist(checklist: boolean) {
  store("guide.checklist", checklist);
  set({ checklist });
}

/** `finished` is going through to the end; then the editor's tour
 * follows the welcome one if a video is already open. */
const endTour = (finished = false) => {
  const ended = state.tour;
  // The editor's tour opened each tab; leave it on the first one.
  if (ended === "editor") find("tab-cut")?.click();
  if (ended) store("guide.seen", [...new Set([...load<string[]>("guide.seen", []), ended])]);
  set({ tour: null });
  if (finished && ended === "welcome" && find("stage")) offerTour("editor");
};

/** The first element with this `data-tour` name that's on screen: clips
 * that aren't showing keep their (hidden) editors. */
function find(name: string): HTMLElement | null {
  const all = document.querySelectorAll<HTMLElement>(`[data-tour="${name}"]`);
  return Array.from(all).find((el) => el.getClientRects().length > 0) ?? null;
}

const PAD = 6;
const GAP = 14;
const MARGIN = 12;

/** The running tour, over everything. */
export function TourHost() {
  const { tour, shortcuts } = useGuide();
  return (
    <>
      {tour && <Tour key={tour} steps={TOURS[tour]} />}
      {shortcuts && <Shortcuts />}
    </>
  );
}

function Tour({ steps }: { steps: Step[] }) {
  const [index, setIndex] = useState(0);
  const step = steps[index];
  const last = index === steps.length - 1;
  const spot = useRef<HTMLDivElement>(null);
  const card = useRef<HTMLDivElement>(null);
  const next = () => (last ? endTour(true) : setIndex(index + 1));
  const back = () => setIndex(Math.max(0, index - 1));

  // The keys drive the tour, and nothing behind it reacts to them.
  const keys = useRef<(e: KeyboardEvent) => void>(() => {});
  keys.current = (e) => {
    e.stopPropagation();
    if (e.key === "Escape") endTour();
    else if (e.key === "ArrowRight" || e.key === "Enter") next();
    else if (e.key === "ArrowLeft") back();
    else return;
    e.preventDefault();
  };
  useEffect(() => {
    const handler = (e: KeyboardEvent) => keys.current(e);
    window.addEventListener("keydown", handler, true);
    return () => window.removeEventListener("keydown", handler, true);
  }, []);

  useEffect(() => {
    if (step.press && step.target) find(step.target)?.click();
  }, [step]);

  // Follow the target every frame: panels open, windows resize.
  useEffect(() => {
    let frame = 0;
    const place = () => {
      frame = requestAnimationFrame(place);
      const s = spot.current;
      const c = card.current;
      if (!s || !c) return;
      const vw = window.innerWidth;
      const vh = window.innerHeight;
      const cw = c.offsetWidth;
      const ch = c.offsetHeight;
      const el = step.target ? find(step.target) : null;
      if (!el) {
        Object.assign(s.style, { left: `${vw / 2}px`, top: `${vh / 2}px`, width: "0px", height: "0px" });
        Object.assign(c.style, { left: `${(vw - cw) / 2}px`, top: `${(vh - ch) / 2}px` });
        return;
      }
      const r = el.getBoundingClientRect();
      const box = {
        left: Math.max(2, r.left - PAD),
        top: Math.max(2, r.top - PAD),
        right: Math.min(vw - 2, r.right + PAD),
        bottom: Math.min(vh - 2, r.bottom + PAD),
      };
      Object.assign(s.style, {
        left: `${box.left}px`,
        top: `${box.top}px`,
        width: `${box.right - box.left}px`,
        height: `${box.bottom - box.top}px`,
      });
      const clampX = (x: number) => Math.min(vw - cw - MARGIN, Math.max(MARGIN, x));
      const clampY = (y: number) => Math.min(vh - ch - MARGIN, Math.max(MARGIN, y));
      const midX = (box.left + box.right) / 2 - cw / 2;
      const midY = (box.top + box.bottom) / 2 - ch / 2;
      const options: Record<Side, [number, number, boolean]> = {
        right: [box.right + GAP, clampY(midY), box.right + GAP + cw <= vw - MARGIN],
        left: [box.left - GAP - cw, clampY(midY), box.left - GAP - cw >= MARGIN],
        below: [clampX(midX), box.bottom + GAP, box.bottom + GAP + ch <= vh - MARGIN],
        above: [clampX(midX), box.top - GAP - ch, box.top - GAP - ch >= MARGIN],
      };
      const order = [...(step.place ?? []), "right", "left", "below", "above"] as Side[];
      const side = order.find((o) => options[o][2]);
      // Nowhere outside fits (a big target): sit inside it, at the bottom.
      const [x, y] = side ? options[side] : [clampX(midX), clampY(box.bottom - ch - GAP)];
      Object.assign(c.style, { left: `${x}px`, top: `${y}px` });
    };
    place();
    return () => cancelAnimationFrame(frame);
  }, [step]);

  return (
    <div className="tour" role="dialog" aria-label="Guided tour">
      <div className="tour-block" />
      <div className="tour-spot" ref={spot} />
      <div className="tour-card" ref={card}>
        <div className="tour-count">
          {index + 1} of {steps.length}
        </div>
        <h2>{step.title}</h2>
        <p>{step.body}</p>
        <div className="row">
          {!last && (
            <button className="btn small ghost" onClick={() => endTour()}>
              Skip tour
            </button>
          )}
          <span className="grow" />
          {index > 0 && (
            <button className="btn small" onClick={back}>
              Back
            </button>
          )}
          <button className="btn small primary" onClick={next} autoFocus>
            {last ? "Done" : index === 0 ? "Show me" : "Next"}
          </button>
        </div>
      </div>
    </div>
  );
}

const SHORTCUTS: [string[], string][] = [
  [["Space"], "Play or pause"],
  [["←", "→"], "Back or forward a second"],
  [["Shift", "←", "→"], "Back or forward five seconds"],
  [[",", "."], "Back or forward one frame"],
  [["I"], "Start a selection at the playhead"],
  [["O"], "End a selection at the playhead"],
  [["X"], "Cut the selection out (Delete works too)"],
  [["Q"], "Cut everything before the playhead"],
  [["W"], "Cut everything after the playhead"],
  [["Delete"], "Delete the layer picked on the timeline"],
  [["Esc"], "Clear the selection"],
  [["Ctrl", "Z"], "Undo any edit"],
  [["Ctrl", "Shift", "Z"], "Redo (Ctrl+Y works too)"],
  [["Ctrl", "Wheel"], "Zoom the timeline at the pointer"],
  [["Alt"], "Hold while dragging a layer to stop it snapping"],
  [["Ctrl", "S"], "Save the project"],
  [["Ctrl", "Shift", "S"], "Save it as a new file"],
  [["?"], "Show this list"],
];

function Shortcuts() {
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape") showShortcuts(false);
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, []);
  return (
    <div className="modal" onClick={() => showShortcuts(false)}>
      <div className="card" onClick={(e) => e.stopPropagation()} style={{ width: 420 }}>
        <div className="row" style={{ marginBottom: 12 }}>
          <strong className="grow">Keyboard shortcuts</strong>
          <button className="btn small ghost" onClick={() => showShortcuts(false)}>
            Close
          </button>
        </div>
        {SHORTCUTS.map(([keys, what]) => (
          <div className="stat-row" key={what}>
            <span className="muted">{what}</span>
            <span>
              {keys.map((k) => (
                <kbd key={k} style={{ marginLeft: 4 }}>
                  {k}
                </kbd>
              ))}
            </span>
          </div>
        ))}
        <p className="faint" style={{ margin: "10px 0 0" }}>
          Keys work in the editor when you're not typing in a box.
        </p>
      </div>
    </div>
  );
}

/** The Help button's menu. */
export function HelpMenu(p: { onClose: () => void; canTourEditor: boolean; onTour: () => void }) {
  const { onClose, canTourEditor } = p;
  const { checklist } = useGuide();
  useEffect(() => {
    const handler = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [onClose]);
  const item = (label: string, hint: string, act: () => void, disabled = false) => (
    <button
      disabled={disabled}
      onClick={() => {
        onClose();
        act();
      }}
    >
      <span>{label}</span>
      <span className="faint">{hint}</span>
    </button>
  );
  return (
    <>
      <div className="menu-backdrop" onClick={onClose} />
      <div className="help-menu" role="menu">
        {item("Tour of mx", "the pages, and where to start", () => {
          p.onTour();
          startTour("welcome");
        })}
        {item(
          "Tour of the editor",
          canTourEditor ? "what each part does" : "open a video first",
          () => {
            p.onTour();
            startTour("editor");
          },
          !canTourEditor,
        )}
        {item("Keyboard shortcuts", "press ? any time", () => showShortcuts(true))}
        {item(checklist ? "Hide quick start" : "Show quick start", "the step-by-step list in the editor", () => showChecklist(!checklist))}
      </div>
    </>
  );
}
