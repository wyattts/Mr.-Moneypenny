/**
 * Stub view shown after setup completes. Phase 4b will replace this with
 * the Insights dashboard, Ledger, Categories, Budgets, Settings, and
 * Household views.
 */
export function PostSetupPlaceholder() {
  return (
    <div className="flex h-screen flex-col bg-graphite-900 text-graphite-100">
      <header className="flex items-center justify-between border-b border-graphite-700 px-8 py-5">
        <div className="flex items-center gap-3">
          <img src="/logo.png" alt="Mr. Moneypenny" className="h-9 w-9" />
          <span className="text-lg font-semibold text-forest-300">Mr. Moneypenny</span>
        </div>
      </header>
      <main className="flex flex-1 flex-col items-center justify-center px-8 text-center">
        <h1 className="text-2xl font-semibold">Setup complete.</h1>
        <p className="mt-2 max-w-lg text-graphite-300">
          Try logging an expense in Telegram. The Insights dashboard, Ledger,
          and Settings views land in Phase 4b — for now, the bot is live and
          listening on your machine.
        </p>
      </main>
    </div>
  );
}
