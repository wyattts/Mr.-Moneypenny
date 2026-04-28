import { useEffect, useState } from "react";

import {
  generatePairingCode,
  listAuthorizedChats,
  removeHouseholdMember,
} from "@/lib/tauri";
import type { AuthorizedChat } from "@/lib/tauri";
import { ViewHeader } from "./ViewHeader";
import { ErrorBanner, InfoBanner } from "@/wizard/components/Layout";
import { GhostButton, PrimaryButton, SecondaryButton } from "@/wizard/components/Buttons";

export function Household() {
  const [members, setMembers] = useState<AuthorizedChat[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const [inviting, setInviting] = useState(false);
  const [inviteName, setInviteName] = useState("");
  const [inviteCode, setInviteCode] = useState<string | null>(null);

  useEffect(() => {
    void load();
    const t = window.setInterval(() => void load(), 2000);
    return () => window.clearInterval(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function load() {
    try {
      const list = await listAuthorizedChats();
      setMembers(list);
      // If we're awaiting a pair and a new chat appeared, clear the code.
      if (inviteCode && list.length > 1) {
        setInviteCode(null);
        setInviteName("");
        setInfo("New member paired.");
      }
    } catch (e) {
      setError(String(e));
    }
  }

  async function startInvite() {
    if (!inviteName.trim()) {
      setError("Enter a display name first.");
      return;
    }
    setError(null);
    try {
      const code = await generatePairingCode(inviteName.trim());
      setInviteCode(code);
    } catch (e) {
      setError(String(e));
    }
  }

  async function remove(chat: AuthorizedChat) {
    if (chat.role === "owner") {
      setError("Owner cannot be removed; transfer ownership first.");
      return;
    }
    if (!window.confirm(`Remove ${chat.display_name}? They'll lose access to the bot.`)) {
      return;
    }
    try {
      await removeHouseholdMember(chat.chat_id);
      void load();
    } catch (e) {
      setError(String(e));
    }
  }

  const owner = members.find((m) => m.role === "owner");

  return (
    <div>
      <ViewHeader
        title="Household"
        subtitle="Authorized Telegram chats. Only the owner can invite or remove members."
        actions={
          owner && !inviting ? (
            <PrimaryButton onClick={() => setInviting(true)}>Invite member</PrimaryButton>
          ) : null
        }
      />
      <div className="mx-auto max-w-3xl space-y-6 px-8 py-8">
        {error ? <ErrorBanner>{error}</ErrorBanner> : null}
        {info ? <InfoBanner>{info}</InfoBanner> : null}

        {inviting ? (
          <section className="rounded-lg border border-graphite-700 bg-graphite-900 p-5">
            <h2 className="text-base font-semibold text-graphite-50">Invite a member</h2>
            {inviteCode ? (
              <div className="mt-3 space-y-2">
                <p className="text-sm text-graphite-300">
                  Share this with the person you&apos;re inviting. Have them open your bot in
                  Telegram and send <code className="font-mono">/start {inviteCode}</code>.
                </p>
                <div className="rounded-md border border-graphite-600 bg-graphite-800 p-4">
                  <div className="text-xs text-graphite-300">Pairing code (10-min TTL):</div>
                  <div className="mt-1 font-mono text-3xl tracking-widest text-forest-200">
                    {inviteCode}
                  </div>
                </div>
                <p className="text-xs text-graphite-400">Listening for the start command…</p>
                <GhostButton
                  onClick={() => {
                    setInviting(false);
                    setInviteCode(null);
                    setInviteName("");
                  }}
                >
                  Cancel
                </GhostButton>
              </div>
            ) : (
              <div className="mt-3 flex items-end gap-2">
                <label className="flex flex-1 flex-col gap-1">
                  <span className="text-xs text-graphite-300">Display name for the new member</span>
                  <input
                    type="text"
                    value={inviteName}
                    onChange={(e) => setInviteName(e.target.value)}
                    placeholder="Spouse"
                    className="rounded-md border border-graphite-600 bg-graphite-800 px-3 py-2 text-sm text-graphite-50"
                  />
                </label>
                <PrimaryButton onClick={startInvite}>Generate code</PrimaryButton>
                <GhostButton onClick={() => setInviting(false)}>Cancel</GhostButton>
              </div>
            )}
          </section>
        ) : null}

        <section className="rounded-lg border border-graphite-700 bg-graphite-900">
          <ul className="divide-y divide-graphite-700">
            {members.length === 0 ? (
              <li className="px-4 py-6 text-sm text-graphite-400">
                No authorized chats yet.
              </li>
            ) : (
              members.map((m) => (
                <li
                  key={m.chat_id}
                  className="flex items-center justify-between gap-3 px-4 py-3"
                >
                  <div>
                    <div className="text-sm text-graphite-50">{m.display_name}</div>
                    <div className="text-xs text-graphite-400">
                      chat_id <code className="font-mono">{m.chat_id}</code>
                    </div>
                  </div>
                  <div className="flex items-center gap-2">
                    <span
                      className={`rounded px-2 py-0.5 text-xs ${
                        m.role === "owner"
                          ? "bg-forest-500/30 text-forest-200"
                          : "bg-graphite-700 text-graphite-200"
                      }`}
                    >
                      {m.role}
                    </span>
                    {m.role !== "owner" ? (
                      <SecondaryButton onClick={() => remove(m)}>Remove</SecondaryButton>
                    ) : null}
                  </div>
                </li>
              ))
            )}
          </ul>
        </section>
      </div>
    </div>
  );
}
