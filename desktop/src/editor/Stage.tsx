// The preview: the video in the frame it will be exported in, with the
// captions drawn the way the burn-in will draw them. Both are CSS
// approximations of the ffmpeg filters, close enough to judge framing
// and timing; the export is exact.

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { CaptionStyle, Frame, MediaSummary, Segment, Title } from "../api";
import { MusicIcon } from "../icons";

type Props = {
  src: string | null;
  media: MediaSummary;
  frame: Frame | null;
  cues: Segment[];
  style: CaptionStyle | null;
  showCaptions: boolean;
  time: number;
  videoRef: React.RefObject<HTMLVideoElement | null>;
  onLoaded: () => void;
  titles: Title[];
  selectedTitle: number | null;
  onSelectTitle: (i: number | null) => void;
  onMoveTitle: (i: number, x: number, y: number) => void;
};

/** The cue showing at `t`, by binary search (cues are in time order). */
function cueAt(cues: Segment[], t: number): number {
  let lo = 0;
  let hi = cues.length - 1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (cues[mid].end <= t) lo = mid + 1;
    else if (cues[mid].start > t) hi = mid - 1;
    else return mid;
  }
  return -1;
}

export default function Stage(p: Props) {
  const view = useRef<HTMLDivElement>(null);
  const blur = useRef<HTMLCanvasElement>(null);
  const [space, setSpace] = useState({ w: 640, h: 360 });

  useLayoutEffect(() => {
    const el = view.current!;
    const ro = new ResizeObserver(() => setSpace({ w: el.clientWidth - 32, h: el.clientHeight - 32 }));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const sourceAspect = p.media.width && p.media.height ? p.media.width / p.media.height : 16 / 9;
  const aspect = p.frame ? p.frame.aspect.w / p.frame.aspect.h : sourceAspect;
  const boxW = Math.max(40, Math.min(space.w, space.h * aspect));
  const boxH = boxW / aspect;

  // Blurred background: every new video frame, drawn small into a canvas
  // that CSS blurs and scales up, like the export's downscaled boxblur.
  const fit = p.frame?.fit.kind;
  useEffect(() => {
    const v = p.videoRef.current;
    const c = blur.current;
    if (!v || !c || fit !== "blur") return;
    const ctx = c.getContext("2d")!;
    let handle = 0;
    let alive = true;
    const draw = () => {
      if (!alive) return;
      if (v.videoWidth) {
        // Cover: scale the frame to fill the canvas, centred.
        const scale = Math.max(c.width / v.videoWidth, c.height / v.videoHeight);
        const w = v.videoWidth * scale;
        const h = v.videoHeight * scale;
        ctx.drawImage(v, (c.width - w) / 2, (c.height - h) / 2, w, h);
      }
      handle = v.requestVideoFrameCallback(draw);
    };
    draw();
    const onSeeked = () => draw();
    v.addEventListener("seeked", onSeeked);
    return () => {
      alive = false;
      v.cancelVideoFrameCallback(handle);
      v.removeEventListener("seeked", onSeeked);
    };
  }, [fit, p.src, p.videoRef]);

  let objectFit: "contain" | "cover" = "contain";
  let objectPosition = "50% 50%";
  if (p.frame?.fit.kind === "crop") {
    objectFit = "cover";
    const pos = p.frame.fit.position * 100;
    objectPosition = `${pos}% ${pos}%`;
  }

  const cue = p.showCaptions && p.style ? cueAt(p.cues, p.time) : -1;
  const fontSize = p.style ? p.style.size * Math.min(boxW, boxH) : 0;
  const shown = cue >= 0 ? p.cues[cue] : null;
  const upper = (s: string) => (p.style!.uppercase ? s.toUpperCase() : s);
  const text = shown ? upper(shown.text) : "";
  // Karaoke: the word being said, by the same rule as the burn-in (each
  // word lasts until the next one starts).
  const words = shown?.words ?? [];
  const spoken = p.style?.highlight ? words.findIndex((w, i) => p.time >= w.start && p.time < (words[i + 1]?.start ?? shown!.end)) : -1;

  return (
    <div className="stage-view" ref={view}>
      <div className="frame-box" style={{ width: boxW, height: boxH }}>
        {fit === "blur" && <canvas ref={blur} width={96} height={Math.round(96 / aspect)} />}
        <video
          ref={p.videoRef}
          src={p.src ?? undefined}
          style={{ objectFit, objectPosition, visibility: p.media.hasVideo ? "visible" : "hidden" }}
          preload="auto"
          onLoadedMetadata={p.onLoaded}
        />
        {!p.media.hasVideo && (
          <div className="audio-only" style={{ position: "absolute", inset: 0 }}>
            <MusicIcon className="" />
            <span>Audio only</span>
          </div>
        )}
        {p.titles.map((t, i) => {
          const showing = p.time >= t.start && p.time < t.end;
          const selected = p.selectedTitle === i;
          if (!showing && !selected) return null;
          const size = t.size * Math.min(boxW, boxH);
          return (
            <div
              key={i}
              className={`title-preview${selected ? " selected" : ""}`}
              style={{
                left: t.x * boxW,
                top: t.y * boxH,
                fontSize: size,
                fontFamily: `"${t.font}", sans-serif`,
                color: t.color,
                opacity: showing ? 1 : 0.45,
                background: t.boxed ? "rgba(0,0,0,0.69)" : undefined,
                padding: t.boxed ? `${size * 0.12}px ${size * 0.3}px` : undefined,
                WebkitTextStroke: t.boxed ? undefined : `${size * 0.14}px black`,
              }}
              onPointerDown={(e) => {
                // Drag to place: the title's centre follows the pointer.
                e.preventDefault();
                e.stopPropagation();
                p.onSelectTitle(i);
                const box = (e.currentTarget.parentElement as HTMLElement).getBoundingClientRect();
                const move = (ev: PointerEvent) =>
                  p.onMoveTitle(
                    i,
                    Math.min(1, Math.max(0, (ev.clientX - box.left) / box.width)),
                    Math.min(1, Math.max(0, (ev.clientY - box.top) / box.height)),
                  );
                const up = () => {
                  window.removeEventListener("pointermove", move);
                  window.removeEventListener("pointerup", up);
                };
                window.addEventListener("pointermove", move);
                window.addEventListener("pointerup", up);
              }}
            >
              {t.text}
            </div>
          );
        })}
        {text && (
          <div
            key={cue}
            className="caption-preview"
            style={{
              bottom: (1 - p.style!.position) * boxH,
              fontSize,
              fontFamily: `"${p.style!.font}", sans-serif`,
              WebkitTextStroke: `${fontSize * 0.16}px black`,
              animation: p.style!.pop ? "pop 90ms ease-out" : undefined,
            }}
          >
            {spoken >= 0
              ? words.map((w, i) => (
                  <span key={i} style={i === spoken ? { color: p.style!.highlight! } : undefined}>
                    {i > 0 ? " " : ""}
                    {upper(w.text)}
                  </span>
                ))
              : text}
          </div>
        )}
      </div>
    </div>
  );
}
