import type { Session } from "./types";
import { shortPath, timeAgo } from "./useSessions";
import { ipc } from "../../lib/ipc";

// Navigation is a capability-based fallback chain, not exact-session focus:
// Aperture never verified a live PID or window handle, so the only honest
// actions are "open the folder" and "reveal the transcript file's folder" in
// the OS file manager. See docs/specification.md "Navigation order".
export function SessionCard({ session: s }: { session: Session }) {
  const provider = s.provider === "codex" ? "Codex" : "Claude Code";
  return <article className={`card status-${s.status}`} aria-label={`${provider}, ${s.status}`}>
    <header className="card-head"><strong className={`provider provider-${s.provider}`}>{provider}</strong><span className="card-status">{s.status}</span><span className="card-time">{timeAgo(s.last_event_at)}</span></header>
    <h3 className="card-name">{s.cwd.split(/[\\/]/).pop() || "Unknown directory"}</h3>
    <p className="card-line">{s.activity ?? "Activity unavailable"}</p>
    <p className="card-detail">Host: {s.host.replace(/_/g, " ")}</p>
    <p className="card-detail">Attention: {s.attention === "unknown" ? "unknown (no confirmed attention signal)" : s.attention.replace(/_/g, " ")}</p>
    <p className="card-detail">Observation: {s.observation.replace(/_/g, " ")} · process liveness unknown</p>
    <footer className="card-foot">
      <code className="card-path">{shortPath(s.cwd)}</code>
      <code title={s.native_id}>{s.native_id.slice(0, 8)}</code>
      <button type="button" disabled={!s.cwd} onClick={() => ipc.openSessionFolder(s.id)}>Open folder</button>
      <button type="button" disabled={!s.transcript_path} onClick={() => ipc.revealTranscript(s.id)}>Reveal transcript</button>
    </footer>
  </article>;
}
export function NeedsYou({ sessions }: { sessions: Session[] }) {
  if (!sessions.length) return null;
  return <section className="needs-you" role="status"><h2>{sessions.length === 1 ? "One session needs attention" : `${sessions.length} sessions need attention`}</h2><ul>{sessions.map(s => <li key={s.id}>{s.provider === "codex" ? "Codex" : "Claude Code"} · {s.native_id.slice(0,8)}: {(s.blocked_on ?? s.attention).replace(/_/g, " ")}</li>)}</ul></section>;
}
