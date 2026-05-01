import { useEffect, useRef, useState } from "react";

import {
  generatePairingCode,
  listAuthorizedChats,
  saveTelegramToken,
  setSetupStep,
} from "@/lib/tauri";
import type { AuthorizedChat, TelegramBotInfo } from "@/lib/tauri";
import { useWizard } from "@/lib/store";
import { StepLayout, ErrorBanner, InfoBanner } from "../components/Layout";
import { PrimaryButton, GhostButton, SecondaryButton } from "../components/Buttons";

type Stage = "token" | "code" | "waiting" | "paired";

export function TelegramStep() {
  const setStep = useWizard((s) => s.setStep);
  const botInfo = useWizard((s) => s.botInfo);
  const setBotInfo = useWizard((s) => s.setBotInfo);
  const pairingCode = useWizard((s) => s.pairingCode);
  const pairingDisplayName = useWizard((s) => s.pairingDisplayName);
  const setPairing = useWizard((s) => s.setPairing);

  const [token, setToken] = useState("");
  const [displayName, setDisplayName] = useState(pairingDisplayName);
  const [pairedChat, setPairedChat] = useState<AuthorizedChat | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const stage: Stage = pairedChat
    ? "paired"
    : pairingCode
      ? "waiting"
      : botInfo
        ? "code"
        : "token";

  // Poll list_authorized_chats once a code has been issued.
  const intervalRef = useRef<number | null>(null);
  useEffect(() => {
    if (stage !== "waiting") return;
    intervalRef.current = window.setInterval(async () => {
      try {
        const chats = await listAuthorizedChats();
        const last = chats[chats.length - 1];
        if (last) {
          setPairedChat(last);
          if (intervalRef.current !== null) {
            window.clearInterval(intervalRef.current);
            intervalRef.current = null;
          }
        }
      } catch {
        /* poll error — keep trying */
      }
    }, 1500);
    return () => {
      if (intervalRef.current !== null) {
        window.clearInterval(intervalRef.current);
        intervalRef.current = null;
      }
    };
  }, [stage]);

  async function submitToken() {
    setBusy(true);
    setError(null);
    try {
      const info: TelegramBotInfo = await saveTelegramToken(token);
      setBotInfo(info);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function issueCode() {
    if (!displayName.trim()) {
      setError("Pick a display name (yours or your household label)");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const code = await generatePairingCode(displayName.trim());
      setPairing(code, displayName.trim());
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function next() {
    await setSetupStep(4);
    setStep("locale");
  }

  return (
    <StepLayout
      stepIndex={4}
      totalSteps={8}
      title="Pair your Telegram bot"
      subtitle="You'll create your own bot in Telegram and pair it with this app. Mr. Moneypenny never sees your bot or its messages."
      footer={
        stage === "paired" ? (
          <PrimaryButton onClick={next}>Continue</PrimaryButton>
        ) : null
      }
    >
      <div className="space-y-6">
        {error ? <ErrorBanner>{error}</ErrorBanner> : null}

        {/* Stage 1: bot token */}
        <Section
          number="1"
          title="Create a personal Telegram bot."
          done={stage !== "token"}
        >
          <p className="text-sm text-graphite-300">
            BotFather is the official bot for creating bots — search for{" "}
            <code className="font-mono">@BotFather</code> in Telegram (it has a
            blue verified checkmark) or open it directly:{" "}
            <a
              href="https://t.me/BotFather"
              target="_blank"
              rel="noreferrer"
              className="text-forest-300 underline"
            >
              t.me/BotFather
            </a>
            .
          </p>
          <ol className="mt-3 ml-5 list-decimal space-y-2 text-sm text-graphite-300">
            <li>
              Tap <strong>Start</strong> in the BotFather chat (or send{" "}
              <code className="font-mono">/start</code> if you&apos;ve used it
              before).
            </li>
            <li>
              Send <code className="font-mono">/newbot</code>.
            </li>
            <li>
              When BotFather asks for a name, reply with whatever display name
              you like — e.g. <em>My Moneypenny</em>. This is what shows up
              in your chats list.
            </li>
            <li>
              When it asks for a username, reply with anything ending in{" "}
              <code className="font-mono">bot</code> — e.g.{" "}
              <code className="font-mono">wyatt_moneypenny_bot</code>.
              Usernames must be globally unique; if BotFather says it&apos;s
              taken, try another.
            </li>
            <li>
              BotFather will reply with a token that looks like{" "}
              <code className="font-mono">123456789:ABC-Def1234ghIkl-zyx57W2v1u</code>.
              Copy it and paste below.
            </li>
          </ol>
          <div className="mt-3 rounded-md border border-graphite-700 bg-graphite-800 px-3 py-2 text-xs text-graphite-400">
            <strong className="text-graphite-200">Privacy:</strong> the bot is
            yours. The token is stored encrypted under a machine-bound key on
            this computer — never in plaintext, never sent off your machine,
            never to the project. Even though anyone could find your
            bot&apos;s username, Mr. Moneypenny only responds to chats
            you&apos;ve explicitly paired in the next step.
          </div>
          {stage === "token" ? (
            <div className="mt-3 flex gap-2">
              <input
                type="password"
                placeholder="123456789:AA..."
                value={token}
                onChange={(e) => setToken(e.target.value)}
                className="flex-1 rounded-md border border-graphite-600 bg-graphite-800 px-3 py-2 font-mono text-sm text-graphite-50 focus:border-forest-400 focus:outline-none"
              />
              <PrimaryButton
                onClick={submitToken}
                disabled={!token.trim() || busy}
              >
                {busy ? "Verifying…" : "Save"}
              </PrimaryButton>
            </div>
          ) : botInfo ? (
            <InfoBanner>
              Connected to <code className="font-mono">@{botInfo.username ?? botInfo.first_name}</code>.
            </InfoBanner>
          ) : null}
        </Section>

        {/* Stage 2: display name + pairing code */}
        {stage !== "token" ? (
          <Section
            number="2"
            title="Pick a display name and generate a pairing code."
            done={stage === "waiting" || stage === "paired"}
          >
            {stage === "code" ? (
              <>
                <p className="text-sm text-graphite-300">
                  This is how the bot will refer to you in summaries (
                  <em>&ldquo;Wyatt spent $42 on dining&rdquo;</em>). Use your
                  first name or a household label. You can rename later in
                  Settings → Household.
                </p>
                <div className="mt-3 flex gap-2">
                  <input
                    type="text"
                    placeholder="Wyatt"
                    value={displayName}
                    onChange={(e) => setDisplayName(e.target.value)}
                    className="flex-1 rounded-md border border-graphite-600 bg-graphite-800 px-3 py-2 text-sm text-graphite-50 focus:border-forest-400 focus:outline-none"
                  />
                  <PrimaryButton onClick={issueCode} disabled={busy}>
                    {busy ? "…" : "Generate code"}
                  </PrimaryButton>
                </div>
              </>
            ) : pairingCode ? (
              <div className="rounded-md border border-graphite-600 bg-graphite-800 p-4">
                <div className="text-sm text-graphite-300">
                  Pairing code for <strong>{pairingDisplayName}</strong>:
                </div>
                <div className="mt-1 font-mono text-3xl tracking-widest text-forest-200">
                  {pairingCode}
                </div>
                <div className="mt-2 text-xs text-graphite-400">
                  Single-use, expires in 10 minutes.
                </div>
              </div>
            ) : null}
          </Section>
        ) : null}

        {/* Stage 3: waiting for /start */}
        {stage === "waiting" ? (
          <Section number="3" title="Send your bot the pairing message.">
            <ol className="ml-5 list-decimal space-y-1 text-sm text-graphite-300">
              <li>
                Open Telegram and find your bot{" "}
                <code className="font-mono">
                  @{botInfo?.username ?? botInfo?.first_name}
                </code>{" "}
                (search by username or scroll your chats — BotFather&apos;s
                reply linked to it).
              </li>
              <li>Tap <strong>Start</strong> at the bottom of the chat.</li>
              <li>Then send exactly:</li>
            </ol>
            <pre className="mt-2 rounded-md border border-graphite-600 bg-graphite-800 px-3 py-2 font-mono text-sm text-forest-200">
              /start {pairingCode}
            </pre>
            <p className="mt-3 flex items-center gap-2 text-xs text-graphite-400">
              <span className="inline-block h-2 w-2 animate-pulse rounded-full bg-forest-400"></span>
              Listening for the message — this page advances automatically.
            </p>
          </Section>
        ) : null}

        {/* Stage 4: paired */}
        {stage === "paired" && pairedChat ? (
          <InfoBanner>
            Paired as <strong>{pairedChat.display_name}</strong> ({pairedChat.role}).
          </InfoBanner>
        ) : null}

        {stage !== "token" ? (
          <div>
            <SecondaryButton
              onClick={() => {
                setBotInfo(null);
                setPairing(null, "");
                setPairedChat(null);
              }}
            >
              Start over
            </SecondaryButton>
          </div>
        ) : (
          <div>
            <GhostButton onClick={() => setStep("choose_llm")}>Back</GhostButton>
          </div>
        )}
      </div>
    </StepLayout>
  );
}

function Section({
  number,
  title,
  done,
  children,
}: {
  number: string;
  title: string;
  done?: boolean;
  children: React.ReactNode;
}) {
  return (
    <section className="rounded-lg border border-graphite-700 bg-graphite-900 p-4">
      <header className="flex items-center gap-3">
        <span
          className={`flex h-6 w-6 items-center justify-center rounded-full text-xs font-bold ${
            done ? "bg-forest-500 text-white" : "bg-graphite-700 text-graphite-200"
          }`}
        >
          {done ? "✓" : number}
        </span>
        <h3 className="text-sm font-medium text-graphite-50">{title}</h3>
      </header>
      <div className="mt-3">{children}</div>
    </section>
  );
}
