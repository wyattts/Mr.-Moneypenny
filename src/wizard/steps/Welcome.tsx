import { setSetupStep } from "@/lib/tauri";
import { useWizard } from "@/lib/store";
import { StepLayout } from "../components/Layout";
import { PrimaryButton } from "../components/Buttons";

export function WelcomeStep() {
  const setStep = useWizard((s) => s.setStep);

  async function accept() {
    await setSetupStep(1);
    setStep("choose_llm");
  }

  return (
    <StepLayout
      stepIndex={1}
      totalSteps={8}
      title="Welcome."
      subtitle="A few things to know before we set things up."
      footer={<PrimaryButton onClick={accept}>I understand</PrimaryButton>}
    >
      <div className="space-y-4 text-graphite-200">
        <p>
          <strong className="text-graphite-50">Your data lives only on this computer.</strong>{" "}
          Mr. Moneypenny does not run any servers. We do not have a copy of your
          expenses, your bot token, or your API key.
        </p>
        <p>
          If you delete the app or lose this computer{" "}
          <strong className="text-graphite-50">your data is gone</strong> unless
          you&apos;ve made a backup. You can export anytime from{" "}
          <em>Settings → Export</em> later.
        </p>
        <p>
          Mr. Moneypenny is{" "}
          <strong className="text-graphite-50">not connected to your bank</strong>.
          It only knows what you tell your bot.
        </p>
        <p className="text-graphite-400">
          By continuing you accept the AGPL-3.0 license and the privacy posture
          described in <code className="text-forest-300">docs/privacy.md</code>.
        </p>
      </div>
    </StepLayout>
  );
}
