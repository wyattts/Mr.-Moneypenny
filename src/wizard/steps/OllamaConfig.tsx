import { useEffect, useState } from "react";

import {
  listOllamaModels,
  saveOllamaConfig,
  setSetupStep,
  testOllama,
} from "@/lib/tauri";
import { useWizard } from "@/lib/store";
import { StepLayout, ErrorBanner, InfoBanner } from "../components/Layout";
import { PrimaryButton, GhostButton, SecondaryButton } from "../components/Buttons";

const DEFAULT_ENDPOINT = "http://localhost:11434";
const DEFAULT_RECOMMENDED = "llama3:8b";

export function OllamaConfigStep() {
  const setStep = useWizard((s) => s.setStep);
  const [endpoint, setEndpoint] = useState(DEFAULT_ENDPOINT);
  const [models, setModels] = useState<string[]>([]);
  const [model, setModel] = useState<string>("");
  const [busy, setBusy] = useState(false);
  const [verified, setVerified] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void refreshModels();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function refreshModels() {
    setError(null);
    setBusy(true);
    try {
      const list = await listOllamaModels(endpoint);
      setModels(list);
      if (!model && list.includes(DEFAULT_RECOMMENDED)) setModel(DEFAULT_RECOMMENDED);
      else if (!model && list.length > 0 && list[0]) setModel(list[0]);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function saveAndTest() {
    setBusy(true);
    setError(null);
    setVerified(false);
    try {
      await saveOllamaConfig(endpoint, model);
      await testOllama();
      setVerified(true);
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
      title="Ollama (local LLM)"
      subtitle={
        <>
          Make sure Ollama is running and you&apos;ve pulled a model that supports
          tools, e.g. <code className="font-mono">llama3:8b</code>.
        </>
      }
      footer={
        <>
          <GhostButton onClick={() => setStep("choose_llm")}>Back</GhostButton>
          {verified ? (
            <PrimaryButton onClick={next}>Continue</PrimaryButton>
          ) : (
            <PrimaryButton onClick={saveAndTest} disabled={!model || busy}>
              {busy ? "Verifying…" : "Save & verify"}
            </PrimaryButton>
          )}
        </>
      }
    >
      <div className="space-y-4">
        <label className="block">
          <span className="text-sm text-graphite-300">Endpoint</span>
          <div className="mt-1 flex gap-2">
            <input
              type="text"
              value={endpoint}
              onChange={(e) => setEndpoint(e.target.value)}
              className="flex-1 rounded-md border border-graphite-600 bg-graphite-800 px-3 py-2 font-mono text-sm text-graphite-50 focus:border-forest-400 focus:outline-none"
            />
            <SecondaryButton onClick={refreshModels} disabled={busy}>
              {busy ? "…" : "Refresh"}
            </SecondaryButton>
          </div>
        </label>
        <label className="block">
          <span className="text-sm text-graphite-300">Model</span>
          {models.length === 0 ? (
            <p className="mt-1 rounded-md border border-graphite-600 bg-graphite-800 px-3 py-2 text-sm text-graphite-400">
              No models found. In a terminal:{" "}
              <code className="font-mono">ollama pull llama3:8b</code>
            </p>
          ) : (
            <select
              value={model}
              onChange={(e) => setModel(e.target.value)}
              className="mt-1 w-full rounded-md border border-graphite-600 bg-graphite-800 px-3 py-2 font-mono text-sm text-graphite-50 focus:border-forest-400 focus:outline-none"
            >
              {models.map((m) => (
                <option key={m} value={m}>
                  {m}
                </option>
              ))}
            </select>
          )}
        </label>
        {error ? <ErrorBanner>{error}</ErrorBanner> : null}
        {verified ? (
          <InfoBanner>Verified — Ollama responded.</InfoBanner>
        ) : null}
      </div>
    </StepLayout>
  );
}
