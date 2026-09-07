import type { Session } from "./types";
import { SessionCard } from "./SessionCard";
export function SessionGrid({ sessions }: { sessions: Session[] }) {
  if (!sessions.length) return <section className="empty"><h2>No sessions observed yet</h2><p>Use Claude Code or Codex externally. Aperture reads their session files automatically without changing settings.</p></section>;
  return <section className="repo"><h2 className="repo-name">Recent activity · {sessions.length} sessions</h2><div className="grid">{sessions.slice(0, 6).map(s => <SessionCard key={s.id} session={s} />)}</div>{sessions.length > 6 && <details className="history"><summary>Show {sessions.length - 6} older sessions</summary><div className="grid">{sessions.slice(6).map(s => <SessionCard key={s.id} session={s} />)}</div></details>}</section>;
}
