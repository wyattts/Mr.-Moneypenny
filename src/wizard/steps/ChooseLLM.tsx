import { useState } from "react";

import { saveLlmProvider, setSetupStep } from "@/lib/tauri";
import { useWizard } from "@/lib/store";
import { StepLayout, ErrorBanner } from "../components/Layout";
import { PrimaryButton, GhostButton } from "../components/Buttons";

export function ChooseLLMStep() {
  const setStep = useWizard((s) => s.setStep);
  const [picked, setPicked] = useState<"anthropic" | "ollama" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function commit() {
    if (!picked) return;
    setBusy(true);
    setError(null);
    try {
      await saveLlmProvider(picked);
      await setSetupStep(2);
      setStep(picked === "anthropic" ? "anthropic" : "ollama");
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <StepLayout
      stepIndex={2}
      totalSteps={8}
      title="Pick your language model."
      subtitle="Mr. Moneypenny needs an LLM to understand plain-English expense messages."
      footer={
        <>
          <GhostButton onClick={() => setStep("welcome")}>Back</GhostButton>
          <PrimaryButton onClick={commit} disabled={!picked || busy}>
            Continue
          </PrimaryButton>
        </>
      }
    >
      {error ? <ErrorBanner>{error}</ErrorBanner> : null}
      <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
        <ProviderCard
          name="Anthropic Claude"
          recommended
          description="Cloud LLM. Roughly $0.50–$2 per month for typical use, paid to Anthropic with your own API key. Reliable, no setup beyond pasting a key."
          privacy="Expense descriptions go to Anthropic's API. Their privacy policy applies. The Mr. Moneypenny project still keeps zero data."
          selected={picked === "anthropic"}
          onClick={() => setPicked("anthropic")}
        />
        <ProviderCard
          name="Ollama (local)"
          description="Run a local model on your machine. Free, fully offline. Requires installing Ollama separately and pulling a model."
          privacy="Nothing leaves your computer. Maximum privacy."
          selected={picked === "ollama"}
          onClick={() => setPicked("ollama")}
        />
      </div>
    </StepLayout>
  );
}

function ProviderCard({
  name,
  recommended,
  description,
  privacy,
  selected,
  onClick,
}: {
  name: string;
  recommended?: boolean;
  description: string;
  privacy: string;
  selected: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={`flex flex-col gap-3 rounded-lg border p-5 text-left transition ${
        selected
          ? "border-forest-400 bg-forest-700/20"
          : "border-graphite-600 hover:border-graphite-500"
      }`}
    >
      <div className="flex items-center justify-between">
        <span className="text-lg font-semibold text-graphite-50">{name}</span>
        {recommended ? (
          <span className="rounded bg-forest-500/30 px-2 py-0.5 text-xs font-medium text-forest-200">
            Recommended
          </span>
        ) : null}
      </div>
      <p className="text-sm text-graphite-300">{description}</p>
      <p className="text-xs text-graphite-400">{privacy}</p>
    </button>
  );
}
