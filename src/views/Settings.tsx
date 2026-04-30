import { useEffect, useRef, useState } from "react";

import {
  checkForUpdate,
  clearAuthorizedChats,
  generatePairingCode,
  getAutostart,
  getCheckUpdatesOnLaunch,
  getRunInBackground,
  getSetupState,
  installUpdate,
  listAuthorizedChats,
  saveAnthropicKey,
  saveCurrencyLocale,
  saveTelegramToken,
  setAutostart,
  setCheckUpdatesOnLaunch,
  setRunInBackground,
  testAnthropic,
} from "@/lib/tauri";
import type { AuthorizedChat, SetupState, TelegramBotInfo } from "@/lib/tauri";
import { ThemeToggle } from "@/components/ThemeToggle";
import { CURRENCIES } from "@/lib/currencies";
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
          title="App updates"
          description="Auto-updates work for AppImage / DMG / MSI / EXE installs. RPM and DEB packages still upgrade through your system package manager."
        >
          <UpdateControls
            onSaved={(msg) => setInfo(msg)}
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
        <select
          value={currency}
          onChange={(e) => setCurrency(e.target.value)}
          className="rounded-md border border-graphite-600 bg-graphite-800 px-2 py-1 text-sm text-graphite-50 focus:border-forest-400 focus:outline-none"
        >
          {CURRENCIES.find((c) => c.code === currency) ? null : (
            <option value={currency}>{currency} (custom)</option>
          )}
          {CURRENCIES.map((c) => (
            <option key={c.code} value={c.code}>
              {c.label}
            </option>
          ))}
        </select>
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

type RotateStage = "idle" | "editing" | "rotated" | "pairing" | "paired";

