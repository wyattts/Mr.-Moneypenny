import { useEffect, useState } from "react";

import { getSetupState } from "@/lib/tauri";
import { stepFromSavedNumber, useWizard } from "@/lib/store";
import { Wizard } from "@/wizard/Wizard";
import { PostSetupPlaceholder } from "@/views/PostSetupPlaceholder";

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

  if (setup?.setup_complete) {
    return <PostSetupPlaceholder />;
  }

  return <Wizard />;
}
