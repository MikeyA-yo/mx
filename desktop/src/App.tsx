import { useEffect, useRef, useState } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { startupFiles } from "./api";
import { DownloadIcon, EditIcon, SetupIcon, TasksIcon } from "./icons";
import { useTasks } from "./tasks";
import { rememberRecent } from "./util";
import DownloadPage from "./pages/Download";
import EditorPage from "./pages/Editor";
import TasksPage from "./pages/Tasks";
import SetupPage from "./pages/Setup";

type Page = "download" | "editor" | "tasks" | "setup";

export default function App() {
  const [page, setPage] = useState<Page>("editor");
  const [request, setRequest] = useState<{ paths: string[]; n: number } | null>(null);
  const running = useTasks().filter((t) => t.status === "running").length;

  const openInEditor = (paths: string[]) => {
    if (paths.length === 0) return;
    paths.forEach(rememberRecent);
    setRequest((r) => ({ paths, n: (r?.n ?? 0) + 1 }));
    setPage("editor");
  };
  const openOne = (path: string) => openInEditor([path]);

  // Once per launch (development mode mounts twice).
  const started = useRef(false);
  useEffect(() => {
    if (started.current) return;
    started.current = true;
    startupFiles().then(openInEditor);
  }, []);

  // Dropping files anywhere on the window adds them as clips.
  useEffect(() => {
    const off = getCurrentWebview().onDragDropEvent((e) => {
      if (e.payload.type === "drop") openInEditor(e.payload.paths);
    });
    return () => {
      off.then((f) => f());
    };
  }, []);

  const nav: [Page, string, React.ReactNode][] = [
    ["download", "Download", <DownloadIcon />],
    ["editor", "Editor", <EditIcon />],
    ["tasks", "Tasks", <TasksIcon />],
  ];

  return (
    <div className="app">
      <nav className="rail">
        <div className="logo">mx</div>
        {nav.map(([id, label, icon]) => (
          <button key={id} className={page === id ? "active" : ""} onClick={() => setPage(id)}>
            {icon}
            {label}
            {id === "tasks" && running > 0 && <span className="badge">{running}</span>}
          </button>
        ))}
        <div className="spacer" />
        <button className={page === "setup" ? "active" : ""} onClick={() => setPage("setup")}>
          <SetupIcon />
          Setup
        </button>
      </nav>
      <main>
        {/* The editor stays mounted so switching pages keeps its work. */}
        <div style={{ display: page === "editor" ? "contents" : "none" }}>
          <EditorPage request={request} onOpen={openInEditor} />
        </div>
        {page === "download" && <DownloadPage onEdit={openOne} />}
        {page === "tasks" && <TasksPage onEdit={openOne} />}
        {page === "setup" && <SetupPage />}
      </main>
    </div>
  );
}
