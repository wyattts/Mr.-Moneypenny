#!/usr/bin/env bash
# Regenerate src-tauri/notices/THIRD_PARTY_RUST.md and
# src-tauri/notices/THIRD_PARTY_NPM.txt from the live dependency graph.
#
# Requires:
#   - cargo-about (cargo install --locked cargo-about --features cli)
#   - npx + a populated node_modules (`npm ci`)
#
# Run from any directory; this script resolves paths relative to itself.

set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" >/dev/null && pwd)"
ROOT="$(cd -- "$SCRIPT_DIR/.." >/dev/null && pwd)"
NOTICES="$ROOT/src-tauri/notices"

if ! command -v cargo-about >/dev/null 2>&1; then
    echo "error: cargo-about not on PATH. Install with:" >&2
    echo "  cargo install --locked cargo-about --features cli" >&2
    exit 1
fi

if ! command -v npx >/dev/null 2>&1; then
    echo "error: npx not on PATH (need Node.js)." >&2
    exit 1
fi

echo "→ Generating Rust notices (this can take ~30s)…"
(cd "$ROOT/src-tauri" && cargo about generate -c about.toml about.hbs -o "$NOTICES/THIRD_PARTY_RUST.md")

echo "→ Generating npm notices…"
(cd "$ROOT" && npx --yes license-checker-rseidelsohn --production --plainVertical --out "$NOTICES/THIRD_PARTY_NPM.txt")

echo "✓ Notices regenerated under $NOTICES"
