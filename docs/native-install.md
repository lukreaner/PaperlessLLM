# Native installation

PaperlessLLM contains one native service executable with the dashboard and database migrations embedded. Release archives also include platform launchers: `PaperlessLLM.app` for macOS, a freedesktop application-menu template and installer for Linux, and a console-free `PaperlessLLM.exe` plus per-user installer for Windows. Node.js, Rust, and a container runtime are not required on the destination computer.

## Choose and verify a release

Download `SHA256SUMS` and the archive matching the computer from the GitHub release:

| Computer | Archive suffix |
| --- | --- |
| Apple silicon Mac | `aarch64-apple-darwin.tar.gz` |
| Intel Mac | `x86_64-apple-darwin.tar.gz` |
| ARM64 Linux | `aarch64-unknown-linux-musl.tar.gz` |
| x86-64 Linux | `x86_64-unknown-linux-musl.tar.gz` |
| x86-64 Windows | `x86_64-pc-windows-msvc.zip` |

Verify the archive before extracting it:

```sh
sha256sum --check SHA256SUMS --ignore-missing
```

On macOS, use `shasum -a 256` and compare its output with the corresponding line in `SHA256SUMS`.

On Windows, open PowerShell in the download directory and compare the printed hash with the matching `SHA256SUMS` line:

```powershell
Get-FileHash .\paperless-llm-<version>-x86_64-pc-windows-msvc.zip -Algorithm SHA256
Select-String -Path .\SHA256SUMS -Pattern 'x86_64-pc-windows-msvc.zip'
```

## Windows

### Double-click installation

Extract the complete ZIP into a normal folder, then double-click **Install PaperlessLLM.cmd**. The installer verifies the two bundled executables against the archive's internal checksum manifest, installs them under `%LOCALAPPDATA%\Programs\PaperlessLLM`, and creates a Start-menu shortcut. It asks whether to create a Desktop shortcut as well. Installation is for the current user and does not require administrator permission.

Choose **PaperlessLLM** from Start or double-click the Desktop shortcut. The console-free `PaperlessLLM.exe` launcher starts or reuses the sibling `paperless-llm.exe` background service and opens the dashboard. A startup problem is shown in a Windows dialog instead of disappearing in a terminal window.

To use the ZIP without installing it, keep `PaperlessLLM.exe` and `paperless-llm.exe` together in the extracted directory and double-click `PaperlessLLM.exe`. Both executables statically link their C runtime, so no separate Microsoft Visual C++ Redistributable installation is required.

The release automation supports optional Authenticode signing. If the maintainers have not configured a signing certificate, Windows may display Microsoft Defender SmartScreen's warning for an unknown publisher. Verify the GitHub Release ZIP checksum before running it; do not disable Defender or other platform protections.

### Command-line operation

The lowercase executable is the normal console and service binary. From PowerShell in the installed directory:

```powershell
.\paperless-llm.exe --help
.\paperless-llm.exe launch
.\paperless-llm.exe serve --open-browser
```

The default Windows data directory is `%LOCALAPPDATA%\paperless-llm`. PaperlessLLM protects the directory and its private files with a non-inheriting ACL for the owning user, local Administrators, and SYSTEM. Paperless and provider credentials are stored through Windows Credential Manager; Windows deliberately has no encrypted-SQLite/master-key fallback. If Credential Manager is unavailable, saving or reading a credential fails closed with an error.

To upgrade, first use **Quit PaperlessLLM** in the dashboard, extract the newer ZIP, and run its installer again. It replaces only installer-managed application files and shortcuts. Your database, settings, statistics, logs, and credentials remain in the separate data directory.

## macOS

### Double-click installation

Extract the archive and double-click `PaperlessLLM.app`. You may first drag it to `/Applications` or `~/Applications` so it remains available after deleting the extracted archive. Each activation invokes `paperless-llm launch`: it starts the background service when needed, reuses an existing instance only when its data-directory and build fingerprints both match, and opens the dashboard in the default browser. If an older or different build still owns the data directory, PaperlessLLM asks you to quit that instance and open the new app again instead of silently serving its stale dashboard.

