import type { ReactNode } from "react";

import { Brand } from "@/components/Brand";

export function StepLayout({
  title,
  subtitle,
  children,
  footer,
  stepIndex,
  totalSteps,
}: {
  title: string;
  subtitle?: ReactNode;
  children: ReactNode;
  footer?: ReactNode;
  stepIndex: number;
  totalSteps: number;
}) {
  return (
    <div className="flex h-full flex-col">
      <header className="flex items-center justify-between border-b border-graphite-700 px-8 py-5">
        <Brand size="lg" />
        <div className="text-xs uppercase tracking-wider text-graphite-400">
          Step {stepIndex} of {totalSteps}
        </div>
      </header>

      <main className="flex flex-1 items-start justify-center overflow-y-auto px-8 py-10">
        <div className="w-full max-w-2xl">
          <h1 className="text-2xl font-semibold text-graphite-50">{title}</h1>
          {subtitle ? (
            <p className="mt-2 text-graphite-300">{subtitle}</p>
          ) : null}
          <div className="mt-8">{children}</div>
        </div>
      </main>

      {footer ? (
        <footer className="border-t border-graphite-700 bg-graphite-900 px-8 py-4">
          <div className="mx-auto flex w-full max-w-2xl items-center justify-end gap-3">
            {footer}
          </div>
        </footer>
      ) : null}
    </div>
  );
}

export function ErrorBanner({ children }: { children: ReactNode }) {
  return (
    <div className="rounded-md border border-red-500/40 bg-red-500/10 px-4 py-3 text-sm text-red-200">
      {children}
    </div>
  );
}

export function InfoBanner({ children }: { children: ReactNode }) {
  return (
    <div className="rounded-md border border-forest-400/40 bg-forest-400/10 px-4 py-3 text-sm text-forest-100">
      {children}
    </div>
  );
}
