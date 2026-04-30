import { useEffect, useState } from "react";

import {
  checkForUpdate,
  getCheckUpdatesOnLaunch,
  installUpdate,
} from "@/lib/tauri";

/**
 * On-launch update check. If `Check for updates on launch` is enabled
 * (default ON; toggleable in Settings) and `check_for_update` reports
 * a newer version, render a sticky banner the user can act on. Skip
 * dismisses for the rest of the session — keyed by the available
 * version so a fresh release re-prompts.
 */
export function UpdateBanner() {
  const [info, setInfo] = useState<{ version: string; notes: string | null } | null>(null);
  const [skipped, setSkipped] = useState<string | null>(null);
  const [installing, setInstalling] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void runCheck();
  }, []);

  async function runCheck() {
    try {
      const enabled = await getCheckUpdatesOnLaunch();
      if (!enabled) return;
      const r = await checkForUpdate();
      if (r.available && r.version) {
        setInfo({ version: r.version, notes: r.notes });
      }
    } catch (e) {
      // Silent; no network on launch is fine, surfacing nothing is correct.
      // The user can hit Settings → Check now if they want a real error.
      console.warn("Update check failed:", e);
    }
  }

  if (!info) return null;
  if (skipped === info.version) return null;

  async function install() {
    if (!info) return;
    setInstalling(true);
    setError(null);
    try {
      // installUpdate triggers a relaunch on success; this promise will
      // not resolve in the happy path. If we do come back here, treat as
      // an error.
      await installUpdate();
    } catch (e) {
      setError(String(e));
      setInstalling(false);
    }
  }

  return (
    <div className="flex items-center justify-between gap-3 border-b border-forest-500/40 bg-forest-700/20 px-4 py-2 text-sm">
      <div className="text-graphite-50">
        <strong className="text-forest-200">v{info.version}</strong> is available.
        {error ? (
          <span className="ml-2 text-red-300">Install failed: {error}</span>
        ) : null}
      </div>
      <div className="flex items-center gap-2">
        <button
          type="button"
          onClick={install}
          disabled={installing}
          className="rounded-md bg-forest-500 px-3 py-1 text-xs font-medium text-white hover:bg-forest-400 disabled:opacity-50"
        >
          {installing ? "Installing…" : "Install"}
        </button>
        <button
          type="button"
          onClick={() => setSkipped(info.version)}
          className="rounded-md border border-graphite-600 bg-graphite-800 px-3 py-1 text-xs text-graphite-200 hover:bg-graphite-700"
        >
          Skip
        </button>
      </div>
    </div>
  );
}
