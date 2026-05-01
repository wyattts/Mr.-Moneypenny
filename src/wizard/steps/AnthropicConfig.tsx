import { useState } from "react";

import { saveAnthropicKey, setSetupStep, testAnthropic } from "@/lib/tauri";
import { useWizard } from "@/lib/store";
import { StepLayout, ErrorBanner, InfoBanner } from "../components/Layout";
import { PrimaryButton, GhostButton } from "../components/Buttons";

export function AnthropicConfigStep() {
  const setStep = useWizard((s) => s.setStep);
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [verifiedModel, setVerifiedModel] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function saveAndTest() {
    setBusy(true);
    setError(null);
    setVerifiedModel(null);
    try {
      await saveAnthropicKey(key);
      const model = await testAnthropic();
      setVerifiedModel(model);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function next() {
    await setSetupStep(3);
    setStep("telegram");
  }

  return (
    <StepLayout
      stepIndex={3}
      totalSteps={8}
      title="Anthropic API key"
      subtitle="Get a key at console.anthropic.com → Settings → API Keys, then paste it here. Stored encrypted under a machine-bound key — never in plaintext, never sent off your computer."
      footer={
        <>
          <GhostButton onClick={() => setStep("choose_llm")}>Back</GhostButton>
          {verifiedModel ? (
            <PrimaryButton onClick={next}>Continue</PrimaryButton>
          ) : (
            <PrimaryButton onClick={saveAndTest} disabled={!key.trim() || busy}>
              {busy ? "Verifying…" : "Save & verify"}
            </PrimaryButton>
          )}
        </>
      }
    >
      <div className="space-y-4">
        <input
          type="password"
          autoComplete="off"
          spellCheck={false}
          value={key}
          onChange={(e) => setKey(e.target.value)}
          placeholder="sk-ant-..."
          className="w-full rounded-md border border-graphite-600 bg-graphite-800 px-3 py-2 font-mono text-sm text-graphite-50 placeholder:text-graphite-500 focus:border-forest-400 focus:outline-none"
        />
        {error ? <ErrorBanner>{error}</ErrorBanner> : null}
        {verifiedModel ? (
          <InfoBanner>
            Verified. Using model <code className="font-mono">{verifiedModel}</code>.
          </InfoBanner>
        ) : null}
        <p className="text-xs text-graphite-400">
          A successful verification spends roughly $0.0001 in API tokens.
        </p>
      </div>
    </StepLayout>
  );
}
