import type { Session } from "./types";
import { groupByRepo, shortPath } from "./useSessions";
import { SessionCard } from "./SessionCard";

interface Props {
  sessions: Session[];
  hooksInstalled: boolean;
  onJump: (id: string) => void;
  onForget: (id: string) => void;
  onInstall: () => void;
  onRescan: () => void;
}

export function SessionGrid({ sessions, hooksInstalled, onJump, onForget, onInstall, onRescan }: Props) {
  if (sessions.length === 0) {
    return (
      <section className="empty">
        <h2>No sessions yet</h2>
        {hooksInstalled ? (
          <p>
            Start Claude Code anywhere on this machine and it will appear here. To see past
            sessions, <button type="button" className="link" onClick={onRescan}>scan transcripts</button>.
          </p>
        ) : (
          <p>
            <button type="button" className="link" onClick={onInstall}>Install hooks</button> so
            new sessions report in, or <button type="button" className="link" onClick={onRescan}>scan transcripts</button> to
            load history.
          </p>
        )}
      </section>
    );
  }

  return (
    <>
      {groupByRepo(sessions).map(([repo, list]) => (
        <section className="repo" key={repo}>
          <h2 className="repo-name">{shortPath(repo)}</h2>
          <div className="grid">
            {list.map((s) => (
              <SessionCard key={s.id} session={s} onJump={onJump} onForget={onForget} />
            ))}
          </div>
        </section>
      ))}
    </>
  );
}
