/**
 * CSV import wizard. Modal-style stepper, launched from Settings.
 *
 * Steps:
 *   1. Pick file (file input).
 *   2. Pick or create profile (auto-detects via header_signature).
 *   3. Map columns (only when creating a new profile).
 *   4. Review unmatched merchants (with optional ✨ AI-suggest).
 *   5. Review probable duplicates (default-skip).
 *   6. Commit + summary.
 *
 * Stays self-contained — no router, no zustand. Local state machine
 * via a `step` discriminator. Closes itself via `onClose`.
 */

import { useEffect, useMemo, useState } from "react";

import {
  csvImportAiSuggest,
  csvImportCategorizeAndDedupe,
  csvImportCommit,
  csvImportParse,
  csvImportPreview,
  csvImportSaveProfile,
  listCategories,
} from "@/lib/tauri";
import type {
  CategoryView,
  ColumnMapping,
  CommittableRow,
  CsvPreview,
  Decision,
  DuplicateMatch,
  ParsedRow,
  RuleToSave,
} from "@/lib/tauri";
import { formatMoney } from "@/lib/format";

type Step =
  | "file"
  | "profile"
  | "mapping"
  | "merchants"
  | "duplicates"
  | "commit";

const DATE_FORMATS = [
  "MM/DD/YYYY",
  "DD/MM/YYYY",
  "YYYY-MM-DD",
  "MM-DD-YYYY",
  "DD-MM-YYYY",
  "M/D/YYYY",
];

interface Props {
  onClose: () => void;
  onImported: (n: number) => void;
}

