import { useEffect, useState } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { startupFile } from "./api";
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
  const [file, setFile] = useState<string | null>(null);
  const running = useTasks().filter((t) => t.status === "running").length;

  const openInEditor = (path: string) => {
    rememberRecent(path);
    setFile(path);
    setPage("editor");
  };

  useEffect(() => {
    startupFile().then((f) => f && openInEditor(f));
  }, []);

  // Dropping a file anywhere on the window opens it.
  useEffect(() => {
    const off = getCurrentWebview().onDragDropEvent((e) => {
      if (e.payload.type === "drop" && e.payload.paths.length > 0) openInEditor(e.payload.paths[0]);
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
          <EditorPage file={file} onOpen={openInEditor} onClose={() => setFile(null)} />
        </div>
        {page === "download" && <DownloadPage onEdit={openInEditor} />}
        {page === "tasks" && <TasksPage onEdit={openInEditor} />}
        {page === "setup" && <SetupPage />}
      </main>
    </div>
  );
}
