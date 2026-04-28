# Building Mr. Moneypenny from source

Mr. Moneypenny is a Tauri 2.x application — Rust backend, React/TypeScript frontend, native webview. This document covers what you need to build the desktop binary on each platform, how to run the dev loop, and how to produce a release artifact.

If you only want to *use* Mr. Moneypenny, grab a pre-built binary from [Releases](https://github.com/wyattts/Mr.-Moneypenny/releases) — see [`docs/distribution.md`](docs/distribution.md).

## Prerequisites

### All platforms
- **Rust** stable, installed via [rustup](https://rustup.rs/) (any version ≥ 1.77).
- **Node.js** 22.x (matches `.nvmrc`). Easiest no-sudo install: [`fnm`](https://github.com/Schniz/fnm) → `fnm install 22 && fnm use 22`.

### Linux (Fedora ≥ 40 / RHEL 9)
```bash
sudo dnf install -y \
  webkit2gtk4.1-devel \
  gtk3-devel \
  openssl-devel \
  libsoup3-devel \
  libappindicator-gtk3-devel \
  librsvg2-devel \
  patchelf
```

### Linux (Ubuntu / Debian)
```bash
sudo apt-get update && sudo apt-get install -y \
  libwebkit2gtk-4.1-dev \
  build-essential \
  curl \
  wget \
  file \
  libssl-dev \
  libgtk-3-dev \
  libayatana-appindicator3-dev \
  librsvg2-dev \
  patchelf
```

### macOS
- Xcode Command Line Tools: `xcode-select --install`
- That's it. `tauri build` produces `.app` and `.dmg`.

### Windows
- Visual Studio Build Tools 2022 with the **Desktop development with C++** workload.
- Microsoft Edge WebView2 (preinstalled on Windows 10 1803+).

## Initial setup

```bash
git clone https://github.com/wyattts/Mr.-Moneypenny.git
cd Mr.-Moneypenny
npm install     # pulls Tauri CLI + frontend deps
```

## Dev loop

Hot-reload frontend, in-place Rust rebuilds:

```bash
npm run tauri:dev
```

First run is slow (compiles ~400 Rust crates). Subsequent runs are seconds.

The dev binary writes its database to:
- Linux: `~/.local/share/moneypenny/db.sqlite`
- macOS: `~/Library/Application Support/moneypenny/db.sqlite`
- Windows: `%APPDATA%\moneypenny\db.sqlite`

Secrets (Anthropic key, Telegram bot token) live in the OS keychain under service `moneypenny` — they survive between dev rebuilds.

## Tests + lint

```bash
# Rust
cd src-tauri
cargo test --no-default-features    # 101 tests, no GTK required
cargo fmt --all -- --check
cargo clippy --no-default-features --all-targets -- -D warnings

# Frontend (from repo root)
npm run typecheck
npm run lint
npm run build                        # produces dist/
```

The `--no-default-features` flag skips the desktop runtime so domain logic tests run without GTK / webkit installed.

## Release builds

```bash
# Linux: NO_STRIP=true is required on systems with binutils ≥ 2.41
# (Fedora ≥ 40, Ubuntu 24.04+) because linuxdeploy ships an older
# `strip` that doesn't understand the `.relr.dyn` ELF section.
NO_STRIP=true npm run tauri:build
```

```bash
# macOS / Windows
npm run tauri:build
```

Artifacts land in:
- Linux: `src-tauri/target/release/bundle/{appimage,deb,rpm}/Mr.Moneypenny_<version>_amd64.{AppImage,deb,rpm}`
- macOS: `src-tauri/target/release/bundle/{macos,dmg}/Mr.Moneypenny.{app,dmg}`
- Windows: `src-tauri/target/release/bundle/{msi,nsis}/Mr.Moneypenny_<version>_x64*.{msi,exe}`

A release build takes 5–10 minutes on a cold cache. Profile config (in `src-tauri/Cargo.toml`):
- LTO on
- One codegen unit
- Strip symbols
- Optimize for size (`opt-level = "s"`)

## Code signing

Currently **not configured** — pre-built releases are unsigned on macOS and Windows. The Linux AppImage is GPG-signed when `GPG_SIGNING_KEY` and `GPG_PASSPHRASE` are set as GitHub secrets in the release workflow.

When funding allows ($300–500/yr), enable platform signing:

### macOS
- Apple Developer Program account ($99/yr).
- Generate a Developer ID Application certificate.
- Set GitHub secrets: `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID`.
- Uncomment the matching env block in `.github/workflows/release.yml`.

### Windows
- Authenticode certificate (OV ~$200–400/yr; EV $400–700/yr for instant SmartScreen reputation).
- Set `WINDOWS_CERTIFICATE` (base64-encoded `.p12`) and `WINDOWS_CERTIFICATE_PASSWORD`.
- Uncomment the matching env block.

### Linux (AppImage)
- Generate a project GPG key:
  ```bash
  gpg --quick-generate-key 'Mr. Moneypenny Releases <wyatts+moneypenny@proton.me>' rsa4096
  gpg --armor --export-secret-keys <key-id> > moneypenny-signing.asc
  ```
- Set GitHub secrets: `GPG_SIGNING_KEY` (the contents of `moneypenny-signing.asc`) and `GPG_PASSPHRASE`.
- Publish the public key fingerprint in `docs/distribution.md` so users can verify.

## Tauri configuration

Key files:
- `src-tauri/tauri.conf.json` — bundle metadata, CSP, HTTP allowlist, icon paths.
- `src-tauri/Cargo.toml` — feature flags. `default = ["desktop"]` enables the Tauri runtime; `cargo test --no-default-features` runs without it.
- `src-tauri/capabilities/default.json` — minimal Tauri permission set (`core:default` only).

The bundle's HTTPS allowlist (in `tauri.conf.json` → `app.security.csp`) restricts outbound requests to:
1. `api.telegram.org` (user's bot token)
2. `api.anthropic.com` (user's API key) and `localhost:11434` (Ollama)

Adding any new outbound endpoint requires updating the CSP **and** documenting the change in `docs/privacy.md`. This is intentional friction.

## Reproducibility

Reproducible builds are a goal but not yet a hard guarantee. Current state:
- `Cargo.lock` and `package-lock.json` are committed.
- Rust toolchain pinned via `src-tauri/rust-toolchain.toml` (channel = stable).
- Bundled SQLite (no system version drift).
- AppImage bundles are byte-stable when built with the same toolchain.
- macOS/Windows binaries embed timestamps and signatures so byte-identical reproduction is hard, but hash-of-unsigned-payload reproduction is achievable.

If you reproduce a build and the hashes don't match, please open an issue — we want to know.

## Linux GUI quirks

Mr. Moneypenny sets two WebKitGTK env vars at startup on Linux because some Wayland compositors (notably Mutter on Fedora) trip over WebKit 2.46+'s default DMABUF rendering pipeline:

```rust
WEBKIT_DISABLE_DMABUF_RENDERER=1
WEBKIT_DISABLE_COMPOSITING_MODE=1
```

If you need a different combination for your compositor, set the env vars yourself before launching — the binary respects existing values. To force XWayland entirely:

```bash
GDK_BACKEND=x11 ./Mr.Moneypenny.AppImage
```

The system tray icon may not appear on GNOME without the [AppIndicator GNOME extension](https://extensions.gnome.org/extension/615/appindicator-support/). Other Linux desktops (KDE, XFCE, Cinnamon) show it natively.
