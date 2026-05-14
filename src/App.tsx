// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Wyatt Smith and contributors
import { lazy, useEffect, useState } from "react";
import { Navigate, Route, Routes } from "react-router-dom";

import { Brand } from "@/components/Brand";
import { getSetupState } from "@/lib/tauri";
import { stepFromSavedNumber, useWizard } from "@/lib/store";
import { Wizard } from "@/wizard/Wizard";
import { MainApp } from "@/views/MainApp";

// Audit Pf-3: code-split the routed views so the initial bundle parses
// only the layout shell + the first route. Forecast (~2.8k LOC) and
// Insights both pull in Recharts; lazy-loading lets the bundler emit
// a shared Recharts chunk that only fetches when a chart view is
// rendered. With v0.3.17 (close-window-on-tray) this is doubly useful:
// every tray-click reload now parses a smaller initial bundle.
//
// MainApp stays eager — it's the layout shell that the Suspense
// fallback (defined in MainApp.tsx) sits inside, so the sidebar +
// header stay mounted while a route chunk fetches.
//
// React.lazy requires default exports; the views are named exports,
// so each thunk shims the rename inline.
const Insights = lazy(() =>
  import("@/views/Insights").then((m) => ({ default: m.Insights })),
);
const Forecast = lazy(() =>
  import("@/views/Forecast").then((m) => ({ default: m.Forecast })),
);
const Ledger = lazy(() =>
  import("@/views/Ledger").then((m) => ({ default: m.Ledger })),
);
const Categories = lazy(() =>
  import("@/views/Categories").then((m) => ({ default: m.Categories })),
);
const Household = lazy(() =>
  import("@/views/Household").then((m) => ({ default: m.Household })),
);
const Settings = lazy(() =>
  import("@/views/Settings").then((m) => ({ default: m.Settings })),
);

export default function App() {
  const setup = useWizard((s) => s.setup);
  const setSetup = useWizard((s) => s.setSetup);
  const setStep = useWizard((s) => s.setStep);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function load() {
    setLoading(true);
    setError(null);
    try {
      const s = await getSetupState();
      setSetup(s);
      setStep(stepFromSavedNumber(s));
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }

  if (loading) {
    return (
      <div className="flex h-screen flex-col items-center justify-center gap-6 bg-graphite-900">
        <Brand size="lg" />
        <p className="text-xs uppercase tracking-widest text-graphite-500">Loading…</p>
      </div>
    );
  }

  if (error) {
    return (
      <div className="flex h-screen flex-col items-center justify-center gap-4 bg-graphite-900 px-8 text-center">
        <Brand size="md" />
        <h1 className="mt-4 text-xl font-semibold text-red-300">
          Couldn&apos;t load setup state
        </h1>
        <p className="max-w-md text-sm text-graphite-400">{error}</p>
      </div>
    );
  }

  if (!setup?.setup_complete) {
    return <Wizard />;
  }

  return (
    <Routes>
      <Route element={<MainApp />}>
        <Route index element={<Navigate to="/insights" replace />} />
        <Route path="/insights" element={<Insights />} />
        <Route path="/forecast" element={<Forecast />} />
        <Route path="/ledger" element={<Ledger />} />
        <Route path="/categories" element={<Categories />} />
        <Route path="/household" element={<Household />} />
        <Route path="/settings" element={<Settings />} />
        <Route path="*" element={<Navigate to="/insights" replace />} />
      </Route>
    </Routes>
  );
}
