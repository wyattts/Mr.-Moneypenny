# Installing Mr. Moneypenny

Pre-built releases are at <https://github.com/wyattts/Mr.-Moneypenny/releases>. Pick the artifact for your platform, verify the signature if available, and run.

If you'd rather build from source, see [`BUILDING.md`](../BUILDING.md).

## Linux (AppImage)

```bash
# Download
curl -L -o Mr.Moneypenny.AppImage \
  https://github.com/wyattts/Mr.-Moneypenny/releases/latest/download/Mr.Moneypenny_x.y.z_amd64.AppImage

# Make executable
chmod +x Mr.Moneypenny.AppImage

# Run
./Mr.Moneypenny.AppImage
```

### Signature verification (recommended)

Mr. Moneypenny's Linux releases are signed with a project GPG key. To verify:

```bash
# One-time: import the project signing key
curl -L https://github.com/wyattts/Mr.-Moneypenny/raw/main/docs/signing-key.asc \
  | gpg --import

# Per-release: download the signature alongside the AppImage
curl -L -o Mr.Moneypenny.AppImage.asc \
  https://github.com/wyattts/Mr.-Moneypenny/releases/latest/download/Mr.Moneypenny_x.y.z_amd64.AppImage.asc

gpg --verify Mr.Moneypenny.AppImage.asc Mr.Moneypenny.AppImage
```

Expect a "Good signature from Mr. Moneypenny Releases" line. The current signing-key fingerprint is:

```
B1C9 DCA0 3318 3DAD AAFC  CD9B 500F 6628 44F8 6E10
```

After importing, you can confirm it locally with:

```bash
gpg --fingerprint 'wyatts+moneypenny@proton.me'
```

### GNOME tray-icon caveat

If you're on GNOME (Fedora's default), system-tray icons aren't shown natively. Install the [AppIndicator GNOME extension](https://extensions.gnome.org/extension/615/appindicator-support/) to see Mr. Moneypenny in the top bar.

KDE, XFCE, Cinnamon, and most other desktops show it without extra setup.

## macOS (unsigned, until funded)

Pre-built `.dmg` releases for macOS are **currently unsigned**. macOS Gatekeeper will refuse to open them by default. Two options:

### Option A — open it manually (one-time per release)

1. Download the `.dmg` from Releases.
2. Open it; drag *Mr. Moneypenny* to `/Applications`.
3. **Right-click** the app → **Open** → confirm "Open" in the dialog.
4. After this one-time bypass, it launches normally.

### Option B — strip the quarantine flag (one command)

```bash
xattr -d com.apple.quarantine /Applications/Mr.\ Moneypenny.app
```

### When will macOS releases be signed?

When the project's GitHub Sponsors funding covers Apple's $99/yr Developer Program fee. See the [Sponsors page](https://github.com/sponsors/wyattts) (TBD).

## Windows (unsigned, until funded)

Pre-built `.msi` and `.exe` installers for Windows are **currently unsigned**. SmartScreen will warn you the first time. Two options:

### Option A — bypass SmartScreen
1. Download the `.msi` or `.exe` from Releases.
2. Right-click → **Properties** → check **Unblock** at the bottom of the General tab → **OK**.
3. Run the installer normally.

If "Unblock" isn't visible, click **More info** on the SmartScreen warning and then **Run anyway**.

### Option B — verify via SHA256
Each release notes page includes the SHA256 checksum of every artifact. Compare locally:
```powershell
Get-FileHash .\Mr.Moneypenny_x.y.z_x64-setup.exe -Algorithm SHA256
```

### When will Windows releases be signed?

Same answer as macOS — once Sponsors funding covers an Authenticode certificate (~$200–400/yr).

## After install

When you launch for the first time, the **Setup Wizard** walks you through:
1. Privacy disclaimer
2. Pick LLM provider (Anthropic recommended; Ollama for fully offline)
3. Paste your Anthropic API key (verified with a ~$0.0001 test call) **or** point at your local Ollama
4. Paste a Telegram bot token (created via [@BotFather](https://t.me/BotFather))
5. Pair this chat with the desktop app via a 6-digit code
6. Pick currency and locale
7. Configure category targets (skippable)
8. Done — start logging

Setup state persists across restarts. If the wizard is interrupted you'll resume where you left off.

## Where your data lives

The app stores all expense data in a SQLite database in your user data directory:

| Platform | Path |
|---|---|
| Linux | `~/.local/share/moneypenny/db.sqlite` |
| macOS | `~/Library/Application Support/moneypenny/db.sqlite` |
| Windows | `%APPDATA%\moneypenny\db.sqlite` |

Secrets (Telegram bot token, Anthropic API key) are stored in the OS keychain under service `moneypenny` — never on disk in plaintext.

## Backing up

Mr. Moneypenny doesn't sync your data to anything by default. To back up:

**Manual:** copy the database file somewhere safe (an encrypted external drive, an end-to-end encrypted cloud folder, etc.). The file is a single SQLite file you can copy any time the app isn't actively writing.

**Automatic via Settings → Export** (Phase 5b — coming soon): on-demand encrypted JSON / CSV exports.

## Migrating to a new machine

1. **On the old machine:** copy the database file (path above).
2. **On the new machine:** install Mr. Moneypenny but DO NOT run the wizard yet.
3. Drop the copied `db.sqlite` into the user data directory on the new machine.
4. **Re-pair Telegram:** the new install needs the OS keychain entries, which can't move automatically. Run the wizard's Settings → "Rotate token" with your existing bot token. The pairing of your existing Telegram chats to the bot is already in the database, so no new `/start <code>` is needed.
5. **Re-enter LLM key** the same way.

## Updating

The app does **not** auto-update by default. To upgrade:
1. Download the new release artifact.
2. Replace your old AppImage / `.app` / `.exe` with the new one.
3. Your data and settings are preserved (they live in the user data directory, not the binary).

A future release will add an opt-in updater (off by default in Privacy Mode). Until then, watch the [Releases](https://github.com/wyattts/Mr.-Moneypenny/releases) page or use GitHub's "Watch → Releases" feature.

## Uninstalling

| Platform | How |
|---|---|
| Linux | Delete the `.AppImage`. Optionally delete `~/.local/share/moneypenny/`. |
| macOS | Drag *Mr. Moneypenny* from `/Applications` to the Trash. Optionally delete `~/Library/Application Support/moneypenny/`. |
| Windows | Use **Add or Remove Programs**. Optionally delete `%APPDATA%\moneypenny\`. |

To wipe secrets from the OS keychain:
- **macOS:** Keychain Access → search for `moneypenny` → delete.
- **Windows:** Credential Manager → search for `moneypenny` → delete.
- **Linux:** `secret-tool clear service moneypenny` (with `libsecret`-tools installed) or use Seahorse / KWallet.
