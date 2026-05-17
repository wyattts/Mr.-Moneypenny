// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Wyatt Smith and contributors
//
// Bulk CSV import: parse a bank/credit-card export, dedupe, categorize
// (optionally with AI suggestions), and commit. Self-contained (its own
// status line) so it can sit at the top of the Ledger tab next to the
// exporter — data in beside data out.

import { useEffect, useState } from "react";

import {
  deleteCsvImportProfile,
  deleteMerchantRule,
  listCsvImportProfiles,
  listMerchantRules,
} from "@/lib/tauri";
import type { CsvImportProfile, MerchantRule } from "@/lib/tauri";
import { CsvImportWizard } from "@/views/CsvImport";
import { PrimaryButton } from "@/wizard/components/Buttons";

export function CsvImportPanel() {
  const [open, setOpen] = useState(false);
  const [profiles, setProfiles] = useState<CsvImportProfile[]>([]);
  const [rules, setRules] = useState<MerchantRule[]>([]);
  const [status, setStatus] = useState<{ ok: boolean; msg: string } | null>(
    null,
  );

  const reload = async () => {
    try {
      const [p, r] = await Promise.all([
        listCsvImportProfiles(),
        listMerchantRules(),
      ]);
      setProfiles(p);
      setRules(r);
    } catch (e) {
      setStatus({ ok: false, msg: String(e) });
    }
  };

  useEffect(() => {
    void reload();
  }, []);

  const removeProfile = async (id: number) => {
    try {
      await deleteCsvImportProfile(id);
      await reload();
    } catch (e) {
      setStatus({ ok: false, msg: String(e) });
    }
  };
  const removeRule = async (id: number) => {
    try {
      await deleteMerchantRule(id);
      await reload();
    } catch (e) {
      setStatus({ ok: false, msg: String(e) });
    }
  };

  return (
    <section className="rounded-lg border border-graphite-700 bg-graphite-900 p-4">
      <header className="mb-3">
        <h2 className="text-sm font-semibold text-graphite-50">
          Import a CSV
        </h2>
        <p className="mt-1 text-xs text-graphite-400">
          Bulk-import a bank or credit-card export. The first import of a new
          bank takes a few categorization clicks; later imports of the same
          export are instant and free.
        </p>
      </header>

      <div className="space-y-3">
        <PrimaryButton onClick={() => setOpen(true)}>
          Import a CSV…
        </PrimaryButton>

        <div className="grid grid-cols-1 gap-3 md:grid-cols-2">
          <div className="rounded-md border border-graphite-700 bg-graphite-800 p-3">
            <div className="mb-2 text-xs uppercase tracking-wide text-graphite-400">
              Saved bank profiles ({profiles.length})
            </div>
            {profiles.length === 0 ? (
              <p className="text-xs text-graphite-500">
                None yet. Each bank you import from gets one saved here so you
                don&apos;t have to remap columns next time.
              </p>
            ) : (
              <ul className="divide-y divide-graphite-700">
                {profiles.map((p) => (
                  <li
                    key={p.id}
                    className="flex items-center justify-between gap-3 py-1.5"
                  >
                    <span className="truncate text-sm text-graphite-100">
                      {p.name}
                    </span>
                    <button
                      onClick={() => void removeProfile(p.id)}
                      className="rounded px-2 py-0.5 text-xs text-red-300 hover:bg-red-500/10"
                    >
                      delete
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </div>

          <div className="rounded-md border border-graphite-700 bg-graphite-800 p-3">
            <div className="mb-2 text-xs uppercase tracking-wide text-graphite-400">
              Merchant rules ({rules.length})
            </div>
            {rules.length === 0 ? (
              <p className="text-xs text-graphite-500">
                Patterns that auto-categorize bank-statement merchants. Built up
                one click at a time on the import review screen.
              </p>
            ) : (
              <ul className="max-h-48 divide-y divide-graphite-700 overflow-y-auto">
                {rules.map((r) => (
                  <li
                    key={r.id}
                    className="flex items-center justify-between gap-3 py-1.5 text-sm"
                  >
                    <span className="truncate font-mono text-graphite-200">
                      {r.pattern}
                    </span>
                    <button
                      onClick={() => void removeRule(r.id)}
                      className="rounded px-2 py-0.5 text-xs text-red-300 hover:bg-red-500/10"
                    >
                      delete
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </div>
        </div>

        {status ? (
          <p
            className={`text-xs ${
              status.ok ? "text-forest-300" : "text-red-300"
            }`}
          >
            {status.msg}
          </p>
        ) : null}
      </div>

      {open && (
        <CsvImportWizard
          onClose={() => {
            setOpen(false);
            void reload();
          }}
          onImported={(n) => {
            setStatus({
              ok: true,
              msg: `Imported ${n} expense${n === 1 ? "" : "s"} from CSV.`,
            });
          }}
        />
      )}
    </section>
  );
}
