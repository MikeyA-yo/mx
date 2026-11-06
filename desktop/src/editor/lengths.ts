// Lengths of the files used as layers, probed once each and shared: the
// timeline needs them to know how far a layer can be stretched.

import { useEffect, useState } from "react";
import { probeMedia } from "../api";

const known = new Map<string, number>();
const asked = new Set<string>();
const listeners = new Set<() => void>();

export function rememberLength(path: string, seconds: number) {
  known.set(path, seconds);
  listeners.forEach((l) => l());
}

/** Seconds per path, for those probed so far. */
export function useLengths(paths: string[]): Map<string, number> {
  const [, bump] = useState(0);
  useEffect(() => {
    const l = () => bump((n) => n + 1);
    listeners.add(l);
    return () => void listeners.delete(l);
  }, []);
  useEffect(() => {
    for (const path of paths) {
      if (asked.has(path)) continue;
      asked.add(path);
      probeMedia(path)
        .then((m) => rememberLength(path, m.duration))
        .catch(() => {});
    }
  }, [paths.join("\n")]);
  return known;
}
