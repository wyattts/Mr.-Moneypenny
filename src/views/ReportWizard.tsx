// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Wyatt Smith and contributors
//
// AI Report Wizard (v0.4.0). Lives at the bottom of the Insights tab.
// Deterministic figures are computed in Rust; this only collects the
// request, runs the pre-flight cost estimate + confirm, and renders the
// structured report. PDF export is wired in Phase 4.
import { useMemo, useState } from "react";

import { save } from "@tauri-apps/plugin-dialog";

import {
  reportEstimate,
  reportGenerate,
  reportSavePdf,
  type GeneratedReportResponse,
  type ReportRequest,
  type ReportSections,
  type ReportTimeframe,
} from "@/lib/tauri";
import { formatMoney } from "@/lib/format";
import { buildReportPdfBase64 } from "@/lib/reportPdf";

/** Micro-dollars → display string, matching the Rust precision buckets. */
function fmtMicros(micros: number): string {
  const d = micros / 1_000_000;
  const abs = Math.abs(d);
  if (abs >= 1) return `$${d.toFixed(2)}`;
  if (abs >= 0.01) return `$${d.toFixed(3)}`;
  return `$${d.toFixed(4)}`;
}

type PresetKind = "last_week" | "last_month" | "last_quarter" | "last_year";

const PRESETS: { kind: PresetKind; label: string }[] = [
  { kind: "last_week", label: "Last week" },
  { kind: "last_month", label: "Last month" },
  { kind: "last_quarter", label: "Last quarter" },
  { kind: "last_year", label: "Last year" },
];

const SECTION_DEFS: { key: keyof ReportSections; label: string; hint: string }[] =
  [
    {
      key: "rebalance",
      label: "Spending rebalance",
      hint: "Which budgets to raise or lower based on what you actually spent.",
    },
    {
      key: "spend_cycles",
      label: "Variable spend cycles",
      hint: "What you tend to spend on early/mid/late in the week and month.",
    },
    {
      key: "cuts",
      label: "Suggested cuts",
      hint: "The discretionary categories with the most room to trim.",
    },
    {
      key: "subscriptions",
      label: "Subscription radar",
      hint: "Recurring charges worth reviewing.",
    },
    {
      key: "savings",
      label: "Savings rate",
      hint: "Income vs. spend for the period (needs monthly income below).",
    },
    {
      key: "anomalies",
      label: "Anomalies & trends",
      hint: "Categories that moved a lot vs. the prior equal period.",
    },
    {
      key: "wins",
      label: "Wins",
      hint: "Where you came in under budget or improved.",
    },
  ];

function Tooltip({ text }: { text: string }) {
  return (
    <span
      className="ml-1 cursor-help text-graphite-500"
      title={text}
      aria-label={text}
    >
      ⓘ
    </span>
  );
}