If the browser does not open, visit `http://127.0.0.1:8765`. The first launch opens the setup wizard.

Release automation always gives the standalone CLI and completed app bundle structurally valid ad-hoc signatures. It replaces those signatures with Developer ID signing, submits the deliverables for notarization, and staples the app only when the project maintainers configure Apple credentials. An ad-hoc signature does not establish publisher trust, so verify the checksum and release source first. You may then right-click the app and choose **Open** to use macOS's confirmation flow. Removing quarantine is a separate, advanced option:

```sh
xattr -dr com.apple.quarantine PaperlessLLM.app
```

### Command-line installation

The archive also contains the standalone executable. To install it for the current user:

```sh
mkdir -p "$HOME/.local/bin"
install -m 0755 paperless-llm "$HOME/.local/bin/paperless-llm"
mkdir -p "$HOME/Library/Application Support/PaperlessLLM"
chmod 700 "$HOME/Library/Application Support/PaperlessLLM"
```

For normal desktop-style launching from a terminal, run:

```sh
"$HOME/.local/bin/paperless-llm" launch
```

To run in the foreground instead:

```sh
"$HOME/.local/bin/paperless-llm" serve \
  --data-dir "$HOME/Library/Application Support/PaperlessLLM" \
  --bind 127.0.0.1 \
  --port 8765 \
  --open-browser
```

Stop the foreground process with Ctrl-C after setup. For an unsigned standalone executable, macOS may also require confirmation or removal of its quarantine attribute after verification.

### Start at login with launchd

The launchd file is a template because launchd does not expand `$HOME` in `ProgramArguments`. Copy it and replace all three placeholders with absolute paths:

```sh
mkdir -p "$HOME/Library/LaunchAgents" "$HOME/Library/Logs/PaperlessLLM"
chmod 700 "$HOME/Library/Logs/PaperlessLLM"
cp packaging/launchd/io.paperlessllm.paperless-llm.plist \
  "$HOME/Library/LaunchAgents/io.paperlessllm.paperless-llm.plist"
```

Replace:

- `__PAPERLESS_LLM_BINARY__` with `/Users/your-name/.local/bin/paperless-llm`
- `__PAPERLESS_LLM_DATA_DIR__` with `/Users/your-name/Library/Application Support/PaperlessLLM`
- `__PAPERLESS_LLM_LOG_DIR__` with `/Users/your-name/Library/Logs/PaperlessLLM`

Validate and load it:

```sh
plutil -lint "$HOME/Library/LaunchAgents/io.paperlessllm.paperless-llm.plist"
launchctl bootstrap "gui/$(id -u)" \
  "$HOME/Library/LaunchAgents/io.paperlessllm.paperless-llm.plist"
launchctl print "gui/$(id -u)/io.paperlessllm.paperless-llm"
```

To restart after an upgrade:

```sh
launchctl kickstart -k "gui/$(id -u)/io.paperlessllm.paperless-llm"
```

## Linux

The Linux archives target musl and do not require a matching glibc version.

### Application-menu and double-click installation

From the extracted `paperless-llm-<version>-<target>` directory, run the included current-user installer:

```sh
./packaging/linux/install-desktop.sh
```

It atomically installs the executable at `$HOME/.local/bin/paperless-llm`, creates the private data directory, and writes `io.paperlessllm.PaperlessLLM.desktop` under `${XDG_DATA_HOME:-$HOME/.local/share}/applications`. Open **PaperlessLLM** from the desktop environment's application menu. The menu entry runs `paperless-llm launch`, which starts or reuses the background service and opens the dashboard.

To also create an executable shortcut in the desktop directory, run:

```sh
./packaging/linux/install-desktop.sh --desktop-shortcut
```

Double-click `PaperlessLLM.desktop` after installation. GNOME Files and some other file managers may show an **Allow Launching** or **Trust and Launch** confirmation the first time. The installer uses `xdg-user-dir` when available so localized desktop-directory names are supported; set `PAPERLESSLLM_DESKTOP_DIR` to an absolute path to override its choice.

The installer never uses `sudo` and only changes the current user's files. It honors `XDG_DATA_HOME`; `PAPERLESSLLM_BIN_DIR` can override the binary destination. Re-run the installer from a newer extracted archive to upgrade both the binary and application entry.

