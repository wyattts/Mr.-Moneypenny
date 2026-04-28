/**
 * Currency / date formatting helpers.
 *
 * The host machine knows the user's locale via navigator.language; we
 * pass through to Intl APIs. Currency code comes from the saved settings.
 */

export function formatMoney(
  amountCents: number,
  currency: string = "USD",
  locale: string | null = null,
): string {
  const lc = locale ?? (typeof navigator !== "undefined" ? navigator.language : "en-US");
  const cents = Math.abs(amountCents);
  const major = cents / 100;
  try {
    return new Intl.NumberFormat(lc, {
      style: "currency",
      currency,
      // JPY etc. are zero-decimal; let Intl handle it.
    }).format(amountCents / 100);
    // (NumberFormat handles negative sign on its own.)
  } catch {
    // Fallback if currency code is unknown.
    return `${amountCents < 0 ? "-" : ""}${currency} ${major.toFixed(2)}`;
  }
}

export function formatDate(iso: string, locale: string | null = null): string {
  const lc = locale ?? (typeof navigator !== "undefined" ? navigator.language : "en-US");
  return new Intl.DateTimeFormat(lc, {
    year: "numeric",
    month: "short",
    day: "numeric",
  }).format(new Date(iso));
}

export function formatDateTime(iso: string, locale: string | null = null): string {
  const lc = locale ?? (typeof navigator !== "undefined" ? navigator.language : "en-US");
  return new Intl.DateTimeFormat(lc, {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  }).format(new Date(iso));
}

/** Format a percent like "+12.3%" or "-5%". */
export function formatDelta(pct: number | null): string {
  if (pct === null || !Number.isFinite(pct)) return "—";
  const sign = pct > 0 ? "+" : "";
  return `${sign}${pct.toFixed(1)}%`;
}
