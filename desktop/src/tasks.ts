// Every task the Rust side reports, kept in one store the whole UI reads.

import { useSyncExternalStore } from "react";
import { listen } from "@tauri-apps/api/event";
import type { TaskUpdate } from "./api";

let tasks: TaskUpdate[] = [];
const listeners = new Set<() => void>();

function emit() {
  for (const l of listeners) l();
}

listen<TaskUpdate>("task", (event) => {
  const update = event.payload;
  const i = tasks.findIndex((t) => t.id === update.id);
  tasks = i === -1 ? [update, ...tasks] : tasks.map((t, j) => (j === i ? update : t));
  emit();
});

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** All tasks, newest first. */
export function useTasks(): TaskUpdate[] {
  return useSyncExternalStore(subscribe, () => tasks);
}

/** One task by id, or undefined before its first update arrives. */
export function useTask(id: string | null): TaskUpdate | undefined {
  const all = useTasks();
  return id ? all.find((t) => t.id === id) : undefined;
}

export function clearFinished() {
  tasks = tasks.filter((t) => t.status === "running");
  emit();
}
