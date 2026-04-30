import { useEffect, useMemo, useState } from "react";

import {
  createCategory,
  deleteCategory,
  getSetupState,
  listCategories,
  setCategoryActive,
  setCategoryTarget,
} from "@/lib/tauri";
import type { CategoryView } from "@/lib/tauri";
import { ViewHeader } from "./ViewHeader";
import { ErrorBanner } from "@/wizard/components/Layout";
import { GhostButton, PrimaryButton, SecondaryButton } from "@/wizard/components/Buttons";
import { formatMoney } from "@/lib/format";

/**
 * Sum monthly_target_cents across the given categories. Inactive
 * categories are excluded — they're not contributing to your live
 * monthly plan even if they have a saved target. Categories without
 * a target contribute zero.
 */
function sumActiveTargets(cats: CategoryView[]): number {
  return cats
    .filter((c) => c.is_active && c.monthly_target_cents != null)
    .reduce((acc, c) => acc + (c.monthly_target_cents ?? 0), 0);
}

export function Categories() {
  const [cats, setCats] = useState<CategoryView[]>([]);
  const [showInactive, setShowInactive] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [adding, setAdding] = useState<"fixed" | "variable" | "investing" | null>(null);
  const [currency, setCurrency] = useState("USD");
  const [locale, setLocale] = useState<string | null>(null);

  useEffect(() => {
    void load();
  }, [showInactive]);

  useEffect(() => {
    void loadSetup();
  }, []);

  async function loadSetup() {
    try {
      const s = await getSetupState();
      setCurrency(s.default_currency);
      setLocale(s.locale);
    } catch (e) {
      // Falling back to defaults is fine — currency just reads as USD.
      console.warn("loadSetup:", e);
    }
  }

  async function load() {
    // Always fetch the full list (including inactive) so the totals
    // reflect a stable "active monthly plan" number regardless of the
    // showInactive UI toggle. The toggle only governs what's *rendered*.
    try {
      const list = await listCategories(true);
      setCats(list);
    } catch (e) {
      setError(String(e));
    }
  }

  async function updateTarget(c: CategoryView, value: string) {
    const cents = value.trim() ? Math.round(Number(value) * 100) : null;
    const newTarget = cents !== null && Number.isFinite(cents) ? cents : null;
    try {
      await setCategoryTarget(c.id, newTarget);
      setCats((cs) =>
        cs.map((x) => (x.id === c.id ? { ...x, monthly_target_cents: newTarget } : x)),
      );
    } catch (e) {
      setError(String(e));
    }
  }

  async function toggleActive(c: CategoryView, active: boolean) {
    try {
      await setCategoryActive(c.id, active);
      setCats((cs) => cs.map((x) => (x.id === c.id ? { ...x, is_active: active } : x)));
    } catch (e) {
      setError(String(e));
    }
  }

  async function remove(c: CategoryView) {
    if (c.is_seed) {
      setError(`"${c.name}" is a seed category — deactivate it instead.`);
      return;
    }
    if (!window.confirm(`Delete category "${c.name}"? Existing expenses will lose their category.`)) {
      return;
    }
    try {
      await deleteCategory(c.id);
      void load();
    } catch (e) {
      setError(String(e));
    }
  }

  async function add(name: string, kind: "fixed" | "variable" | "investing", targetDollars: string) {
    const cents = targetDollars.trim()
      ? Math.round(Number(targetDollars) * 100)
      : undefined;
    try {
      await createCategory({
        name: name.trim(),
        kind,
        monthly_target_cents: cents,
      });
      setAdding(null);
      void load();
    } catch (e) {
      setError(String(e));
    }
  }

  const fixed = cats.filter((c) => c.kind === "fixed");
  const variable = cats.filter((c) => c.kind === "variable");
  const investing = cats.filter((c) => c.kind === "investing");

  // The four sum-total figures the user sees at the top. All count
  // only is_active = true rows with a saved monthly_target_cents.
  const totals = useMemo(() => {
    const fixedTotal = sumActiveTargets(fixed);
    const variableTotal = sumActiveTargets(variable);
    const investingTotal = sumActiveTargets(investing);
    return {
      fixed: fixedTotal,
      variable: variableTotal,
      investing: investingTotal,
      grand: fixedTotal + variableTotal + investingTotal,
    };
  }, [fixed, variable, investing]);

  // What gets rendered inside each section — the showInactive toggle
  // only affects display, not the totals.
  const renderFixed = showInactive ? fixed : fixed.filter((c) => c.is_active);
  const renderVariable = showInactive ? variable : variable.filter((c) => c.is_active);
  const renderInvesting = showInactive ? investing : investing.filter((c) => c.is_active);

  return (
    <div>
      <ViewHeader
        title="Categories"
        subtitle="Set monthly budgets, toggle activation, and add or remove categories. Seed categories can be deactivated but not deleted."
        actions={
          <label className="flex items-center gap-2 text-xs text-graphite-300">
            <input
              type="checkbox"
              checked={showInactive}
              onChange={(e) => setShowInactive(e.target.checked)}
              className="h-4 w-4 rounded border-graphite-500 bg-graphite-800 text-forest-500"
            />
            Show inactive
          </label>
        }
      />
      <div className="mx-auto max-w-4xl space-y-6 px-8 py-8">
        {error ? <ErrorBanner>{error}</ErrorBanner> : null}

        <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
          <SumTotalCard
            label="Sum total"
            value={formatMoney(totals.grand, currency, locale)}
            emphasize
          />
          <SumTotalCard label="Fixed" value={formatMoney(totals.fixed, currency, locale)} />
          <SumTotalCard label="Variable" value={formatMoney(totals.variable, currency, locale)} />
          <SumTotalCard
            label="Saving / Investing"
            value={formatMoney(totals.investing, currency, locale)}
          />
        </div>

        <CategoryGroup
          label="Fixed (recurring monthly)"
          cats={renderFixed}
          adding={adding === "fixed"}
          onAdd={() => setAdding("fixed")}
          onCancelAdd={() => setAdding(null)}
          onSubmit={(n, t) => add(n, "fixed", t)}
          onTargetChange={updateTarget}
          onToggleActive={toggleActive}
          onRemove={remove}
        />
        <CategoryGroup
          label="Variable (discretionary)"
          cats={renderVariable}
          adding={adding === "variable"}
          onAdd={() => setAdding("variable")}
          onCancelAdd={() => setAdding(null)}
          onSubmit={(n, t) => add(n, "variable", t)}
          onTargetChange={updateTarget}
          onToggleActive={toggleActive}
          onRemove={remove}
        />
        <CategoryGroup
          label="Saving / Investing"
          cats={renderInvesting}
          adding={adding === "investing"}
          onAdd={() => setAdding("investing")}
          onCancelAdd={() => setAdding(null)}
          onSubmit={(n, t) => add(n, "investing", t)}
          onTargetChange={updateTarget}
          onToggleActive={toggleActive}
          onRemove={remove}
        />
      </div>
    </div>
  );
}

