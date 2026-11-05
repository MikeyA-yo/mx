// The preview: the video in the frame it will be exported in, with the
// captions drawn the way the burn-in will draw them. Both are CSS
// approximations of the ffmpeg filters, close enough to judge framing
// and timing; the export is exact.

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { CaptionStyle, Frame, MediaSummary, Segment } from "../api";
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
  const text = cue >= 0 ? (p.style!.uppercase ? p.cues[cue].text.toUpperCase() : p.cues[cue].text) : "";

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
            {text}
          </div>
        )}
      </div>
    </div>
  );
}
