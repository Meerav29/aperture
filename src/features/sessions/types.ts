// Mirrors src-tauri/src/observer/model.rs. Keep in sync by hand for now.

export type SessionStatus =
  | "unknown"
  | "idle"
  | "working"
  | "blocked"
  | "errored"
  | "ended";

export type Host = "terminal" | "vs_code" | "desktop_app" | "headless" | "unknown";

export interface Session {
  id: string;
  native_id: string;
  provider: string;
  attention: string;
  observation: string;
  status: SessionStatus;
  host: Host;
  cwd: string;
  repo_root: string | null;
  git_branch: string | null;
  transcript_path: string | null;
  pid: number | null;
  title: string | null;
  activity: string | null;
  blocked_on: string | null;
  started_at: string | null;
  last_event_at: string;
  user_messages: number;
  assistant_messages: number;
  input_tokens: number;
  output_tokens: number;
  live: boolean;
}

export interface IntegrationHealth { provider: string; state: string; root: string; files: number; last_event_at: string | null; detail: string; }
export interface Snapshot {
  revision: number;
  integrations: IntegrationHealth[];
  sessions: Session[];
  hooks_installed: boolean;
  listener_port: number;
}