export function ReportWizard({
  provider,
  currency,
  locale,
}: {
  provider: string | null;
  currency: string;
  locale: string | null;
}) {
  const isOllama = provider === "ollama";

  const [preset, setPreset] = useState<PresetKind | "custom">("last_month");
  const [customFrom, setCustomFrom] = useState("");
  const [customTo, setCustomTo] = useState("");
  const [sections, setSections] = useState<ReportSections>({
    rebalance: true,
    spend_cycles: true,
    cuts: true,
    subscriptions: false,
    savings: false,
    anomalies: false,
    wins: false,
  });
  const [incomeStr, setIncomeStr] = useState("");
  // Anthropic: opt-in & off by default. Ollama: local, always on, hidden.
  const [merchantOptIn, setMerchantOptIn] = useState(false);
  const [blurbEnabled, setBlurbEnabled] = useState(false);
  const [blurb, setBlurb] = useState("");
  const [goalsSummary, setGoalsSummary] = useState(false);

  const [estimate, setEstimate] = useState<{
    micros: number;
    today: number;
    cap: number;
    exceed: boolean;
    model: string;
    prov: string;
  } | null>(null);
  const [busy, setBusy] = useState<null | "estimating" | "generating">(null);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<GeneratedReportResponse | null>(null);

  const anySection = useMemo(
    () => Object.values(sections).some(Boolean),
    [sections],
  );
  const customValid =
    preset !== "custom" || (customFrom !== "" && customTo !== "" && customFrom <= customTo);

  function buildRequest(): ReportRequest {
    const timeframe: ReportTimeframe =
      preset === "custom"
        ? { kind: "custom", from: customFrom, to: customTo }
        : { kind: preset };
    const includeMerchants = isOllama ? true : merchantOptIn;
    const incomeCents = (() => {
      const n = Number.parseFloat(incomeStr);
      return Number.isFinite(n) && n > 0 ? Math.round(n * 100) : undefined;
    })();
    const trimmedBlurb = blurbEnabled ? blurb.trim() : "";
    const req: ReportRequest = {
      timeframe,
      sections,
      include_merchant_samples: includeMerchants,
      include_goals_summary: blurbEnabled && goalsSummary && trimmedBlurb !== "",
    };
    if (trimmedBlurb !== "") req.blurb = trimmedBlurb;
    if (incomeCents !== undefined) req.monthly_income_cents = incomeCents;
    return req;
  }

  async function onEstimate() {
    setError(null);
    setResult(null);
    setBusy("estimating");
    try {
      const e = await reportEstimate(buildRequest());
      setEstimate({
        micros: e.estimate_micros,
        today: e.today_micros,
        cap: e.cap_micros,
        exceed: e.would_exceed_cap,
        model: e.model,
        prov: e.provider,
      });
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(null);
    }
  }

  async function onGenerate() {
    setError(null);
    setBusy("generating");
    try {
      const r = await reportGenerate(buildRequest());
      setResult(r);
      setEstimate(null);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(null);
    }
  }

  return (
    <section className="rounded-md border border-graphite-700 bg-graphite-900/40 p-5">
      <h2 className="text-lg font-semibold text-graphite-50">AI report</h2>
      <p className="mt-1 text-sm text-graphite-400">
        Generate a written analysis of a period. Numbers are computed
        locally; {isOllama ? "your local Ollama model" : "Claude (Sonnet)"}{" "}
        writes the narrative.
      </p>

      <div className="mt-4 grid grid-cols-1 gap-5 lg:grid-cols-2">
        {/* Timeframe */}
        <div>
          <div className="text-xs uppercase tracking-wide text-graphite-400">
            Timeframe
          </div>
          <div className="mt-2 flex flex-wrap gap-2" role="group" aria-label="Report timeframe">
            {PRESETS.map((p) => (
              <button
                key={p.kind}
                onClick={() => setPreset(p.kind)}
                aria-pressed={preset === p.kind}
                className={`rounded-md border px-3 py-1 text-sm transition ${
                  preset === p.kind
                    ? "border-forest-500 bg-forest-700/30 text-forest-100"
                    : "border-graphite-700 text-graphite-300 hover:border-graphite-500"
                }`}
              >
                {p.label}
              </button>
            ))}
            <button
              onClick={() => setPreset("custom")}
              aria-pressed={preset === "custom"}
              className={`rounded-md border px-3 py-1 text-sm transition ${
                preset === "custom"
                  ? "border-forest-500 bg-forest-700/30 text-forest-100"
                  : "border-graphite-700 text-graphite-300 hover:border-graphite-500"
              }`}
            >
              Custom
            </button>
          </div>
          {preset === "custom" && (
            <div className="mt-2 flex items-center gap-2 text-sm">
              <input
                type="date"
                value={customFrom}
                onChange={(e) => setCustomFrom(e.target.value)}
                aria-label="Custom range start date"
                className="rounded border border-graphite-700 bg-graphite-800 px-2 py-1 text-graphite-100"
              />
              <span className="text-graphite-500">to</span>
              <input
                type="date"
                value={customTo}
                onChange={(e) => setCustomTo(e.target.value)}
                aria-label="Custom range end date"
                className="rounded border border-graphite-700 bg-graphite-800 px-2 py-1 text-graphite-100"
              />
            </div>
          )}
        </div>

        {/* Sections */}
        <div>
          <div className="text-xs uppercase tracking-wide text-graphite-400">
            Include in report
          </div>
          <div className="mt-2 grid grid-cols-1 gap-1.5 sm:grid-cols-2">
            {SECTION_DEFS.map((s) => (
              <label
                key={s.key}
                className="flex cursor-pointer items-center gap-2 text-sm text-graphite-200"
              >
                <input
                  type="checkbox"
                  checked={sections[s.key]}
                  onChange={(e) =>
                    setSections((prev) => ({
                      ...prev,
                      [s.key]: e.target.checked,
                    }))
                  }
                />
                {s.label}
                <Tooltip text={s.hint} />
              </label>
            ))}
          </div>
          {sections.savings && (
            <label className="mt-2 block text-xs text-graphite-400">
              Monthly income (for savings rate)
              <input
                type="text"
                inputMode="decimal"
                value={incomeStr}
                onChange={(e) => setIncomeStr(e.target.value)}
                placeholder="e.g. 5000"
                aria-label="Monthly income in dollars"
                className="mt-1 w-32 rounded border border-graphite-700 bg-graphite-800 px-2 py-1 text-sm text-graphite-100"
              />
            </label>
          )}
        </div>
      </div>

      {/* Merchant opt-in (Anthropic only — Ollama is local) */}
      {!isOllama && (
        <label className="mt-4 flex items-start gap-2 text-sm text-graphite-200">
          <input
            type="checkbox"
            checked={merchantOptIn}
            onChange={(e) => setMerchantOptIn(e.target.checked)}
            className="mt-0.5"
          />
          <span>
            Include merchant names for more specific suggestions
            <Tooltip text="Your top merchant/description labels for the discussed categories would be sent to Anthropic's API along with the figures. Leave off to keep merchant names on your device; aggregate analysis still works." />
          </span>
        </label>
      )}

      {/* Optional financial-situation blurb */}
      <div className="mt-4">
        <label className="flex items-center gap-2 text-sm text-graphite-200">
          <input
            type="checkbox"
            checked={blurbEnabled}
            onChange={(e) => setBlurbEnabled(e.target.checked)}
          />
          Add a note about my financial situation
          <Tooltip
            text={
              isOllama
                ? "Household size, area, income, goals — stays on your machine (local Ollama)."
                : "Household size, area, income, goals. This text is sent to Anthropic's API to generate the report."
            }
          />
        </label>
        {blurbEnabled && (
          <div className="mt-2">
            <textarea
              value={blurb}
              onChange={(e) => setBlurb(e.target.value)}
              rows={3}
              maxLength={2000}
              placeholder="e.g. Household of 3 in a high cost-of-living metro, ~$110k income, saving for a down payment and maxing a Roth IRA."
              aria-label="Financial situation note"
              className="w-full rounded border border-graphite-700 bg-graphite-800 px-3 py-2 text-sm text-graphite-100"
            />
            {!isOllama && (
              <p className="mt-1 text-[11px] text-yellow-200/80">
                This note is sent to Anthropic&apos;s API for report
                generation.
              </p>
            )}
            <label className="mt-2 flex items-center gap-2 text-sm text-graphite-200">
              <input
                type="checkbox"
                checked={goalsSummary}
                onChange={(e) => setGoalsSummary(e.target.checked)}
              />
              Compare my spending to my goals and area
            </label>
          </div>
        )}
      </div>

      {/* Actions */}
      <div className="mt-5 flex flex-wrap items-center gap-3">
        {!estimate ? (
          <button
            onClick={onEstimate}
            disabled={!anySection || !customValid || busy !== null}
            className="rounded-md bg-forest-600 px-4 py-2 text-sm font-medium text-graphite-50 transition hover:bg-forest-500 disabled:cursor-not-allowed disabled:opacity-50"
          >
            {busy === "estimating" ? "Estimating…" : "Estimate cost"}
          </button>
        ) : (
          <div className="flex flex-wrap items-center gap-3 rounded-md border border-graphite-700 bg-graphite-800 px-4 py-3">
            <span className="text-sm text-graphite-200">
              {estimate.prov === "ollama" ? (
                <>Local generation — no API cost.</>
              ) : (
                <>
                  Estimated cost{" "}
                  <strong className="tabular-nums text-graphite-50">
                    {fmtMicros(estimate.micros)}
                  </strong>{" "}
                  ({estimate.model}).{" "}
                  <span className="text-graphite-400">
                    Today {fmtMicros(estimate.today)}
                    {estimate.cap >= 0
                      ? ` of ${fmtMicros(estimate.cap)} cap`
                      : " (no cap)"}
                    .
                  </span>
                </>
              )}
            </span>
            {estimate.exceed && (
              <span className="text-xs text-yellow-200">
                ⚠ This would exceed today&apos;s cost cap.
              </span>
            )}
            <button
              onClick={onGenerate}
              disabled={busy !== null}
              className="rounded-md bg-forest-600 px-4 py-2 text-sm font-medium text-graphite-50 transition hover:bg-forest-500 disabled:opacity-50"
            >
              {busy === "generating" ? "Generating…" : "Confirm & generate"}
            </button>
            <button
              onClick={() => setEstimate(null)}
              disabled={busy !== null}
              className="text-sm text-graphite-400 hover:text-graphite-200"
            >
              Cancel
            </button>
          </div>
        )}
      </div>

      {error && (
        <div className="mt-4 rounded-md border border-red-700/50 bg-red-900/20 px-4 py-3 text-sm text-red-200">
          {error}
        </div>
      )}

      {result && (
        <ReportView result={result} currency={currency} locale={locale} />
      )}
    </section>
  );
}

