import type { ButtonHTMLAttributes } from "react";

const base =
  "inline-flex items-center justify-center rounded-md px-4 py-2 text-sm font-medium transition focus:outline-none focus-visible:ring-2 focus-visible:ring-forest-400 disabled:opacity-50 disabled:cursor-not-allowed";

export function PrimaryButton(props: ButtonHTMLAttributes<HTMLButtonElement>) {
  const { className = "", ...rest } = props;
  return (
    <button
      {...rest}
      className={`${base} bg-forest-600 hover:bg-forest-500 text-white shadow-sm ${className}`}
    />
  );
}

export function SecondaryButton(props: ButtonHTMLAttributes<HTMLButtonElement>) {
  const { className = "", ...rest } = props;
  return (
    <button
      {...rest}
      className={`${base} bg-graphite-700 hover:bg-graphite-600 text-graphite-50 ${className}`}
    />
  );
}

export function GhostButton(props: ButtonHTMLAttributes<HTMLButtonElement>) {
  const { className = "", ...rest } = props;
  return (
    <button
      {...rest}
      className={`${base} bg-transparent hover:bg-graphite-700 text-graphite-200 ${className}`}
    />
  );
}
