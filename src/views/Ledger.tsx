import { useEffect, useState } from "react";

import {
  deleteExpense,
  getSetupState,
  listCategories,
  listExpenses,
} from "@/lib/tauri";
import type { CategoryView, LedgerRow, SetupState } from "@/lib/tauri";
import { ViewHeader } from "./ViewHeader";
import { ErrorBanner } from "@/wizard/components/Layout";
import { GhostButton, SecondaryButton } from "@/wizard/components/Buttons";
import { formatDateTime, formatMoney } from "@/lib/format";

const PAGE = 100;

export function Ledger() {
  const [setup, setSetup] = useState<SetupState | null>(null);
  const [cats, setCats] = useState<CategoryView[]>([]);
  const [rows, setRows] = useState<LedgerRow[]>([]);
  const [filterCat, setFilterCat] = useState<number | "">("");
  const [search, setSearch] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [hasMore, setHasMore] = useState(false);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    void boot();
  }, []);

  useEffect(() => {
    void load(0);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [filterCat, search]);

  async function boot() {
    try {
      const [s, c] = await Promise.all([getSetupState(), listCategories(true)]);
      setSetup(s);
      setCats(c);
    } catch (e) {
      setError(String(e));
    }
  }

  async function load(offset: number) {
    setBusy(true);
    setError(null);
    try {
      const filters = {
        category_id: filterCat === "" ? undefined : (filterCat as number),
        search: search.trim() || undefined,
        limit: PAGE,
        offset,
      };
      const next = await listExpenses(filters);
      setHasMore(next.length === PAGE);
      setRows(offset === 0 ? next : [...rows, ...next]);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function remove(r: LedgerRow) {
    if (
      !window.confirm(
        `Delete ${formatMoney(r.amount_cents, r.currency)} for ${r.category_name ?? "(uncategorized)"} on ${formatDateTime(r.occurred_at)}?`,
      )
    )
      return;
    try {
      await deleteExpense(r.id);
      setRows((rs) => rs.filter((x) => x.id !== r.id));
    } catch (e) {
      setError(String(e));
    }
  }

  return (
    <div>
      <ViewHeader title="Ledger" subtitle="Every recorded expense." />
      <div className="mx-auto max-w-5xl space-y-4 px-8 py-8">
        {error ? <ErrorBanner>{error}</ErrorBanner> : null}

        <div className="flex items-center gap-3">
          <select
            value={filterCat}
            onChange={(e) => setFilterCat(e.target.value === "" ? "" : Number(e.target.value))}
            className="rounded-md border border-graphite-600 bg-graphite-800 px-3 py-2 text-sm text-graphite-50"
          >
            <option value="">All categories</option>
            <optgroup label="Fixed">
              {cats.filter((c) => c.kind === "fixed").map((c) => (
                <option key={c.id} value={c.id}>
                  {c.name}
                </option>
              ))}
            </optgroup>
            <optgroup label="Variable">
              {cats.filter((c) => c.kind === "variable").map((c) => (
                <option key={c.id} value={c.id}>
                  {c.name}
                </option>
              ))}
            </optgroup>
          </select>
          <input
            type="text"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder="Search description / message..."
            className="flex-1 rounded-md border border-graphite-600 bg-graphite-800 px-3 py-2 text-sm text-graphite-50"
          />
          <GhostButton
            onClick={() => {
              setFilterCat("");
              setSearch("");
            }}
          >
            Reset
          </GhostButton>
        </div>

        <div className="overflow-hidden rounded-lg border border-graphite-700">
          <table className="w-full text-sm">
            <thead className="bg-graphite-800 text-left text-xs uppercase tracking-wide text-graphite-400">
              <tr>
                <th className="px-3 py-2">When</th>
                <th className="px-3 py-2">Category</th>
                <th className="px-3 py-2">Description</th>
                <th className="px-3 py-2 text-right">Amount</th>
                <th className="px-3 py-2">Logged by</th>
                <th className="px-3 py-2"></th>
              </tr>
            </thead>
            <tbody className="divide-y divide-graphite-700">
              {rows.length === 0 && !busy ? (
                <tr>
                  <td colSpan={6} className="px-3 py-6 text-center text-graphite-500">
                    No expenses yet.
                  </td>
                </tr>
              ) : null}
              {rows.map((r) => (
                <tr key={r.id} className="hover:bg-graphite-800/50">
                  <td className="px-3 py-2 text-graphite-300">{formatDateTime(r.occurred_at)}</td>
                  <td className="px-3 py-2 text-graphite-200">
                    {r.category_name ?? "(uncategorized)"}
                    {r.category_kind ? (
                      <span className="ml-1 text-xs text-graphite-500">[{r.category_kind}]</span>
                    ) : null}
                  </td>
                  <td className="px-3 py-2 text-graphite-300">{r.description ?? ""}</td>
                  <td className="px-3 py-2 text-right font-mono text-graphite-50">
                    {formatMoney(r.amount_cents, r.currency, setup?.locale ?? null)}
                  </td>
                  <td className="px-3 py-2 text-graphite-400">{r.logged_by_name ?? "—"}</td>
                  <td className="px-3 py-2 text-right">
                    <button
                      type="button"
                      onClick={() => remove(r)}
                      className="rounded px-2 py-1 text-xs text-red-300 hover:bg-red-500/10"
                    >
                      Delete
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        {hasMore ? (
          <div className="text-center">
            <SecondaryButton onClick={() => load(rows.length)} disabled={busy}>
              {busy ? "…" : "Load more"}
            </SecondaryButton>
          </div>
        ) : null}
      </div>
    </div>
  );
}
