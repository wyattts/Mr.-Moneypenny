import { useEffect, useState } from "react";

import {
  getAutostart,
  getRunInBackground,
  getSetupState,
  saveAnthropicKey,
  saveCurrencyLocale,
  saveTelegramToken,
  setAutostart,
  setRunInBackground,
  testAnthropic,
} from "@/lib/tauri";
import type { SetupState } from "@/lib/tauri";
import { ThemeToggle } from "@/components/ThemeToggle";
import { ViewHeader } from "./ViewHeader";
import { ErrorBanner, InfoBanner } from "@/wizard/components/Layout";
import { GhostButton, PrimaryButton, SecondaryButton } from "@/wizard/components/Buttons";

export function Settings() {
  const [setup, setSetup] = useState<SetupState | null>(null);
  const [bgMode, setBgMode] = useState(true);
  const [autostart, setAuto] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);

  useEffect(() => {
    void load();
  }, []);

  async function load() {
    try {
      const [s, bg, au] = await Promise.all([
        getSetupState(),
        getRunInBackground(),
        getAutostart(),
      ]);
      setSetup(s);
      setBgMode(bg);
      setAuto(au);
    } catch (e) {
      setError(String(e));
    }
  }

  async function toggleBg(enabled: boolean) {
    try {
      await setRunInBackground(enabled);
      setBgMode(enabled);
    } catch (e) {
      setError(String(e));
    }
  }

  async function toggleAutostart(enabled: boolean) {
    try {
      await setAutostart(enabled);
      setAuto(enabled);
    } catch (e) {
      setError(String(e));
    }
  }

  return (
    <div>
      <ViewHeader
        title="Settings"
        subtitle="Background mode, secrets rotation, locale, and privacy."
      />
      <div className="mx-auto max-w-3xl space-y-6 px-8 py-8">
        {error ? <ErrorBanner>{error}</ErrorBanner> : null}
        {info ? <InfoBanner>{info}</InfoBanner> : null}

        <Section title="Appearance" description="How Mr. Moneypenny is themed in the desktop window.">
          <div className="flex items-center justify-between">
            <div>
              <span className="block text-sm text-graphite-50">Theme</span>
              <span className="block text-xs text-graphite-400">
                System follows your OS preference and updates live.
              </span>
            </div>
            <ThemeToggle />
          </div>
        </Section>

        <Section title="Background mode" description="How should the app behave when you close the window?">
          <ToggleRow
            label="Run in background"
            description="When you close the window, the app keeps running in the system tray so the bot stays online."
            checked={bgMode}
            onChange={toggleBg}
          />
          <ToggleRow
            label="Start on login"
            description="Launch automatically when you log in (hidden, in the tray). On Linux you may need the AppIndicator GNOME extension to see the tray icon."
            checked={autostart}
            onChange={toggleAutostart}
          />
        </Section>

        <Section
          title="Currency & locale"
          description="Default currency for amounts the bot logs."
        >
          <CurrencyLocaleEditor
            initialCurrency={setup?.default_currency ?? "USD"}
            initialLocale={setup?.locale ?? "en-US"}
            onSaved={(msg) => {
              setInfo(msg);
              void load();
            }}
            onError={setError}
          />
        </Section>

        <Section
          title="Anthropic API key"
          description="Rotate the API key Mr. Moneypenny uses for the cloud LLM."
        >
          <RotateAnthropicKey
            keyIsSet={setup?.anthropic_key_set ?? false}
            onSaved={(msg) => {
              setInfo(msg);
              void load();
            }}
            onError={setError}
          />
        </Section>

        <Section
          title="Telegram bot token"
          description="Rotate the bot token. After rotating you'll need to send /start <code> from your phone again to re-pair."
        >
          <RotateTelegramToken
            tokenIsSet={setup?.telegram_token_set ?? false}
            onSaved={(msg) => {
              setInfo(msg);
              void load();
            }}
            onError={setError}
          />
        </Section>
      </div>
    </div>
  );
}

function Section({
  title,
  description,
  children,
}: {
  title: string;
  description?: string;
  children: React.ReactNode;
}) {
  return (
    <section className="rounded-lg border border-graphite-700 bg-graphite-900 p-5">
      <header className="mb-4">
        <h2 className="text-base font-semibold text-graphite-50">{title}</h2>
        {description ? (
          <p className="mt-1 text-xs text-graphite-400">{description}</p>
        ) : null}
      </header>
      <div className="space-y-3">{children}</div>
    </section>
  );
}

function ToggleRow({
  label,
  description,
  checked,
  onChange,
}: {
  label: string;
  description?: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
}) {
  return (
    <label className="flex cursor-pointer items-start gap-3">
      <input
        type="checkbox"
        checked={checked}
        onChange={(e) => onChange(e.target.checked)}
        className="mt-1 h-4 w-4 rounded border-graphite-500 bg-graphite-800 text-forest-500 focus:ring-forest-400"
      />
      <span>
        <span className="block text-sm text-graphite-50">{label}</span>
        {description ? (
          <span className="block text-xs text-graphite-400">{description}</span>
        ) : null}
      </span>
    </label>
  );
}

