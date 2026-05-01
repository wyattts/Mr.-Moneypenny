import { useEffect, useMemo, useState } from "react";
import {
  Bar,
  BarChart,
  CartesianGrid,
  Cell,
  Legend,
  Line,
  LineChart,
  Pie,
  PieChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";

import {
  getDashboard,
  getSetupState,
  listAuthorizedChats,
} from "@/lib/tauri";
import type {
  AuthorizedChat,
  DashboardSnapshot,
  RangeArg,
  SetupState,
} from "@/lib/tauri";
import { ViewHeader } from "./ViewHeader";
import { ErrorBanner } from "@/wizard/components/Layout";
import { formatDate, formatDelta, formatMoney } from "@/lib/format";

/**
 * Build a list of the last 12 calendar months (current first, working
 * backwards). Returned as `{year, month, label}` triples. The dashboard
 * dropdown lets users pick any of these; "this month" maps to the
 * first entry.
 */
function lastTwelveMonths(): { year: number; month: number; label: string }[] {
  const now = new Date();
  const out: { year: number; month: number; label: string }[] = [];
  for (let i = 0; i < 12; i++) {
    const d = new Date(now.getFullYear(), now.getMonth() - i, 1);
    const label = d.toLocaleDateString(undefined, {
      year: "numeric",
      month: "long",
    });
    out.push({ year: d.getFullYear(), month: d.getMonth() + 1, label });
  }
  return out;
}

// Forest-green ramp + accents for charts.
const CATEGORY_COLORS = [
  "#3d7a4f",
  "#598e6a",
  "#76a285",
  "#9ebda9",
  "#c5d8cc",
  "#facc15",
  "#fb923c",
  "#f87171",
  "#a78bfa",
  "#60a5fa",
];

const REFRESH_INTERVAL_MS = 5_000;

export function Insights() {
  const [setup, setSetup] = useState<SetupState | null>(null);
  const [members, setMembers] = useState<AuthorizedChat[]>([]);
  const months = useMemo(() => lastTwelveMonths(), []);
  const [selected, setSelected] = useState(() => {
    const m = months[0];
    return m ? `${m.year}-${m.month}` : "";
  });
  const [data, setData] = useState<DashboardSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);

  const currency = setup?.default_currency ?? "USD";
  const locale = setup?.locale ?? null;

  useEffect(() => {
    void boot();
  }, []);

  useEffect(() => {
    void load();
    const t = window.setInterval(() => void load(), REFRESH_INTERVAL_MS);
    return () => window.clearInterval(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selected]);

  async function boot() {
    try {
      const [s, m] = await Promise.all([getSetupState(), listAuthorizedChats()]);
      setSetup(s);
      setMembers(m);
    } catch (e) {
      setError(String(e));
    }
  }

  async function load() {
    const parts = selected.split("-");
    if (parts.length !== 2) return;
    const year = Number(parts[0]);
    const month = Number(parts[1]);
    if (!Number.isFinite(year) || !Number.isFinite(month)) return;
    try {
      const arg: RangeArg = { kind: "month", year, month };
      const snap = await getDashboard(arg);
      setData(snap);
    } catch (e) {
      setError(String(e));
    }
  }

  return (
    <div>
      <ViewHeader
        title="Insights"
        subtitle="Where you stand."
        actions={
          <select
            value={selected}
            onChange={(e) => setSelected(e.target.value)}
            className="rounded-md border border-graphite-600 bg-graphite-800 px-3 py-2 text-sm text-graphite-50"
          >
            {months.map((m) => (
              <option key={`${m.year}-${m.month}`} value={`${m.year}-${m.month}`}>
                {m.label}
              </option>
            ))}
          </select>
        }
      />
      <div className="space-y-6 px-8 py-8">
        {error ? <ErrorBanner>{error}</ErrorBanner> : null}
        {data ? (
          <>
            <KpiStrip data={data} currency={currency} locale={locale} />
            <ChartsRow data={data} currency={currency} locale={locale} />
            <VariableTrajectory data={data} currency={currency} locale={locale} />
            <CategoryBarRow data={data} currency={currency} locale={locale} />
            {members.length > 1 ? (
              <MemberRow data={data} currency={currency} locale={locale} />
            ) : null}
            <DetailRow data={data} currency={currency} locale={locale} />
          </>
        ) : (
          <p className="text-sm text-graphite-400">Loading…</p>
        )}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------
// KPI strip
// ---------------------------------------------------------------------

function KpiStrip({
  data,
  currency,
  locale,
}: {
  data: DashboardSnapshot;
  currency: string;
  locale: string | null;
}) {
  // `isCurrent` is true only when the user is viewing the current
  // calendar month — that's the only state where pacing fields
  // (variable_remaining, daily_allowance, on_pace) are meaningful.
  // Static monthly totals (total_budget / total_remaining / total_spent
  // / over_budget) work for any monthly range, so they render regardless.
  const isCurrent = !!data.period;
  const paceClass = data.kpi.on_pace
    ? "text-forest-200"
    : data.period && data.period.variable_spent_cents > data.period.variable_budget_cents
      ? "text-red-300"
      : "text-yellow-300";
  const totalBudget = data.kpi.total_budget_cents;
  const totalRemaining = data.kpi.total_remaining_cents;
  const totalRemainingClass =
    totalBudget === 0
      ? "text-graphite-50"
      : totalRemaining < 0
        ? "text-red-300"
        : totalRemaining < totalBudget / 10
          ? "text-yellow-300"
          : "text-forest-200";
  return (
    <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-6">
      <KpiCard
        label="Variable remaining"
        primary={isCurrent ? formatMoney(data.kpi.variable_remaining_cents, currency, locale) : "—"}
        secondary={
          isCurrent
            ? `of ${formatMoney(data.fixed_vs_variable.variable_spent_cents + data.kpi.variable_remaining_cents, currency, locale)}`
            : "current month only"
        }
        valueClass={paceClass}
        emphasize
      />
      <KpiCard
        label="Daily allowance"
        primary={
          isCurrent
            ? formatMoney(data.kpi.daily_variable_allowance_cents, currency, locale) + "/day"
            : "—"
        }
        secondary={isCurrent ? `for ${data.kpi.days_remaining} days remaining` : "current month only"}
      />
      <KpiCard
        label="Total budget"
        primary={formatMoney(totalBudget, currency, locale)}
        secondary="fixed + variable monthly targets"
      />
      <KpiCard
        label="Total remaining"
        primary={formatMoney(totalRemaining, currency, locale)}
        secondary={
          totalBudget > 0
            ? `${((data.kpi.total_spent_cents / totalBudget) * 100).toFixed(2)}% of budget spent`
            : "no budget set"
        }
        valueClass={totalRemainingClass}
      />
      <KpiCard
        label="Total spent"
        primary={formatMoney(data.kpi.total_spent_cents, currency, locale)}
        secondary={`${data.category_totals.length} active categories`}
      />
      <KpiCard
        label={isCurrent ? "Status" : "Period"}
        primary={
          isCurrent
            ? data.kpi.on_pace
              ? "On pace"
              : "Trending over"
            : `${formatDate(data.start)} – ${formatDate(data.end)}`
        }
        secondary={
          data.mom_comparison && data.mom_comparison.delta_pct !== null
            ? `vs last month: ${formatDelta(data.mom_comparison.delta_pct)}`
            : ""
        }
        valueClass={isCurrent ? paceClass : "text-graphite-50"}
      />
    </div>
  );
}

function KpiCard({
  label,
  primary,
  secondary,
  valueClass,
  emphasize,
}: {
  label: string;
  primary: string;
  secondary?: string;
  valueClass?: string;
  emphasize?: boolean;
}) {
  return (
    <div
      className={`rounded-lg border p-4 ${
        emphasize ? "border-forest-400 bg-forest-700/20" : "border-graphite-700 bg-graphite-900"
      }`}
    >
      <div className="text-xs uppercase tracking-wide text-graphite-400">{label}</div>
      <div
        className={`mt-1 break-words font-mono text-xl ${valueClass ?? "text-graphite-50"}`}
      >
        {primary}
      </div>
      {secondary ? (
        <div className="mt-1 break-words text-xs text-graphite-500">{secondary}</div>
      ) : null}
    </div>
  );
}

// ---------------------------------------------------------------------
// Charts row
// ---------------------------------------------------------------------

function ChartsRow({
  data,
  currency,
  locale,
}: {
  data: DashboardSnapshot;
  currency: string;
  locale: string | null;
}) {
  const topCats = useMemo(() => {
    const sorted = [...data.category_totals].sort((a, b) => b.total_cents - a.total_cents);
    if (sorted.length <= 8) return sorted;
    const top = sorted.slice(0, 8);
    const otherCents = sorted.slice(8).reduce((acc, c) => acc + c.total_cents, 0);
    return [
      ...top,
      {
        category_id: -1,
        name: "Other",
        kind: "variable" as const,
        total_cents: otherCents,
        monthly_target_cents: null,
      },
    ];
  }, [data.category_totals]);

  const trendData = useMemo(
    () =>
      data.daily_trend.map((p) => ({
        date: p.date,
        Variable: p.variable_cents / 100,
        Fixed: p.fixed_cents / 100,
      })),
    [data.daily_trend],
  );

  return (
    <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
      <ChartPanel title="Spending by category">
        {topCats.length === 0 ? (
          <Empty />
        ) : (
          <ResponsiveContainer width="100%" height={260}>
            <PieChart>
              <Pie
                data={topCats}
                dataKey="total_cents"
                nameKey="name"
                innerRadius={60}
                outerRadius={100}
                stroke="var(--c-graphite-900)"
                strokeWidth={2}
              >
                {topCats.map((_, i) => (
                  <Cell key={i} fill={CATEGORY_COLORS[i % CATEGORY_COLORS.length]} />
                ))}
              </Pie>
              <Tooltip
                formatter={(v: number) => formatMoney(Number(v), currency, locale)}
                contentStyle={tooltipStyle}
                labelStyle={tooltipLabelStyle}
                itemStyle={tooltipItemStyle}
              />
              <Legend wrapperStyle={{ color: "var(--c-graphite-300)", fontSize: 12 }} />
            </PieChart>
          </ResponsiveContainer>
        )}
      </ChartPanel>

      <ChartPanel title="Daily trend">
        {trendData.length === 0 ? (
          <Empty />
        ) : (
          <ResponsiveContainer width="100%" height={260}>
            <LineChart data={trendData}>
              <CartesianGrid stroke={gridStroke} strokeDasharray="3 3" />
              <XAxis
                dataKey="date"
                stroke={axisStroke}
                tick={{ fontSize: 10 }}
                tickFormatter={(d) => d.slice(5)}
              />
              <YAxis stroke={axisStroke} tick={{ fontSize: 10 }} />
              <Tooltip
                formatter={(v: number) =>
                  formatMoney(Math.round(Number(v) * 100), currency, locale)
                }
                contentStyle={tooltipStyle}
                labelStyle={tooltipLabelStyle}
                itemStyle={tooltipItemStyle}
              />
              <Legend wrapperStyle={{ color: "var(--c-graphite-300)", fontSize: 12 }} />
              <Line
                type="monotone"
                dataKey="Variable"
                stroke="#3d7a4f"
                strokeWidth={2}
                dot={false}
              />
              <Line
                type="monotone"
                dataKey="Fixed"
                stroke="var(--c-graphite-400)"
                strokeDasharray="5 5"
                strokeWidth={2}
                dot={false}
              />
            </LineChart>
          </ResponsiveContainer>
        )}
      </ChartPanel>
    </div>
  );
}

function ChartPanel({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="rounded-lg border border-graphite-700 bg-graphite-900 p-4">
      <h3 className="mb-2 text-sm font-semibold text-forest-300">{title}</h3>
      {children}
    </div>
  );
}

// ---------------------------------------------------------------------
// Variable-spending trajectory
//
// Cumulative variable spend per day, plus a least-squares regression
// extrapolating to month-end, plus the variable budget as a flat
// reference line. Tells the user at a glance whether they're on track
// to hit, blow, or undershoot their variable budget.
// ---------------------------------------------------------------------

function VariableTrajectory({
  data,
  currency,
  locale,
}: {
  data: DashboardSnapshot;
  currency: string;
  locale: string | null;
}) {
  const variableBudget = data.kpi.variable_budget_cents;
  const trend = data.daily_trend;
  const isCurrent = !!data.period;
  // For the current month we know "today's day-of-month"; for past
  // months the regression covers every day.
  const todayDom = isCurrent && data.period ? data.period.day_of_month : trend.length;

  const series = useMemo(() => {
    if (trend.length === 0) return [];
    // Build (day, cumulative_variable_dollars) rows. Day index is
    // 1-based so it matches a calendar day-of-month.
    let cum = 0;
    const days = trend.map((p, i) => {
      cum += p.variable_cents;
      return {
        day: i + 1,
        cumulative_cents: cum,
        actual: cum / 100,
      };
    });

    // Linear regression on days the user has actually reached. For
    // current month: days 1..todayDom. For past month: every day.
    const points = days.slice(0, todayDom);
    let slope = 0;
    let intercept = 0;
    if (points.length >= 2) {
      const n = points.length;
      const sumX = points.reduce((acc, p) => acc + p.day, 0);
      const sumY = points.reduce((acc, p) => acc + p.cumulative_cents, 0);
      const sumXY = points.reduce((acc, p) => acc + p.day * p.cumulative_cents, 0);
      const sumX2 = points.reduce((acc, p) => acc + p.day * p.day, 0);
      const denom = n * sumX2 - sumX * sumX;
      if (denom !== 0) {
        slope = (n * sumXY - sumX * sumY) / denom;
        intercept = (sumY - slope * sumX) / n;
      }
    }

    // Compose chart-ready rows. `Trend` is null for days before the
    // user could have generated any signal (so the dashed line
    // doesn't sit at $0 for the whole month).
    const haveTrend = points.length >= 2;
    return days.map((d) => ({
      day: d.day,
      Actual: d.day <= todayDom ? d.actual : null,
      Trend: haveTrend ? Math.max(0, (slope * d.day + intercept) / 100) : null,
      Budget: variableBudget / 100,
    }));
  }, [trend, todayDom, variableBudget]);

  if (series.length === 0) return null;

  const projectedEndOfMonth =
    series.length > 0
      ? (series[series.length - 1] as { Trend: number | null }).Trend ?? null
      : null;
  const subtitleHint =
    isCurrent && projectedEndOfMonth !== null && variableBudget > 0
      ? projectedEndOfMonth * 100 > variableBudget
        ? `Projecting ${formatMoney(Math.round(projectedEndOfMonth * 100), currency, locale)} by month-end — over the ${formatMoney(variableBudget, currency, locale)} variable budget.`
        : `Projecting ${formatMoney(Math.round(projectedEndOfMonth * 100), currency, locale)} by month-end — under the ${formatMoney(variableBudget, currency, locale)} variable budget.`
      : null;

  return (
    <div className="rounded-lg border border-graphite-700 bg-graphite-900 p-4">
      <h3 className="text-sm font-semibold text-forest-300">Variable spending trajectory</h3>
      {subtitleHint ? (
        <p className="mb-2 text-xs text-graphite-400">{subtitleHint}</p>
      ) : (
        <div className="mb-2" />
      )}
      <ResponsiveContainer width="100%" height={260}>
        <LineChart data={series}>
          <CartesianGrid stroke={gridStroke} strokeDasharray="3 3" />
          <XAxis
            dataKey="day"
            stroke={axisStroke}
            tick={{ fontSize: 10 }}
            tickFormatter={(d) => String(d)}
          />
          <YAxis
            stroke={axisStroke}
            tick={{ fontSize: 10 }}
            tickFormatter={(v: number) => formatMoney(Math.round(Number(v) * 100), currency, locale)}
          />
          <Tooltip
            formatter={(v: number) =>
              v == null ? "—" : formatMoney(Math.round(Number(v) * 100), currency, locale)
            }
            labelFormatter={(d) => `Day ${d}`}
            contentStyle={tooltipStyle}
            labelStyle={tooltipLabelStyle}
            itemStyle={tooltipItemStyle}
          />
          <Legend wrapperStyle={{ color: "var(--c-graphite-300)", fontSize: 12 }} />
          <Line
            type="monotone"
            dataKey="Actual"
            stroke="#3d7a4f"
            strokeWidth={2}
            dot={false}
            connectNulls={false}
          />
          <Line
            type="monotone"
            dataKey="Trend"
            stroke="var(--c-graphite-300)"
            strokeDasharray="6 4"
            strokeWidth={2}
            dot={false}
            connectNulls
          />
          <Line
            type="monotone"
            dataKey="Budget"
            stroke="#fb923c"
            strokeDasharray="2 4"
            strokeWidth={1.5}
            dot={false}
          />
        </LineChart>
      </ResponsiveContainer>
    </div>
  );
}

function Empty() {
  return (
    <div className="flex h-[260px] items-center justify-center text-sm text-graphite-500">
      No data yet.
    </div>
  );
}

// Recharts tooltip styling. `contentStyle` styles the wrapper div, but the
// label (category name / X-axis value) and items (data rows) inherit black
// from the chart unless we set them explicitly. We point at CSS variables
// so the tooltip swaps with the theme.
const tooltipStyle = {
  background: "var(--c-graphite-800)",
  border: "1px solid var(--c-graphite-600)",
  color: "var(--c-graphite-50)",
  fontSize: 12,
};
const tooltipLabelStyle = { color: "var(--c-graphite-50)" };
const tooltipItemStyle = { color: "var(--c-graphite-50)" };
const gridStroke = "var(--c-graphite-700)";
const axisStroke = "var(--c-graphite-400)";

// ---------------------------------------------------------------------
// Per-category bar chart
//
// One bar per category that had spend in the selected range, regardless
// of kind (fixed / variable / investing). Coloring rules:
//   - Fixed or variable: graphite-200 by default; turns ORANGE when
//     spend > monthly_target_cents (i.e., over budget).
//   - Investing: light forest green by default; turns DEEP forest green
//     when spend >= monthly_target_cents (i.e., savings goal hit).
// Categories with no monthly_target_cents stay at the default tone for
// their kind (no over/under to compare against).
// ---------------------------------------------------------------------

const BAR_COLOR_FIXED_VARIABLE_DEFAULT = "var(--c-graphite-300)";
const BAR_COLOR_OVER_BUDGET = "#fb923c"; // orange-400
const BAR_COLOR_INVESTING_DEFAULT = "#9ebda9"; // light forest
const BAR_COLOR_INVESTING_GOAL_MET = "#225c34"; // deep forest

function barColor(c: {
  kind: "fixed" | "variable" | "investing";
  total_cents: number;
  monthly_target_cents: number | null;
}): string {
  if (c.kind === "investing") {
    if (c.monthly_target_cents != null && c.total_cents >= c.monthly_target_cents) {
      return BAR_COLOR_INVESTING_GOAL_MET;
    }
    return BAR_COLOR_INVESTING_DEFAULT;
  }
  // fixed | variable
  if (c.monthly_target_cents != null && c.total_cents > c.monthly_target_cents) {
    return BAR_COLOR_OVER_BUDGET;
  }
  return BAR_COLOR_FIXED_VARIABLE_DEFAULT;
}

function CategoryBarRow({
  data,
  currency,
  locale,
}: {
  data: DashboardSnapshot;
  currency: string;
  locale: string | null;
}) {
  const rows = useMemo(
    () =>
      [...data.category_totals]
        .sort((a, b) => b.total_cents - a.total_cents)
        .map((c) => ({
          name: c.name,
          kind: c.kind,
          monthly_target_cents: c.monthly_target_cents,
          total_cents: c.total_cents,
          Spent: c.total_cents / 100,
          fill: barColor(c),
        })),
    [data.category_totals],
  );

  if (rows.length === 0) return null;
  // Vertical layout so long category names don't get truncated. Each
  // row gets a fixed height regardless of how many rows there are, so
  // bars don't balloon when the user has only 1-2 categories with spend.
  const ROW_HEIGHT = 32;
  const BAR_THICKNESS = 18;
  const height = Math.max(160, rows.length * ROW_HEIGHT + 40);

  return (
    <ChartPanel title="Spending by category">
      <ResponsiveContainer width="100%" height={height}>
        <BarChart data={rows} layout="vertical" margin={{ left: 24, right: 16 }}>
          <CartesianGrid stroke={gridStroke} strokeDasharray="3 3" horizontal={false} />
          <XAxis
            type="number"
            stroke={axisStroke}
            tick={{ fontSize: 10 }}
            tickFormatter={(v: number) => formatMoney(Math.round(Number(v) * 100), currency, locale)}
          />
          <YAxis
            type="category"
            dataKey="name"
            stroke={axisStroke}
            tick={{ fontSize: 12 }}
            width={140}
          />
          <Tooltip
            formatter={(v: number, _n, item) => {
              const target = (item.payload as { monthly_target_cents: number | null })
                .monthly_target_cents;
              const spent = formatMoney(Math.round(Number(v) * 100), currency, locale);
              if (target == null) return [spent, "Spent"];
              const tgt = formatMoney(target, currency, locale);
              return [`${spent} / ${tgt}`, "Spent / Budget"];
            }}
            contentStyle={tooltipStyle}
            labelStyle={tooltipLabelStyle}
            itemStyle={tooltipItemStyle}
            // Recharts paints a translucent white rectangle behind the
            // hovered bar by default — looks like a flash on the dark
            // theme. Kill the cursor *and* the active-bar restyle.
            cursor={false}
          />
          <Bar
            dataKey="Spent"
            radius={[0, 4, 4, 0]}
            barSize={BAR_THICKNESS}
            activeBar={false}
          >
            {rows.map((r, i) => (
              <Cell key={i} fill={r.fill} />
            ))}
          </Bar>
        </BarChart>
      </ResponsiveContainer>
    </ChartPanel>
  );
}

// ---------------------------------------------------------------------
// Member spend (only with >1 household members)
// ---------------------------------------------------------------------

function MemberRow({
  data,
  currency,
  locale,
}: {
  data: DashboardSnapshot;
  currency: string;
  locale: string | null;
}) {
  if (data.member_spend.length === 0) return null;
  const chartData = data.member_spend.map((m) => ({
    name: m.display_name,
    Spent: m.total_cents / 100,
  }));
  return (
    <ChartPanel title="Spend by household member">
      <ResponsiveContainer width="100%" height={Math.max(120, data.member_spend.length * 36)}>
        <BarChart data={chartData} layout="vertical" margin={{ left: 60 }}>
          <CartesianGrid stroke={gridStroke} strokeDasharray="3 3" horizontal={false} />
          <XAxis type="number" stroke={axisStroke} tick={{ fontSize: 10 }} />
          <YAxis type="category" dataKey="name" stroke={axisStroke} tick={{ fontSize: 12 }} />
          <Tooltip
            formatter={(v: number) =>
              formatMoney(Math.round(Number(v) * 100), currency, locale)
            }
            contentStyle={tooltipStyle}
            labelStyle={tooltipLabelStyle}
            itemStyle={tooltipItemStyle}
            cursor={false}
          />
          <Bar dataKey="Spent" fill="#3d7a4f" radius={[0, 4, 4, 0]} activeBar={false} />
        </BarChart>
      </ResponsiveContainer>
    </ChartPanel>
  );
}

// ---------------------------------------------------------------------
// Detail row: top 5 + over-budget + upcoming fixed
// ---------------------------------------------------------------------

function DetailRow({
  data,
  currency,
  locale,
}: {
  data: DashboardSnapshot;
  currency: string;
  locale: string | null;
}) {
  return (
    <div className="grid grid-cols-1 gap-4 lg:grid-cols-3">
      <Panel title="Top 5 expenses">
        {data.top_expenses.length === 0 ? (
          <PanelEmpty />
        ) : (
          <ul className="divide-y divide-graphite-700 text-sm">
            {data.top_expenses.map((e) => (
              <li key={e.id} className="flex items-baseline justify-between gap-2 py-2">
                <div className="truncate">
                  <div className="text-graphite-200">{e.description ?? "—"}</div>
                  <div className="text-xs text-graphite-500">{formatDate(e.occurred_at)}</div>
                </div>
                <span className="font-mono text-graphite-50">
                  {formatMoney(e.amount_cents, e.currency, locale)}
                </span>
              </li>
            ))}
          </ul>
        )}
      </Panel>
      <Panel title="Over budget">
        {data.over_budget.length === 0 ? (
          <p className="text-sm text-forest-200">Everything&apos;s on track.</p>
        ) : (
          <ul className="divide-y divide-graphite-700 text-sm">
            {data.over_budget.map((c) => (
              <li
                key={c.category_id}
                className="flex items-baseline justify-between gap-2 py-2"
              >
                <span className="text-graphite-200">{c.name}</span>
                <span className="font-mono text-red-300">
                  +{formatMoney(c.overage_cents, currency, locale)}
                </span>
              </li>
            ))}
          </ul>
        )}
      </Panel>
      <Panel title="Upcoming fixed">
        {data.upcoming_fixed.length === 0 ? (
          <p className="text-sm text-graphite-400">Nothing scheduled.</p>
        ) : (
          <ul className="divide-y divide-graphite-700 text-sm">
            {data.upcoming_fixed.map((u) => (
              <li
                key={u.category_id}
                className="flex items-baseline justify-between gap-2 py-2"
              >
                <span className="text-graphite-200">{u.name}</span>
                <span className="text-xs text-graphite-400">
                  due day {u.recurrence_day_of_month}
                </span>
              </li>
            ))}
          </ul>
        )}
      </Panel>
    </div>
  );
}

function Panel({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="rounded-lg border border-graphite-700 bg-graphite-900 p-4">
      <h3 className="mb-2 text-sm font-semibold text-forest-300">{title}</h3>
      {children}
    </div>
  );
}

function PanelEmpty() {
  return <p className="text-sm text-graphite-500">No data yet.</p>;
}