function ReportView({
  result,
  currency,
  locale,
}: {
  result: GeneratedReportResponse;
  currency: string;
  locale: string | null;
}) {
  const { report, figures } = result;
  const [pdfBusy, setPdfBusy] = useState(false);
  const [pdfError, setPdfError] = useState<string | null>(null);

  async function onDownloadPdf() {
    setPdfError(null);
    setPdfBusy(true);
    try {
      const slug = result.timeframe_label.replace(/[^0-9A-Za-z]+/g, "-");
      const path = await save({
        defaultPath: `moneypenny-report-${slug}.pdf`,
        filters: [{ name: "PDF", extensions: ["pdf"] }],
      });
      if (!path) return; // user cancelled the save dialog
      const b64 = await buildReportPdfBase64(result, currency, locale);
      await reportSavePdf(path, b64);
    } catch (e) {
      setPdfError(String(e));
    } finally {
      setPdfBusy(false);
    }
  }

  return (
    <article className="mt-6 rounded-md border border-graphite-700 bg-graphite-800 p-6">
      <header className="flex items-start justify-between gap-4 border-b border-graphite-700 pb-3">
        <div>
          <h3 className="text-xl font-semibold text-graphite-50">
            Spending report — {result.timeframe_label}
          </h3>
          <p className="mt-1 text-xs text-graphite-400">
          {result.provider === "ollama"
            ? `Generated locally (${result.model})`
            : `Generated with ${result.model}`}
          {" · "}
          {result.provider === "ollama"
            ? "no API cost"
            : `API cost ${fmtMicros(result.cost_micros)}`}
          {" · "}
          total spend{" "}
          {formatMoney(figures.total_spent_cents, currency, locale)} over{" "}
          {figures.days} days
          </p>
        </div>
        <div className="shrink-0 text-right">
          <button
            onClick={onDownloadPdf}
            disabled={pdfBusy}
            className="rounded-md border border-graphite-600 px-3 py-1.5 text-sm text-graphite-200 transition hover:border-graphite-400 disabled:opacity-50"
          >
            {pdfBusy ? "Saving…" : "Download PDF"}
          </button>
          {pdfError && (
            <p className="mt-1 max-w-[16rem] text-[11px] text-red-300">
              {pdfError}
            </p>
          )}
        </div>
      </header>

      {report.overall_summary && (
        <p className="mt-4 text-sm leading-relaxed text-graphite-100">
          {report.overall_summary}
        </p>
      )}

      <div className="mt-4 space-y-5">
        {report.sections.map((s, i) => (
          <div key={`${s.id}-${i}`}>
            <h4 className="text-base font-semibold text-forest-100">
              {s.heading}
            </h4>
            {s.summary && (
              <p className="mt-1 text-sm leading-relaxed text-graphite-200">
                {s.summary}
              </p>
            )}
            {s.bullets.length > 0 && (
              <ul className="mt-2 list-disc space-y-1 pl-5 text-sm text-graphite-300">
                {s.bullets.map((b, j) => (
                  <li key={j}>{b}</li>
                ))}
              </ul>
            )}
          </div>
        ))}
      </div>

      <p className="mt-6 border-t border-graphite-700 pt-3 text-[11px] text-graphite-500">
        Figures are computed locally from your data; the narrative is
        AI-generated and may contain errors — verify before acting.
      </p>
    </article>
  );
}
