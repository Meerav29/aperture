import type { Session, SessionStatus, Host } from "./types";
import { fmtTokens, shortPath, timeAgo } from "./useSessions";

const STATUS_LABEL: Record<SessionStatus, string> = {
  unknown: "Not tracked",
  idle: "Waiting for you",
  working: "Working",
  blocked: "Needs permission",
  errored: "Errored",
  ended: "Ended",
};

const HOST_LABEL: Record<Host, string> = {
  terminal: "Terminal",
  vs_code: "VS Code",
  desktop_app: "Claude app",
  headless: "Headless",
  unknown: "",
};

interface Props {
  session: Session;
  onJump: (id: string) => void;
  onForget: (id: string) => void;
}

export function SessionCard({ session: s, onJump, onForget }: Props) {
  const name = s.git_branch ?? s.cwd.split(/[\\/]/).pop() ?? s.id.slice(0, 8);
  const line = s.status === "blocked" ? s.blocked_on : s.activity;

  return (
    <article className={`card status-${s.status}`} aria-label={`${name}, ${STATUS_LABEL[s.status]}`}>
      <header className="card-head">
        <span className="dot" aria-hidden="true" />
        <span className="card-status">{STATUS_LABEL[s.status]}</span>
        {s.host !== "unknown" && <span className="card-host">{HOST_LABEL[s.host]}</span>}
        <span className="card-time">{timeAgo(s.last_event_at)}</span>
      </header>

      <h3 className="card-name">{name}</h3>
      {s.title && <p className="card-title">{s.title}</p>}
      <p className="card-line">{line ?? (s.live ? "Nothing running" : "History only")}</p>

      <footer className="card-foot">
        <code className="card-path">{shortPath(s.cwd)}</code>
        <span className="card-meta">
          {s.user_messages} turns, {fmtTokens(s.input_tokens + s.output_tokens)} tokens
        </span>
        <span className="card-actions">
          {s.live && (
            <button type="button" onClick={() => onJump(s.id)}>
              Go to session
            </button>
          )}
          <button type="button" className="quiet" onClick={() => onForget(s.id)}>
            Remove
          </button>
        </span>
      </footer>
    </article>
  );
}

/** The one loud element. Rendered only when something is blocked. */
export function NeedsYou({ sessions, onJump }: { sessions: Session[]; onJump: (id: string) => void }) {
  if (sessions.length === 0) return null;
  return (
    <section className="needs-you" role="alert">
      <h2>
        {sessions.length === 1 ? "One session needs you" : `${sessions.length} sessions need you`}
      </h2>
      <ul>
        {sessions.map((s) => (
          <li key={s.id}>
            <span className="ny-name">{s.git_branch ?? shortPath(s.cwd)}</span>
            <span className="ny-what">{s.blocked_on ?? "Permission prompt"}</span>
            <button type="button" onClick={() => onJump(s.id)}>
              Go to session
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}
