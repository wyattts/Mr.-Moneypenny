import { useEffect, useState } from "react";

import { listCategories, setSetupStep } from "@/lib/tauri";
import type { CategoryView } from "@/lib/tauri";
import { useWizard } from "@/lib/store";
import { StepLayout, ErrorBanner } from "../components/Layout";
import { PrimaryButton, GhostButton } from "../components/Buttons";

export function SanityStep() {
  const setStep = useWizard((s) => s.setStep);
  const [cats, setCats] = useState<CategoryView[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void load();
  }, []);

  async function load() {
    try {
      setCats(await listCategories(false));
    } catch (e) {
      setError(String(e));
    }
  }

  const fixedTotal = cats
    .filter((c) => c.kind === "fixed" && c.is_active)
    .reduce((acc, c) => acc + (c.monthly_target_cents ?? 0), 0);
  const variableTotal = cats
    .filter((c) => c.kind === "variable" && c.is_active)
    .reduce((acc, c) => acc + (c.monthly_target_cents ?? 0), 0);
  const grand = fixedTotal + variableTotal;

  async function next() {
    await setSetupStep(7);
    setStep("done");
  }

  return (
    <StepLayout
      stepIndex={7}
      totalSteps={8}
      title="Sanity check"
      subtitle="A quick look at the totals you just set. No judgement; just numbers."
      footer={
        <>
          <GhostButton onClick={() => setStep("categories")}>Back</GhostButton>
          <PrimaryButton onClick={next}>Looks right</PrimaryButton>
        </>
      }
    >
      {error ? <ErrorBanner>{error}</ErrorBanner> : null}
      <div className="grid grid-cols-1 gap-4 sm:grid-cols-3">
        <Tile label="Fixed monthly" cents={fixedTotal} muted />
        <Tile label="Variable monthly" cents={variableTotal} muted />
        <Tile label="Total / month" cents={grand} highlight />
      </div>
      <p className="mt-6 text-sm text-graphite-400">
        You can refine targets and add new categories any time from the
        Settings → Categories view.
      </p>
    </StepLayout>
  );
}

function Tile({
  label,
  cents,
  muted,
  highlight,
}: {
  label: string;
  cents: number;
  muted?: boolean;
  highlight?: boolean;
}) {
  return (
    <div
      className={`rounded-lg border p-4 ${
        highlight
          ? "border-forest-400 bg-forest-700/20"
          : "border-graphite-700 bg-graphite-900"
      }`}
    >
      <div
        className={`text-xs uppercase tracking-wide ${
          muted ? "text-graphite-400" : "text-forest-300"
        }`}
      >
        {label}
      </div>
      <div className="mt-1 font-mono text-2xl text-graphite-50">
        ${(cents / 100).toFixed(2)}
      </div>
    </div>
  );
}
