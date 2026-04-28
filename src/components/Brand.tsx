/**
 * Brand lockup: butler logo paired with a typographic wordmark.
 *
 * No extra font assets — just Inter weights and tracking. "Mr." is set
 * lighter and italic for a slight gentleman-butler feel; "Moneypenny"
 * carries the brand color and weight.
 */

type Size = "sm" | "md" | "lg";

const ICON: Record<Size, string> = {
  sm: "h-6 w-6",
  md: "h-10 w-10",
  lg: "h-12 w-12",
};

const PREFIX_TEXT: Record<Size, string> = {
  sm: "text-xs",
  md: "text-sm",
  lg: "text-base",
};

const MAIN_TEXT: Record<Size, string> = {
  sm: "text-sm",
  md: "text-base",
  lg: "text-xl",
};

export function Wordmark({ size = "md" }: { size?: Size }) {
  return (
    <span className="flex items-baseline gap-1 select-none whitespace-nowrap">
      <span className={`${PREFIX_TEXT[size]} font-medium italic text-graphite-200`}>
        Mr.
      </span>
      <span
        className={`${MAIN_TEXT[size]} font-semibold tracking-tight text-forest-300`}
      >
        Moneypenny
      </span>
    </span>
  );
}

export function Brand({ size = "md" }: { size?: Size }) {
  return (
    <div className="flex items-center gap-3">
      <img src="/logo.png" alt="" className={`${ICON[size]} shrink-0`} />
      <Wordmark size={size} />
    </div>
  );
}
