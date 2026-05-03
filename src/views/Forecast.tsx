import { useEffect, useMemo, useState } from "react";
import {
  Area,
  CartesianGrid,
  ComposedChart,
  Line,
  LineChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";

import {
  analyzeCategory,
  debtGoalSeek,
  debtSimulatePortfolio,
  debtSimulateSchedule,
  listCategories,
  listInvestmentCategories,
  runScenario,
  simulatorComputeProbability,
  simulatorHeatmap,
  simulatorSolveRequired,
} from "@/lib/tauri";
import type {
  AnalysisWindow,
  CategoryAnalysis,
  CategoryView,
  CompoundingFrequency,
  DebtGoalSeekResult,
  DebtInput,
  DebtScheduleResult,
  HeatmapResult,
  InvestmentSummary,
  LumpSum,
  PortfolioResult,
  PortfolioStrategy,
  ProbabilityResult,
  RequiredContributionResult,
  ScenarioResult,
  TargetMode,
  TrajectoryPoint,
} from "@/lib/tauri";
import { ErrorBanner } from "@/wizard/components/Layout";
import { formatMoney } from "@/lib/format";
import { ViewHeader } from "./ViewHeader";

// Annualized volatility tied to the user's chosen return preset. See
// `src-tauri/src/insights/monte_carlo.rs` for the rationale.
function volatilityForReturn(returnPct: number): number {
  if (returnPct <= 5) return 5;
  if (returnPct <= 8) return 10;
  return 15;
}

// Compact $ axis label that scales by magnitude — keeps long
// projection numbers from bleeding off the chart's left edge.
function formatYAxisDollars(v: number): string {
  const abs = Math.abs(v);
  if (abs >= 1_000_000) return `$${(v / 1_000_000).toFixed(1)}M`;
  if (abs >= 1_000) return `$${(v / 1_000).toFixed(0)}k`;
  return `$${v.toFixed(0)}`;
}

interface ProjectionDatum {
  year: number;
  Nominal: number;
  Real: number;
  Contributions: number;
  pLo: number;
  pHi: number;
}

// Custom Recharts tooltip content. We read pLo / pHi straight from
// the data row instead of relying on transparent-Line registrations
// (which Recharts will sometimes drop from the active payload, leaving
// the band edges invisible on hover — the v0.3.5 bug this fixes).
function ProjectionTooltipContent(props: {
  active: boolean | undefined;
  payload: { payload?: ProjectionDatum }[] | undefined;
  label: number | undefined;
  loPctLabel: string;
  hiPctLabel: string;
  showContributions: boolean;
  showReal: boolean;
}) {
  const { active, payload, label, loPctLabel, hiPctLabel, showContributions, showReal } = props;
  if (!active || !payload || payload.length === 0) return null;
  const d = payload[0]?.payload;
  if (!d) return null;
  return (
    <div
      className="rounded-md border border-graphite-700 bg-graphite-900 p-2 text-xs"
      style={{ minWidth: 180 }}
    >
      <div className="mb-1 text-graphite-400">Year {label}</div>
      <Row label="Nominal" color="#34d399" value={d.Nominal} />
      {showReal && <Row label="Real (today's $)" color="#facc15" value={d.Real} />}
      <Row label={`Lower (P${loPctLabel})`} color="#60a5fa" value={d.pLo} />
      <Row label={`Upper (P${hiPctLabel})`} color="#60a5fa" value={d.pHi} />
      {showContributions && (
        <Row label="Contributions" color="#94a3b8" value={d.Contributions} />
      )}
    </div>
  );
}

function Row({ label, color, value }: { label: string; color: string; value: number }) {
  return (
    <div className="flex items-baseline justify-between gap-3">
      <span className="flex items-center gap-1.5">
        <span
          className="inline-block h-2 w-2 rounded-full"
          style={{ background: color }}
        />
        <span className="text-graphite-300">{label}</span>
      </span>
      <span className="font-mono tabular-nums text-graphite-50">
        {formatMoney(Math.round(value * 100))}
      </span>
    </div>
  );
}

// Hand-off payload from the Debt Manager into the Simulator. Carries
// the freed-up monthly payment and the remaining horizon after payoff,
// so the user can see "if I redirect this into investing, where do I
// end up?" The `bump` counter lets the effect re-fire even when the
// numeric values are identical to the prior hand-off.
export interface SimulatorPrefill {
  monthly_contribution_cents: number;
  starting_balance_cents: number;
  horizon_years: number;
  bump: number;
}

export function Forecast() {
  const [error, setError] = useState<string | null>(null);
  const [simulatorPrefill, setSimulatorPrefill] =
    useState<SimulatorPrefill | null>(null);
  // Surface the Simulator's current return assumption back to the Debt
  // Manager so it can show the APR-vs-return context line. Lifted to
  // Forecast so both children can read/write it.
  const [simulatorReturnPct, setSimulatorReturnPct] = useState<number>(7);
  return (
    <div>
      <ViewHeader
        title="Forecast"
        subtitle="Look-forward tools — investment simulator, debt manager, scenario sliders, and a category analyzer."
      />
      <div className="space-y-6 px-8 py-6">
        {error && <ErrorBanner>{error}</ErrorBanner>}
        <Simulator
          onError={setError}
          prefill={simulatorPrefill}
          onReturnPctChange={setSimulatorReturnPct}
        />
        <DebtManager
          onError={setError}
          onSendToSimulator={setSimulatorPrefill}
          simulatorReturnPct={simulatorReturnPct}
        />
        <CategoryAnalyzer onError={setError} />
        <ScenarioTool onError={setError} />
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------
// Forecast Simulator — bidirectional Monte Carlo with embedded chart.
// ---------------------------------------------------------------------

type SimMode = "required" | "probability";

const RETURN_PRESETS = [
  { label: "Conservative (HYSA / bonds)", rate: 4 },
  { label: "Balanced (60/40)", rate: 7 },
  { label: "Stock-heavy (S&P 500 historical)", rate: 10 },
];

function Simulator({
  onError,
  prefill,
  onReturnPctChange,
}: {
  onError: (m: string) => void;
  prefill?: SimulatorPrefill | null;
  onReturnPctChange?: (pct: number) => void;
}) {
  const [mode, setMode] = useState<SimMode>("required");
  const [targetDollars, setTargetDollars] = useState("1000000.00");
  const [horizon, setHorizon] = useState(30);
  const [returnPct, setReturnPct] = useState(7);
  const [inflationPct, setInflationPct] = useState(2.5);
  const [startingDollars, setStartingDollars] = useState("0.00");
  const [confidence, setConfidence] = useState(0.8);
  const [contributionDollars, setContributionDollars] = useState("1000.00");
  const [targetMode, setTargetMode] = useState<TargetMode>("todays_dollars");
  const [advanced, setAdvanced] = useState(false);
  const [sigmaOverride, setSigmaOverride] = useState<number | null>(null);

  // Pre-fill from saved investing-kind categories.
  const [accounts, setAccounts] = useState<InvestmentSummary[]>([]);
  const [accountId, setAccountId] = useState<number | "all" | null>(null);

  const [required, setRequired] = useState<RequiredContributionResult | null>(null);
  const [probability, setProbability] = useState<ProbabilityResult | null>(null);
  const [heatmap, setHeatmap] = useState<HeatmapResult | null>(null);

  const [showContributions, setShowContributions] = useState(false);

  const sigma = sigmaOverride ?? volatilityForReturn(returnPct);
  const targetCents = Math.round((parseFloat(targetDollars) || 0) * 100);
  const startingCents = Math.round((parseFloat(startingDollars) || 0) * 100);
  const contribCents = Math.round((parseFloat(contributionDollars) || 0) * 100);

  // Load saved investing accounts once for the prefill dropdown.
  useEffect(() => {
    void (async () => {
      try {
        const list = await listInvestmentCategories();
        setAccounts(list);
      } catch (e) {
        onError(String(e));
      }
    })();
  }, [onError]);

  // Notify the parent whenever the return assumption changes so the
  // Debt Manager can frame APR vs. return.
  useEffect(() => {
    onReturnPctChange?.(returnPct);
  }, [returnPct, onReturnPctChange]);

  // External prefill — fired by the Debt Manager's "send freed payment
  // to Simulator" button. Switches to probability mode (we have a
  // fixed payment, the user wants to see odds), and stamps in the
  // amount + starting balance + horizon. The `bump` counter on the
  // payload guarantees the effect re-runs even when values are
  // identical to the prior hand-off.
  useEffect(() => {
    if (!prefill) return;
    setContributionDollars(
      (prefill.monthly_contribution_cents / 100).toFixed(2),
    );
    setStartingDollars((prefill.starting_balance_cents / 100).toFixed(2));
    setHorizon(prefill.horizon_years);
    setMode("probability");
    // Drop any account-prefill selection so the explicit hand-off
    // sticks instead of getting overwritten next render.
    setAccountId(null);
  }, [prefill]);

  // Apply prefill when the user picks an account. "all" sums everything.
  useEffect(() => {
    if (accountId === null) return;
    if (accountId === "all") {
      const startTotal = accounts.reduce(
        (acc, a) => acc + (a.starting_balance_cents ?? 0),
        0,
      );
      const monthlyTotal = accounts.reduce(
        (acc, a) => acc + (a.avg_monthly_contribution_cents ?? 0),
        0,
      );
      setStartingDollars((startTotal / 100).toFixed(2));
      if (monthlyTotal > 0) {
        setContributionDollars((monthlyTotal / 100).toFixed(2));
      }
      return;
    }
    const a = accounts.find((x) => x.category_id === accountId);
    if (!a) return;
    setStartingDollars(((a.starting_balance_cents ?? 0) / 100).toFixed(2));
    if (a.avg_monthly_contribution_cents) {
      setContributionDollars(
        (a.avg_monthly_contribution_cents / 100).toFixed(2),
      );
    }
  }, [accountId, accounts]);

  const common = useMemo(
    () => ({
      target_cents: targetCents,
      horizon_years: horizon,
      starting_balance_cents: startingCents,
      annual_return_pct: returnPct,
      annual_volatility_pct: sigma,
      annual_inflation_pct: inflationPct,
      target_mode: targetMode,
      n_paths: 1000,
      seed: null as number | null,
    }),
    [targetCents, horizon, startingCents, returnPct, sigma, inflationPct, targetMode],
  );

  // Recompute the active solver + heatmap when inputs change.
  useEffect(() => {
    let cancelled = false;
    const run = async () => {
      try {
        if (mode === "required") {
          const r = await simulatorSolveRequired({ ...common, confidence });
          if (cancelled) return;
          setRequired(r);
          setProbability(null);
          const x_max = Math.max(r.required_monthly_cents * 2, 100_000);
          const h = await simulatorHeatmap({
            ...common,
            contribution_min_cents: 0,
            contribution_max_cents: x_max,
            horizon_min_years: 1,
            horizon_max_years: Math.min(50, Math.max(2, horizon * 2)),
          });
          if (!cancelled) setHeatmap(h);
        } else {
          const p = await simulatorComputeProbability({
            ...common,
            monthly_contribution_cents: contribCents,
          });
          if (cancelled) return;
          setProbability(p);
          setRequired(null);
          const x_max = Math.max(contribCents * 2, 100_000);
          const h = await simulatorHeatmap({
            ...common,
            contribution_min_cents: 0,
            contribution_max_cents: x_max,
            horizon_min_years: 1,
            horizon_max_years: Math.min(50, Math.max(2, horizon * 2)),
          });
          if (!cancelled) setHeatmap(h);
        }
      } catch (e) {
        if (!cancelled) onError(String(e));
      }
    };
    void run();
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mode, common, confidence, contribCents]);

  const histogram = useMemo(() => {
    const lo = (required?.final_p_lo_cents ?? probability?.final_p_lo_cents) ?? null;
    const mid = (required?.final_p50_cents ?? probability?.final_p50_cents) ?? null;
    const hi = (required?.final_p_hi_cents ?? probability?.final_p_hi_cents) ?? null;
    if (lo === null || mid === null || hi === null) return null;
    return { lo, mid, hi };
  }, [required, probability]);

  const trajectory: TrajectoryPoint[] = useMemo(() => {
    if (mode === "required") return required?.trajectory ?? [];
    return probability?.trajectory ?? [];
  }, [mode, required, probability]);

  const bandPct: number = useMemo(() => {
    if (mode === "required") return required?.band_pct ?? confidence;
    return probability?.band_pct ?? probability?.probability ?? 0.8;
  }, [mode, required, probability, confidence]);

  const chartData = useMemo(() => {
    return trajectory.map((p) => ({
      year: +(p.month / 12).toFixed(2),
      Nominal: p.nominal_cents / 100,
      Real: p.real_cents / 100,
      Contributions: p.contributions_cents / 100,
      pLo: p.p_lo_cents / 100,
      pHi: p.p_hi_cents / 100,
      // Stacking helpers consumed by the two visual Areas only.
      // band_offset is the invisible base; band_span sits on top.
      band_offset: p.p_lo_cents / 100,
      band_span: (p.p_hi_cents - p.p_lo_cents) / 100,
    }));
  }, [trajectory]);

  // Percentile labels for the tooltip, derived from the active band.
  const loPctLabel = useMemo(
    () => ((1 - bandPct) / 2 * 100).toFixed(1),
    [bandPct],
  );
  const hiPctLabel = useMemo(
    () => (100 - (1 - bandPct) / 2 * 100).toFixed(1),
    [bandPct],
  );

  return (
    <Section title="Simulator">
      <p className="mb-3 text-sm text-graphite-400">
        Find the contribution that hits a target with a chosen confidence,
        or check the probability of a contribution you&apos;re already
        considering. Probability bands on the chart scale with confidence
        in &quot;Find required&quot; mode and with the resulting probability in
        &quot;Show probability&quot; mode. Heatmap below answers the broader
        trade-off — how do contribution and horizon together affect your
        odds?
      </p>

      <div className="mb-4 flex flex-wrap items-center gap-3">
        <div className="inline-flex rounded-md border border-graphite-700 bg-graphite-800 p-0.5 text-sm">
          <button
            onClick={() => setMode("required")}
            className={`rounded px-3 py-1 transition ${
              mode === "required"
                ? "bg-forest-600 text-graphite-50"
                : "text-graphite-300 hover:bg-graphite-700"
            }`}
          >
            Find required contribution
          </button>
          <button
            onClick={() => setMode("probability")}
            className={`rounded px-3 py-1 transition ${
              mode === "probability"
                ? "bg-forest-600 text-graphite-50"
                : "text-graphite-300 hover:bg-graphite-700"
            }`}
          >
            Show probability
          </button>
        </div>
        {accounts.length > 0 && (
          <label className="text-xs text-graphite-400">
            Pre-fill from account:&nbsp;
            <select
              value={accountId === "all" ? "all" : (accountId ?? "")}
              onChange={(e) => {
                const v = e.target.value;
                if (v === "") setAccountId(null);
                else if (v === "all") setAccountId("all");
                else setAccountId(Number(v));
              }}
              className="rounded-md border border-graphite-700 bg-graphite-800 px-2 py-1 text-xs text-graphite-100"
            >
              <option value="">— manual —</option>
              <option value="all">All investing accounts (sum)</option>
              {accounts.map((a) => (
                <option key={a.category_id} value={a.category_id}>
                  {a.name}
                </option>
              ))}
            </select>
          </label>
        )}
      </div>

      <div className="grid grid-cols-1 gap-4 lg:grid-cols-[1fr_2fr]">
        <div className="space-y-3">
          <NumberField
            label="Target amount"
            value={targetDollars}
            onChange={setTargetDollars}
            prefix="$"
          />
          <NumberSlider
            label={`Horizon: ${horizon} years`}
            min={1}
            max={50}
            step={1}
            value={horizon}
            onChange={setHorizon}
          />
          <NumberField
            label="Starting balance"
            value={startingDollars}
            onChange={setStartingDollars}
            prefix="$"
          />
          {mode === "required" ? (
            <div>
              <span className="text-xs uppercase tracking-wide text-graphite-400">
                Confidence: {(confidence * 100).toFixed(2)}%
              </span>
              <input
                type="range"
                min={0.5}
                max={0.95}
                step={0.01}
                value={confidence}
                onChange={(e) => setConfidence(Number(e.target.value))}
                className="mt-1 w-full"
              />
              <div className="mt-1 flex gap-2">
                {[0.7, 0.8, 0.9].map((v) => (
                  <button
                    key={v}
                    onClick={() => setConfidence(v)}
                    className={`rounded-md border px-2 py-0.5 text-xs ${
                      Math.abs(confidence - v) < 0.005
                        ? "border-forest-500 bg-forest-700/30 text-forest-100"
                        : "border-graphite-700 text-graphite-300 hover:border-graphite-500"
                    }`}
                  >
                    {(v * 100).toFixed(0)}%
                  </button>
                ))}
              </div>
            </div>
          ) : (
            <NumberField
              label="Monthly contribution"
              value={contributionDollars}
              onChange={setContributionDollars}
              prefix="$"
            />
          )}
          <NumberSlider
            label={`Annual return: ${returnPct.toFixed(2)}%`}
            min={0}
            max={15}
            step={0.5}
            value={returnPct}
            onChange={setReturnPct}
          />
          <div className="flex flex-wrap gap-2">
            {RETURN_PRESETS.map((p) => (
              <button
                key={p.rate}
                onClick={() => setReturnPct(p.rate)}
                className={`rounded-md border px-2 py-1 text-xs transition ${
                  Math.abs(returnPct - p.rate) < 0.05
                    ? "border-forest-500 bg-forest-700/30 text-forest-100"
                    : "border-graphite-700 text-graphite-300 hover:border-graphite-500"
                }`}
              >
                {p.label}
              </button>
            ))}
          </div>
          <NumberSlider
            label={`Annual inflation: ${inflationPct.toFixed(2)}%`}
            min={0}
            max={6}
            step={0.5}
            value={inflationPct}
            onChange={setInflationPct}
          />
          <div>
            <span className="text-xs uppercase tracking-wide text-graphite-400">
              Target is in
            </span>
            <div className="mt-1 inline-flex rounded-md border border-graphite-700 bg-graphite-800 p-0.5 text-xs">
              <button
                onClick={() => setTargetMode("todays_dollars")}
                className={`rounded px-2 py-0.5 ${
                  targetMode === "todays_dollars"
                    ? "bg-forest-600 text-graphite-50"
                    : "text-graphite-300"
                }`}
              >
                Today&apos;s $
              </button>
              <button
                onClick={() => setTargetMode("nominal_future")}
                className={`rounded px-2 py-0.5 ${
                  targetMode === "nominal_future"
                    ? "bg-forest-600 text-graphite-50"
                    : "text-graphite-300"
                }`}
              >
                Nominal future $
              </button>
            </div>
          </div>
          <div>
            <button
              onClick={() => setAdvanced((s) => !s)}
              className="text-xs text-graphite-400 hover:text-graphite-200"
            >
              {advanced ? "▾" : "▸"} Advanced (override volatility)
            </button>
            {advanced && (
              <div className="mt-2">
                <NumberSlider
                  label={`Annual volatility (σ): ${sigma.toFixed(2)}%`}
                  min={0}
                  max={30}
                  step={0.5}
                  value={sigma}
                  onChange={(v) => setSigmaOverride(v)}
                />
                {sigmaOverride !== null && (
                  <button
                    onClick={() => setSigmaOverride(null)}
                    className="mt-1 text-xs text-graphite-500 hover:text-graphite-300"
                  >
                    reset to preset ({volatilityForReturn(returnPct).toFixed(2)}%)
                  </button>
                )}
              </div>
            )}
          </div>
        </div>

        <div className="space-y-4">
          {mode === "required" && required && (
            <div className="rounded-md border border-graphite-700 bg-graphite-800 p-4">
              <div className="text-xs uppercase tracking-wide text-graphite-400">
                Required monthly contribution
              </div>
              <div className="mt-1 text-3xl font-semibold tabular-nums text-graphite-50">
                {formatMoney(required.required_monthly_cents)}
                <span className="ml-1 text-sm text-graphite-500">/ mo</span>
              </div>
              <div className="mt-1 text-xs text-graphite-400">
                to hit {formatMoney(targetCents)}{" "}
                {targetMode === "todays_dollars"
                  ? "(today's $)"
                  : "(nominal $)"} in {horizon} years with{" "}
                {(required.realized_probability * 100).toFixed(2)}% confidence
                at {returnPct.toFixed(2)}% return / {sigma.toFixed(2)}% σ.
              </div>
              {targetMode === "todays_dollars" && (
                <div className="mt-1 text-xs text-graphite-500">
                  ≈ {formatMoney(required.effective_target_cents)} nominal at the horizon
                </div>
              )}
            </div>
          )}
          {mode === "probability" && probability && (
            <div className="rounded-md border border-graphite-700 bg-graphite-800 p-4">
              <div className="text-xs uppercase tracking-wide text-graphite-400">
                Probability of hitting target
              </div>
              <div
                className={`mt-1 text-3xl font-semibold tabular-nums ${
                  probability.probability >= 0.7
                    ? "text-forest-100"
                    : probability.probability >= 0.4
                      ? "text-yellow-100"
                      : "text-red-200"
                }`}
              >
                {(probability.probability * 100).toFixed(2)}%
              </div>
              <div className="mt-1 text-xs text-graphite-400">
                at {formatMoney(contribCents)} / mo for {horizon} years toward{" "}
                {formatMoney(targetCents)}{" "}
                {targetMode === "todays_dollars" ? "(today's $)" : "(nominal $)"}.
              </div>
            </div>
          )}

          {chartData.length > 0 && (
            <div className="rounded-md border border-graphite-700 bg-graphite-800 p-3">
              <div className="mb-1 flex items-baseline justify-between">
                <div className="text-xs uppercase tracking-wide text-graphite-400">
                  Projection
                </div>
                <div className="text-xs text-graphite-500">
                  {(bandPct * 100).toFixed(2)}% probability band ·
                  hover for values
                </div>
              </div>
              <ResponsiveContainer width="100%" height={260}>
                <ComposedChart
                  data={chartData}
                  margin={{ top: 5, right: 10, left: 5, bottom: 5 }}
                >
                  <CartesianGrid stroke="#2a3138" strokeDasharray="3 3" />
                  <XAxis
                    dataKey="year"
                    stroke="#94a3b8"
                    tickFormatter={(v) => `${v}y`}
                  />
                  <YAxis
                    stroke="#94a3b8"
                    width={78}
                    tickFormatter={formatYAxisDollars}
                  />
                  <Tooltip
                    cursor={{ stroke: "#3b4148", strokeWidth: 1 }}
                    content={(p) => (
                      <ProjectionTooltipContent
                        active={p.active === true}
                        payload={
                          p.payload as
                            | { payload?: ProjectionDatum }[]
                            | undefined
                        }
                        label={
                          typeof p.label === "number" ? p.label : undefined
                        }
                        loPctLabel={loPctLabel}
                        hiPctLabel={hiPctLabel}
                        showContributions={showContributions}
                        showReal={inflationPct > 0}
                      />
                    )}
                  />
                  <Area
                    type="monotone"
                    dataKey="band_offset"
                    stackId="band"
                    stroke="none"
                    fill="transparent"
                    isAnimationActive={false}
                    legendType="none"
                    activeDot={false}
                  />
                  <Area
                    type="monotone"
                    dataKey="band_span"
                    stackId="band"
                    stroke="none"
                    fill="#60a5fa"
                    fillOpacity={0.18}
                    isAnimationActive={false}
                    legendType="none"
                    activeDot={false}
                  />
                  <Line
                    type="monotone"
                    dataKey="Nominal"
                    stroke="#34d399"
                    strokeWidth={2}
                    dot={false}
                  />
                  {inflationPct > 0 && (
                    <Line
                      type="monotone"
                      dataKey="Real"
                      stroke="#facc15"
                      strokeWidth={2}
                      strokeDasharray="4 3"
                      dot={false}
                    />
                  )}
                  {showContributions && (
                    <Line
                      type="monotone"
                      dataKey="Contributions"
                      stroke="#94a3b8"
                      strokeWidth={2}
                      strokeDasharray="2 4"
                      dot={false}
                    />
                  )}
                </ComposedChart>
              </ResponsiveContainer>
              <div className="mt-2 flex flex-wrap items-center gap-4 text-xs text-graphite-400">
                <span className="flex items-center gap-1.5">
                  <span className="inline-block h-0.5 w-4 bg-forest-400" /> Nominal
                </span>
                {inflationPct > 0 && (
                  <span className="flex items-center gap-1.5">
                    <span className="inline-block h-0.5 w-4 bg-yellow-400" /> Real
                    (today&apos;s $)
                  </span>
                )}
                <span className="flex items-center gap-1.5">
                  <span className="inline-block h-2 w-4 rounded-sm bg-blue-400/30" />{" "}
                  {(bandPct * 100).toFixed(2)}% band
                </span>
                <label className="ml-auto flex cursor-pointer items-center gap-1.5 select-none">
                  <input
                    type="checkbox"
                    checked={showContributions}
                    onChange={(e) => setShowContributions(e.target.checked)}
                    className="h-3 w-3 accent-forest-500"
                  />
                  <span className="flex items-center gap-1.5">
                    <span className="inline-block h-0.5 w-4 bg-graphite-300" />{" "}
                    Show contributions
                  </span>
                </label>
              </div>
            </div>
          )}

          {histogram && (
            <div className="rounded-md border border-graphite-700 bg-graphite-800 p-3 text-xs text-graphite-300">
              <div className="mb-1 text-xs uppercase tracking-wide text-graphite-500">
                Final-value distribution (1,000 paths,{" "}
                {(bandPct * 100).toFixed(2)}% band)
              </div>
              <div className="flex items-baseline justify-between gap-3">
                <span>
                  Lower:{" "}
                  <span className="font-mono tabular-nums text-graphite-100">
                    {formatMoney(histogram.lo)}
                  </span>
                </span>
                <span>
                  Median:{" "}
                  <span className="font-mono tabular-nums text-graphite-50">
                    {formatMoney(histogram.mid)}
                  </span>
                </span>
                <span>
                  Upper:{" "}
                  <span className="font-mono tabular-nums text-graphite-100">
                    {formatMoney(histogram.hi)}
                  </span>
                </span>
              </div>
            </div>
          )}
          {heatmap && <Heatmap data={heatmap} />}
        </div>
      </div>
    </Section>
  );
}

// ---------------------------------------------------------------------
// Debt Manager (v0.3.7)
// ---------------------------------------------------------------------
//
// Pure deterministic debt amortization. Two modes:
// - Forward calc:  payment + horizon → schedule, total interest, payoff.
// - Goal seek:     target months → required payment.
//
// A portfolio toggle (off by default) switches the form to a multi-debt
// layout that distributes a fixed monthly budget across debts using
// either snowball (smallest balance first) or avalanche (highest APR
// first). Goal-seek is intentionally single-debt only: the portfolio
// equivalent (bisecting total budget) is straightforward but adds UI
// complexity that doesn't pay off until users ask for it.
//
// "Send to Simulator" pipes the freed-up monthly payment + remaining
// horizon into the Simulator above so the user can see the after-payoff
// investing trajectory. The APR-as-guaranteed-return callout next to
// the result frames the comparison: paying off this debt is equivalent
// to a guaranteed return at the debt's APR.

type DebtMode = "forward" | "goal";

const COMPOUNDING_OPTIONS: { value: CompoundingFrequency; label: string }[] = [
  { value: "monthly", label: "Monthly" },
  { value: "daily", label: "Daily" },
  { value: "yearly", label: "Yearly" },
  { value: "continuous", label: "Continuous" },
];

interface PortfolioDebtRow {
  id: number;
  label: string;
  balance: string;
  apr: string;
  compounding: CompoundingFrequency;
  minimum: string;
}

interface LumpSumRow {
  id: number;
  month: string;
  amount: string;
}

let nextRowId = 1;
const newId = () => nextRowId++;

function DebtManager({
  onError,
  onSendToSimulator,
  simulatorReturnPct,
}: {
  onError: (m: string) => void;
  onSendToSimulator: (p: SimulatorPrefill) => void;
  simulatorReturnPct: number;
}) {
  const [mode, setMode] = useState<DebtMode>("forward");
  const [portfolioMode, setPortfolioMode] = useState(false);

  // --- Single-debt inputs ---
  const [balance, setBalance] = useState("10000.00");
  const [apr, setApr] = useState("18.99");
  const [compounding, setCompounding] = useState<CompoundingFrequency>("monthly");
  const [monthlyPayment, setMonthlyPayment] = useState("250.00");
  // Goal seek expressed as years + months so the user can think in
  // either; the months unit is what we send to the backend.
  const [targetYears, setTargetYears] = useState(2);
  const [targetMonths, setTargetMonths] = useState(0);
  const [inflation, setInflation] = useState(2.5);
  const [lumpSums, setLumpSums] = useState<LumpSumRow[]>([]);

  // --- Portfolio inputs ---
  const [portfolioBudget, setPortfolioBudget] = useState("800.00");
  const [strategy, setStrategy] = useState<PortfolioStrategy>("avalanche");
  const [debts, setDebts] = useState<PortfolioDebtRow[]>(() => [
    {
      id: newId(),
      label: "Card A",
      balance: "5000.00",
      apr: "22.99",
      compounding: "monthly",
      minimum: "100.00",
    },
    {
      id: newId(),
      label: "Card B",
      balance: "10000.00",
      apr: "12.99",
      compounding: "monthly",
      minimum: "200.00",
    },
  ]);

  // --- Results ---
  const [forwardResult, setForwardResult] =
    useState<DebtScheduleResult | null>(null);
  const [goalResult, setGoalResult] = useState<DebtGoalSeekResult | null>(null);
  const [portfolioResult, setPortfolioResult] =
    useState<PortfolioResult | null>(null);
  // Side-by-side comparison: when in portfolio mode, also run the
  // *other* strategy so the user sees the trade-off without toggling.
  const [portfolioOther, setPortfolioOther] =
    useState<PortfolioResult | null>(null);

  // --- Derived numeric values ---
  const balanceCents = useMemo(
    () => Math.round((parseFloat(balance) || 0) * 100),
    [balance],
  );
  const aprPct = useMemo(() => parseFloat(apr) || 0, [apr]);
  const monthlyPaymentCents = useMemo(
    () => Math.round((parseFloat(monthlyPayment) || 0) * 100),
    [monthlyPayment],
  );
  const targetMonthsTotal = useMemo(
    () => Math.max(1, targetYears * 12 + targetMonths),
    [targetYears, targetMonths],
  );
  const portfolioBudgetCents = useMemo(
    () => Math.round((parseFloat(portfolioBudget) || 0) * 100),
    [portfolioBudget],
  );

  const lumpSumsPayload: LumpSum[] = useMemo(
    () =>
      lumpSums
        .map((l) => ({
          month_offset: Math.max(0, parseInt(l.month, 10) || 0),
          amount_cents: Math.round((parseFloat(l.amount) || 0) * 100),
        }))
        .filter((l) => l.amount_cents > 0),
    [lumpSums],
  );

  // --- Single-debt fetches ---
  useEffect(() => {
    if (portfolioMode) return;
    if (balanceCents <= 0) return;
    let cancelled = false;
    const run = async () => {
      try {
        if (mode === "forward") {
          const r = await debtSimulateSchedule({
            debt: {
              balance_cents: balanceCents,
              apr_pct: aprPct,
              compounding,
            },
            monthly_payment_cents: monthlyPaymentCents,
            lump_sums: lumpSumsPayload,
            annual_inflation_pct: inflation,
          });
          if (!cancelled) {
            setForwardResult(r);
            setGoalResult(null);
          }
        } else {
          const r = await debtGoalSeek({
            debt: {
              balance_cents: balanceCents,
              apr_pct: aprPct,
              compounding,
            },
            target_months: targetMonthsTotal,
            lump_sums: lumpSumsPayload,
            annual_inflation_pct: inflation,
          });
          if (!cancelled) {
            setGoalResult(r);
            setForwardResult(null);
          }
        }
      } catch (e) {
        if (!cancelled) onError(String(e));
      }
    };
    void run();
    return () => {
      cancelled = true;
    };
  }, [
    portfolioMode,
    mode,
    balanceCents,
    aprPct,
    compounding,
    monthlyPaymentCents,
    targetMonthsTotal,
    inflation,
    lumpSumsPayload,
    onError,
  ]);

  // --- Portfolio fetch (and parallel "other strategy" for comparison). ---
  useEffect(() => {
    if (!portfolioMode) return;
    if (debts.length === 0) return;
    let cancelled = false;
    const debtPayload: DebtInput[] = debts.map((d) => ({
      label: d.label || null,
      balance_cents: Math.round((parseFloat(d.balance) || 0) * 100),
      apr_pct: parseFloat(d.apr) || 0,
      compounding: d.compounding,
      minimum_payment_cents: Math.round((parseFloat(d.minimum) || 0) * 100),
    }));
    const run = async () => {
      try {
        const [primary, other] = await Promise.all([
          debtSimulatePortfolio({
            debts: debtPayload,
            total_monthly_budget_cents: portfolioBudgetCents,
            strategy,
            lump_sums: lumpSumsPayload,
            annual_inflation_pct: inflation,
          }),
          debtSimulatePortfolio({
            debts: debtPayload,
            total_monthly_budget_cents: portfolioBudgetCents,
            strategy: strategy === "snowball" ? "avalanche" : "snowball",
            lump_sums: lumpSumsPayload,
            annual_inflation_pct: inflation,
          }),
        ]);
        if (!cancelled) {
          setPortfolioResult(primary);
          setPortfolioOther(other);
        }
      } catch (e) {
        if (!cancelled) onError(String(e));
      }
    };
    void run();
    return () => {
      cancelled = true;
    };
  }, [
    portfolioMode,
    debts,
    portfolioBudgetCents,
    strategy,
    lumpSumsPayload,
    inflation,
    onError,
  ]);

  const addLumpSum = () =>
    setLumpSums((rows) => [
      ...rows,
      { id: newId(), month: "12", amount: "1000.00" },
    ]);
  const removeLumpSum = (id: number) =>
    setLumpSums((rows) => rows.filter((r) => r.id !== id));
  const updateLumpSum = (id: number, patch: Partial<LumpSumRow>) =>
    setLumpSums((rows) =>
      rows.map((r) => (r.id === id ? { ...r, ...patch } : r)),
    );

  const addDebt = () =>
    setDebts((rows) => [
      ...rows,
      {
        id: newId(),
        label: `Card ${String.fromCharCode(65 + rows.length)}`,
        balance: "1000.00",
        apr: "15.00",
        compounding: "monthly",
        minimum: "50.00",
      },
    ]);
  const removeDebt = (id: number) =>
    setDebts((rows) => rows.filter((r) => r.id !== id));
  const updateDebt = (id: number, patch: Partial<PortfolioDebtRow>) =>
    setDebts((rows) =>
      rows.map((r) => (r.id === id ? { ...r, ...patch } : r)),
    );

  // The "send to Simulator" handler. Picks a sensible horizon for the
  // post-payoff window: the user's likely retirement runway, capped at
  // 50 years to match the Simulator's slider. We don't try to be cute
  // about ages — 30 years is the default the Simulator boots with.
  const sendToSimulator = (
    monthlyCents: number,
    payoffMonth: number | null,
  ) => {
    const POST_PAYOFF_YEARS = 30;
    const horizon = payoffMonth ? POST_PAYOFF_YEARS : POST_PAYOFF_YEARS;
    onSendToSimulator({
      monthly_contribution_cents: monthlyCents,
      starting_balance_cents: 0,
      horizon_years: horizon,
      bump: Date.now(),
    });
    // Scroll the Simulator into view so the user sees the prefill take
    // effect.
    requestAnimationFrame(() => {
      const root = document.getElementById("forecast-root");
      root?.scrollTo?.({ top: 0, behavior: "smooth" });
      window.scrollTo({ top: 0, behavior: "smooth" });
    });
  };

  return (
    <Section title="Debt Manager">
      <p className="mb-3 text-sm text-graphite-400">
        Plan a debt payoff. Forward calc takes a monthly payment and shows
        your timeline; goal seek finds the smallest payment that hits a
        deadline. Lump sums and inflation are factored in. Toggle{" "}
        <em>Portfolio mode</em> for multiple debts at once.
      </p>

      <div className="mb-4 flex flex-wrap items-center gap-3">
        {!portfolioMode && (
          <div className="inline-flex rounded-md border border-graphite-700 bg-graphite-800 p-0.5 text-sm">
            <button
              onClick={() => setMode("forward")}
              className={`rounded px-3 py-1 transition ${
                mode === "forward"
                  ? "bg-forest-600 text-graphite-50"
                  : "text-graphite-300 hover:bg-graphite-700"
              }`}
            >
              Forward calc
            </button>
            <button
              onClick={() => setMode("goal")}
              className={`rounded px-3 py-1 transition ${
                mode === "goal"
                  ? "bg-forest-600 text-graphite-50"
                  : "text-graphite-300 hover:bg-graphite-700"
              }`}
            >
              Goal seek
            </button>
          </div>
        )}
        <label className="flex cursor-pointer select-none items-center gap-1.5 text-xs text-graphite-300">
          <input
            type="checkbox"
            checked={portfolioMode}
            onChange={(e) => setPortfolioMode(e.target.checked)}
            className="h-3 w-3 accent-forest-500"
          />
          Portfolio mode
        </label>
      </div>

      <div className="grid grid-cols-1 gap-4 lg:grid-cols-[1fr_2fr]">
        <div className="space-y-3">
          {!portfolioMode ? (
            <>
              <NumberField
                label="Debt balance"
                value={balance}
                onChange={setBalance}
                prefix="$"
              />
              <NumberField label="APR" value={apr} onChange={setApr} prefix="%" />
              <SelectField
                label="Compounding"
                value={compounding}
                onChange={(v) => setCompounding(v as CompoundingFrequency)}
                options={COMPOUNDING_OPTIONS}
              />
              {mode === "forward" ? (
                <NumberField
                  label="Monthly payment"
                  value={monthlyPayment}
                  onChange={setMonthlyPayment}
                  prefix="$"
                />
              ) : (
                <div className="grid grid-cols-2 gap-2">
                  <NumberSlider
                    label={`Target years: ${targetYears}`}
                    min={0}
                    max={30}
                    step={1}
                    value={targetYears}
                    onChange={setTargetYears}
                  />
                  <NumberSlider
                    label={`Target months: ${targetMonths}`}
                    min={0}
                    max={11}
                    step={1}
                    value={targetMonths}
                    onChange={setTargetMonths}
                  />
                </div>
              )}
            </>
          ) : (
            <>
              <NumberField
                label="Total monthly budget"
                value={portfolioBudget}
                onChange={setPortfolioBudget}
                prefix="$"
                hint="Must be at least the sum of every debt's minimum."
              />
              <div>
                <span className="text-xs uppercase tracking-wide text-graphite-400">
                  Strategy
                </span>
                <div className="mt-1 inline-flex rounded-md border border-graphite-700 bg-graphite-800 p-0.5 text-xs">
                  <button
                    onClick={() => setStrategy("avalanche")}
                    className={`rounded px-2 py-0.5 ${
                      strategy === "avalanche"
                        ? "bg-forest-600 text-graphite-50"
                        : "text-graphite-300"
                    }`}
                  >
                    Avalanche (highest APR)
                  </button>
                  <button
                    onClick={() => setStrategy("snowball")}
                    className={`rounded px-2 py-0.5 ${
                      strategy === "snowball"
                        ? "bg-forest-600 text-graphite-50"
                        : "text-graphite-300"
                    }`}
                  >
                    Snowball (smallest balance)
                  </button>
                </div>
              </div>
              <div className="space-y-2">
                <div className="text-xs uppercase tracking-wide text-graphite-400">
                  Debts
                </div>
                {debts.map((d) => (
                  <div
                    key={d.id}
                    className="rounded-md border border-graphite-700 bg-graphite-800 p-2"
                  >
                    <div className="flex items-center gap-2">
                      <input
                        type="text"
                        value={d.label}
                        onChange={(e) =>
                          updateDebt(d.id, { label: e.target.value })
                        }
                        className="w-24 rounded border border-graphite-700 bg-graphite-900 px-2 py-1 text-xs text-graphite-100"
                      />
                      <button
                        onClick={() => removeDebt(d.id)}
                        className="ml-auto text-xs text-graphite-500 hover:text-red-300"
                        title="Remove debt"
                      >
                        ×
                      </button>
                    </div>
                    <div className="mt-2 grid grid-cols-2 gap-2 text-xs">
                      <label>
                        <span className="text-graphite-400">Balance</span>
                        <input
                          type="text"
                          inputMode="decimal"
                          value={d.balance}
                          onChange={(e) =>
                            updateDebt(d.id, { balance: e.target.value })
                          }
                          className="mt-0.5 w-full rounded border border-graphite-700 bg-graphite-900 px-2 py-1 text-graphite-100"
                        />
                      </label>
                      <label>
                        <span className="text-graphite-400">APR %</span>
                        <input
                          type="text"
                          inputMode="decimal"
                          value={d.apr}
                          onChange={(e) =>
                            updateDebt(d.id, { apr: e.target.value })
                          }
                          className="mt-0.5 w-full rounded border border-graphite-700 bg-graphite-900 px-2 py-1 text-graphite-100"
                        />
                      </label>
                      <label>
                        <span className="text-graphite-400">Min payment</span>
                        <input
                          type="text"
                          inputMode="decimal"
                          value={d.minimum}
                          onChange={(e) =>
                            updateDebt(d.id, { minimum: e.target.value })
                          }
                          className="mt-0.5 w-full rounded border border-graphite-700 bg-graphite-900 px-2 py-1 text-graphite-100"
                        />
                      </label>
                      <label>
                        <span className="text-graphite-400">Compounding</span>
                        <select
                          value={d.compounding}
                          onChange={(e) =>
                            updateDebt(d.id, {
                              compounding: e.target.value as CompoundingFrequency,
                            })
                          }
                          className="mt-0.5 w-full rounded border border-graphite-700 bg-graphite-900 px-2 py-1 text-graphite-100"
                        >
                          {COMPOUNDING_OPTIONS.map((o) => (
                            <option key={o.value} value={o.value}>
                              {o.label}
                            </option>
                          ))}
                        </select>
                      </label>
                    </div>
                  </div>
                ))}
                <button
                  onClick={addDebt}
                  className="rounded-md border border-graphite-700 px-2 py-1 text-xs text-graphite-300 hover:border-graphite-500"
                >
                  + Add debt
                </button>
              </div>
            </>
          )}

          <NumberSlider
            label={`Annual inflation: ${inflation.toFixed(2)}%`}
            min={0}
            max={6}
            step={0.5}
            value={inflation}
            onChange={setInflation}
          />

          <div className="space-y-2">
            <div className="text-xs uppercase tracking-wide text-graphite-400">
              Lump sums
            </div>
            {lumpSums.length === 0 && (
              <div className="text-xs text-graphite-500">
                None — add a one-time payment (tax refund, bonus, etc.)
              </div>
            )}
            {lumpSums.map((l) => (
              <div key={l.id} className="flex items-center gap-2 text-xs">
                <span className="text-graphite-400">At month</span>
                <input
                  type="text"
                  inputMode="numeric"
                  value={l.month}
                  onChange={(e) => updateLumpSum(l.id, { month: e.target.value })}
                  className="w-14 rounded border border-graphite-700 bg-graphite-800 px-2 py-1 text-graphite-100"
                />
                <span className="text-graphite-400">$</span>
                <input
                  type="text"
                  inputMode="decimal"
                  value={l.amount}
                  onChange={(e) =>
                    updateLumpSum(l.id, { amount: e.target.value })
                  }
                  className="w-24 rounded border border-graphite-700 bg-graphite-800 px-2 py-1 text-graphite-100"
                />
                <button
                  onClick={() => removeLumpSum(l.id)}
                  className="text-graphite-500 hover:text-red-300"
                  title="Remove lump sum"
                >
                  ×
                </button>
              </div>
            ))}
            <button
              onClick={addLumpSum}
              className="rounded-md border border-graphite-700 px-2 py-1 text-xs text-graphite-300 hover:border-graphite-500"
            >
              + Add lump sum
            </button>
          </div>
        </div>

        <div className="space-y-3">
          {!portfolioMode && mode === "forward" && forwardResult && (
            <SingleResultCard
              result={forwardResult}
              monthlyPaymentCents={monthlyPaymentCents}
              aprPct={aprPct}
              simulatorReturnPct={simulatorReturnPct}
              onSendToSimulator={() =>
                sendToSimulator(monthlyPaymentCents, forwardResult.payoff_month)
              }
            />
          )}
          {!portfolioMode && mode === "goal" && goalResult && (
            <SingleResultCard
              result={goalResult.schedule}
              monthlyPaymentCents={goalResult.required_monthly_payment_cents}
              aprPct={aprPct}
              simulatorReturnPct={simulatorReturnPct}
              feasible={goalResult.feasible}
              isGoalSeek
              onSendToSimulator={() =>
                sendToSimulator(
                  goalResult.required_monthly_payment_cents,
                  goalResult.schedule.payoff_month,
                )
              }
            />
          )}
          {portfolioMode && portfolioResult && (
            <PortfolioResultCard
              primary={portfolioResult}
              other={portfolioOther}
              budgetCents={portfolioBudgetCents}
              onSendToSimulator={() =>
                sendToSimulator(
                  portfolioBudgetCents,
                  portfolioResult.payoff_month,
                )
              }
            />
          )}

          {!portfolioMode && (forwardResult || goalResult) && (
            <DebtChart
              trajectory={
                (forwardResult?.trajectory ?? goalResult?.schedule.trajectory) ??
                []
              }
              startingBalanceCents={balanceCents}
            />
          )}
          {portfolioMode && portfolioResult && (
            <PortfolioChart result={portfolioResult} other={portfolioOther} />
          )}
        </div>
      </div>
    </Section>
  );
}

// Inline result card for the single-debt mode. Used by both Forward
// and Goal seek; the latter passes `isGoalSeek` so we frame the
// monthly-payment value as "required" rather than "your" payment.
function SingleResultCard({
  result,
  monthlyPaymentCents,
  aprPct,
  simulatorReturnPct,
  onSendToSimulator,
  isGoalSeek = false,
  feasible = true,
}: {
  result: DebtScheduleResult;
  monthlyPaymentCents: number;
  aprPct: number;
  simulatorReturnPct: number;
  onSendToSimulator: () => void;
  isGoalSeek?: boolean;
  feasible?: boolean;
}) {
  const aprBeatsReturn = aprPct > simulatorReturnPct;
  return (
    <div className="rounded-md border border-graphite-700 bg-graphite-800 p-4">
      {isGoalSeek && (
        <>
          <div className="text-xs uppercase tracking-wide text-graphite-400">
            Required monthly payment
          </div>
          <div className="mt-1 text-3xl font-semibold tabular-nums text-graphite-50">
            {formatMoney(monthlyPaymentCents)}
            <span className="ml-1 text-sm text-graphite-500">/ mo</span>
          </div>
          {!feasible && (
            <div className="mt-1 text-xs text-amber-300">
              Couldn&apos;t reach the target within practical payment levels.
            </div>
          )}
        </>
      )}
      <div className="mt-2 grid grid-cols-2 gap-3 text-xs text-graphite-300">
        <div>
          <div className="text-graphite-500">Pays off</div>
          <div className="font-mono tabular-nums text-graphite-50">
            {result.payoff_month
              ? `Year ${result.payoff_year_offset! + 1}, month ${
                  result.payoff_month_in_year
                } (${result.payoff_month} mo total)`
              : "Doesn't pay off"}
          </div>
        </div>
        <div>
          <div className="text-graphite-500">Total interest</div>
          <div className="font-mono tabular-nums text-graphite-50">
            {formatMoney(result.total_interest_cents)}
          </div>
        </div>
        <div>
          <div className="text-graphite-500">Total paid (nominal)</div>
          <div className="font-mono tabular-nums text-graphite-50">
            {formatMoney(result.total_paid_cents)}
          </div>
        </div>
        <div>
          <div className="text-graphite-500">In today&apos;s $</div>
          <div className="font-mono tabular-nums text-graphite-50">
            {formatMoney(result.total_paid_today_cents)}
          </div>
        </div>
      </div>
      {result.warning && (
        <div className="mt-3 rounded border border-amber-700/40 bg-amber-900/20 p-2 text-xs text-amber-200">
          ⚠ {result.warning}
        </div>
      )}
      <div className="mt-3 rounded border border-graphite-700 bg-graphite-900 p-2 text-xs text-graphite-300">
        <div className="text-graphite-400">
          Equivalent guaranteed return
        </div>
        <div className="mt-0.5">
          Paying off this debt is equivalent to a{" "}
          <span className="font-semibold text-graphite-50">
            {aprPct.toFixed(2)}%
          </span>{" "}
          guaranteed return — your Simulator&apos;s nominal return is{" "}
          {simulatorReturnPct.toFixed(2)}%.{" "}
          {aprBeatsReturn ? (
            <span className="text-amber-200">
              The debt&apos;s APR is higher, so paying it down likely beats
              investing the same dollars.
            </span>
          ) : (
            <span className="text-forest-200">
              Investing at the assumed return likely beats extra debt
              payoff — but a guaranteed return is worth more than an
              expected one.
            </span>
          )}
        </div>
      </div>
      <button
        onClick={onSendToSimulator}
        className="mt-3 rounded-md border border-forest-600/40 bg-forest-700/20 px-3 py-1.5 text-xs text-forest-100 hover:bg-forest-700/30"
      >
        After payoff: invest {formatMoney(monthlyPaymentCents)}/mo →
        Simulator
      </button>
    </div>
  );
}

function PortfolioResultCard({
  primary,
  other,
  budgetCents,
  onSendToSimulator,
}: {
  primary: PortfolioResult;
  other: PortfolioResult | null;
  budgetCents: number;
  onSendToSimulator: () => void;
}) {
  const fmtMonth = (r: PortfolioResult) =>
    r.payoff_month
      ? `Year ${r.payoff_year_offset! + 1}, month ${r.payoff_month_in_year} (${
          r.payoff_month
        } mo)`
      : "Doesn't pay off";
  return (
    <div className="rounded-md border border-graphite-700 bg-graphite-800 p-4">
      <div className="text-xs uppercase tracking-wide text-graphite-400">
        {primary.strategy === "avalanche" ? "Avalanche" : "Snowball"} strategy
      </div>
      <div className="mt-2 grid grid-cols-2 gap-3 text-xs text-graphite-300">
        <div>
          <div className="text-graphite-500">Pays off</div>
          <div className="font-mono tabular-nums text-graphite-50">
            {fmtMonth(primary)}
          </div>
        </div>
        <div>
          <div className="text-graphite-500">Total interest</div>
          <div className="font-mono tabular-nums text-graphite-50">
            {formatMoney(primary.total_interest_cents)}
          </div>
        </div>
        <div>
          <div className="text-graphite-500">Total paid (nominal)</div>
          <div className="font-mono tabular-nums text-graphite-50">
            {formatMoney(primary.total_paid_cents)}
          </div>
        </div>
        <div>
          <div className="text-graphite-500">In today&apos;s $</div>
          <div className="font-mono tabular-nums text-graphite-50">
            {formatMoney(primary.total_paid_today_cents)}
          </div>
        </div>
      </div>
      {other && (
        <div className="mt-3 rounded border border-graphite-700 bg-graphite-900 p-2 text-xs">
          <div className="text-graphite-400">
            vs. {other.strategy === "avalanche" ? "avalanche" : "snowball"}
          </div>
          <div className="mt-0.5 grid grid-cols-2 gap-2 text-graphite-300">
            <span>
              Pays off:{" "}
              <span className="font-mono tabular-nums text-graphite-100">
                {fmtMonth(other)}
              </span>
            </span>
            <span>
              Interest:{" "}
              <span className="font-mono tabular-nums text-graphite-100">
                {formatMoney(other.total_interest_cents)}
              </span>
            </span>
          </div>
          {primary.paid_off && other.paid_off && (
            <div className="mt-1 text-graphite-500">
              Difference:{" "}
              <span
                className={
                  primary.total_interest_cents <= other.total_interest_cents
                    ? "text-forest-200"
                    : "text-amber-200"
                }
              >
                {formatMoney(
                  Math.abs(
                    primary.total_interest_cents - other.total_interest_cents,
                  ),
                )}{" "}
                {primary.total_interest_cents <= other.total_interest_cents
                  ? "less"
                  : "more"}{" "}
                interest with {primary.strategy}.
              </span>
            </div>
          )}
        </div>
      )}
      {primary.warning && (
        <div className="mt-3 rounded border border-amber-700/40 bg-amber-900/20 p-2 text-xs text-amber-200">
          ⚠ {primary.warning}
        </div>
      )}
      <div className="mt-3 text-xs text-graphite-500">
        Sum of minimums: {formatMoney(primary.minimum_budget_cents)} · Budget:{" "}
        {formatMoney(budgetCents)}
      </div>
      <button
        onClick={onSendToSimulator}
        className="mt-3 rounded-md border border-forest-600/40 bg-forest-700/20 px-3 py-1.5 text-xs text-forest-100 hover:bg-forest-700/30"
      >
        After payoff: invest {formatMoney(budgetCents)}/mo → Simulator
      </button>
    </div>
  );
}

interface DebtChartDatum {
  month: number;
  balance: number;
  principal: number;
  interest: number;
}

function DebtChart({
  trajectory,
  startingBalanceCents,
}: {
  trajectory: DebtScheduleResult["trajectory"];
  startingBalanceCents: number;
}) {
  const data: DebtChartDatum[] = useMemo(() => {
    if (trajectory.length === 0) return [];
    const seed: DebtChartDatum = {
      month: 0,
      balance: startingBalanceCents / 100,
      principal: 0,
      interest: 0,
    };
    const rest = trajectory.map((t) => ({
      month: t.month,
      balance: t.balance_cents / 100,
      principal: t.cumulative_principal_cents / 100,
      interest: t.cumulative_interest_cents / 100,
    }));
    return [seed, ...rest];
  }, [trajectory, startingBalanceCents]);

  if (data.length <= 1) return null;
  return (
    <div className="rounded-md border border-graphite-700 bg-graphite-800 p-3">
      <div className="mb-1 flex items-baseline justify-between">
        <div className="text-xs uppercase tracking-wide text-graphite-400">
          Payoff trajectory
        </div>
        <div className="text-xs text-graphite-500">
          balance falls · paid principal/interest stack rises
        </div>
      </div>
      <ResponsiveContainer width="100%" height={260}>
        <ComposedChart
          data={data}
          margin={{ top: 5, right: 10, left: 5, bottom: 5 }}
        >
          <CartesianGrid stroke="#2a3138" strokeDasharray="3 3" />
          <XAxis
            dataKey="month"
            stroke="#94a3b8"
            tickFormatter={(v) => `${v}m`}
          />
          <YAxis stroke="#94a3b8" width={78} tickFormatter={formatYAxisDollars} />
          <Tooltip
            cursor={{ stroke: "#3b4148", strokeWidth: 1 }}
            content={(p) => (
              <DebtTooltipContent
                active={p.active === true}
                payload={
                  p.payload as { payload?: DebtChartDatum }[] | undefined
                }
                label={typeof p.label === "number" ? p.label : undefined}
              />
            )}
          />
          <Area
            type="monotone"
            dataKey="principal"
            stackId="paid"
            stroke="none"
            fill="#34d399"
            fillOpacity={0.3}
            isAnimationActive={false}
            activeDot={false}
          />
          <Area
            type="monotone"
            dataKey="interest"
            stackId="paid"
            stroke="none"
            fill="#facc15"
            fillOpacity={0.3}
            isAnimationActive={false}
            activeDot={false}
          />
          <Line
            type="monotone"
            dataKey="balance"
            stroke="#f87171"
            strokeWidth={2}
            dot={false}
          />
        </ComposedChart>
      </ResponsiveContainer>
      <div className="mt-2 flex flex-wrap gap-4 text-xs text-graphite-400">
        <span className="flex items-center gap-1.5">
          <span className="inline-block h-0.5 w-4 bg-red-400" /> Balance
        </span>
        <span className="flex items-center gap-1.5">
          <span className="inline-block h-2 w-4 rounded-sm bg-forest-400/30" />{" "}
          Cumulative principal
        </span>
        <span className="flex items-center gap-1.5">
          <span className="inline-block h-2 w-4 rounded-sm bg-yellow-400/30" />{" "}
          Cumulative interest
        </span>
      </div>
    </div>
  );
}

function DebtTooltipContent({
  active,
  payload,
  label,
}: {
  active: boolean | undefined;
  payload: { payload?: DebtChartDatum }[] | undefined;
  label: number | undefined;
}) {
  if (!active || !payload || payload.length === 0) return null;
  const d = payload[0]?.payload;
  if (!d) return null;
  return (
    <div className="rounded-md border border-graphite-700 bg-graphite-900 p-2 text-xs">
      <div className="mb-1 text-graphite-400">Month {label}</div>
      <Row label="Balance" color="#f87171" value={d.balance} />
      <Row label="Principal paid" color="#34d399" value={d.principal} />
      <Row label="Interest paid" color="#facc15" value={d.interest} />
      <Row label="Total out" color="#94a3b8" value={d.principal + d.interest} />
    </div>
  );
}

interface PortfolioChartDatum {
  month: number;
  balance: number;
  otherBalance?: number;
  interest: number;
  principal: number;
}

function PortfolioChart({
  result,
  other,
}: {
  result: PortfolioResult;
  other: PortfolioResult | null;
}) {
  const data: PortfolioChartDatum[] = useMemo(() => {
    const otherById = new Map<number, number>();
    if (other) {
      for (const m of other.trajectory) {
        otherById.set(m.month, m.total_balance_cents / 100);
      }
    }
    return result.trajectory.map((t) => {
      const datum: PortfolioChartDatum = {
        month: t.month,
        balance: t.total_balance_cents / 100,
        interest: t.cumulative_interest_cents / 100,
        principal: t.cumulative_principal_cents / 100,
      };
      const ob = otherById.get(t.month);
      if (ob !== undefined) datum.otherBalance = ob;
      return datum;
    });
  }, [result, other]);

  if (data.length === 0) return null;
  const otherLabel =
    other?.strategy === "avalanche" ? "Avalanche" : "Snowball";
  return (
    <div className="rounded-md border border-graphite-700 bg-graphite-800 p-3">
      <div className="mb-1 flex items-baseline justify-between">
        <div className="text-xs uppercase tracking-wide text-graphite-400">
          Portfolio balance over time
        </div>
        <div className="text-xs text-graphite-500">
          {result.strategy === "avalanche" ? "avalanche" : "snowball"} solid ·{" "}
          {other ? `${otherLabel.toLowerCase()} dashed` : ""}
        </div>
      </div>
      <ResponsiveContainer width="100%" height={260}>
        <ComposedChart
          data={data}
          margin={{ top: 5, right: 10, left: 5, bottom: 5 }}
        >
          <CartesianGrid stroke="#2a3138" strokeDasharray="3 3" />
          <XAxis
            dataKey="month"
            stroke="#94a3b8"
            tickFormatter={(v) => `${v}m`}
          />
          <YAxis stroke="#94a3b8" width={78} tickFormatter={formatYAxisDollars} />
          <Tooltip
            cursor={{ stroke: "#3b4148", strokeWidth: 1 }}
            content={(p) => (
              <PortfolioTooltipContent
                active={p.active === true}
                payload={
                  p.payload as { payload?: PortfolioChartDatum }[] | undefined
                }
                label={typeof p.label === "number" ? p.label : undefined}
                otherLabel={otherLabel}
              />
            )}
          />
          <Area
            type="monotone"
            dataKey="principal"
            stackId="paid"
            stroke="none"
            fill="#34d399"
            fillOpacity={0.25}
            isAnimationActive={false}
            activeDot={false}
          />
          <Area
            type="monotone"
            dataKey="interest"
            stackId="paid"
            stroke="none"
            fill="#facc15"
            fillOpacity={0.25}
            isAnimationActive={false}
            activeDot={false}
          />
          <Line
            type="monotone"
            dataKey="balance"
            stroke="#f87171"
            strokeWidth={2}
            dot={false}
          />
          {other && (
            <Line
              type="monotone"
              dataKey="otherBalance"
              stroke="#a78bfa"
              strokeWidth={2}
              strokeDasharray="4 3"
              dot={false}
            />
          )}
        </ComposedChart>
      </ResponsiveContainer>
    </div>
  );
}

function PortfolioTooltipContent({
  active,
  payload,
  label,
  otherLabel,
}: {
  active: boolean | undefined;
  payload: { payload?: PortfolioChartDatum }[] | undefined;
  label: number | undefined;
  otherLabel: string;
}) {
  if (!active || !payload || payload.length === 0) return null;
  const d = payload[0]?.payload;
  if (!d) return null;
  return (
    <div className="rounded-md border border-graphite-700 bg-graphite-900 p-2 text-xs">
      <div className="mb-1 text-graphite-400">Month {label}</div>
      <Row label="Balance" color="#f87171" value={d.balance} />
      {d.otherBalance !== undefined && (
        <Row label={otherLabel} color="#a78bfa" value={d.otherBalance} />
      )}
      <Row label="Principal paid" color="#34d399" value={d.principal} />
      <Row label="Interest paid" color="#facc15" value={d.interest} />
    </div>
  );
}

// Tiny dropdown helper used only by the Debt Manager. Mirrors the
// styling of NumberField for consistency with the rest of the form.
function SelectField<T extends string>({
  label,
  value,
  onChange,
  options,
}: {
  label: string;
  value: T;
  onChange: (v: T) => void;
  options: { value: T; label: string }[];
}) {
  return (
    <label className="block">
      <span className="text-xs uppercase tracking-wide text-graphite-400">
        {label}
      </span>
      <select
        value={value}
        onChange={(e) => onChange(e.target.value as T)}
        className="mt-1 w-full rounded-md border border-graphite-700 bg-graphite-800 px-3 py-2 text-sm text-graphite-100"
      >
        {options.map((o) => (
          <option key={o.value} value={o.value}>
            {o.label}
          </option>
        ))}
      </select>
    </label>
  );
}

function Heatmap({ data }: { data: HeatmapResult }) {
  // Render a 12×12 grid. Color cells red→yellow→green by probability.
  // Click snaps an event to the parent (omitted for v1; tooltip on
  // hover is enough to read the trade-space).
  const n = 12;
  const grouped: typeof data.cells[] = [];
  for (let j = 0; j < n; j++) {
    grouped.push(data.cells.slice(j * n, (j + 1) * n));
  }
  return (
    <div className="rounded-md border border-graphite-700 bg-graphite-800 p-3">
      <div className="mb-2 flex items-baseline justify-between">
        <div className="text-xs uppercase tracking-wide text-graphite-400">
          Probability heatmap (contribution × horizon)
        </div>
        <div className="text-xs text-graphite-500">
          hover a cell for exact values
        </div>
      </div>
      <div className="overflow-x-auto">
        <table className="text-[10px] tabular-nums">
          <thead>
            <tr className="text-graphite-500">
              <th className="px-1 py-0.5 text-right">y\$</th>
              {grouped[0]?.map((c, i) => (
                <th key={i} className="px-1 py-0.5 font-normal">
                  {c.contribution_cents >= 100_000
                    ? `$${Math.round(c.contribution_cents / 100_000)}k`
                    : `$${Math.round(c.contribution_cents / 100)}`}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {grouped.map((row, j) => (
              <tr key={j}>
                <td className="px-1 py-0.5 text-right text-graphite-500">
                  {row[0]?.horizon_years ?? "-"}y
                </td>
                {row.map((c, i) => (
                  <td
                    key={i}
                    title={`$${(c.contribution_cents / 100).toFixed(0)}/mo for ${c.horizon_years}y → ${(c.probability * 100).toFixed(0)}% probability`}
                    className="px-1 py-0.5 text-center"
                    style={{
                      backgroundColor: heatmapColor(c.probability),
                      // Charcoal on amber/lime (light bgs); white on
                      // red/green (dark bgs). Picked so the number
                      // never disappears into the tile.
                      color: heatmapTextColor(c.probability),
                    }}
                  >
                    {(c.probability * 100).toFixed(0)}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <div className="mt-2 flex gap-3 text-[10px] text-graphite-500">
        <span className="flex items-center gap-1">
          <span className="inline-block h-2 w-3" style={{ background: "#7f1d1d" }} /> &lt;50%
        </span>
        <span className="flex items-center gap-1">
          <span className="inline-block h-2 w-3" style={{ background: "#a16207" }} /> 50–70%
        </span>
        <span className="flex items-center gap-1">
          <span className="inline-block h-2 w-3" style={{ background: "#4d7c0f" }} /> 70–90%
        </span>
        <span className="flex items-center gap-1">
          <span className="inline-block h-2 w-3" style={{ background: "#166534" }} /> ≥90%
        </span>
      </div>
    </div>
  );
}

function heatmapColor(p: number): string {
  if (p < 0.5) return "#7f1d1d"; // red-900
  if (p < 0.7) return "#a16207"; // amber-700
  if (p < 0.9) return "#4d7c0f"; // lime-700
  return "#166534"; // green-800
}

function heatmapTextColor(p: number): string {
  // Light text on dark tiles (red-900, green-800), charcoal on the
  // medium-luminance tiles (amber-700, lime-700) where light text
  // disappears into the background.
  if (p < 0.5) return "#f3f4f6"; // light on red-900
  if (p < 0.7) return "#1f2937"; // charcoal on amber-700
  if (p < 0.9) return "#1f2937"; // charcoal on lime-700
  return "#f3f4f6"; // light on green-800
}

// ---------------------------------------------------------------------
// Category Analyzer (v0.3.3) — opt-in dropdown, dual stats.
// ---------------------------------------------------------------------

const WINDOW_OPTIONS: { value: AnalysisWindow; label: string }[] = [
  { value: "two_weeks", label: "2 weeks" },
  { value: "month", label: "Month" },
  { value: "quarter", label: "Quarter" },
  { value: "half_year", label: "Half year" },
  { value: "year", label: "Year" },
];

function CategoryAnalyzer({ onError }: { onError: (m: string) => void }) {
  const [categories, setCategoriesState] = useState<CategoryView[]>([]);
  const [categoryId, setCategoryId] = useState<number | null>(null);
  const [window, setWindow] = useState<AnalysisWindow>("quarter");
  const [analysis, setAnalysis] = useState<CategoryAnalysis | null>(null);

  useEffect(() => {
    void (async () => {
      try {
        const cats = await listCategories(false);
        setCategoriesState(cats.filter((c) => c.is_active));
      } catch (e) {
        onError(String(e));
      }
    })();
  }, [onError]);

  async function analyze() {
    if (categoryId === null) return;
    try {
      setAnalysis(await analyzeCategory(categoryId, window));
    } catch (e) {
      onError(String(e));
    }
  }

  const slopePerYear = analysis ? analysis.slope_cents_per_month_per_year * 12 : 0;

  return (
    <Section title="Category analyzer">
      <p className="mb-3 text-sm text-graphite-400">
        Pick a category and a window to see what your spending looks like:
        per-transaction stats (typical purchase), per-bucket totals, and a
        regression line over the period.
      </p>
      <div className="grid grid-cols-1 gap-3 md:grid-cols-[1fr_auto_auto]">
        <label className="block">
          <span className="text-xs uppercase tracking-wide text-graphite-400">
            Category
          </span>
          <select
            value={categoryId ?? ""}
            onChange={(e) =>
              setCategoryId(e.target.value === "" ? null : Number(e.target.value))
            }
            className="mt-1 w-full rounded-md border border-graphite-700 bg-graphite-800 px-3 py-2 text-sm text-graphite-100"
          >
            <option value="">— pick one —</option>
            {categories.map((c) => (
              <option key={c.id} value={c.id}>
                {c.name} ({c.kind})
              </option>
            ))}
          </select>
        </label>
        <label className="block">
          <span className="text-xs uppercase tracking-wide text-graphite-400">
            Window
          </span>
          <select
            value={window}
            onChange={(e) => setWindow(e.target.value as AnalysisWindow)}
            className="mt-1 rounded-md border border-graphite-700 bg-graphite-800 px-3 py-2 text-sm text-graphite-100"
          >
            {WINDOW_OPTIONS.map((opt) => (
              <option key={opt.value} value={opt.value}>
                {opt.label}
              </option>
            ))}
          </select>
        </label>
        <div className="flex items-end">
          <button
            onClick={() => void analyze()}
            disabled={categoryId === null}
            className="rounded-md bg-forest-600 px-3 py-2 text-sm font-medium text-graphite-50 hover:bg-forest-500 disabled:opacity-50"
          >
            Analyze
          </button>
        </div>
      </div>
      {analysis && (
        <div className="mt-4 space-y-3">
          <div
            className={`rounded-md border px-3 py-2 text-sm ${
              analysis.direction === "rising"
                ? "border-yellow-600/40 bg-yellow-700/15 text-yellow-100"
                : analysis.direction === "falling"
                  ? "border-forest-600/40 bg-forest-700/15 text-forest-100"
                  : "border-graphite-700 bg-graphite-800 text-graphite-200"
            }`}
          >
            {analysis.headline}
            {analysis.direction !== "flat" && (
              <span className="ml-2 text-xs text-graphite-400">
                ({slopePerYear > 0 ? "+" : ""}
                {formatMoney(Math.round(slopePerYear))}/mo per year, R²=
                {analysis.r_squared.toFixed(2)})
              </span>
            )}
          </div>

          <div className="grid grid-cols-1 gap-3 md:grid-cols-2">
            <div className="rounded-md border border-graphite-700 bg-graphite-800 p-3">
              <div className="text-xs uppercase tracking-wide text-graphite-400">
                Per-transaction
              </div>
              {analysis.per_transaction ? (
                <div className="mt-1 grid grid-cols-2 gap-x-3 gap-y-1 text-xs">
                  <span>n purchases</span>
                  <span className="text-right tabular-nums text-graphite-100">
                    {analysis.per_transaction.n}
                  </span>
                  <span>Mean</span>
                  <span className="text-right tabular-nums text-graphite-100">
                    {formatMoney(analysis.per_transaction.mean_cents)}
                  </span>
                  <span>Median</span>
                  <span className="text-right tabular-nums text-graphite-100">
                    {formatMoney(analysis.per_transaction.median_cents)}
                  </span>
                  <span>σ</span>
                  <span className="text-right tabular-nums text-graphite-100">
                    {formatMoney(analysis.per_transaction.stddev_cents)}
                  </span>
                  <span>Min / Max</span>
                  <span className="text-right tabular-nums text-graphite-100">
                    {formatMoney(analysis.per_transaction.min_cents)} /{" "}
                    {formatMoney(analysis.per_transaction.max_cents)}
                  </span>
                </div>
              ) : (
                <div className="mt-1 text-xs text-graphite-500">
                  Need ≥3 charges in window for stats.
                </div>
              )}
              <div className="mt-2 border-t border-graphite-700 pt-2 text-xs text-graphite-400">
                Net spent: {formatMoney(analysis.refunds.net_spent_cents)}
                {analysis.refunds.count > 0 && (
                  <span className="ml-1 text-graphite-500">
                    ({analysis.refunds.count} refund
                    {analysis.refunds.count === 1 ? "" : "s"},{" "}
                    {formatMoney(analysis.refunds.total_cents)} total)
                  </span>
                )}
              </div>
            </div>
            <div className="rounded-md border border-graphite-700 bg-graphite-800 p-3">
              <div className="text-xs uppercase tracking-wide text-graphite-400">
                Per-bucket ({analysis.bucket_label})
              </div>
              {analysis.per_bucket ? (
                <div className="mt-1 grid grid-cols-2 gap-x-3 gap-y-1 text-xs">
                  <span>Buckets</span>
                  <span className="text-right tabular-nums text-graphite-100">
                    {analysis.per_bucket.n_buckets}
                  </span>
                  <span>Mean</span>
                  <span className="text-right tabular-nums text-graphite-100">
                    {formatMoney(analysis.per_bucket.mean_cents)}
                  </span>
                  <span>Median</span>
                  <span className="text-right tabular-nums text-graphite-100">
                    {formatMoney(analysis.per_bucket.median_cents)}
                  </span>
                  <span>σ</span>
                  <span className="text-right tabular-nums text-graphite-100">
                    {formatMoney(analysis.per_bucket.stddev_cents)}
                  </span>
                  <span>Min / Max</span>
                  <span className="text-right tabular-nums text-graphite-100">
                    {formatMoney(analysis.per_bucket.min_cents)} /{" "}
                    {formatMoney(analysis.per_bucket.max_cents)}
                  </span>
                </div>
              ) : (
                <div className="mt-1 text-xs text-graphite-500">
                  Need ≥3 buckets with data.
                </div>
              )}
            </div>
          </div>

          {analysis.buckets.length >= 2 && (
            <ResponsiveContainer width="100%" height={220}>
              <LineChart
                data={analysis.buckets.map((b) => ({
                  label: b.label,
                  Spend: b.total_cents / 100,
                  Trend:
                    (analysis.slope_cents_per_month_per_year *
                      12 *
                      (b.bucket_index /
                        Math.max(
                          1,
                          analysis.buckets.length - 1,
                        )) +
                      (analysis.per_bucket?.mean_cents ?? 0)) /
                    100,
                }))}
              >
                <CartesianGrid stroke="#2a3138" strokeDasharray="3 3" />
                <XAxis dataKey="label" stroke="#94a3b8" tick={{ fontSize: 10 }} />
                <YAxis
                  stroke="#94a3b8"
                  width={78}
                  tickFormatter={formatYAxisDollars}
                />
                <Tooltip
                  cursor={false}
                  contentStyle={{
                    backgroundColor: "#1a1f24",
                    border: "1px solid #2a3138",
                    borderRadius: 6,
                    color: "#e5e7eb",
                  }}
                  formatter={(v: number) => formatMoney(Math.round(v * 100))}
                />
                <Line
                  type="monotone"
                  dataKey="Spend"
                  stroke="#34d399"
                  strokeWidth={2}
                  dot={false}
                />
                <Line
                  type="monotone"
                  dataKey="Trend"
                  stroke="#facc15"
                  strokeWidth={2}
                  strokeDasharray="4 3"
                  dot={false}
                />
              </LineChart>
            </ResponsiveContainer>
          )}
        </div>
      )}
    </Section>
  );
}

// ---------------------------------------------------------------------
// Scenario tool
// ---------------------------------------------------------------------

function ScenarioTool({ onError }: { onError: (m: string) => void }) {
  const [variables, setVariables] = useState<CategoryView[]>([]);
  const [cuts, setCuts] = useState<Record<number, number>>({});
  const [result, setResult] = useState<ScenarioResult | null>(null);

  useEffect(() => {
    void (async () => {
      try {
        const cats = await listCategories(false);
        const vars = cats.filter(
          (c) =>
            c.kind === "variable" &&
            c.is_active &&
            (c.monthly_target_cents ?? 0) > 0,
        );
        setVariables(vars);
      } catch (e) {
        onError(String(e));
      }
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function recompute(nextCuts: Record<number, number>) {
    try {
      const r = await runScenario(
        Object.entries(nextCuts).map(([id, pct]) => ({
          category_id: Number(id),
          pct_change: pct,
        })),
      );
      setResult(r);
    } catch (e) {
      onError(String(e));
    }
  }

  function setCut(id: number, pct: number) {
    const next = { ...cuts, [id]: pct };
    setCuts(next);
    void recompute(next);
  }

  if (variables.length === 0) {
    return (
      <Section title="Scenario: what if I changed...">
        <p className="text-sm text-graphite-400">
          Set monthly targets on variable categories first (Categories tab) and
          this tool will let you simulate changes.
        </p>
      </Section>
    );
  }

  return (
    <Section title="Scenario: what if I changed...">
      <p className="mb-3 text-sm text-graphite-400">
        Drag the sliders to raise or trim a category by a percent. Annual
        impact shown on the right.
      </p>
      <div className="grid grid-cols-1 gap-6 lg:grid-cols-[2fr_1fr]">
        <div className="space-y-2">
          {variables.map((c) => {
            const pct = cuts[c.id] ?? 0;
            const target = c.monthly_target_cents ?? 0;
            const dollars = (target * (1 + pct / 100)) / 100;
            return (
              <div
                key={c.id}
                className="flex items-center gap-3 rounded-md border border-graphite-700 bg-graphite-800 px-3 py-2"
              >
                <div className="w-32 text-sm text-graphite-200">{c.name}</div>
                <input
                  type="range"
                  min={-100}
                  max={50}
                  step={5}
                  value={pct}
                  onChange={(e) => setCut(c.id, Number(e.target.value))}
                  className="flex-1"
                />
                <div className="w-16 text-right text-sm tabular-nums text-graphite-300">
                  {pct > 0 ? "+" : ""}
                  {pct}%
                </div>
                <div className="w-20 text-right text-sm tabular-nums text-graphite-200">
                  {formatMoney(Math.round(dollars * 100))}
                </div>
              </div>
            );
          })}
        </div>
        <div className="space-y-3 rounded-md border border-graphite-700 bg-graphite-800 p-4">
          <div>
            <div className="text-xs uppercase tracking-wide text-graphite-400">
              Original variable budget
            </div>
            <div className="text-lg tabular-nums text-graphite-200">
              {result
                ? formatMoney(result.original_variable_budget_cents)
                : "—"}
              <span className="ml-1 text-xs text-graphite-500">/mo</span>
            </div>
          </div>
          <div>
            <div className="text-xs uppercase tracking-wide text-graphite-400">
              After your changes
            </div>
            <div className="text-lg tabular-nums text-graphite-100">
              {result
                ? formatMoney(result.adjusted_variable_budget_cents)
                : "—"}
              <span className="ml-1 text-xs text-graphite-500">/mo</span>
            </div>
          </div>
          <div>
            <div
              className={`text-xs uppercase tracking-wide ${
                (result?.savings_per_year_cents ?? 0) >= 0
                  ? "text-forest-300"
                  : "text-yellow-300"
              }`}
            >
              {(result?.savings_per_year_cents ?? 0) >= 0
                ? "Saves per year"
                : "Costs per year"}
            </div>
            <div
              className={`text-2xl font-semibold tabular-nums ${
                (result?.savings_per_year_cents ?? 0) >= 0
                  ? "text-forest-100"
                  : "text-yellow-100"
              }`}
            >
              {result
                ? formatMoney(Math.abs(result.savings_per_year_cents))
                : "—"}
            </div>
          </div>
        </div>
      </div>
    </Section>
  );
}

// ---------------------------------------------------------------------
// Tiny shared bits
// ---------------------------------------------------------------------

function Section({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section className="rounded-lg border border-graphite-700 bg-graphite-900 p-5">
      <h2 className="mb-4 text-base font-semibold text-graphite-100">{title}</h2>
      {children}
    </section>
  );
}

function NumberField({
  label,
  value,
  onChange,
  prefix,
  hint,
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
  prefix?: string;
  hint?: string;
}) {
  return (
    <label className="block">
      <span className="text-xs uppercase tracking-wide text-graphite-400">
        {label}
      </span>
      <div className="mt-1 flex items-center rounded-md border border-graphite-700 bg-graphite-800 px-3">
        {prefix && <span className="pr-1 text-graphite-400">{prefix}</span>}
        <input
          type="text"
          inputMode="decimal"
          value={value}
          onChange={(e) => onChange(e.target.value)}
          className="w-full bg-transparent py-2 text-sm text-graphite-100 outline-none"
        />
      </div>
      {hint && <p className="mt-1 text-xs text-graphite-500">{hint}</p>}
    </label>
  );
}

function NumberSlider({
  label,
  min,
  max,
  step,
  value,
  onChange,
}: {
  label: string;
  min: number;
  max: number;
  step: number;
  value: number;
  onChange: (v: number) => void;
}) {
  return (
    <label className="block">
      <span className="text-xs uppercase tracking-wide text-graphite-400">
        {label}
      </span>
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(e) => onChange(Number(e.target.value))}
        className="mt-1 w-full"
      />
    </label>
  );
}


