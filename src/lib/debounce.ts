// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Wyatt Smith and contributors
//
// Tiny debouncing primitive for slider-driven Tauri commands (audit
// Pf-1). The Simulator and Debt Manager views fire IPC + Monte Carlo
// recompute on every drag tick; at 60fps a horizon drag from 1y→50y
// can issue ~100 sequential heatmap requests, each of which is a
// 12×12=144-cell × 1000-path simulation. Even with a cancellation
// flag on the JS side, the Rust side runs every job to completion.
//
// `useDebouncedValue(value, 250)` returns a "trailing" copy of
// `value`: holds the previous one until 250ms after the last change,
// then propagates. React effects keyed on the debounced value rerun
// once at the end of a drag, not 100 times during.

import { useEffect, useState } from "react";

export function useDebouncedValue<T>(value: T, delayMs: number): T {
  const [debounced, setDebounced] = useState<T>(value);
  useEffect(() => {
    const id = window.setTimeout(() => setDebounced(value), delayMs);
    return () => window.clearTimeout(id);
  }, [value, delayMs]);
  return debounced;
}