function CategoryGroup({
  label,
  cats,
  adding,
  onAdd,
  onCancelAdd,
  onSubmit,
  onTargetChange,
  onToggleActive,
  onRemove,
}: {
  label: string;
  cats: CategoryView[];
  adding: boolean;
  onAdd: () => void;
  onCancelAdd: () => void;
  onSubmit: (name: string, target: string) => void;
  onTargetChange: (c: CategoryView, target: string) => void;
  onToggleActive: (c: CategoryView, active: boolean) => void;
  onRemove: (c: CategoryView) => void;
}) {
  const [newName, setNewName] = useState("");
  const [newTarget, setNewTarget] = useState("");

  return (
    <section>
      <header className="mb-2 flex items-center justify-between">
        <h2 className="text-sm font-semibold uppercase tracking-wide text-forest-300">
          {label}
        </h2>
        <SecondaryButton onClick={onAdd}>+ Add</SecondaryButton>
      </header>
      {adding ? (
        <div className="mb-2 flex items-end gap-2 rounded-md border border-graphite-700 bg-graphite-900 p-3">
          <label className="flex flex-1 flex-col gap-1">
            <span className="text-xs text-graphite-300">Name</span>
            <input
              type="text"
              value={newName}
              onChange={(e) => setNewName(e.target.value)}
              className="rounded-md border border-graphite-600 bg-graphite-800 px-2 py-1 text-sm text-graphite-50"
            />
          </label>
          <label className="flex flex-col gap-1">
            <span className="text-xs text-graphite-300">Monthly budget ($)</span>
            <input
              type="number"
              step="0.01"
              value={newTarget}
              onChange={(e) => setNewTarget(e.target.value)}
              className="w-28 rounded-md border border-graphite-600 bg-graphite-800 px-2 py-1 text-right font-mono text-sm text-graphite-50"
            />
          </label>
          <PrimaryButton
            onClick={() => {
              onSubmit(newName, newTarget);
              setNewName("");
              setNewTarget("");
            }}
            disabled={!newName.trim()}
          >
            Add
          </PrimaryButton>
          <GhostButton onClick={onCancelAdd}>Cancel</GhostButton>
        </div>
      ) : null}
      <ul className="divide-y divide-graphite-700 rounded-md border border-graphite-700">
        {cats.map((c) => (
          <li key={c.id} className="flex items-center gap-3 px-3 py-2">
            <input
              type="checkbox"
              checked={c.is_active}
              onChange={(e) => onToggleActive(c, e.target.checked)}
              className="h-4 w-4 rounded border-graphite-500 bg-graphite-800 text-forest-500"
            />
            <span
              className={`flex-1 text-sm ${
                c.is_active ? "text-graphite-50" : "text-graphite-500"
              }`}
            >
              {c.name}
              {c.is_seed ? (
                <span className="ml-2 rounded bg-graphite-700 px-1.5 py-0.5 text-xs text-graphite-300">
                  seed
                </span>
              ) : null}
            </span>
            <div className="flex items-center gap-1">
              <span className="text-xs text-graphite-400">$</span>
              <input
                type="number"
                step="0.01"
                placeholder="0"
                defaultValue={
                  c.monthly_target_cents != null
                    ? (c.monthly_target_cents / 100).toFixed(2)
                    : ""
                }
                onBlur={(e) => onTargetChange(c, e.target.value)}
                disabled={!c.is_active}
                className="w-24 rounded-md border border-graphite-600 bg-graphite-800 px-2 py-1 text-right font-mono text-sm text-graphite-50 disabled:opacity-50"
              />
            </div>
            {!c.is_seed ? (
              <button
                type="button"
                onClick={() => onRemove(c)}
                className="rounded px-2 py-1 text-xs text-red-300 hover:bg-red-500/10"
              >
                Delete
              </button>
            ) : null}
          </li>
        ))}
      </ul>
    </section>
  );
}

function SumTotalCard({
  label,
  value,
  emphasize,
}: {
  label: string;
  value: string;
  emphasize?: boolean;
}) {
  return (
    <div
      className={`rounded-lg border p-3 ${
        emphasize ? "border-forest-400 bg-forest-700/20" : "border-graphite-700 bg-graphite-900"
      }`}
    >
      <div className="text-xs uppercase tracking-wide text-graphite-400">{label}</div>
      <div className="mt-1 break-words font-mono text-lg text-graphite-50">{value}</div>
    </div>
  );
}