function RotateTelegramToken({
  tokenIsSet,
  onSaved,
  onError,
}: {
  tokenIsSet: boolean;
  onSaved: (msg: string) => void;
  onError: (msg: string) => void;
}) {
  const [stage, setStage] = useState<RotateStage>("idle");
  const [val, setVal] = useState("");
  const [factoryReset, setFactoryReset] = useState(false);
  const [busy, setBusy] = useState(false);

  // Carried across stages.
  const [bot, setBot] = useState<TelegramBotInfo | null>(null);
  const [clearedCount, setClearedCount] = useState<number | null>(null);

  // Pairing-stage state.
  const [displayName, setDisplayName] = useState("");
  const [pairingCode, setPairingCode] = useState<string | null>(null);
  const [pairedChat, setPairedChat] = useState<AuthorizedChat | null>(null);
  const baselineChatCount = useRef<number>(0);
  const pollRef = useRef<number | null>(null);

  function reset() {
    setStage("idle");
    setVal("");
    setFactoryReset(false);
    setBot(null);
    setClearedCount(null);
    setDisplayName("");
    setPairingCode(null);
    setPairedChat(null);
    if (pollRef.current !== null) {
      window.clearInterval(pollRef.current);
      pollRef.current = null;
    }
  }

  // Poll list_authorized_chats while we're waiting for /start <code> to land.
  useEffect(() => {
    if (stage !== "pairing" || !pairingCode) return;
    pollRef.current = window.setInterval(async () => {
      try {
        const chats = await listAuthorizedChats();
        if (chats.length > baselineChatCount.current) {
          // The new pair is the most recently added; can't always tell
          // by ordering, so pick the one whose chat_id we hadn't seen
          // before the code was issued. Easier: take the last entry.
          const last = chats[chats.length - 1];
          if (last) {
            setPairedChat(last);
            setStage("paired");
            if (pollRef.current !== null) {
              window.clearInterval(pollRef.current);
              pollRef.current = null;
            }
          }
        }
      } catch {
        /* keep polling */
      }
    }, 1500);
    return () => {
      if (pollRef.current !== null) {
        window.clearInterval(pollRef.current);
        pollRef.current = null;
      }
    };
  }, [stage, pairingCode]);

  async function saveToken() {
    setBusy(true);
    try {
      const info = await saveTelegramToken(val);
      let cleared: number | null = null;
      if (factoryReset) {
        cleared = await clearAuthorizedChats();
      }
      setBot(info);
      setClearedCount(cleared);
      setVal("");
      setStage("rotated");
      onSaved(
        cleared !== null
          ? `Connected to @${info.username ?? info.first_name}. Cleared ${cleared} authorized chat${cleared === 1 ? "" : "s"}.`
          : `Connected to @${info.username ?? info.first_name}.`,
      );
    } catch (e) {
      onError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function issueCode() {
    if (!displayName.trim()) {
      onError("Pick a display name first.");
      return;
    }
    setBusy(true);
    try {
      // Snapshot current chat count so we can detect the new pair.
      const before = await listAuthorizedChats();
      baselineChatCount.current = before.length;
      const code = await generatePairingCode(displayName.trim());
      setPairingCode(code);
    } catch (e) {
      onError(String(e));
    } finally {
      setBusy(false);
    }
  }

  if (stage === "idle") {
    return (
      <div className="flex items-center gap-2">
        <span className="text-sm text-graphite-300">
          {tokenIsSet ? "Token is saved (in OS keychain)." : "No token saved."}
        </span>
        <SecondaryButton onClick={() => setStage("editing")}>
          {tokenIsSet ? "Rotate token" : "Add token"}
        </SecondaryButton>
      </div>
    );
  }

  if (stage === "editing") {
    return (
      <div className="space-y-3">
        <div className="flex items-center gap-2">
          <input
            type="password"
            autoComplete="off"
            value={val}
            onChange={(e) => setVal(e.target.value)}
            placeholder="123456789:AA..."
            className="flex-1 rounded-md border border-graphite-600 bg-graphite-800 px-2 py-1 font-mono text-sm text-graphite-50"
          />
          <PrimaryButton onClick={saveToken} disabled={!val.trim() || busy}>
            {busy ? "…" : "Save"}
          </PrimaryButton>
          <GhostButton onClick={reset}>Cancel</GhostButton>
        </div>
        <label className="flex cursor-pointer items-start gap-2">
          <input
            type="checkbox"
            checked={factoryReset}
            onChange={(e) => setFactoryReset(e.target.checked)}
            className="mt-0.5 h-4 w-4 rounded border-graphite-500 bg-graphite-800 text-forest-500 focus:ring-forest-400"
          />
          <span>
            <span className="block text-xs text-graphite-200">
              Also clear all authorized chats (factory reset)
            </span>
            <span className="block text-xs text-graphite-400">
              Useful when paired to a brand-new bot. You&apos;ll get a fresh pairing code below
              after saving — the first /start &lt;code&gt; redemption becomes the new owner.
            </span>
          </span>
        </label>
      </div>
    );
  }

  if (stage === "rotated") {
    return (
      <div className="space-y-3">
        <InfoBanner>
          Connected to <code className="font-mono">@{bot?.username ?? bot?.first_name}</code>
          {clearedCount !== null
            ? `. ${clearedCount} authorized chat${clearedCount === 1 ? "" : "s"} cleared.`
            : "."}
        </InfoBanner>
        <p className="text-xs text-graphite-400">
          {clearedCount !== null
            ? "No one is paired right now — generate a code below and message your new bot to pair."
            : "Existing chat IDs still work, but a fresh pairing code is the easiest way to verify the new bot answers."}
        </p>
        <div className="flex items-center gap-2">
          <PrimaryButton onClick={() => setStage("pairing")}>Generate pairing code</PrimaryButton>
          <GhostButton onClick={reset}>Done</GhostButton>
        </div>
      </div>
    );
  }

  if (stage === "pairing") {
    return (
      <div className="space-y-3">
        {!pairingCode ? (
          <>
            <p className="text-sm text-graphite-300">
              Pick a display name for the chat that will redeem this code. If the authorized list
              is empty, this chat becomes the household owner.
            </p>
            <div className="flex items-end gap-2">
              <input
                type="text"
                placeholder="Wyatt"
                value={displayName}
                onChange={(e) => setDisplayName(e.target.value)}
                className="flex-1 rounded-md border border-graphite-600 bg-graphite-800 px-3 py-2 text-sm text-graphite-50 focus:border-forest-400 focus:outline-none"
              />
              <PrimaryButton onClick={issueCode} disabled={!displayName.trim() || busy}>
                {busy ? "…" : "Generate code"}
              </PrimaryButton>
              <GhostButton onClick={reset}>Cancel</GhostButton>
            </div>
          </>
        ) : (
          <>
            <div className="rounded-md border border-graphite-600 bg-graphite-800 p-4">
              <div className="text-sm text-graphite-300">
                Pairing code for <strong>{displayName}</strong>:
              </div>
              <div className="mt-1 font-mono text-3xl tracking-widest text-forest-200">
                {pairingCode}
              </div>
              <div className="mt-2 text-xs text-graphite-400">
                Single-use, expires in 10 minutes. Open{" "}
                <code className="font-mono">@{bot?.username ?? bot?.first_name}</code> in Telegram and send:
              </div>
              <pre className="mt-2 rounded-md border border-graphite-700 bg-graphite-900 px-3 py-2 font-mono text-sm text-forest-200">
                /start {pairingCode}
              </pre>
            </div>
            <p className="flex items-center gap-2 text-xs text-graphite-400">
              <span className="inline-block h-2 w-2 animate-pulse rounded-full bg-forest-400"></span>
              Listening for the start command…
            </p>
            <GhostButton onClick={reset}>Cancel</GhostButton>
          </>
        )}
      </div>
    );
  }

  // stage === "paired"
  return (
    <div className="space-y-3">
      <InfoBanner>
        Paired as <strong>{pairedChat?.display_name}</strong> ({pairedChat?.role}).
      </InfoBanner>
      <PrimaryButton onClick={reset}>Done</PrimaryButton>
    </div>
  );
}

function UpdateControls({
  onSaved,
  onError,
}: {
  onSaved: (msg: string) => void;
  onError: (msg: string) => void;
}) {
  const [onLaunch, setOnLaunch] = useState(true);
  const [busy, setBusy] = useState(false);
  const [pending, setPending] = useState<{ version: string } | null>(null);
  const [installing, setInstalling] = useState(false);

  useEffect(() => {
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function load() {
    try {
      const v = await getCheckUpdatesOnLaunch();
      setOnLaunch(v);
    } catch (e) {
      onError(String(e));
    }
  }

  async function toggle(enabled: boolean) {
    try {
      await setCheckUpdatesOnLaunch(enabled);
      setOnLaunch(enabled);
    } catch (e) {
      onError(String(e));
    }
  }

  async function checkNow() {
    setBusy(true);
    setPending(null);
    try {
      const r = await checkForUpdate();
      if (r.available && r.version) {
        setPending({ version: r.version });
        onSaved(`Update available: v${r.version}.`);
      } else {
        onSaved(`You're on the latest version (v${r.current_version}).`);
      }
    } catch (e) {
      onError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function install() {
    setInstalling(true);
    try {
      // Resolves only on failure; success triggers a relaunch.
      await installUpdate();
    } catch (e) {
      onError(String(e));
      setInstalling(false);
    }
  }

  return (
    <>
      <ToggleRow
        label="Check for updates on launch"
        description="On startup, ask GitHub Releases whether a newer version exists. No telemetry — just one HEAD-style request to api.github.com."
        checked={onLaunch}
        onChange={toggle}
      />
      <div className="flex items-center gap-2">
        <SecondaryButton onClick={checkNow} disabled={busy || installing}>
          {busy ? "…" : "Check now"}
        </SecondaryButton>
        {pending ? (
          <PrimaryButton onClick={install} disabled={installing}>
            {installing ? "Installing…" : `Install v${pending.version}`}
          </PrimaryButton>
        ) : null}
      </div>
    </>
  );
}
