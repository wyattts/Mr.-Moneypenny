import type { ReactNode } from "react";

export function ViewHeader({
  title,
  subtitle,
  actions,
}: {
  title: string;
  subtitle?: ReactNode;
  actions?: ReactNode;
}) {
  return (
    <header className="border-b border-graphite-700 bg-graphite-900 px-8 py-5">
      <div className="flex items-end justify-between gap-4">
        <div>
          <h1 className="text-2xl font-semibold text-graphite-50">{title}</h1>
          {subtitle ? <p className="mt-1 text-sm text-graphite-400">{subtitle}</p> : null}
        </div>
        {actions ? <div className="flex items-center gap-2">{actions}</div> : null}
      </div>
    </header>
  );
}
