// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Wyatt Smith and contributors
//
// The "open hub" control: a complete, faithful one-way photocopy of the
// ledger to a file the user chooses. Snapshot, desktop-only, no third
// party. Self-contained (its own status line) so it can be dropped into
// any host view; it lives in the Ledger tab.

import { useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";

import { exportData } from "@/lib/tauri";
import type { ExportFormat, ExportTimeframe } from "@/lib/tauri";
import { PrimaryButton } from "@/wizard/components/Buttons";

const EXPORT_FORMATS: {
  value: ExportFormat;
  label: string;
  ext: string;
  hint: string;
}[] = [
  {
    value: "csv",
    label: "Spreadsheet (CSV)",
    ext: "csv",
    hint: "Opens in Excel, Numbers, or LibreOffice.",
  },
  {
    value: "jsonl",
    label: "Programmer file (JSON Lines)",
    ext: "jsonl",
    hint: "One JSON record per line, for scripts and tools.",
  },
  {
    value: "beancount",
    label: "Plain-text accounting (Beancount)",
    ext: "beancount",
    hint: "Plugs into Fava and the plain-text-accounting ecosystem. A shorter timeframe is still valid but less self-contained than All time.",
  },
];

const EXPORT_TIMEFRAMES: { value: ExportTimeframe; label: string }[] = [
  { value: "all", label: "All time (complete copy)" },
  { value: "year", label: "This year" },
  { value: "quarter", label: "This quarter" },
  { value: "month", label: "This month" },
];

export function DataExportPanel() {
  const [format, setFormat] = useState<ExportFormat>("csv");
  const [timeframe, setTimeframe] = useState<ExportTimeframe>("all");
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState<{ ok: boolean; msg: string } | null>(
    null,
  );

  const selected = EXPORT_FORMATS.find((f) => f.value === format)!;

  async function onExport() {
    setStatus(null);
    setBusy(true);
    try {
      // Stamp the export date into the filename so snapshots taken on
      // different days don't collide and each file says when it was
      // taken. Local date, zero-padded YYYY-MM-DD.
      const d = new Date();
      const today = `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(
        2,
        "0",
      )}-${String(d.getDate()).padStart(2, "0")}`;
      const path = await save({
        defaultPath: `moneypenny-export-${timeframe}-${today}.${selected.ext}`,
        filters: [{ name: selected.label, extensions: [selected.ext] }],
      });
      if (!path) return; // user cancelled the save dialog
      const summary = await exportData(path, format, timeframe);
      setStatus({
        ok: true,
        msg: `Exported ${summary.row_count} transaction${
          summary.row_count === 1 ? "" : "s"
        } to ${summary.path}.`,
      });
    } catch (e) {
      setStatus({ ok: false, msg: String(e) });
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="rounded-lg border border-graphite-700 bg-graphite-900 p-4">
      <header className="mb-3">
        <h2 className="text-sm font-semibold text-graphite-50">
          Export your data
        </h2>
        <p className="mt-1 text-xs text-graphite-400">
          A complete, faithful copy you can take anywhere — a point-in-time
          snapshot written to your own disk, no third party involved. All time
          is the full portability copy; the shorter ranges are for taxes or
          handing a year to an accountant.
        </p>
      </header>
      <div className="flex flex-wrap items-end gap-2">
        <label className="flex flex-col gap-1">
          <span className="text-xs text-graphite-300">Format</span>
          <select
            value={format}
            onChange={(e) => setFormat(e.target.value as ExportFormat)}
            className="rounded-md border border-graphite-600 bg-graphite-800 px-2 py-1 text-sm text-graphite-50 focus:border-forest-400 focus:outline-none"
          >
            {EXPORT_FORMATS.map((f) => (
              <option key={f.value} value={f.value}>
                {f.label}
              </option>
            ))}
          </select>
        </label>
        <label className="flex flex-col gap-1">
          <span className="text-xs text-graphite-300">Timeframe</span>
          <select
            value={timeframe}
            onChange={(e) => setTimeframe(e.target.value as ExportTimeframe)}
            className="rounded-md border border-graphite-600 bg-graphite-800 px-2 py-1 text-sm text-graphite-50 focus:border-forest-400 focus:outline-none"
          >
            {EXPORT_TIMEFRAMES.map((t) => (
              <option key={t.value} value={t.value}>
                {t.label}
              </option>
            ))}
          </select>
        </label>
        <PrimaryButton onClick={onExport} disabled={busy}>
          {busy ? "Exporting…" : "Export…"}
        </PrimaryButton>
      </div>
      <p className="mt-2 text-xs text-graphite-400">{selected.hint}</p>
      {status ? (
        <p
          className={`mt-2 text-xs ${
            status.ok ? "text-forest-300" : "text-red-300"
          }`}
        >
          {status.msg}
        </p>
      ) : null}
    </section>
  );
}
