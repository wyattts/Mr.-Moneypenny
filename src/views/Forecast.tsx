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
  listCategories,
  listInvestmentCategories,
  monteCarloInvestment,
  projectInvestment,
  runScenario,
  simulatorComputeProbability,
  simulatorHeatmap,
  simulatorSolveRequired,
  solveGoalSeek,
} from "@/lib/tauri";
import type {
  AnalysisWindow,
  CategoryAnalysis,
  CategoryView,
  HeatmapResult,
  InvestmentProjection,
  InvestmentSummary,
  PathBands,
  ProbabilityResult,
  RequiredContributionResult,
  ScenarioResult,
  TargetMode,
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

export function Forecast() {
  const [error, setError] = useState<string | null>(null);
  return (
    <div>
      <ViewHeader
        title="Forecast"
        subtitle="Look-forward tools — investment projection, simulator, goal-seek, scenario sliders, and a category analyzer."
      />
      <div className="space-y-6 px-8 py-6">
        {error && <ErrorBanner>{error}</ErrorBanner>}
        <InvestmentCalculator onError={setError} />
        <Simulator onError={setError} />
        <GoalSeekTool onError={setError} />
        <CategoryAnalyzer onError={setError} />
        <ScenarioTool onError={setError} />
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------
// Investment calculator
// ---------------------------------------------------------------------

const RETURN_PRESETS = [
  { label: "Conservative (HYSA / bonds)", rate: 4 },
  { label: "Balanced (60/40)", rate: 7 },
  { label: "Stock-heavy (S&P 500 historical)", rate: 10 },
];

function InvestmentCalculator({ onError }: { onError: (m: string) => void }) {
  const [accounts, setAccounts] = useState<InvestmentSummary[]>([]);
  const [accountId, setAccountId] = useState<number | "all" | null>(null);
  const [startingDollars, setStartingDollars] = useState("0.00");
  const [contributionDollars, setContributionDollars] = useState("500.00");
  const [returnPct, setReturnPct] = useState(7);
  const [inflationPct, setInflationPct] = useState(2.5);
  const [horizonYears, setHorizonYears] = useState(30);
  const [showContributions, setShowContributions] = useState(false);
  const [showBands, setShowBands] = useState(false);
  const [projection, setProjection] = useState<InvestmentProjection | null>(null);
  const [bands, setBands] = useState<PathBands | null>(null);

  useEffect(() => {
    void (async () => {
      try {
        const list = await listInvestmentCategories();
        setAccounts(list);
        if (list.length > 0 && accountId === null) {
          const first = list[0];
          if (first) {
            setAccountId(first.category_id);
            if (first.avg_monthly_contribution_cents) {
              setContributionDollars(
                ((first.avg_monthly_contribution_cents ?? 0) / 100).toFixed(2),
              );
            }
            setStartingDollars(
              ((first.starting_balance_cents ?? 0) / 100).toFixed(2),
            );
          }
        }
      } catch (e) {
        onError(String(e));
      }
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const selectedAccount = useMemo(() => {
    if (accountId === "all" || accountId === null) return null;
    return accounts.find((a) => a.category_id === accountId) ?? null;
  }, [accounts, accountId]);

  const aggregate = useMemo(() => {
    if (accountId !== "all") return null;
    const startingTotal = accounts.reduce(
      (acc, a) => acc + (a.starting_balance_cents ?? 0),
      0,
    );
    const avgTotal = accounts.reduce(
      (acc, a) => acc + (a.avg_monthly_contribution_cents ?? 0),
      0,
    );
    return { startingTotal, avgTotal };
  }, [accounts, accountId]);

  // Auto-prefill contribution + starting balance when switching accounts.
  // The starting-balance input is editable — switching accounts overwrites
  // it (matches the contribution behavior). The user can then tweak.
  useEffect(() => {
    if (accountId === "all" && aggregate) {
      setContributionDollars((aggregate.avgTotal / 100).toFixed(2));
      setStartingDollars((aggregate.startingTotal / 100).toFixed(2));
    } else if (selectedAccount) {
      if (selectedAccount.avg_monthly_contribution_cents) {
        setContributionDollars(
          (selectedAccount.avg_monthly_contribution_cents / 100).toFixed(2),
        );
      }
      setStartingDollars(
        ((selectedAccount.starting_balance_cents ?? 0) / 100).toFixed(2),
      );
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [accountId]);

  async function recompute() {
    try {
      const startingDollarsNum = parseFloat(startingDollars) || 0;
      const startingCents = Math.round(startingDollarsNum * 100);
      const monthlyDollars = parseFloat(contributionDollars) || 0;
      const monthlyCents = Math.round(monthlyDollars * 100);
      const [proj, mc] = await Promise.all([
        projectInvestment({
          starting_balance_cents: startingCents,
          monthly_contribution_cents: monthlyCents,
          annual_return_pct: returnPct,
          annual_inflation_pct: inflationPct,
          horizon_years: horizonYears,
          trajectory_points: 30,
        }),
        showBands
          ? monteCarloInvestment({
              starting_balance_cents: startingCents,
              monthly_contribution_cents: monthlyCents,
              annual_return_pct: returnPct,
              annual_volatility_pct: volatilityForReturn(returnPct),
              horizon_years: horizonYears,
              n_paths: 1000,
              time_points: 30,
              seed: null,
            })
          : Promise.resolve(null as PathBands | null),
      ]);
      setProjection(proj);
      setBands(mc);
    } catch (e) {
      onError(String(e));
    }
  }

  // Recompute when inputs change.
  useEffect(() => {
    if (accountId === null) return;
    void recompute();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    accountId,
    startingDollars,
    contributionDollars,
    returnPct,
    inflationPct,
    horizonYears,
    showBands,
    accounts,
  ]);

  const chartData = useMemo(() => {
    if (!projection) return [];
    const startingCents = Math.round((parseFloat(startingDollars) || 0) * 100);
    const monthlyCents = Math.round((parseFloat(contributionDollars) || 0) * 100);
    // Map Monte Carlo bands by month for fast lookup.
    const byMonth = new Map<number, PathBands["points"][number]>();
    if (bands) for (const pt of bands.points) byMonth.set(pt.month, pt);
    return projection.trajectory.map((p) => {
      const mc = byMonth.get(p.month);
      return {
        year: +(p.month / 12).toFixed(2),
        Nominal: p.nominal_cents / 100,
        Real: p.real_cents / 100,
        Contributions: (startingCents + monthlyCents * p.month) / 100,
        // Stack-friendly band keys for Recharts: render an invisible
        // bottom area at p10, then a filled area on top with span
        // (p90 - p10). Plus we expose p10 / p90 directly so the
        // tooltip can show the LCL/UCL dollar values.
        band_lo: mc ? mc.p10 / 100 : null,
        band_span: mc ? (mc.p90 - mc.p10) / 100 : null,
        Lower: mc ? mc.p10 / 100 : null,
        Upper: mc ? mc.p90 / 100 : null,
      };
    });
  }, [projection, startingDollars, contributionDollars, bands]);

  if (accounts.length === 0) {
    return (
      <Section title="Investment calculator">
        <p className="text-sm text-graphite-400">
          No investing-kind categories yet. Activate <em>Savings</em>,{" "}
          <em>401k</em>, <em>Roth IRA</em>, or <em>Investing</em> in Categories
          (or create your own with kind = investing) to project them here.
        </p>
      </Section>
    );
  }

  return (
    <Section title="Investment calculator">
      <div className="grid grid-cols-1 gap-4 lg:grid-cols-[1fr_2fr]">
        <div className="space-y-3">
          <label className="block text-xs uppercase tracking-wide text-graphite-400">
            Account
            <select
              value={accountId === "all" ? "all" : (accountId ?? "")}
              onChange={(e) =>
                setAccountId(
                  e.target.value === "all" ? "all" : Number(e.target.value),
                )
              }
              className="mt-1 w-full rounded-md border border-graphite-700 bg-graphite-800 px-3 py-2 text-sm text-graphite-100"
            >
              <option value="all">All investing accounts (sum)</option>
              {accounts.map((a) => (
                <option key={a.category_id} value={a.category_id}>
                  {a.name}
                </option>
              ))}
            </select>
          </label>
          <NumberField
            label="Starting balance"
            value={startingDollars}
            onChange={setStartingDollars}
            prefix="$"
            hint={
              accountId === "all"
                ? `Sum of all account starting balances: ${formatMoney(aggregate?.startingTotal ?? 0)}`
                : (selectedAccount?.starting_balance_cents ?? 0) > 0
                  ? `From your saved balance — edit to test scenarios.`
                  : "Already invested? Enter it here. The projection compounds it alongside your contributions."
            }
          />
          <NumberField
            label="Monthly contribution"
            value={contributionDollars}
            onChange={setContributionDollars}
            prefix="$"
            hint={
              accountId === "all"
                ? `Sum of last 12 months avg: ${formatMoney(aggregate?.avgTotal ?? 0)}/mo`
                : selectedAccount?.avg_monthly_contribution_cents
                  ? `12-mo avg: ${formatMoney(selectedAccount.avg_monthly_contribution_cents)}/mo`
                  : "No history yet — enter what you plan to contribute."
            }
          />
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
          <NumberSlider
            label={`Time horizon: ${horizonYears} years`}
            min={1}
            max={50}
            step={1}
            value={horizonYears}
            onChange={setHorizonYears}
          />
          <p className="text-xs text-graphite-500">
            Projections illustrate one possible path under the given assumptions.
            Past returns don&apos;t predict future returns; you can lose money.
            Not financial advice.
          </p>
        </div>
        <div>
          {projection && (
            <>
              <div className="grid grid-cols-3 gap-3">
                <ResultCard
                  label="Final value (nominal)"
                  value={projection.final_nominal_cents}
                />
                <ResultCard
                  label={`Final value (in today's $${
                    inflationPct > 0 ? `, ${inflationPct}% inflation` : ""
                  })`}
                  value={projection.final_real_cents}
                />
                <ResultCard
                  label="Of that, growth"
                  value={projection.total_growth_cents}
                  subtle={`+ ${formatMoney(projection.total_contributed_cents)} contributed`}
                />
              </div>
              <div className="mt-4">
                <ResponsiveContainer width="100%" height={280}>
                  <ComposedChart data={chartData}>
                    <CartesianGrid stroke="#2a3138" strokeDasharray="3 3" />
                    <XAxis
                      dataKey="year"
                      stroke="#94a3b8"
                      tickFormatter={(v) => `${v}y`}
                    />
                    <YAxis
                      stroke="#94a3b8"
                      tickFormatter={(v) =>
                        v >= 1000 ? `$${(v / 1000).toFixed(0)}k` : `$${v.toFixed(0)}`
                      }
                    />
                    <Tooltip
                      cursor={false}
                      contentStyle={{
                        backgroundColor: "#1a1f24",
                        border: "1px solid #2a3138",
                        borderRadius: 6,
                        color: "#e5e7eb",
                      }}
                      formatter={(v: number, name: string) => {
                        // Hide the internal stacking keys; the user
                        // sees Lower/Upper instead, which carry the
                        // band's actual LCL/UCL values.
                        if (name === "band_lo" || name === "band_span")
                          return null as unknown as string;
                        return formatMoney(Math.round(v * 100));
                      }}
                      labelFormatter={(l) => `Year ${l}`}
                    />
                    {showBands && (
                      <>
                        <Area
                          type="monotone"
                          dataKey="band_lo"
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
                          fill="#34d399"
                          fillOpacity={0.18}
                          isAnimationActive={false}
                          legendType="none"
                          activeDot={false}
                        />
                        {/* Invisible Lower/Upper lines so the tooltip
                            picks them up by name with the actual
                            band-edge $ values. They share fully-
                            transparent stroke so no extra line
                            renders on the chart. */}
                        <Line
                          type="monotone"
                          dataKey="Lower"
                          stroke="transparent"
                          dot={false}
                          activeDot={false}
                          legendType="none"
                          isAnimationActive={false}
                        />
                        <Line
                          type="monotone"
                          dataKey="Upper"
                          stroke="transparent"
                          dot={false}
                          activeDot={false}
                          legendType="none"
                          isAnimationActive={false}
                        />
                      </>
                    )}
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
                  {showBands && bands && bands.points.length > 0 && (
                    <span className="flex items-center gap-1.5">
                      <span className="inline-block h-2 w-4 rounded-sm bg-forest-400/30" />{" "}
                      80% band ({formatMoney(bands.points[bands.points.length - 1]!.p10)}{" "}
                      – {formatMoney(bands.points[bands.points.length - 1]!.p90)} at year{" "}
                      {horizonYears}). Hover for per-year values.
                    </span>
                  )}
                  <div className="ml-auto flex flex-wrap gap-3">
                    <label className="flex cursor-pointer items-center gap-1.5 select-none">
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
                    <label className="flex cursor-pointer items-center gap-1.5 select-none">
                      <input
                        type="checkbox"
                        checked={showBands}
                        onChange={(e) => setShowBands(e.target.checked)}
                        className="h-3 w-3 accent-forest-500"
                      />
                      <span className="flex items-center gap-1.5">
                        <span className="inline-block h-2 w-4 rounded-sm bg-forest-400/30" />{" "}
                        80% probability bands
                      </span>
                    </label>
                  </div>
                </div>
              </div>
            </>
          )}
        </div>
      </div>
    </Section>
  );
}

// ---------------------------------------------------------------------
// Goal-seek
// ---------------------------------------------------------------------

function GoalSeekTool({ onError }: { onError: (m: string) => void }) {
  const [target, setTarget] = useState("100000");
  const [horizon, setHorizon] = useState(20);
  const [returnPct, setReturnPct] = useState(7);
  const [startingBalance, setStartingBalance] = useState("0");
  const [result, setResult] = useState<{
    monthly: number;
    onTrack: boolean;
  } | null>(null);

  async function compute() {
    try {
      const r = await solveGoalSeek({
        target_cents: Math.round((parseFloat(target) || 0) * 100),
        starting_balance_cents: Math.round(
          (parseFloat(startingBalance) || 0) * 100,
        ),
        annual_return_pct: returnPct,
        horizon_years: horizon,
      });
      setResult({
        monthly: r.required_monthly_cents,
        onTrack: r.already_on_track,
      });
    } catch (e) {
      onError(String(e));
    }
  }
  useEffect(() => {
    void compute();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [target, horizon, returnPct, startingBalance]);

  return (
    <Section title="Goal-seek">
      <p className="mb-3 text-sm text-graphite-400">
        How much do I need to save monthly to hit a target?
      </p>
      <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
        <div className="space-y-3">
          <NumberField
            label="Target amount"
            value={target}
            onChange={setTarget}
            prefix="$"
          />
          <NumberField
            label="Starting balance"
            value={startingBalance}
            onChange={setStartingBalance}
            prefix="$"
          />
          <NumberSlider
            label={`In ${horizon} years`}
            min={1}
            max={50}
            step={1}
            value={horizon}
            onChange={setHorizon}
          />
          <NumberSlider
            label={`Annual return: ${returnPct.toFixed(2)}%`}
            min={0}
            max={15}
            step={0.5}
            value={returnPct}
            onChange={setReturnPct}
          />
        </div>
        <div className="flex flex-col items-center justify-center rounded-md border border-graphite-700 bg-graphite-800 p-6">
          {result && (
            <>
              {result.onTrack ? (
                <div className="text-center">
                  <div className="text-xs uppercase tracking-wide text-forest-300">
                    Already on track
                  </div>
                  <div className="mt-2 text-sm text-graphite-300">
                    Your starting balance compounds past the target on its own.
                  </div>
                </div>
              ) : (
                <>
                  <div className="text-xs uppercase tracking-wide text-graphite-400">
                    Required monthly
                  </div>
                  <div className="mt-1 text-3xl font-semibold tabular-nums text-graphite-50">
                    {formatMoney(result.monthly)}
                  </div>
                  <div className="mt-2 text-xs text-graphite-500">
                    to hit {formatMoney(Math.round((parseFloat(target) || 0) * 100))} in{" "}
                    {horizon} years at {returnPct.toFixed(2)}%
                  </div>
                </>
              )}
            </>
          )}
        </div>
      </div>
    </Section>
  );
}

// ---------------------------------------------------------------------
// Forecast Simulator (v0.3.3) — bidirectional Monte Carlo.
// ---------------------------------------------------------------------

type SimMode = "required" | "probability";

function Simulator({ onError }: { onError: (m: string) => void }) {
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

  const [required, setRequired] = useState<RequiredContributionResult | null>(null);
  const [probability, setProbability] = useState<ProbabilityResult | null>(null);
  const [heatmap, setHeatmap] = useState<HeatmapResult | null>(null);

  const sigma = sigmaOverride ?? volatilityForReturn(returnPct);
  const targetCents = Math.round((parseFloat(targetDollars) || 0) * 100);
  const startingCents = Math.round((parseFloat(startingDollars) || 0) * 100);
  const contribCents = Math.round((parseFloat(contributionDollars) || 0) * 100);

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

  // Recompute the active solver + heatmap when inputs change. Heatmap
  // axes anchor on the current solver's answer so users see "what if"
  // around the answer they just got.
  useEffect(() => {
    let cancelled = false;
    const run = async () => {
      try {
        if (mode === "required") {
          const r = await simulatorSolveRequired({ ...common, confidence });
          if (cancelled) return;
          setRequired(r);
          setProbability(null);
          // Heatmap centered on the answer, X spans 0..2× answer,
          // Y spans 1..2× horizon (capped at 50).
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
    const lo = (required?.final_p10_cents ?? probability?.final_p10_cents) ?? null;
    const mid = (required?.final_p50_cents ?? probability?.final_p50_cents) ?? null;
    const hi = (required?.final_p90_cents ?? probability?.final_p90_cents) ?? null;
    if (lo === null || mid === null || hi === null) return null;
    return { lo, mid, hi };
  }, [required, probability]);

  return (
    <Section title="Simulator">
      <p className="mb-3 text-sm text-graphite-400">
        Find the contribution that hits a target with a chosen confidence,
        or check the probability of a contribution you&apos;re already
        considering. Swap modes any time. The heatmap below answers the
        broader trade-off: how do contribution and horizon together affect
        your odds?
      </p>

      <div className="mb-4 inline-flex rounded-md border border-graphite-700 bg-graphite-800 p-0.5 text-sm">
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
          {histogram && (
            <div className="rounded-md border border-graphite-700 bg-graphite-800 p-3 text-xs text-graphite-300">
              <div className="mb-1 text-xs uppercase tracking-wide text-graphite-500">
                Final-value distribution (1,000 paths)
              </div>
              <div className="flex items-baseline justify-between gap-3">
                <span>
                  P10:{" "}
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
                  P90:{" "}
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
        <table className="text-[10px] tabular-nums text-graphite-400">
          <thead>
            <tr>
              <th className="px-1 py-0.5 text-right text-graphite-500">y\$</th>
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
                    style={{ backgroundColor: heatmapColor(c.probability) }}
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
                  tickFormatter={(v) =>
                    v >= 1000 ? `$${(v / 1000).toFixed(0)}k` : `$${v.toFixed(0)}`
                  }
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

function ResultCard({
  label,
  value,
  subtle,
}: {
  label: string;
  value: number;
  subtle?: string;
}) {
  return (
    <div className="rounded-md border border-graphite-700 bg-graphite-800 p-3">
      <div className="text-xs uppercase tracking-wide text-graphite-400">
        {label}
      </div>
      <div className="mt-1 text-xl font-semibold tabular-nums text-graphite-100">
        {formatMoney(value)}
      </div>
      {subtle && (
        <div className="mt-0.5 text-xs text-graphite-500">{subtle}</div>
      )}
    </div>
  );
}

