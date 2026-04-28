/**
 * Main application shell shown after setup completes. Persistent sidebar
 * + Outlet for the current view.
 */
import { NavLink, Outlet } from "react-router-dom";

import { Brand } from "@/components/Brand";

const NAV: { to: string; label: string; icon: string }[] = [
  { to: "/insights", label: "Insights", icon: "▤" },
  { to: "/ledger", label: "Ledger", icon: "≡" },
  { to: "/categories", label: "Categories", icon: "⊞" },
  { to: "/household", label: "Household", icon: "♕" },
  { to: "/settings", label: "Settings", icon: "⚙" },
];

export function MainApp() {
  return (
    <div className="flex h-screen bg-graphite-900 text-graphite-100">
      <aside className="flex w-60 shrink-0 flex-col border-r border-graphite-700 bg-graphite-950">
        <header className="border-b border-graphite-800 px-4 py-4">
          <Brand size="md" />
        </header>
        <nav className="flex-1 px-2 py-4">
          {NAV.map((n) => (
            <NavLink
              key={n.to}
              to={n.to}
              className={({ isActive }) =>
                `mb-1 flex items-center gap-3 rounded-md px-3 py-2 text-sm transition ${
                  isActive
                    ? "bg-forest-700/40 text-forest-100"
                    : "text-graphite-300 hover:bg-graphite-800 hover:text-graphite-50"
                }`
              }
            >
              <span className="w-4 text-center text-graphite-400">{n.icon}</span>
              <span>{n.label}</span>
            </NavLink>
          ))}
        </nav>
        <footer className="border-t border-graphite-800 px-4 py-3 text-xs text-graphite-500">
          Local-only. AGPL-3.0.
        </footer>
      </aside>
      <main className="flex-1 overflow-y-auto">
        <Outlet />
      </main>
    </div>
  );
}
