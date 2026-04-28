/**
 * Theme management. Pure-frontend: persists in localStorage, listens to
 * the OS `prefers-color-scheme` media query when on `auto`. The first
 * paint is handled by an inline script in index.html so the window
 * doesn't flash the wrong theme before React mounts.
 */
import { useCallback, useEffect, useState } from "react";

export type Theme = "auto" | "light" | "dark";

const STORAGE_KEY = "moneypenny.theme";

function readSaved(): Theme {
  try {
    const v = localStorage.getItem(STORAGE_KEY);
    return v === "light" || v === "dark" || v === "auto" ? v : "auto";
  } catch {
    return "auto";
  }
}

function effectiveLight(theme: Theme): boolean {
  if (theme === "light") return true;
  if (theme === "dark") return false;
  // auto — follow OS
  return !window.matchMedia("(prefers-color-scheme: dark)").matches;
}

function applyClass(isLight: boolean) {
  document.documentElement.classList.toggle("light", isLight);
}

export function useTheme() {
  const [theme, setThemeState] = useState<Theme>(readSaved);

  // Apply the class whenever theme changes, and listen to OS changes when auto.
  useEffect(() => {
    applyClass(effectiveLight(theme));
    if (theme !== "auto") return;
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const onChange = () => applyClass(effectiveLight("auto"));
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, [theme]);

  const setTheme = useCallback((next: Theme) => {
    try {
      localStorage.setItem(STORAGE_KEY, next);
    } catch {
      /* private browsing etc. — best-effort */
    }
    setThemeState(next);
  }, []);

  return { theme, setTheme };
}