### Manual command-line installation

To install without an application-menu entry:

```sh
install -d -m 0755 "$HOME/.local/bin"
install -m 0755 paperless-llm "$HOME/.local/bin/paperless-llm"
install -d -m 0700 "$HOME/.local/share/paperless-llm"
```

Start or open PaperlessLLM:

```sh
"$HOME/.local/bin/paperless-llm" launch
```

For foreground operation instead:

```sh
"$HOME/.local/bin/paperless-llm" serve \
  --data-dir "$HOME/.local/share/paperless-llm" \
  --bind 127.0.0.1 \
  --port 8765 \
  --open-browser
```

PaperlessLLM creates a mode-`0600` `.master-key` in the data directory unless `PAPERLESSLLM_MASTER_KEY` is supplied. An environment-provided key must be exactly 32 bytes encoded as base64 or hexadecimal. Supply it through a credential manager or equivalently protected service environment, not a committed `.env` file, shell history, or world-readable unit. Do not rotate or delete that key while encrypted secrets remain in the database; there is no in-place rekey operation.

### Run as a systemd user service

```sh
install -d -m 0755 "$HOME/.config/systemd/user"
install -m 0644 packaging/systemd/paperless-llm.service \
  "$HOME/.config/systemd/user/paperless-llm.service"
systemctl --user daemon-reload
systemctl --user enable --now paperless-llm.service
systemctl --user status paperless-llm.service
```

The unit uses the default Linux data directory, binds only to loopback, and applies user-service hardening. If `XDG_DATA_HOME` is not `$HOME/.local/share`, edit `ExecStart` and `ReadWritePaths` together before enabling it.

To start the service at boot without an interactive login, an administrator may enable lingering:

```sh
sudo loginctl enable-linger "$USER"
```

View logs with `journalctl --user -u paperless-llm.service -f`.

## Stop PaperlessLLM

Closing the browser does not stop PaperlessLLM; it closes only the dashboard tab while the worker intentionally remains active. To stop a copy started with `launch`, from a Windows shortcut, by double-clicking `PaperlessLLM.app`, or from the Linux application menu, open the dashboard and choose **Quit PaperlessLLM** at the bottom of the sidebar. Confirm **Quit PaperlessLLM** in the dialog to stop the worker gracefully. Launch the app again whenever you want to restart it.

If the dashboard is unavailable, use Windows Task Manager, macOS Activity Monitor, or the Linux system monitor. The long-running Windows process is `paperless-llm.exe`; `PaperlessLLM.exe` is only the short-lived launcher. On Linux, the equivalent terminal fallback is:

```sh
pkill -TERM -x paperless-llm
```

Do not use the dashboard Quit control for an instance managed by launchd or systemd. The service manager owns that process and may restart it after a dashboard-requested exit. Stop or unload it through the same manager instead:

```sh
# macOS launchd
launchctl bootout "gui/$(id -u)" \
  "$HOME/Library/LaunchAgents/io.paperlessllm.paperless-llm.plist"

# Linux systemd user service
systemctl --user stop paperless-llm.service
```

## View diagnostic logs

Open **Logs** in the dashboard sidebar to view recent desktop-launcher service events and the current worker state. The view polls every five seconds while the page is visible, can be paused or refreshed manually, and supports safe-field search plus level and subsystem filters.

The API does not send raw log messages to the browser. It maps recognized lines to fixed event and subsystem names and returns only validated operational fields such as status, counts, document/job identifiers, and normalized error codes. Unknown messages are shown as a generic service event. Prompts, OCR/document content, credentials, headers, local filesystem paths, model names, and upstream response bodies are not included.

The desktop child keeps `paperless-llm.log` below roughly 4 MiB and retains one previous generation as `paperless-llm.log.previous`, so diagnostic storage remains below roughly 8 MiB plus small headers and a lock file. Starting a new desktop service rotates a versioned prior current log instead of deleting it. Both generations and the writer lock are private `0600` files. The dashboard deliberately reads only the current generation, with the newest entries first; it never exposes the previous or temporary rotation file. During the first launch after upgrading, an unversioned legacy log is deleted rather than retained because older output may not meet the current privacy contract. Quit and restart PaperlessLLM once before opening **Logs** after an upgrade.

