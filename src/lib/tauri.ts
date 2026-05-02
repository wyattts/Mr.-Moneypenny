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
  kind: "fixed" | "variable" | "investing";
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
  | { kind: "custom"; from: string; to: string } // ISO yyyy-mm-dd
  | { kind: "month"; year: number; month: number }; // month is 1-12

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
  total_budget_cents: number;
  total_remaining_cents: number;
  variable_budget_cents: number;
  fixed_budget_cents: number;
}

export interface CategoryTotal {
  category_id: number;
  name: string;
  kind: "fixed" | "variable" | "investing";
  total_cents: number;
  monthly_target_cents: number | null;
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
  category_kind: "fixed" | "variable" | "investing" | null;
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
  kind: "fixed" | "variable" | "investing";
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

// ---------------------------------------------------------------------
// Auto-update.
// ---------------------------------------------------------------------

export interface UpdateInfo {
  available: boolean;
  version: string | null;
  current_version: string;
  notes: string | null;
}

export const checkForUpdate = (): Promise<UpdateInfo> => invoke("check_for_update");
export const installUpdate = (): Promise<void> => invoke("install_update");
export const getCheckUpdatesOnLaunch = (): Promise<boolean> =>
  invoke("get_check_updates_on_launch");
export const setCheckUpdatesOnLaunch = (enabled: boolean): Promise<void> =>
  invoke("set_check_updates_on_launch", { enabled });

export const getWeeklySummaryEnabled = (): Promise<boolean> =>
  invoke("get_weekly_summary_enabled");
export const setWeeklySummaryEnabled = (enabled: boolean): Promise<void> =>
  invoke("set_weekly_summary_enabled", { enabled });
export const getBudgetAlertsEnabled = (): Promise<boolean> =>
  invoke("get_budget_alerts_enabled");
export const setBudgetAlertsEnabled = (enabled: boolean): Promise<void> =>
  invoke("set_budget_alerts_enabled", { enabled });

export interface ModelSummary {
  model: string;
  provider: string;
  calls: number;
  input_tokens: number;
  output_tokens: number;
  cost_micros: number;
}

export interface UsageSummary {
  today_micros: number;
  this_month_micros: number;
  lifetime_micros: number;
  today_calls: number;
  this_month_calls: number;
  lifetime_calls: number;
  by_model: ModelSummary[];
}

export const getLlmUsageSummary = (): Promise<UsageSummary> =>
  invoke("get_llm_usage_summary");

// --- Forecast / stats (v0.3.0) ---

export interface DescriptiveStats {
  n: number;
  min_cents: number;
  max_cents: number;
  mean_cents: number;
  median_cents: number;
  p10_cents: number;
  p90_cents: number;
  stddev_cents: number;
}

export interface Histogram {
  bucket_count: number;
  min_cents: number;
  max_cents: number;
  bucket_width_cents: number;
  counts: number[];
}

export interface CategoryStatsResponse {
  category_id: number;
  months_back: number;
  stats: DescriptiveStats | null;
  histogram: Histogram | null;
  monthly_totals_cents: number[];
}

export const getCategoryStats = (
  categoryId: number,
  monthsBack: number,
): Promise<CategoryStatsResponse> =>
  invoke("get_category_stats", { categoryId, monthsBack });

export interface ProjectionPoint {
  month: number;
  nominal_cents: number;
  real_cents: number;
}

export interface InvestmentProjection {
  trajectory: ProjectionPoint[];
  final_nominal_cents: number;
  final_real_cents: number;
  total_contributed_cents: number;
  total_growth_cents: number;
}

export interface ProjectInvestmentInput {
  starting_balance_cents: number;
  monthly_contribution_cents: number;
  annual_return_pct: number;
  annual_inflation_pct: number;
  horizon_years: number;
  trajectory_points: number;
}

export const projectInvestment = (
  input: ProjectInvestmentInput,
): Promise<InvestmentProjection> => invoke("project_investment", { input });

export interface GoalSeekInput {
  target_cents: number;
  starting_balance_cents: number;
  annual_return_pct: number;
  horizon_years: number;
}

export interface GoalSeekResult {
  required_monthly_cents: number;
  already_on_track: boolean;
}

export const solveGoalSeek = (input: GoalSeekInput): Promise<GoalSeekResult> =>
  invoke("solve_goal_seek", { input });

export interface ScenarioCut {
  category_id: number;
  pct_change: number;
}

export interface ScenarioResult {
  original_variable_budget_cents: number;
  adjusted_variable_budget_cents: number;
  savings_per_year_cents: number;
}

export const runScenario = (cuts: ScenarioCut[]): Promise<ScenarioResult> =>
  invoke("run_scenario", { input: { cuts } });

export interface InvestmentSummary {
  category_id: number;
  name: string;
  starting_balance_cents: number | null;
  balance_as_of: string | null;
  avg_monthly_contribution_cents: number | null;
  last_12mo_contribution_cents: number;
}

export const listInvestmentCategories = (): Promise<InvestmentSummary[]> =>
  invoke("list_investment_categories");

export interface SetStartingBalanceInput {
  category_id: number;
  starting_balance_cents: number | null;
  balance_as_of: string | null;
}

export const setStartingBalance = (
  input: SetStartingBalanceInput,
): Promise<void> => invoke("set_starting_balance", { input });

// -------------------------------------------------------------------
// Forecast wave 2 (v0.3.3): Monte Carlo bands, simulator, category analyzer.
// -------------------------------------------------------------------

export interface MonteCarloInput {
  starting_balance_cents: number;
  monthly_contribution_cents: number;
  annual_return_pct: number;
  annual_volatility_pct: number;
  horizon_years: number;
  n_paths: number;
  time_points: number;
  seed?: number | null;
}

export interface MonthBand {
  month: number;
  p5: number;
  p10: number;
  p25: number;
  p50: number;
  p75: number;
  p90: number;
  p95: number;
}

export interface PathBands {
  points: MonthBand[];
  final_p5_cents: number;
  final_p10_cents: number;
  final_p50_cents: number;
  final_p90_cents: number;
  final_p95_cents: number;
  n_paths: number;
}

export const monteCarloInvestment = (
  input: MonteCarloInput,
): Promise<PathBands> => invoke("monte_carlo_investment", { input });

export type TargetMode = "todays_dollars" | "nominal_future";

export interface SimulatorCommonInputs {
  target_cents: number;
  horizon_years: number;
  starting_balance_cents: number;
  annual_return_pct: number;
  annual_volatility_pct: number;
  annual_inflation_pct: number;
  target_mode: TargetMode;
  n_paths: number;
  seed?: number | null;
}

export interface RequiredContributionInput extends SimulatorCommonInputs {
  confidence: number;
}

export interface RequiredContributionResult {
  required_monthly_cents: number;
  realized_probability: number;
  effective_target_cents: number;
  final_p10_cents: number;
  final_p50_cents: number;
  final_p90_cents: number;
  iterations: number;
}

export const simulatorSolveRequired = (
  input: RequiredContributionInput,
): Promise<RequiredContributionResult> =>
  invoke("simulator_solve_required_contribution", { input });

export interface ProbabilityInput extends SimulatorCommonInputs {
  monthly_contribution_cents: number;
}

export interface ProbabilityResult {
  probability: number;
  effective_target_cents: number;
  final_p10_cents: number;
  final_p50_cents: number;
  final_p90_cents: number;
}

export const simulatorComputeProbability = (
  input: ProbabilityInput,
): Promise<ProbabilityResult> =>
  invoke("simulator_compute_probability", { input });

export interface HeatmapInput extends SimulatorCommonInputs {
  contribution_min_cents: number;
  contribution_max_cents: number;
  horizon_min_years: number;
  horizon_max_years: number;
}

export interface HeatmapCell {
  contribution_cents: number;
  horizon_years: number;
  probability: number;
}

export interface HeatmapResult {
  cells: HeatmapCell[];
  effective_target_cents_at_each_horizon: number[];
}

export const simulatorHeatmap = (input: HeatmapInput): Promise<HeatmapResult> =>
  invoke("simulator_heatmap", { input });

export type AnalysisWindow =
  | "two_weeks"
  | "month"
  | "quarter"
  | "half_year"
  | "year";

export interface PerTransactionStats {
  n: number;
  mean_cents: number;
  median_cents: number;
  stddev_cents: number;
  min_cents: number;
  max_cents: number;
}

export interface PerBucketStats {
  n_buckets: number;
  mean_cents: number;
  median_cents: number;
  stddev_cents: number;
  min_cents: number;
  max_cents: number;
}

export interface BucketPoint {
  bucket_index: number;
  label: string;
  total_cents: number;
}

export interface RefundSummary {
  count: number;
  total_cents: number;
  net_spent_cents: number;
}

export interface CategoryAnalysis {
  window: AnalysisWindow;
  bucket_label: string;
  buckets: BucketPoint[];
  per_transaction: PerTransactionStats | null;
  per_bucket: PerBucketStats | null;
  refunds: RefundSummary;
  slope_cents_per_month_per_year: number;
  r_squared: number;
  direction: "rising" | "falling" | "flat";
  headline: string;
}

export const analyzeCategory = (
  categoryId: number,
  window: AnalysisWindow,
): Promise<CategoryAnalysis> =>
  invoke("analyze_category", { categoryId, window });

// -------------------------------------------------------------------
// CSV import (v0.3.2)
// -------------------------------------------------------------------

export interface ColumnMapping {
  date_col: number;
  amount_col: number;
  merchant_col: number;
  description_col: number | null;
  category_col: number | null;
  date_format: string;
  neg_means_refund: boolean;
  skip_rows: number;
}

export interface CsvImportProfile {
  id: number;
  name: string;
  header_signature: string | null;
  mapping: ColumnMapping;
  created_at: string;
  last_used_at: string | null;
}

export interface PreviewResult {
  headers: string[];
  sample_rows: string[][];
  header_signature: string;
  total_rows: number;
}

export interface CsvPreview {
  preview: PreviewResult;
  suggested_profile: CsvImportProfile | null;
  profiles: CsvImportProfile[];
}

export const csvImportPreview = (content: string): Promise<CsvPreview> =>
  invoke("csv_import_preview", { content });

export const csvImportSaveProfile = (input: {
  name: string;
  header_signature: string | null;
  mapping: ColumnMapping;
}): Promise<number> => invoke("csv_import_save_profile", { input });

export interface ParsedRow {
  source_row_index: number;
  occurred_at: string;
  amount_cents: number;
  merchant: string;
  description: string | null;
  raw_category: string | null;
  is_refund: boolean;
}

export const csvImportParse = (input: {
  content: string;
  mapping: ColumnMapping;
}): Promise<ParsedRow[]> => invoke("csv_import_parse", { input });

export interface Decision {
  row_index: number;
  source: "rule" | "history" | "unmatched";
  category_id: number | null;
  override_is_refund: boolean | null;
}

export interface DuplicateMatch {
  row_index: number;
  kind: "csv" | "db";
  existing_expense_id: number | null;
  reason: string;
}

export interface CategorizeAndDedupeResult {
  decisions: Decision[];
  duplicates: DuplicateMatch[];
}

export const csvImportCategorizeAndDedupe = (
  rows: ParsedRow[],
): Promise<CategorizeAndDedupeResult> =>
  invoke("csv_import_categorize_and_dedupe", { input: { rows } });

export interface AiSuggestResponse {
  suggestions: Record<string, number>;
  cost_micros: number;
}

export const csvImportAiSuggest = (
  merchants: string[],
): Promise<AiSuggestResponse> =>
  invoke("csv_import_ai_suggest", { input: { merchants } });

export interface CommittableRow {
  occurred_at: string;
  amount_cents: number;
  category_id: number | null;
  merchant: string;
  description: string | null;
  is_refund: boolean;
}

export interface RuleToSave {
  pattern: string;
  category_id: number;
  default_is_refund: boolean;
}

export interface CommitInput {
  rows: CommittableRow[];
  rules_to_save: RuleToSave[];
  profile_id: number | null;
}

export interface CommitResult {
  inserted: number;
  rules_added: number;
}

export const csvImportCommit = (input: CommitInput): Promise<CommitResult> =>
  invoke("csv_import_commit", { input });

export const listCsvImportProfiles = (): Promise<CsvImportProfile[]> =>
  invoke("list_csv_import_profiles");

export const deleteCsvImportProfile = (id: number): Promise<void> =>
  invoke("delete_csv_import_profile", { id });

export interface MerchantRule {
  id: number;
  pattern: string;
  category_id: number;
  default_is_refund: boolean;
  priority: number;
  created_at: string;
}

export const listMerchantRules = (): Promise<MerchantRule[]> =>
  invoke("list_merchant_rules");

export const deleteMerchantRule = (id: number): Promise<void> =>
  invoke("delete_merchant_rule", { id });
