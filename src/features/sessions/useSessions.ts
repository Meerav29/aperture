import { useCallback, useEffect, useState } from "react";
import { ipc } from "../../lib/ipc";
import type { Session, Snapshot } from "./types";

const EMPTY: Snapshot = { sessions: [], hooks_installed: false, listener_port: 0 };

export function useSessions() {
  const [snap, setSnap] = useState<Snapshot>(EMPTY);
  const [notice, setNotice] = useState<string | null>(null);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    ipc.getSnapshot().then(setSnap).catch((e) => setNotice(String(e)));
    ipc.onSnapshot(setSnap).then((u) => (unlisten = u));
    return () => unlisten?.();
  }, []);

  const run = useCallback(async (label: string, fn: () => Promise<unknown>) => {
    try {
      const r = await fn();
      setNotice(typeof r === "number" ? `${label}: ${r} transcripts` : typeof r === "string" ? r : null);
    } catch (e) {
      setNotice(`${label} failed: ${String(e)}`);
    }
  }, []);

  return {
    snap,
    notice,
    dismissNotice: () => setNotice(null),
    installHooks: () => run("Hooks installed", ipc.installHooks),
    uninstallHooks: () => run("Hooks removed", ipc.uninstallHooks),
    rescan: () => run("Rescanned", ipc.rescanTranscripts),
    forget: (id: string) => run("Removed", () => ipc.forgetSession(id)),
    jump: (id: string) => run("Jump", () => ipc.jumpToSession(id)),
  };
}

/** Group by repo root (or cwd when unknown), preserving the store's order. */
export function groupByRepo(sessions: Session[]): [string, Session[]][] {
  const map = new Map<string, Session[]>();
  for (const s of sessions) {
    const key = s.repo_root ?? s.cwd ?? "(no directory)";
    map.set(key, [...(map.get(key) ?? []), s]);
  }
  return [...map.entries()];
}

export function shortPath(p: string): string {
  const home = /^(\/Users\/[^/]+|\/home\/[^/]+|[A-Z]:\\Users\\[^\\]+)/;
  return p.replace(home, "~");
}

export function timeAgo(iso: string): string {
  const s = Math.max(0, (Date.now() - new Date(iso).getTime()) / 1000);
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  return `${Math.floor(s / 86400)}d ago`;
}

export function fmtTokens(n: number): string {
  if (n < 1000) return String(n);
  if (n < 1_000_000) return `${(n / 1000).toFixed(n < 10_000 ? 1 : 0)}k`;
  return `${(n / 1_000_000).toFixed(1)}M`;
}
