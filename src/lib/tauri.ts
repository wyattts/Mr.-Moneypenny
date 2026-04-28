/**
 * Typed wrappers around Tauri IPC commands.
 *
 * Keep this file thin — declare the shape of each command and forward to
 * `invoke`. Real logic lives in `src-tauri/src/commands.rs`.
 */
import { invoke } from "@tauri-apps/api/core";

export interface SetupState {
  setup_complete: boolean;
  last_completed_step: number;
  llm_provider: string | null;
  anthropic_key_set: boolean;
  telegram_token_set: boolean;
  authorized_chat_count: number;
  default_currency: string;
  locale: string | null;
  ollama_endpoint: string | null;
  ollama_model: string | null;
}

export interface TelegramBotInfo {
  id: number;
  username: string | null;
  first_name: string;
}

export interface AuthorizedChat {
  chat_id: number;
  display_name: string;
  role: "owner" | "member";
}

export interface CategoryView {
  id: number;
  name: string;
  kind: "fixed" | "variable";
  monthly_target_cents: number | null;
  is_recurring: boolean;
  recurrence_day_of_month: number | null;
  is_active: boolean;
  is_seed: boolean;
}

export const ping = (): Promise<string> => invoke("ping");

export const getSetupState = (): Promise<SetupState> => invoke("get_setup_state");

export const setSetupStep = (step: number): Promise<void> =>
  invoke("set_setup_step", { step });

export const saveLlmProvider = (provider: "anthropic" | "ollama"): Promise<void> =>
  invoke("save_llm_provider", { provider });

export const saveAnthropicKey = (apiKey: string): Promise<void> =>
  invoke("save_anthropic_key", { apiKey });

export const testAnthropic = (): Promise<string> => invoke("test_anthropic");

export const saveOllamaConfig = (endpoint: string, model: string): Promise<void> =>
  invoke("save_ollama_config", { endpoint, model });

export const listOllamaModels = (endpoint: string): Promise<string[]> =>
  invoke("list_ollama_models", { endpoint });

export const testOllama = (): Promise<string> => invoke("test_ollama");

export const saveTelegramToken = (token: string): Promise<TelegramBotInfo> =>
  invoke("save_telegram_token", { token });

export const generatePairingCode = (displayName: string): Promise<string> =>
  invoke("generate_pairing_code", { displayName });

export const listAuthorizedChats = (): Promise<AuthorizedChat[]> =>
  invoke("list_authorized_chats");

export const saveCurrencyLocale = (currency: string, locale: string): Promise<void> =>
  invoke("save_currency_locale", { currency, locale });

export const listCategories = (includeInactive: boolean): Promise<CategoryView[]> =>
  invoke("list_categories", { includeInactive });

export const setCategoryTarget = (
  id: number,
  monthlyTargetCents: number | null,
): Promise<void> =>
  invoke("set_category_target", { id, monthlyTargetCents });

export const setCategoryActive = (id: number, isActive: boolean): Promise<void> =>
  invoke("set_category_active", { id, isActive });

export const finalizeSetup = (): Promise<void> => invoke("finalize_setup");