function CurrencyLocaleEditor({
  initialCurrency,
  initialLocale,
  onSaved,
  onError,
}: {
  initialCurrency: string;
  initialLocale: string;
  onSaved: (msg: string) => void;
  onError: (msg: string) => void;
}) {
  const [currency, setCurrency] = useState(initialCurrency);
  const [locale, setLocale] = useState(initialLocale);
  const [busy, setBusy] = useState(false);

  async function save() {
    setBusy(true);
    try {
      await saveCurrencyLocale(currency, locale);
      onSaved("Saved.");
    } catch (e) {
      onError(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="flex items-end gap-2">
      <label className="flex flex-col gap-1">
        <span className="text-xs text-graphite-300">Currency</span>
        <input
          type="text"
          value={currency}
          onChange={(e) => setCurrency(e.target.value.toUpperCase())}
          className="w-24 rounded-md border border-graphite-600 bg-graphite-800 px-2 py-1 font-mono text-sm text-graphite-50"
        />
      </label>
      <label className="flex flex-col gap-1">
        <span className="text-xs text-graphite-300">Locale</span>
        <input
          type="text"
          value={locale}
          onChange={(e) => setLocale(e.target.value)}
          className="w-32 rounded-md border border-graphite-600 bg-graphite-800 px-2 py-1 font-mono text-sm text-graphite-50"
        />
      </label>
      <PrimaryButton onClick={save} disabled={busy}>
        {busy ? "…" : "Save"}
      </PrimaryButton>
    </div>
  );
}

function RotateAnthropicKey({
  keyIsSet,
  onSaved,
  onError,
}: {
  keyIsSet: boolean;
  onSaved: (msg: string) => void;
  onError: (msg: string) => void;
}) {
  const [editing, setEditing] = useState(false);
  const [val, setVal] = useState("");
  const [busy, setBusy] = useState(false);

  async function save() {
    setBusy(true);
    try {
      await saveAnthropicKey(val);
      const model = await testAnthropic();
      onSaved(`Verified. Using model ${model}.`);
      setEditing(false);
      setVal("");
    } catch (e) {
      onError(String(e));
    } finally {
      setBusy(false);
    }
  }

  if (!editing) {
    return (
      <div className="flex items-center gap-2">
        <span className="text-sm text-graphite-300">
          {keyIsSet ? "Key is saved (in OS keychain)." : "No key saved."}
        </span>
        <SecondaryButton onClick={() => setEditing(true)}>
          {keyIsSet ? "Rotate key" : "Add key"}
        </SecondaryButton>
      </div>
    );
  }

  return (
    <div className="flex items-center gap-2">
      <input
        type="password"
        autoComplete="off"
        value={val}
        onChange={(e) => setVal(e.target.value)}
        placeholder="sk-ant-..."
        className="flex-1 rounded-md border border-graphite-600 bg-graphite-800 px-2 py-1 font-mono text-sm text-graphite-50"
      />
      <PrimaryButton onClick={save} disabled={!val.trim() || busy}>
        {busy ? "…" : "Save & verify"}
      </PrimaryButton>
      <GhostButton onClick={() => setEditing(false)}>Cancel</GhostButton>
    </div>
  );
}

function RotateTelegramToken({
  tokenIsSet,
  onSaved,
  onError,
}: {
  tokenIsSet: boolean;
  onSaved: (msg: string) => void;
  onError: (msg: string) => void;
}) {
  const [editing, setEditing] = useState(false);
  const [val, setVal] = useState("");
  const [busy, setBusy] = useState(false);

  async function save() {
    setBusy(true);
    try {
      const info = await saveTelegramToken(val);
      onSaved(`Connected to @${info.username ?? info.first_name}.`);
      setEditing(false);
      setVal("");
    } catch (e) {
      onError(String(e));
    } finally {
      setBusy(false);
    }
  }

  if (!editing) {
    return (
      <div className="flex items-center gap-2">
        <span className="text-sm text-graphite-300">
          {tokenIsSet ? "Token is saved (in OS keychain)." : "No token saved."}
        </span>
        <SecondaryButton onClick={() => setEditing(true)}>
          {tokenIsSet ? "Rotate token" : "Add token"}
        </SecondaryButton>
      </div>
    );
  }

  return (
    <div className="flex items-center gap-2">
      <input
        type="password"
        autoComplete="off"
        value={val}
        onChange={(e) => setVal(e.target.value)}
        placeholder="123456789:AA..."
        className="flex-1 rounded-md border border-graphite-600 bg-graphite-800 px-2 py-1 font-mono text-sm text-graphite-50"
      />
      <PrimaryButton onClick={save} disabled={!val.trim() || busy}>
        {busy ? "…" : "Save"}
      </PrimaryButton>
      <GhostButton onClick={() => setEditing(false)}>Cancel</GhostButton>
    </div>
  );
}
