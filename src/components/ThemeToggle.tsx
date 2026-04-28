/**
 * Three-way theme selector: System / Light / Dark.
 */
import { useTheme } from "@/lib/theme";
import type { Theme } from "@/lib/theme";

const OPTIONS: { value: Theme; label: string }[] = [
  { value: "auto", label: "System" },
  { value: "light", label: "Light" },
  { value: "dark", label: "Dark" },
];

export function ThemeToggle() {
  const { theme, setTheme } = useTheme();
  return (
    <div className="inline-flex rounded-md border border-graphite-700 bg-graphite-800 p-0.5">
      {OPTIONS.map((o) => (
        <button
          key={o.value}
          type="button"
          onClick={() => setTheme(o.value)}
          className={`rounded-sm px-3 py-1 text-xs font-medium transition ${
            theme === o.value
              ? "bg-forest-600 text-white"
              : "text-graphite-300 hover:text-graphite-100"
          }`}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}
