/**
 * Typed wrappers around Tauri IPC commands.
 *
 * Keep this file thin — it should declare the shape of each command and
 * forward to `invoke`. The real logic lives in `src-tauri/src/commands/`.
 */
import { invoke } from "@tauri-apps/api/core";

export async function ping(): Promise<string> {
  return invoke<string>("ping");
}
