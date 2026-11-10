// The start screen's projects: files saved or opened, and autosaved
// sessions, newest first; and a search for project files saved somewhere
// and forgotten.

import { useEffect, useState } from "react";
import { desktopDir, documentDir, downloadDir, videoDir } from "@tauri-apps/api/path";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { errorText, findProjects, recentProjects, type ProjectInfo } from "../api";
import { ago, dirName, forgetProject, load, recentProjectPaths, rememberProject, stem } from "../util";

const SHOWN = 6;

type Props = { onOpen: (path: string, restore?: { session: string; savedTo: string | null }) => void };

export default function RecentProjects({ onOpen }: Props) {
  const [list, setList] = useState<ProjectInfo[] | null>(null);
  const [all, setAll] = useState(false);
  const [finding, setFinding] = useState(false);
  const [note, setNote] = useState<string | null>(null);

  const refresh = () =>
    recentProjects(recentProjectPaths())
      .then(setList)
      .catch(() => setList([]));
  useEffect(() => void refresh(), []);

  // Where projects end up: the usual folders, and each recent video's own
  // (older versions saved projects next to the first clip).
  const find = async () => {
    setFinding(true);
    setNote(null);
    try {
      const homes = await Promise.all([videoDir(), documentDir(), desktopDir(), downloadDir()].map((d) => d.catch(() => "")));
      const dirs = [...homes, ...load<string[]>("recent", []).map(dirName)].filter(Boolean);
      const found = await findProjects([...new Set(dirs)]);
      // Oldest first, so the newest ends up at the top.
      [...found].reverse().forEach((f) => rememberProject(f.path));
      setNote(
        found.length
          ? `Found ${found.length} project${found.length === 1 ? "" : "s"}.`
          : "No project files in Videos, Documents, Desktop, Downloads or your videos' folders.",
      );
      setAll(true);
      await refresh();
    } catch (e) {
      setNote(errorText(e));
    } finally {
      setFinding(false);
    }
  };

  const shown = list ? (all ? list : list.slice(0, SHOWN)) : [];
  return (
    <div className="recent">
      <div className="row">
        <h3 className="section grow">Recent projects</h3>
        <a
          href="#"
          className="faint"
          onClick={(e) => {
            e.preventDefault();
            if (!finding) find();
          }}
        >
          {finding ? "Looking..." : "Find projects on this computer"}
        </a>
      </div>
      {note && (
        <p className="faint" style={{ margin: "0 0 8px" }}>
          {note}
        </p>
      )}
      {list?.length === 0 && !note && <p className="faint">Projects you save show up here, and any you didn't save are kept here too.</p>}
      {shown.map((p) => {
        const name = p.session ? (p.savedTo ? `${stem(p.savedTo)} (unsaved changes)` : "Unsaved project") : stem(p.path);
        const where = p.missing ? "moved or deleted" : p.session ? "autosaved" : dirName(p.path);
        const clips = p.clips.length
          ? `${p.clips.length} clip${p.clips.length === 1 ? "" : "s"}: ${p.clips.slice(0, 2).join(", ")}${p.clips.length > 2 ? "..." : ""}`
          : null;
        return (
          <div key={p.path} className={`project-row${p.missing ? " missing" : ""}`}>
            <button
              className="grow project-open"
              disabled={p.missing}
              title={p.session ? "Pick up where you left off" : p.path}
              onClick={() => (p.session ? onOpen(p.path, { session: p.session, savedTo: p.savedTo }) : onOpen(p.path))}
            >
              <span className="project-name ellipsis">{name}</span>
              <span className="faint ellipsis">{[where, p.modified ? ago(p.modified) : null, clips].filter(Boolean).join(" · ")}</span>
            </button>
            {!p.session && !p.missing && (
              <button className="btn small ghost" title="Show in folder" onClick={() => revealItemInDir(p.path)}>
                Show in folder
              </button>
            )}
            {!p.session && p.missing && (
              <button
                className="btn small ghost"
                title="Take it off this list"
                onClick={() => {
                  forgetProject(p.path);
                  refresh();
                }}
              >
                Forget
              </button>
            )}
          </div>
        );
      })}
      {list && list.length > SHOWN && (
        <a
          href="#"
          className="faint"
          onClick={(e) => {
            e.preventDefault();
            setAll(!all);
          }}
        >
          {all ? "Show fewer" : `Show all ${list.length}`}
        </a>
      )}
    </div>
  );
}
