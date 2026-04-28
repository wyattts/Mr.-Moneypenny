import { useState } from "react";

import { saveCurrencyLocale, setSetupStep } from "@/lib/tauri";
import { useWizard } from "@/lib/store";
import { StepLayout, ErrorBanner } from "../components/Layout";
import { PrimaryButton, GhostButton } from "../components/Buttons";

const CURRENCIES = [
  { code: "USD", label: "US Dollar — $" },
  { code: "EUR", label: "Euro — €" },
  { code: "GBP", label: "British Pound — £" },
  { code: "JPY", label: "Japanese Yen — ¥" },
  { code: "CAD", label: "Canadian Dollar — CA$" },
  { code: "AUD", label: "Australian Dollar — A$" },
  { code: "CHF", label: "Swiss Franc — CHF" },
  { code: "CNY", label: "Chinese Yuan — ¥" },
  { code: "INR", label: "Indian Rupee — ₹" },
  { code: "MXN", label: "Mexican Peso — MX$" },
  { code: "BRL", label: "Brazilian Real — R$" },
  { code: "ZAR", label: "South African Rand — R" },
];

export function CurrencyLocaleStep() {
  const setStep = useWizard((s) => s.setStep);
  const setup = useWizard((s) => s.setup);
  const [currency, setCurrency] = useState(setup?.default_currency ?? "USD");
  const [locale, setLocale] = useState(
    setup?.locale ?? guessLocale(),
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function next() {
    setBusy(true);
    setError(null);
    try {
      await saveCurrencyLocale(currency, locale);
      await setSetupStep(5);
      setStep("categories");
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <StepLayout
      stepIndex={5}
      totalSteps={8}
      title="Currency & locale"
      subtitle={`How should amounts be displayed by default? You can override this per-expense by saying e.g. "€7 espresso".`}
      footer={
        <>
          <GhostButton onClick={() => setStep("telegram")}>Back</GhostButton>
          <PrimaryButton onClick={next} disabled={busy}>
            Continue
          </PrimaryButton>
        </>
      }
    >
      <div className="space-y-4">
        <label className="block">
          <span className="text-sm text-graphite-300">Currency</span>
          <select
            value={currency}
            onChange={(e) => setCurrency(e.target.value)}
            className="mt-1 w-full rounded-md border border-graphite-600 bg-graphite-800 px-3 py-2 text-sm text-graphite-50 focus:border-forest-400 focus:outline-none"
          >
            {CURRENCIES.map((c) => (
              <option key={c.code} value={c.code}>
                {c.label}
              </option>
            ))}
          </select>
        </label>
        <label className="block">
          <span className="text-sm text-graphite-300">Locale (BCP-47)</span>
          <input
            type="text"
            value={locale}
            onChange={(e) => setLocale(e.target.value)}
            placeholder="en-US"
            className="mt-1 w-full rounded-md border border-graphite-600 bg-graphite-800 px-3 py-2 font-mono text-sm text-graphite-50 focus:border-forest-400 focus:outline-none"
          />
          <p className="mt-1 text-xs text-graphite-400">
            Used for date/number formatting. e.g. <code>en-US</code>,{" "}
            <code>en-GB</code>, <code>de-DE</code>.
          </p>
        </label>
        {error ? <ErrorBanner>{error}</ErrorBanner> : null}
      </div>
    </StepLayout>
  );
}

function guessLocale(): string {
  if (typeof navigator !== "undefined" && navigator.language) {
    return navigator.language;
  }
  return "en-US";
}
