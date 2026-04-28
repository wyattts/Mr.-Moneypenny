/**
 * Wizard state.
 *
 * The Tauri backend is the source of truth — this store mirrors what the
 * user has done locally so we can navigate without a round-trip on every
 * step transition. On mount and after async operations, the wizard
 * refreshes from `get_setup_state`.
 */
import { create } from "zustand";

import type { SetupState, TelegramBotInfo } from "./tauri";

export type WizardStep =
  | "welcome"
  | "choose_llm"
  | "anthropic"
  | "ollama"
  | "telegram"
  | "locale"
  | "categories"
  | "sanity"
  | "done";

interface WizardState {
  step: WizardStep;
  /** Mirror of last `getSetupState()` result. */
  setup: SetupState | null;
  /** Display name we generated the active pairing code for. */
  pairingDisplayName: string;
  /** Active pairing code, if any. */
  pairingCode: string | null;
  /** Bot info from the last successful `saveTelegramToken`. */
  botInfo: TelegramBotInfo | null;

  setStep: (step: WizardStep) => void;
  setSetup: (setup: SetupState) => void;
  setPairing: (code: string | null, displayName: string) => void;
  setBotInfo: (info: TelegramBotInfo | null) => void;
}

export const useWizard = create<WizardState>((set) => ({
  step: "welcome",
  setup: null,
  pairingDisplayName: "",
  pairingCode: null,
  botInfo: null,

  setStep: (step) => set({ step }),
  setSetup: (setup) => set({ setup }),
  setPairing: (pairingCode, pairingDisplayName) =>
    set({ pairingCode, pairingDisplayName }),
  setBotInfo: (botInfo) => set({ botInfo }),
}));

/**
 * Map a saved-step number (the Rust side persists `setup_step`) to the
 * frontend step name. Used on first load so the wizard resumes where the
 * user left off.
 */
export function stepFromSavedNumber(setup: SetupState): WizardStep {
  if (setup.setup_complete) return "done";
  switch (setup.last_completed_step) {
    case 0:
      return "welcome";
    case 1:
      return "choose_llm";
    case 2:
      return setup.llm_provider === "ollama" ? "ollama" : "anthropic";
    case 3:
      return "telegram";
    case 4:
      return "locale";
    case 5:
      return "categories";
    case 6:
      return "sanity";
    default:
      return "done";
  }
}
