import { useEffect, useState } from "react";
import { Navigate, Route, Routes } from "react-router-dom";

import { getSetupState } from "@/lib/tauri";
import { stepFromSavedNumber, useWizard } from "@/lib/store";
import { Wizard } from "@/wizard/Wizard";
import { MainApp } from "@/views/MainApp";
import { Insights } from "@/views/Insights";
import { Ledger } from "@/views/Ledger";
import { Categories } from "@/views/Categories";
import { Budgets } from "@/views/Budgets";
import { Household } from "@/views/Household";
import { Settings } from "@/views/Settings";

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
      <div className="flex h-screen items-center justify-center bg-graphite-900 text-graphite-300">
        Loading…
      </div>
    );
  }

  if (error) {
    return (
      <div className="flex h-screen flex-col items-center justify-center gap-3 bg-graphite-900 px-8 text-center text-graphite-200">
        <h1 className="text-xl font-semibold text-red-300">Couldn&apos;t load setup state</h1>
        <p className="text-sm text-graphite-400">{error}</p>
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
        <Route path="/ledger" element={<Ledger />} />
        <Route path="/categories" element={<Categories />} />
        <Route path="/budgets" element={<Budgets />} />
        <Route path="/household" element={<Household />} />
        <Route path="/settings" element={<Settings />} />
        <Route path="*" element={<Navigate to="/insights" replace />} />
      </Route>
    </Routes>
  );
}
