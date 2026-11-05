import { cancelTask } from "../api";
import { FileActions, ProgressBar, statusText } from "../components";
import { clearFinished, useTasks } from "../tasks";

const KIND: Record<string, string> = {
  download: "Download",
  export: "Export",
  analyse: "Analysis",
  transcribe: "Transcript",
  preview: "Preview",
};

export default function TasksPage({ onEdit }: { onEdit: (path: string) => void }) {
  const tasks = useTasks();
  return (
    <div className="page">
      <div className="row" style={{ maxWidth: 820 }}>
        <div className="grow">
          <h1>Tasks</h1>
          <p className="lede">Downloads, exports and analysis since mx started.</p>
        </div>
        <button className="btn" onClick={clearFinished} disabled={!tasks.some((t) => t.status !== "running")}>
          Clear finished
        </button>
      </div>
      {tasks.length === 0 && <p className="faint">Nothing yet.</p>}
      <div className="list">
        {tasks.map((t) => (
          <div className="list-item" key={t.id}>
            <div style={{ minWidth: 0 }}>
              <div className="ellipsis">
                <span className="faint">{KIND[t.kind] ?? t.kind} · </span>
                {t.title}
              </div>
              <div className={t.status === "failed" ? "error" : "faint"}>{statusText(t)}</div>
            </div>
            {t.status === "running" ? (
              <button className="btn small" onClick={() => cancelTask(t.id)}>
                Stop
              </button>
            ) : t.status === "done" && t.output ? (
              <div className="row">
                {(t.kind === "download" || t.kind === "export") && (
                  <button className="btn small primary" onClick={() => onEdit(t.output!)}>
                    Edit
                  </button>
                )}
                <FileActions path={t.output} />
              </div>
            ) : (
              <span />
            )}
            {t.status === "running" && <ProgressBar percent={t.percent} />}
          </div>
        ))}
      </div>
    </div>
  );
}
