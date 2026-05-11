#!/usr/bin/env bash
# Idempotently prepend an SPDX license header to every Rust + TS/TSX
# source file under src/, src-tauri/src/, and src-tauri/tests/. Skips
# files that already declare an SPDX-License-Identifier (so re-running
# is a no-op). See audit Co-3 in docs/audit-v0.3.7.md.
#
# Header format follows the SPDX style guide:
#   // SPDX-License-Identifier: AGPL-3.0-or-later
#   // Copyright (C) 2026 Wyatt Smith and contributors

set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." >/dev/null && pwd)"
HEADER="// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Wyatt Smith and contributors
"

count_added=0
count_skipped=0

while IFS= read -r f; do
    if grep -q "SPDX-License-Identifier" "$f"; then
        count_skipped=$((count_skipped + 1))
        continue
    fi
    # Prepend header. Use a temp file so the operation is atomic per
    # file (no half-written source if the script is interrupted).
    tmp="$(mktemp "$f.spdx.XXXXXX")"
    printf '%s' "$HEADER" > "$tmp"
    cat "$f" >> "$tmp"
    mv "$tmp" "$f"
    count_added=$((count_added + 1))
done < <(
    find "$ROOT/src" "$ROOT/src-tauri/src" "$ROOT/src-tauri/tests" \
        -type f \( -name "*.rs" -o -name "*.ts" -o -name "*.tsx" \) \
        -not -path "*/node_modules/*" \
        -not -path "*/target/*" \
        -not -path "*/dist/*" \
        2>/dev/null
)

echo "Added: $count_added file(s)"
echo "Skipped (already had header): $count_skipped file(s)"