The dashboard log is intended for instances started through `paperless-llm launch`, the Windows launcher, the macOS app, or the Linux desktop entry. For launchd-managed instances, inspect the configured `stdout.log` and `stderr.log` files. For systemd-managed instances, continue to use:

```sh
journalctl --user -u paperless-llm.service -f
```

## Scan and control the queue

PaperlessLLM discovers every matching Paperless document by default. It requests bounded, minimal Paperless pages and updates the Overview scan counter after each committed page, so a library with thousands of documents does not stop after 50 or need to be loaded into memory at once. In **Settings → Processing**, leave **Maximum documents per scan** empty for this behavior or enter a deliberate cap.

Overview and Activity update roughly every two seconds while the tab is visible. Both pages show the document currently being analyzed/applied or the live discovery totals. Use **Pause processing** to prevent the next scan or job from starting; an operation already in flight finishes its safe boundary first. The paused state survives a restart, and **Continue processing** resumes the durable queue immediately.

To remove a waiting item, expand its Activity row and choose **Remove from queue**. Only `queued` and `budget-blocked` jobs can be removed; an analyzing/applying job is allowed to finish. Removal retains the activity and token-usage audit record. The same document can be queued again through **Reprocess**, or automatically when Paperless receives a new file version.

## Initial configuration

The setup wizard needs:

1. A reachable Paperless-ngx v3 URL and API token. PaperlessLLM checks API v10 compatibility.
2. An OpenAI, Claude, Mistral, Kimi global, Moonshot China, Google Gemini, LM Studio, or custom OpenAI-compatible provider and model. Hosted compatible presets use their official Chat Completions base URL, require an API key, omit temperature by default, and leave the model ID for discovery or manual entry.
3. Processing mode, enabled AI functions, taxonomy restrictions, and optional trigger tags.
4. Per-document token limits and any global/provider daily or monthly budgets.

Keep the service on `127.0.0.1` for a single-machine installation. Before making it reachable from another computer, configure the administrator password and follow [Reverse proxy and LAN access](reverse-proxy.md).

## Upgrade and uninstall

Back up the data directory before an upgrade. The documented backup is a sensitive plaintext archive until separately encrypted. Stop the service, replace only the executable, and start it again; embedded database migrations run at startup. Database migrations are forward-only, so keep the matching encrypted backup until the new version has been verified.

Uninstalling the executable does not remove configuration or statistics. A Linux application-menu installation can be removed with:

```sh
rm "$HOME/.local/bin/paperless-llm"
rm "${XDG_DATA_HOME:-$HOME/.local/share}/applications/io.paperlessllm.PaperlessLLM.desktop"
```

Remove a separately created `PaperlessLLM.desktop` shortcut from the desktop in the usual file-manager way. On Linux, also remove any externally supplied `PAPERLESSLLM_MASTER_KEY` from its credential manager when it is no longer used. Delete the data directory only when its configuration, statistics, and encrypted credentials are no longer needed.

On Windows, uninstall from **Settings → Apps → Installed apps → PaperlessLLM**, or double-click **Uninstall PaperlessLLM.cmd** in `%LOCALAPPDATA%\Programs\PaperlessLLM`. The uninstaller removes only its marked application files, shortcuts, and uninstall entry. It deliberately preserves `%LOCALAPPDATA%\paperless-llm` and Windows Credential Manager values. After stopping and uninstalling every PaperlessLLM instance that uses them, remove those separately only if the configuration, history, statistics, and saved credentials are no longer needed.

On macOS, deleting `PaperlessLLM.app` or its data directory does not delete Keychain-backed credentials. After stopping PaperlessLLM, open **Keychain Access**, search for generic-password entries whose service is `com.paperless-llm.credentials`, and remove only the entries belonging to the PaperlessLLM instance being retired. If other PaperlessLLM data directories are still in use, verify the entry account before deleting it. Then remove the data directory and any plaintext backups only when their configuration, history, fallback credentials, and statistics are no longer needed.
