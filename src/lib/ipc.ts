// Typed wrappers around Tauri IPC. The frontend never calls invoke directly.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { Snapshot } from "../features/sessions/types";

export const ipc = {
  getSnapshot: () => invoke<Snapshot>("get_snapshot"),
  installHooks: () => invoke<void>("install_hooks"),
  uninstallHooks: () => invoke<void>("uninstall_hooks"),
  rescanTranscripts: () => invoke<number>("rescan_transcripts"),
  forgetSession: (id: string) => invoke<void>("forget_session", { id }),
  jumpToSession: (id: string) => invoke<string>("jump_to_session", { id }),
  onSnapshot: (cb: (s: Snapshot) => void): Promise<UnlistenFn> =>
    listen<Snapshot>("sessions:snapshot", (e) => cb(e.payload)),
};