export function CsvImportWizard({ onClose, onImported }: Props) {
  const [step, setStep] = useState<Step>("file");
  const [error, setError] = useState<string | null>(null);

  // File-stage state.
  const [content, setContent] = useState<string>("");
  const [filename, setFilename] = useState<string>("");

  // Profile-stage state.
  const [preview, setPreview] = useState<CsvPreview | null>(null);
  const [chosenProfileId, setChosenProfileId] = useState<number | "new" | null>(
    null,
  );

  // Mapping-stage state.
  const [mapping, setMapping] = useState<ColumnMapping>({
    date_col: 0,
    amount_col: 1,
    merchant_col: 2,
    description_col: null,
    category_col: null,
    date_format: "MM/DD/YYYY",
    neg_means_refund: true,
    skip_rows: 0,
  });
  const [profileName, setProfileName] = useState<string>("");

  // Categorize+dedupe stage state.
  const [parsed, setParsed] = useState<ParsedRow[]>([]);
  const [decisions, setDecisions] = useState<Decision[]>([]);
  const [duplicates, setDuplicates] = useState<DuplicateMatch[]>([]);
  const [overrides, setOverrides] = useState<Record<number, number>>({}); // rowIndex → categoryId
  const [refundOverrides] = useState<Record<number, boolean>>({});
  const [skipDup, setSkipDup] = useState<Record<number, boolean>>({}); // rowIndex → skip?
  const [savedRules, setSavedRules] = useState<RuleToSave[]>([]);
  const [aiCostMicros, setAiCostMicros] = useState<number>(0);

  // Categories for the dropdowns.
  const [categories, setCategories] = useState<CategoryView[]>([]);

  useEffect(() => {
    void (async () => {
      try {
        const cats = await listCategories(false);
        setCategories(cats.filter((c) => c.is_active));
      } catch (e) {
        setError(String(e));
      }
    })();
  }, []);

  const onPickFile = async (file: File) => {
    try {
      setError(null);
      const text = await file.text();
      setContent(text);
      setFilename(file.name);
      const p = await csvImportPreview(text);
      setPreview(p);
      // Default mapping starts pointed at the auto-suggested profile if
      // we hit one; otherwise the user picks "Set up new profile".
      if (p.suggested_profile) {
        setChosenProfileId(p.suggested_profile.id);
        setMapping(p.suggested_profile.mapping);
      } else {
        setChosenProfileId(p.profiles.length > 0 ? p.profiles[0]!.id : "new");
        if (p.profiles.length > 0) {
          setMapping(p.profiles[0]!.mapping);
        }
      }
      setStep("profile");
    } catch (e) {
      setError(String(e));
    }
  };

  const onConfirmProfile = async () => {
    if (!preview) return;
    if (chosenProfileId === "new") {
      setStep("mapping");
      return;
    }
    // Existing profile — go straight to parse + categorize + dedupe.
    await runParseAndCategorize(mapping);
  };

  const onSaveNewProfile = async () => {
    if (!preview) return;
    if (!profileName.trim()) {
      setError("Give the profile a name (e.g., 'Chase Checking')");
      return;
    }
    try {
      const id = await csvImportSaveProfile({
        name: profileName.trim(),
        header_signature: preview.preview.header_signature,
        mapping,
      });
      setChosenProfileId(id);
      await runParseAndCategorize(mapping);
    } catch (e) {
      setError(String(e));
    }
  };

  const runParseAndCategorize = async (mappingToUse: ColumnMapping) => {
    try {
      const rows = await csvImportParse({ content, mapping: mappingToUse });
      setParsed(rows);
      const r = await csvImportCategorizeAndDedupe(rows);
      setDecisions(r.decisions);
      setDuplicates(r.duplicates);
      // Pre-fill skipDup from the initial findings (default-skip per
      // the locked decision D4).
      const initialSkip: Record<number, boolean> = {};
      for (const d of r.duplicates) initialSkip[d.row_index] = true;
      setSkipDup(initialSkip);
      setStep("merchants");
    } catch (e) {
      setError(String(e));
    }
  };

  // Build the list of unique unmatched merchants (rows whose decision
  // came back as "unmatched" and don't yet have an override). Every
  // distinct merchant gets one entry; the user picks a category once
  // and it applies to every row with that merchant.
  const unmatchedMerchants = useMemo(() => {
    const seen = new Map<string, number[]>();
    for (const d of decisions) {
      if (d.source !== "unmatched") continue;
      if (overrides[d.row_index] != null) continue;
      const m = parsed[d.row_index]?.merchant ?? "";
      const lower = m.trim();
      if (!lower) continue;
      const list = seen.get(lower) ?? [];
      list.push(d.row_index);
      seen.set(lower, list);
    }
    return Array.from(seen.entries()).map(([merchant, rowIndices]) => ({
      merchant,
      rowIndices,
    }));
  }, [decisions, overrides, parsed]);

  const blockCommit = unmatchedMerchants.length > 0;

  // When the user picks a category for a merchant, fan it out across all
  // rows with that merchant and queue a rule for save-on-commit.
  const setMerchantCategory = (
    merchant: string,
    rowIndices: number[],
    categoryId: number,
  ) => {
    setOverrides((prev) => {
      const next = { ...prev };
      for (const idx of rowIndices) next[idx] = categoryId;
      return next;
    });
    setSavedRules((prev) => {
      // Suggest a pattern from the merchant string. Take first
      // alphanumeric word + "*"; user can edit later in Settings.
      const pattern = suggestPattern(merchant);
      if (prev.some((r) => r.pattern === pattern)) return prev;
      return [
        ...prev,
        { pattern, category_id: categoryId, default_is_refund: false },
      ];
    });
  };

  const aiSuggest = async () => {
    try {
      setError(null);
      const resp = await csvImportAiSuggest(
        unmatchedMerchants.map((m) => m.merchant),
      );
      // Apply any suggestions whose category_id is in the user's list.
      const valid = new Set(categories.map((c) => c.id));
      for (const u of unmatchedMerchants) {
        const cat = resp.suggestions[u.merchant];
        if (cat != null && valid.has(cat)) {
          setMerchantCategory(u.merchant, u.rowIndices, cat);
        }
      }
      setAiCostMicros((m) => m + resp.cost_micros);
    } catch (e) {
      setError(String(e));
    }
  };

  const goCommit = async () => {
    try {
      setError(null);
      // Build the committable rows. Skip duplicate rows that the user
      // left checked-skip.
      const toCommit: CommittableRow[] = [];
      for (let i = 0; i < parsed.length; i++) {
        if (skipDup[i]) continue;
        const row = parsed[i]!;
        const decision = decisions[i];
        const cat =
          overrides[i] ??
          decision?.category_id ??
          null;
        const isRefund = refundOverrides[i] ?? row.is_refund;
        toCommit.push({
          occurred_at: row.occurred_at,
          amount_cents: row.amount_cents,
          category_id: cat,
          merchant: row.merchant,
          description: row.description,
          is_refund: isRefund,
        });
      }
      const result = await csvImportCommit({
        rows: toCommit,
        rules_to_save: savedRules,
        profile_id:
          typeof chosenProfileId === "number" ? chosenProfileId : null,
      });
      onImported(result.inserted);
      setStep("commit");
      // Brief summary on the commit step is fine — caller re-renders.
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-graphite-950/80 p-4">
      <div className="flex max-h-[90vh] w-full max-w-3xl flex-col overflow-hidden rounded-lg border border-graphite-700 bg-graphite-900 shadow-xl">
        <div className="flex items-center justify-between border-b border-graphite-700 px-5 py-3">
          <h2 className="text-base font-semibold text-graphite-100">
            Import CSV
            <span className="ml-3 text-xs text-graphite-500">
              {stepLabel(step)}
            </span>
          </h2>
          <button
            onClick={onClose}
            className="rounded px-2 py-1 text-sm text-graphite-300 hover:bg-graphite-700"
          >
            Close
          </button>
        </div>
        {error && (
          <div className="border-b border-red-500/40 bg-red-500/10 px-5 py-2 text-sm text-red-200">
            {error}
          </div>
        )}
        <div className="flex-1 overflow-auto px-5 py-4">
          {step === "file" && <FileStep onPickFile={onPickFile} />}
          {step === "profile" && preview && (
            <ProfileStep
              preview={preview}
              chosen={chosenProfileId}
              onChange={(v) => {
                setChosenProfileId(v);
                if (typeof v === "number") {
                  const p = preview.profiles.find((p) => p.id === v);
                  if (p) setMapping(p.mapping);
                }
              }}
            />
          )}
          {step === "mapping" && preview && (
            <MappingStep
              preview={preview}
              mapping={mapping}
              setMapping={setMapping}
              profileName={profileName}
              setProfileName={setProfileName}
            />
          )}
          {step === "merchants" && (
            <MerchantsStep
              filename={filename}
              decisions={decisions}
              parsed={parsed}
              unmatchedMerchants={unmatchedMerchants}
              categories={categories}
              setMerchantCategory={setMerchantCategory}
              aiSuggest={aiSuggest}
              aiCostMicros={aiCostMicros}
            />
          )}
          {step === "duplicates" && (
            <DuplicatesStep
              parsed={parsed}
              duplicates={duplicates}
              skipDup={skipDup}
              setSkipDup={setSkipDup}
            />
          )}
          {step === "commit" && (
            <CommitStep
              filename={filename}
              parsed={parsed}
              skipDup={skipDup}
              onClose={onClose}
            />
          )}
        </div>
        <div className="flex items-center justify-between border-t border-graphite-700 bg-graphite-800 px-5 py-3">
          <div className="text-xs text-graphite-500">
            {filename ? `File: ${filename}` : "No file selected"}
          </div>
          <div className="flex gap-2">
            {step !== "file" && step !== "commit" && (
              <button
                onClick={() => setStep(prevStep(step))}
                className="rounded-md border border-graphite-600 px-3 py-1.5 text-sm text-graphite-200 hover:bg-graphite-700"
              >
                ← Back
              </button>
            )}
            {step === "profile" && (
              <button
                onClick={onConfirmProfile}
                disabled={chosenProfileId === null}
                className="rounded-md bg-forest-600 px-3 py-1.5 text-sm font-medium text-graphite-50 hover:bg-forest-500 disabled:opacity-50"
              >
                {chosenProfileId === "new" ? "Map columns →" : "Use this profile →"}
              </button>
            )}
            {step === "mapping" && (
              <button
                onClick={onSaveNewProfile}
                className="rounded-md bg-forest-600 px-3 py-1.5 text-sm font-medium text-graphite-50 hover:bg-forest-500"
              >
                Save profile + parse →
              </button>
            )}
            {step === "merchants" && (
              <button
                onClick={() => setStep("duplicates")}
                disabled={blockCommit}
                title={
                  blockCommit
                    ? "Categorize every merchant before continuing"
                    : ""
                }
                className="rounded-md bg-forest-600 px-3 py-1.5 text-sm font-medium text-graphite-50 hover:bg-forest-500 disabled:opacity-50"
              >
                Review duplicates →
              </button>
            )}
            {step === "duplicates" && (
              <button
                onClick={goCommit}
                className="rounded-md bg-forest-600 px-3 py-1.5 text-sm font-medium text-graphite-50 hover:bg-forest-500"
              >
                Commit import →
              </button>
            )}
            {step === "commit" && (
              <button
                onClick={onClose}
                className="rounded-md bg-forest-600 px-3 py-1.5 text-sm font-medium text-graphite-50 hover:bg-forest-500"
              >
                Done
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

function stepLabel(step: Step): string {
  switch (step) {
    case "file":
      return "Step 1 of 5 — pick file";
    case "profile":
      return "Step 2 of 5 — pick profile";
    case "mapping":
      return "Step 2b of 5 — map columns";
    case "merchants":
      return "Step 3 of 5 — categorize merchants";
    case "duplicates":
      return "Step 4 of 5 — review duplicates";
    case "commit":
      return "Step 5 of 5 — done";
  }
}

function prevStep(step: Step): Step {
  switch (step) {
    case "profile":
      return "file";
    case "mapping":
      return "profile";
    case "merchants":
      return "profile";
    case "duplicates":
      return "merchants";
    default:
      return "file";
  }
}

function suggestPattern(merchant: string): string {
  const m = merchant.trim();
  let i = 0;
  while (i < m.length && /[a-zA-Z0-9_]/.test(m[i] ?? "")) i++;
  const head = m.slice(0, i).toUpperCase();
  return head.length > 0 ? `${head}*` : m.toUpperCase();
}

function FileStep({ onPickFile }: { onPickFile: (f: File) => void }) {
  return (
    <div className="space-y-3">
      <p className="text-sm text-graphite-300">
        Pick a CSV exported from your bank or credit-card. Common formats
        (Chase, Amex, Discover, Capital One, etc.) work out of the box. The
        file content stays on your machine — nothing leaves your computer
        except optional AI-suggest calls (which only send merchant strings).
      </p>
      <input
        type="file"
        accept=".csv,text/csv"
        onChange={(e) => {
          const f = e.target.files?.[0];
          if (f) onPickFile(f);
        }}
        className="w-full rounded-md border border-graphite-600 bg-graphite-800 px-3 py-2 text-sm text-graphite-100 file:mr-3 file:rounded file:border-0 file:bg-forest-600 file:px-3 file:py-1 file:text-graphite-50"
      />
    </div>
  );
}

function ProfileStep({
  preview,
  chosen,
  onChange,
}: {
  preview: CsvPreview;
  chosen: number | "new" | null;
  onChange: (v: number | "new") => void;
}) {
  const matched = preview.suggested_profile;
  return (
    <div className="space-y-3">
      {matched && (
        <div className="rounded-md border border-forest-600/40 bg-forest-700/20 px-3 py-2 text-sm text-forest-100">
          ✓ Matched a saved profile by column-header signature:{" "}
          <strong>{matched.name}</strong>. The mapping below is filled in for
          you.
        </div>
      )}
      <label className="block">
        <span className="text-xs uppercase tracking-wide text-graphite-400">
          Profile
        </span>
        <select
          value={chosen ?? ""}
          onChange={(e) =>
            onChange(e.target.value === "new" ? "new" : Number(e.target.value))
          }
          className="mt-1 w-full rounded-md border border-graphite-600 bg-graphite-800 px-3 py-2 text-sm text-graphite-100"
        >
          {preview.profiles.map((p) => (
            <option key={p.id} value={p.id}>
              {p.name}
            </option>
          ))}
          <option value="new">— Set up new profile —</option>
        </select>
      </label>
      <div className="text-xs text-graphite-500">
        Headers detected: {preview.preview.headers.join(", ")} ·{" "}
        {preview.preview.total_rows} rows
      </div>
    </div>
  );
}

function MappingStep({
  preview,
  mapping,
  setMapping,
  profileName,
  setProfileName,
}: {
  preview: CsvPreview;
  mapping: ColumnMapping;
  setMapping: (m: ColumnMapping) => void;
  profileName: string;
  setProfileName: (s: string) => void;
}) {
  const headers = preview.preview.headers;
  const update = <K extends keyof ColumnMapping>(
    k: K,
    v: ColumnMapping[K],
  ) => {
    setMapping({ ...mapping, [k]: v });
  };
  return (
    <div className="space-y-3">
      <p className="text-sm text-graphite-300">
        Tell us which column is which. Once you save this profile, future
        imports of the same bank&apos;s CSV will skip this step automatically.
      </p>
      <label className="block">
        <span className="text-xs uppercase tracking-wide text-graphite-400">
          Profile name
        </span>
        <input
          value={profileName}
          onChange={(e) => setProfileName(e.target.value)}
          placeholder="e.g., Chase Checking"
          className="mt-1 w-full rounded-md border border-graphite-600 bg-graphite-800 px-3 py-2 text-sm text-graphite-100"
        />
      </label>
      <div className="grid grid-cols-1 gap-3 md:grid-cols-2">
        <ColumnPicker
          label="Date column"
          headers={headers}
          value={mapping.date_col}
          onChange={(v) => update("date_col", v)}
        />
        <ColumnPicker
          label="Amount column"
          headers={headers}
          value={mapping.amount_col}
          onChange={(v) => update("amount_col", v)}
        />
        <ColumnPicker
          label="Merchant column"
          headers={headers}
          value={mapping.merchant_col}
          onChange={(v) => update("merchant_col", v)}
        />
        <NullableColumnPicker
          label="Description column (optional)"
          headers={headers}
          value={mapping.description_col}
          onChange={(v) => update("description_col", v)}
        />
        <NullableColumnPicker
          label="Category column (optional, hint only)"
          headers={headers}
          value={mapping.category_col}
          onChange={(v) => update("category_col", v)}
        />
        <label className="block">
          <span className="text-xs uppercase tracking-wide text-graphite-400">
            Date format
          </span>
          <select
            value={mapping.date_format}
            onChange={(e) => update("date_format", e.target.value)}
            className="mt-1 w-full rounded-md border border-graphite-600 bg-graphite-800 px-3 py-2 text-sm text-graphite-100"
          >
            {DATE_FORMATS.map((f) => (
              <option key={f}>{f}</option>
            ))}
          </select>
        </label>
        <label className="flex items-center gap-2 self-end pb-1 text-sm text-graphite-200">
          <input
            type="checkbox"
            checked={mapping.neg_means_refund}
            onChange={(e) => update("neg_means_refund", e.target.checked)}
            className="h-4 w-4 accent-forest-500"
          />
          Negative amounts are refunds/credits
        </label>
        <label className="block">
          <span className="text-xs uppercase tracking-wide text-graphite-400">
            Skip leading rows
          </span>
          <input
            type="number"
            min={0}
            max={20}
            value={mapping.skip_rows}
            onChange={(e) =>
              update("skip_rows", Number(e.target.value) || 0)
            }
            className="mt-1 w-full rounded-md border border-graphite-600 bg-graphite-800 px-3 py-2 text-sm text-graphite-100"
          />
        </label>
      </div>
      <div className="mt-2 overflow-x-auto rounded-md border border-graphite-700 bg-graphite-950 p-2">
        <table className="text-xs text-graphite-300">
          <thead>
            <tr>
              {headers.map((h) => (
                <th key={h} className="px-2 py-1 text-left text-graphite-400">
                  {h}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {preview.preview.sample_rows.slice(0, 5).map((r, i) => (
              <tr key={i} className="border-t border-graphite-800">
                {r.map((c, j) => (
                  <td key={j} className="px-2 py-1">
                    {c}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

function ColumnPicker({
  label,
  headers,
  value,
  onChange,
}: {
  label: string;
  headers: string[];
  value: number;
  onChange: (v: number) => void;
}) {
  return (
    <label className="block">
      <span className="text-xs uppercase tracking-wide text-graphite-400">
        {label}
      </span>
      <select
        value={value}
        onChange={(e) => onChange(Number(e.target.value))}
        className="mt-1 w-full rounded-md border border-graphite-600 bg-graphite-800 px-3 py-2 text-sm text-graphite-100"
      >
        {headers.map((h, i) => (
          <option key={i} value={i}>
            {i}: {h}
          </option>
        ))}
      </select>
    </label>
  );
}

function NullableColumnPicker({
  label,
  headers,
  value,
  onChange,
}: {
  label: string;
  headers: string[];
  value: number | null;
  onChange: (v: number | null) => void;
}) {
  return (
    <label className="block">
      <span className="text-xs uppercase tracking-wide text-graphite-400">
        {label}
      </span>
      <select
        value={value ?? ""}
        onChange={(e) =>
          onChange(e.target.value === "" ? null : Number(e.target.value))
        }
        className="mt-1 w-full rounded-md border border-graphite-600 bg-graphite-800 px-3 py-2 text-sm text-graphite-100"
      >
        <option value="">(none)</option>
        {headers.map((h, i) => (
          <option key={i} value={i}>
            {i}: {h}
          </option>
        ))}
      </select>
    </label>
  );
}

function MerchantsStep({
  filename,
  decisions,
  parsed,
  unmatchedMerchants,
  categories,
  setMerchantCategory,
  aiSuggest,
  aiCostMicros,
}: {
  filename: string;
  decisions: Decision[];
  parsed: ParsedRow[];
  unmatchedMerchants: { merchant: string; rowIndices: number[] }[];
  categories: CategoryView[];
  setMerchantCategory: (
    m: string,
    rowIndices: number[],
    catId: number,
  ) => void;
  aiSuggest: () => Promise<void>;
  aiCostMicros: number;
}) {
  const counts = useMemo(() => {
    let rule = 0,
      history = 0,
      unmatched = 0;
    for (const d of decisions) {
      if (d.source === "rule") rule++;
      else if (d.source === "history") history++;
      else unmatched++;
    }
    return { rule, history, unmatched };
  }, [decisions]);
  return (
    <div className="space-y-3">
      <p className="text-sm text-graphite-300">
        Parsed <strong>{parsed.length}</strong> rows from{" "}
        <strong>{filename}</strong>. Auto-matched{" "}
        <span className="text-forest-300">{counts.rule}</span> via saved
        rules,{" "}
        <span className="text-forest-300">{counts.history}</span> via your
        existing expense history. Categorize the{" "}
        <span className="text-yellow-300">{unmatchedMerchants.length}</span>{" "}
        unique unmatched merchants below — each pick saves a rule for next
        time.
      </p>
      {unmatchedMerchants.length > 0 && (
        <div className="flex items-center gap-3">
          <button
            onClick={aiSuggest}
            className="rounded-md border border-graphite-600 bg-graphite-800 px-3 py-1.5 text-sm text-graphite-100 hover:border-graphite-500"
          >
            ✨ AI-suggest categories
          </button>
          {aiCostMicros > 0 && (
            <span className="text-xs text-graphite-500">
              AI cost so far: {formatMoney(Math.round(aiCostMicros / 10000))}
            </span>
          )}
        </div>
      )}
      {unmatchedMerchants.length === 0 ? (
        <div className="rounded-md border border-forest-600/40 bg-forest-700/20 px-3 py-2 text-sm text-forest-100">
          ✓ All merchants categorized. Ready to review duplicates.
        </div>
      ) : (
        <div className="overflow-y-auto rounded-md border border-graphite-700">
          <table className="w-full text-sm">
            <thead className="bg-graphite-800 text-xs uppercase tracking-wide text-graphite-400">
              <tr>
                <th className="px-3 py-2 text-left">Merchant</th>
                <th className="px-3 py-2 text-left">Rows</th>
                <th className="px-3 py-2 text-left">Category</th>
              </tr>
            </thead>
            <tbody>
              {unmatchedMerchants.map(({ merchant, rowIndices }) => (
                <tr key={merchant} className="border-t border-graphite-700">
                  <td className="px-3 py-2 text-graphite-100">{merchant}</td>
                  <td className="px-3 py-2 text-graphite-400">
                    {rowIndices.length}
                  </td>
                  <td className="px-3 py-2">
                    <select
                      defaultValue=""
                      onChange={(e) => {
                        const v = Number(e.target.value);
                        if (v) setMerchantCategory(merchant, rowIndices, v);
                      }}
                      className="w-full rounded-md border border-graphite-600 bg-graphite-800 px-2 py-1 text-sm text-graphite-100"
                    >
                      <option value="">— choose —</option>
                      {categories.map((c) => (
                        <option key={c.id} value={c.id}>
                          {c.name} ({c.kind})
                        </option>
                      ))}
                    </select>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}

function DuplicatesStep({
  parsed,
  duplicates,
  skipDup,
  setSkipDup,
}: {
  parsed: ParsedRow[];
  duplicates: DuplicateMatch[];
  skipDup: Record<number, boolean>;
  setSkipDup: (s: Record<number, boolean>) => void;
}) {
  if (duplicates.length === 0) {
    return (
      <div className="rounded-md border border-forest-600/40 bg-forest-700/20 px-3 py-2 text-sm text-forest-100">
        ✓ No probable duplicates detected. Click <em>Commit import</em> to
        finish.
      </div>
    );
  }
  const skipCount = Object.values(skipDup).filter(Boolean).length;
  return (
    <div className="space-y-3">
      <p className="text-sm text-graphite-300">
        Found <strong>{duplicates.length}</strong> probable duplicate
        {duplicates.length === 1 ? "" : "s"}. Default is to skip them; uncheck
        any you want to import anyway. Currently skipping <strong>{skipCount}</strong>.
      </p>
      <div className="overflow-y-auto rounded-md border border-graphite-700">
        <table className="w-full text-sm">
          <thead className="bg-graphite-800 text-xs uppercase tracking-wide text-graphite-400">
            <tr>
              <th className="px-3 py-2 text-left">Skip?</th>
              <th className="px-3 py-2 text-left">Date</th>
              <th className="px-3 py-2 text-left">Amount</th>
              <th className="px-3 py-2 text-left">Merchant</th>
              <th className="px-3 py-2 text-left">Why flagged</th>
            </tr>
          </thead>
          <tbody>
            {duplicates.map((d) => {
              const r = parsed[d.row_index];
              if (!r) return null;
              return (
                <tr key={d.row_index} className="border-t border-graphite-700">
                  <td className="px-3 py-2">
                    <input
                      type="checkbox"
                      checked={!!skipDup[d.row_index]}
                      onChange={(e) =>
                        setSkipDup({
                          ...skipDup,
                          [d.row_index]: e.target.checked,
                        })
                      }
                      className="h-4 w-4 accent-forest-500"
                    />
                  </td>
                  <td className="px-3 py-2 text-graphite-300">
                    {r.occurred_at.slice(0, 10)}
                  </td>
                  <td className="px-3 py-2 tabular-nums text-graphite-200">
                    {formatMoney(r.amount_cents)}
                  </td>
                  <td className="px-3 py-2 text-graphite-100">{r.merchant}</td>
                  <td className="px-3 py-2 text-xs text-graphite-400">
                    {d.reason}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </div>
  );
}

function CommitStep({
  filename,
  parsed,
  skipDup,
  onClose,
}: {
  filename: string;
  parsed: ParsedRow[];
  skipDup: Record<number, boolean>;
  onClose: () => void;
}) {
  const skipped = Object.values(skipDup).filter(Boolean).length;
  const inserted = parsed.length - skipped;
  return (
    <div className="space-y-3 py-4 text-center">
      <div className="text-3xl">✅</div>
      <div className="text-base font-semibold text-graphite-100">
        Imported {inserted} expense{inserted === 1 ? "" : "s"} from {filename}
      </div>
      <div className="text-sm text-graphite-400">
        {skipped > 0
          ? `Skipped ${skipped} probable duplicate${skipped === 1 ? "" : "s"}.`
          : "No duplicates were skipped."}
      </div>
      <button
        onClick={onClose}
        className="rounded-md bg-forest-600 px-4 py-2 text-sm text-graphite-50 hover:bg-forest-500"
      >
        Done
      </button>
    </div>
  );
}
