import { useState } from "react";

import { finalizeSetup, getSetupState } from "@/lib/tauri";
import { useWizard } from "@/lib/store";
import { StepLayout, ErrorBanner } from "../components/Layout";
import { PrimaryButton } from "../components/Buttons";

export function DoneStep() {
  const setSetup = useWizard((s) => s.setSetup);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function finish() {
    setBusy(true);
    setError(null);
    try {
      await finalizeSetup();
      setSetup(await getSetupState());
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  }

  return (
    <StepLayout
      stepIndex={8}
      totalSteps={8}
      title="You're set."
      subtitle="Open your bot in Telegram and send something like “$5 coffee”."
      footer={
        <PrimaryButton onClick={finish} disabled={busy}>
          {busy ? "Finishing…" : "Open Mr. Moneypenny"}
        </PrimaryButton>
      }
    >
      {error ? <ErrorBanner>{error}</ErrorBanner> : null}
      <ul className="space-y-2 text-sm text-graphite-300">
        <li>• Try logging an expense in plain English from your phone or any device.</li>
        <li>• Ask “how am I doing this month” for a budget check.</li>
        <li>• Use Settings → Household to invite a partner with a fresh pairing code.</li>
        <li>• Your data stays on this computer. Back it up regularly.</li>
      </ul>
    </StepLayout>
  );
}
