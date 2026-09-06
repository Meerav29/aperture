import { NeedsYou } from "./features/sessions/SessionCard";
import { SessionGrid } from "./features/sessions/SessionGrid";
import { useSessions } from "./features/sessions/useSessions";

export default function App() {
  const { snap, notice, dismissNotice, installHooks, uninstallHooks, rescan, forget, jump } =
    useSessions();

  const blocked = snap.sessions.filter((s) => s.status === "blocked");
  const live = snap.sessions.filter((s) => s.live && s.status !== "ended").length;

  return (
    <main className="app">
      <header className="topbar">
        <h1>Aperture</h1>
        <p className="summary">
          {live === 0 ? "No live sessions" : live === 1 ? "1 live session" : `${live} live sessions`}
          {snap.listener_port > 0 && <span className="port">listening on {snap.listener_port}</span>}
        </p>
        <nav className="actions">
          <button type="button" onClick={rescan}>Rescan transcripts</button>
          {snap.hooks_installed ? (
            <button type="button" className="quiet" onClick={uninstallHooks}>Remove hooks</button>
          ) : (
            <button type="button" className="primary" onClick={installHooks}>Install hooks</button>
          )}
        </nav>
      </header>

      {notice && (
        <p className="notice" role="status">
          {notice}
          <button type="button" className="quiet" onClick={dismissNotice} aria-label="Dismiss">
            ×
          </button>
        </p>
      )}

      <NeedsYou sessions={blocked} onJump={jump} />

      <SessionGrid
        sessions={snap.sessions}
        hooksInstalled={snap.hooks_installed}
        onJump={jump}
        onForget={forget}
        onInstall={installHooks}
        onRescan={rescan}
      />
    </main>
  );
}
