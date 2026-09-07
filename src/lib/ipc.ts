// Typed wrappers around Tauri IPC. The frontend never calls invoke directly.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { Snapshot } from "../features/sessions/types";

export const ipc = {
  getSnapshot: () => invoke<Snapshot>("get_snapshot"),
  rescanTranscripts: () => invoke<number>("rescan_transcripts"),
  openSessionFolder: (id: string) => invoke<void>("open_session_folder", { id }),
  revealTranscript: (id: string) => invoke<void>("reveal_transcript", { id }),
  onSnapshot: (cb: (s: Snapshot) => void): Promise<UnlistenFn> =>
    listen<Snapshot>("sessions:snapshot", (e) => cb(e.payload)),
};
