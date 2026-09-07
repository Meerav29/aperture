import { NeedsYou } from "./features/sessions/SessionCard";
import { SessionGrid } from "./features/sessions/SessionGrid";
import { useSessions } from "./features/sessions/useSessions";
import { useState } from "react";
export default function App() {
  const { snap, notice, dismissNotice, rescan } = useSessions();
  const recent = snap.sessions.filter(s => s.observation === "recent").length;
  const [folder, setFolder] = useState("");
  const folders = [...new Set(snap.sessions.map(s => s.cwd))].sort();
  const visible = snap.sessions.filter(s => !folder || s.cwd === folder);
  return <main className="app">
    <header className="topbar"><h1>Aperture</h1><p className="summary">{recent} recently updating sessions · Read-only observer</p><button onClick={rescan}>Rescan</button></header>
    <section className="integrations" aria-label="Integration health">
      {snap.integrations.map(h => <article className="integration" key={h.provider}>
        <h2>{h.provider === "codex" ? "Codex" : "Claude Code"} <span>{h.state.replace(/_/g, " ")}</span></h2>
        <p>{h.files} session files · Last activity {h.last_event_at ? new Date(h.last_event_at).toLocaleString() : "not observed"}</p>
        <p>{h.detail}</p><code>{h.root}</code>
      </article>)}
    </section>
    {notice && <p role="status" className="notice">{notice}<button onClick={dismissNotice}>Dismiss</button></p>}
    <label className="folder-filter">Folder <select value={folder} onChange={e => setFolder(e.target.value)}><option value="">All folders</option>{folders.map(p => <option key={p} value={p}>{p || "Unknown directory"}</option>)}</select></label>
    <NeedsYou sessions={visible.filter(s => s.attention !== "unknown" && s.attention !== "none" && s.observation === "recent")} />
    <SessionGrid sessions={visible} />
  </main>;
}
