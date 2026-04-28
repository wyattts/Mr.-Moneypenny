import { useEffect, useState } from "react";

import {
  listCategories,
  setCategoryActive,
  setCategoryTarget,
  setSetupStep,
} from "@/lib/tauri";
import type { CategoryView } from "@/lib/tauri";
import { useWizard } from "@/lib/store";
import { StepLayout, ErrorBanner } from "../components/Layout";
import { PrimaryButton, GhostButton } from "../components/Buttons";

export function CategoriesStep() {
  const setStep = useWizard((s) => s.setStep);
  const [cats, setCats] = useState<CategoryView[]>([]);
  const [drafts, setDrafts] = useState<Record<number, string>>({});
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void load();
  }, []);

  async function load() {
    setBusy(true);
    setError(null);
    try {
      const list = await listCategories(false);
      setCats(list);
      const newDrafts: Record<number, string> = {};
      for (const c of list) {
        newDrafts[c.id] =
          c.monthly_target_cents != null
            ? (c.monthly_target_cents / 100).toFixed(2)
            : "";
      }
      setDrafts(newDrafts);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function commit() {
    setBusy(true);
    setError(null);
    try {
      for (const c of cats) {
        const draft = drafts[c.id]?.trim() ?? "";
        const cents = draft ? Math.round(Number(draft) * 100) : null;
        const newTarget = cents !== null && Number.isFinite(cents) ? cents : null;
        if (newTarget !== c.monthly_target_cents) {
          await setCategoryTarget(c.id, newTarget);
        }
      }
      await setSetupStep(6);
      setStep("sanity");
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
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

  const fixed = cats.filter((c) => c.kind === "fixed");
  const variable = cats.filter((c) => c.kind === "variable");

  return (
    <StepLayout
      stepIndex={6}
      totalSteps={8}
      title="Categories"
      subtitle="Set monthly amounts so Mr. Moneypenny can pace your spending. You can skip and configure later."
      footer={
        <>
          <GhostButton onClick={() => setStep("locale")}>Back</GhostButton>
          <PrimaryButton onClick={commit} disabled={busy}>
            {busy ? "…" : "Continue"}
          </PrimaryButton>
        </>
      }
    >
      <div className="space-y-6">
        {error ? <ErrorBanner>{error}</ErrorBanner> : null}
        <CategoryGroup
          label="Fixed (recurring monthly)"
          hint="Rent, insurance, subscriptions — things that post on a schedule."
          cats={fixed}
          drafts={drafts}
          onTargetChange={(id, v) => setDrafts((d) => ({ ...d, [id]: v }))}
          onToggle={toggleActive}
        />
        <CategoryGroup
          label="Variable (discretionary)"
          hint="Groceries, dining, entertainment — set a budget you want to pace against."
          cats={variable}
          drafts={drafts}
          onTargetChange={(id, v) => setDrafts((d) => ({ ...d, [id]: v }))}
          onToggle={toggleActive}
        />
      </div>
    </StepLayout>
  );
}

function CategoryGroup({
  label,
  hint,
  cats,
  drafts,
  onTargetChange,
  onToggle,
}: {
  label: string;
  hint: string;
  cats: CategoryView[];
  drafts: Record<number, string>;
  onTargetChange: (id: number, value: string) => void;
  onToggle: (c: CategoryView, active: boolean) => void;
}) {
  return (
    <section>
      <header className="mb-2">
        <h3 className="text-sm font-semibold uppercase tracking-wide text-forest-300">
          {label}
        </h3>
        <p className="text-xs text-graphite-400">{hint}</p>
      </header>
      <ul className="divide-y divide-graphite-700 rounded-md border border-graphite-700">
        {cats.map((c) => (
          <li key={c.id} className="flex items-center gap-3 px-3 py-2">
            <input
              type="checkbox"
              checked={c.is_active}
              onChange={(e) => onToggle(c, e.target.checked)}
              className="h-4 w-4 rounded border-graphite-500 bg-graphite-800 text-forest-500 focus:ring-forest-400"
            />
            <span
              className={`flex-1 text-sm ${
                c.is_active ? "text-graphite-50" : "text-graphite-500"
              }`}
            >
              {c.name}
            </span>
            <div className="flex items-center gap-1">
              <span className="text-xs text-graphite-400">$</span>
              <input
                type="number"
                step="0.01"
                placeholder="0"
                value={drafts[c.id] ?? ""}
                onChange={(e) => onTargetChange(c.id, e.target.value)}
                disabled={!c.is_active}
                className="w-24 rounded-md border border-graphite-600 bg-graphite-800 px-2 py-1 text-right font-mono text-sm text-graphite-50 disabled:opacity-50"
              />
            </div>
          </li>
        ))}
      </ul>
    </section>
  );
}
