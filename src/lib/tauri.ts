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

export const clearAuthorizedChats = (): Promise<number> =>
  invoke("clear_authorized_chats");

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

// ---------------------------------------------------------------------
// Phase 4b: dashboard, ledger, categories CRUD, budgets, household,
// background-mode + autostart toggles.
// ---------------------------------------------------------------------

export type RangeKind =
  | "this_week"
  | "this_month"
  | "this_quarter"
  | "this_year"
  | "ytd";

export type RangeArg =
  | { kind: RangeKind }
  | { kind: "custom"; from: string; to: string }; // ISO yyyy-mm-dd

export interface PeriodSnapshot {
  progress: number;
  day_of_month: number;
  days_in_period: number;
  days_remaining: number;
  fixed_budget_cents: number;
  fixed_actual_cents: number;
  fixed_pending_cents: number;
  variable_budget_cents: number;
  variable_spent_cents: number;
  variable_remaining_cents: number;
  variable_pace_expected_cents: number;
  on_pace: boolean;
  daily_variable_allowance_cents: number;
}

export interface KpiCard {
  variable_remaining_cents: number;
  daily_variable_allowance_cents: number;
  total_spent_cents: number;
  days_remaining: number;
  on_pace: boolean;
}

export interface CategoryTotal {
  category_id: number;
  name: string;
  kind: "fixed" | "variable";
  total_cents: number;
}

export interface DailyTrendPoint {
  date: string; // YYYY-MM-DD
  fixed_cents: number;
  variable_cents: number;
}

export interface FixedVariableBreakdown {
  fixed_committed_cents: number;
  variable_spent_cents: number;
  variable_remaining_cents: number;
}

export interface MemberSpend {
  chat_id: number;
  display_name: string;
  total_cents: number;
}

export interface ExpenseRow {
  id: number;
  amount_cents: number;
  currency: string;
  category_id: number | null;
  description: string | null;
  occurred_at: string;
  created_at: string;
  source: "telegram" | "manual";
  raw_message: string | null;
  llm_confidence: number | null;
  logged_by_chat_id: number | null;
}

export interface OverBudgetCategory {
  category_id: number;
  name: string;
  spent_cents: number;
  target_cents: number;
  overage_cents: number;
}

export interface UpcomingFixed {
  category_id: number;
  name: string;
  recurrence_day_of_month: number;
  expected_amount_cents: number | null;
}

export interface MoMComparison {
  variable_spent_this_period_cents: number;
  variable_spent_same_point_last_month_cents: number;
  delta_cents: number;
  delta_pct: number | null;
}

export interface DashboardSnapshot {
  range: RangeArg;
  start: string;
  end: string;
  period: PeriodSnapshot | null;
  kpi: KpiCard;
  category_totals: CategoryTotal[];
  daily_trend: DailyTrendPoint[];
  fixed_vs_variable: FixedVariableBreakdown;
  member_spend: MemberSpend[];
  top_expenses: ExpenseRow[];
  over_budget: OverBudgetCategory[];
  upcoming_fixed: UpcomingFixed[];
  mom_comparison: MoMComparison | null;
}

export const getDashboard = (range: RangeArg): Promise<DashboardSnapshot> =>
  invoke("get_dashboard", { range });

export interface LedgerRow {
  id: number;
  amount_cents: number;
  currency: string;
  category_id: number | null;
  category_name: string | null;
  category_kind: "fixed" | "variable" | null;
  description: string | null;
  occurred_at: string;
  source: "telegram" | "manual";
  logged_by_chat_id: number | null;
  logged_by_name: string | null;
}

export interface ExpenseFilters {
  category_id?: number | undefined;
  start_date?: string | undefined; // YYYY-MM-DD
  end_date?: string | undefined;
  search?: string | undefined;
  limit?: number | undefined;
  offset?: number | undefined;
}

export const listExpenses = (filters: ExpenseFilters): Promise<LedgerRow[]> =>
  invoke("list_expenses", { filters });

export const deleteExpense = (id: number): Promise<boolean> =>
  invoke("delete_expense", { id });

export interface NewCategoryArg {
  name: string;
  kind: "fixed" | "variable";
  monthly_target_cents?: number | undefined;
  is_recurring?: boolean | undefined;
  recurrence_day_of_month?: number | undefined;
}

export const createCategory = (arg: NewCategoryArg): Promise<number> =>
  invoke("create_category", { arg });

export const deleteCategory = (id: number): Promise<boolean> =>
  invoke("delete_category", { id });

export const removeHouseholdMember = (chatId: number): Promise<boolean> =>
  invoke("remove_household_member", { chatId });

export const getRunInBackground = (): Promise<boolean> => invoke("get_run_in_background");
export const setRunInBackground = (enabled: boolean): Promise<void> =>
  invoke("set_run_in_background", { enabled });
export const getAutostart = (): Promise<boolean> => invoke("get_autostart");
export const setAutostart = (enabled: boolean): Promise<void> =>
  invoke("set_autostart", { enabled });
