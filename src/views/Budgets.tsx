import { useEffect, useState } from "react";

import {
  getSetupState,
  listBudgetsForCategory,
  listCategories,
  setCategoryBudget,
} from "@/lib/tauri";
import type { BudgetView, CategoryView, SetupState } from "@/lib/tauri";
import { ViewHeader } from "./ViewHeader";
import { ErrorBanner } from "@/wizard/components/Layout";
import { PrimaryButton } from "@/wizard/components/Buttons";
import { formatMoney } from "@/lib/format";

export function Budgets() {
  const [setup, setSetup] = useState<SetupState | null>(null);
  const [cats, setCats] = useState<CategoryView[]>([]);
  const [budgetsByCat, setBudgetsByCat] = useState<Record<number, BudgetView[]>>({});
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void load();
  }, []);

  async function load() {
    try {
      const [s, list] = await Promise.all([getSetupState(), listCategories(false)]);
      setSetup(s);
      setCats(list);
      // Fetch budgets per category in parallel.
      const entries = await Promise.all(
        list.map(async (c) => [c.id, await listBudgetsForCategory(c.id)] as const),
      );
      const map: Record<number, BudgetView[]> = {};
      for (const [id, bs] of entries) map[id] = bs;
      setBudgetsByCat(map);
    } catch (e) {
      setError(String(e));
    }
  }

  async function applyBudget(c: CategoryView, dollars: string) {
    if (!dollars.trim()) return;
    const cents = Math.round(Number(dollars) * 100);
    if (!Number.isFinite(cents) || cents < 0) {
      setError("Invalid amount.");
      return;
    }
    try {
      await setCategoryBudget(c.id, cents, "monthly");
      void load();
    } catch (e) {
      setError(String(e));
    }
  }

  return (
    <div>
      <ViewHeader
        title="Budgets"
        subtitle="Set monthly budgets per category. The bot uses these to pace your spending."
      />
      <div className="mx-auto max-w-4xl space-y-3 px-8 py-8">
        {error ? <ErrorBanner>{error}</ErrorBanner> : null}
        {cats.map((c) => {
          const current = (budgetsByCat[c.id] ?? []).find((b) => !b.effective_to);
          return (
            <BudgetRow
              key={c.id}
              category={c}
              current={current ?? null}
              currency={setup?.default_currency ?? "USD"}
              onApply={(v) => applyBudget(c, v)}
            />
          );
        })}
      </div>
    </div>
  );
}

function BudgetRow({
  category,
  current,
  currency,
  onApply,
}: {
  category: CategoryView;
  current: BudgetView | null;
  currency: string;
  onApply: (val: string) => void;
}) {
  const [draft, setDraft] = useState(
    current ? (current.amount_cents / 100).toFixed(2) : "",
  );
  return (
    <div className="flex items-center gap-4 rounded-md border border-graphite-700 bg-graphite-900 px-4 py-3">
      <div className="flex-1">
        <div className="text-sm text-graphite-50">{category.name}</div>
        <div className="text-xs text-graphite-400">
          {category.kind === "fixed" ? "Fixed" : "Variable"} ·{" "}
          {current ? `Current: ${formatMoney(current.amount_cents, currency)}/mo` : "No budget set"}
        </div>
      </div>
      <input
        type="number"
        step="0.01"
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        className="w-28 rounded-md border border-graphite-600 bg-graphite-800 px-2 py-1 text-right font-mono text-sm text-graphite-50"
      />
      <PrimaryButton onClick={() => onApply(draft)}>Apply</PrimaryButton>
    </div>
  );
}
