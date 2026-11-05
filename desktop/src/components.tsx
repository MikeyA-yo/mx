// Small pieces shared by every page.

import { openPath, revealItemInDir } from "@tauri-apps/plugin-opener";
import { cancelTask, type TaskUpdate } from "./api";
import { useTask } from "./tasks";

export function ProgressBar({ percent }: { percent: number | null | undefined }) {
  const known = percent !== null && percent !== undefined;
  return (
    <div className={`bar${known ? "" : " indeterminate"}`}>
      <div style={{ width: `${known ? percent : 30}%` }} />
    </div>
  );
}

/** What a running task is doing, with a way to stop it. */
export function TaskProgress({ id, label }: { id: string | null; label?: string }) {
  const task = useTask(id);
  if (!id || !task || task.status !== "running") return null;
  return (
    <div className="task-inline">
      <div className="row">
        <div className="spinner" />
        <span className="grow ellipsis">{task.stage ?? label ?? "working"}</span>
        <span className="muted mono">{task.percent !== null ? `${task.percent.toFixed(0)}%` : ""}</span>
        <button className="btn small ghost danger" onClick={() => cancelTask(task.id)}>
          Stop
        </button>
      </div>
      <ProgressBar percent={task.percent} />
      {task.detail && <span className="faint">{task.detail}</span>}
    </div>
  );
}

export function FileActions({ path }: { path: string }) {
  return (
    <div className="row">
      <button className="btn small" onClick={() => openPath(path)}>
        Open
      </button>
      <button className="btn small" onClick={() => revealItemInDir(path)}>
        Show in folder
      </button>
    </div>
  );
}

export function statusText(t: TaskUpdate): string {
  switch (t.status) {
    case "running":
      return [t.stage, t.percent !== null ? `${t.percent.toFixed(0)}%` : null, t.detail].filter(Boolean).join(" · ");
    case "done":
      return "Done";
    case "cancelled":
      return "Stopped";
    case "failed":
      return t.error ?? "Failed";
  }
}
