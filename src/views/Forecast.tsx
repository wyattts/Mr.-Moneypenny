import { useEffect, useMemo, useState } from "react";
import {
  CartesianGrid,
  Line,
  LineChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";

import {
  listCategories,
  listInvestmentCategories,
  projectInvestment,
  runScenario,
  solveGoalSeek,
} from "@/lib/tauri";
import type {
  CategoryView,
  InvestmentProjection,
  InvestmentSummary,
  ScenarioResult,
} from "@/lib/tauri";
import { ErrorBanner } from "@/wizard/components/Layout";
import { formatMoney } from "@/lib/format";
import { ViewHeader } from "./ViewHeader";

export function Forecast() {
  const [error, setError] = useState<string | null>(null);
  return (
    <div>
      <ViewHeader
        title="Forecast"
        subtitle="Look-forward tools — investment projection, goal-seek, and a what-if for your variable categories. Deterministic for now; Monte Carlo paths land in v0.3.2."
      />
      <div className="space-y-6 px-8 py-6">
        {error && <ErrorBanner>{error}</ErrorBanner>}
        <InvestmentCalculator onError={setError} />
        <GoalSeekTool onError={setError} />
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
  const [projection, setProjection] = useState<InvestmentProjection | null>(null);

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
      const r = await projectInvestment({
        starting_balance_cents: startingCents,
        monthly_contribution_cents: Math.round(monthlyDollars * 100),
        annual_return_pct: returnPct,
        annual_inflation_pct: inflationPct,
        horizon_years: horizonYears,
        trajectory_points: 30,
      });
      setProjection(r);
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
    accounts,
  ]);

  const chartData = useMemo(() => {
    if (!projection) return [];
    const startingCents = Math.round((parseFloat(startingDollars) || 0) * 100);
    const monthlyCents = Math.round((parseFloat(contributionDollars) || 0) * 100);
    return projection.trajectory.map((p) => ({
      year: +(p.month / 12).toFixed(2),
      Nominal: p.nominal_cents / 100,
      Real: p.real_cents / 100,
      Contributions: (startingCents + monthlyCents * p.month) / 100,
    }));
  }, [projection, startingDollars, contributionDollars]);

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
                  <LineChart data={chartData}>
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
                      formatter={(v: number) =>
                        formatMoney(Math.round(v * 100))
                      }
                      labelFormatter={(l) => `Year ${l}`}
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
                  </LineChart>
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

