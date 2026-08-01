# Native installation

PaperlessLLM contains one native service executable with the dashboard and database migrations embedded. Release archives also include platform launchers: `PaperlessLLM.app` for macOS and a freedesktop application-menu template and installer for Linux. Node.js, Rust, and a container runtime are not required on the destination computer.

## Choose and verify a release

Download `SHA256SUMS` and the archive matching the computer from the GitHub release:

| Computer | Archive suffix |
| --- | --- |
| Apple silicon Mac | `aarch64-apple-darwin.tar.gz` |
| Intel Mac | `x86_64-apple-darwin.tar.gz` |
| ARM64 Linux | `aarch64-unknown-linux-musl.tar.gz` |
| x86-64 Linux | `x86_64-unknown-linux-musl.tar.gz` |

Verify the archive before extracting it:

```sh
sha256sum --check SHA256SUMS --ignore-missing
```

On macOS, use `shasum -a 256` and compare its output with the corresponding line in `SHA256SUMS`.

## macOS

### Double-click installation

Extract the archive and double-click `PaperlessLLM.app`. You may first drag it to `/Applications` or `~/Applications` so it remains available after deleting the extracted archive. Each activation invokes `paperless-llm launch`: it starts the background service when needed, reuses an existing instance, and opens the dashboard in the default browser.

If the browser does not open, visit `http://127.0.0.1:8765`. The first launch opens the setup wizard.

Release automation signs the standalone CLI and app, submits both for notarization, and staples the app only when the project maintainers configure Apple credentials. For an unsigned build, verify the checksum and release source first. You may then right-click the app and choose **Open** to use macOS's confirmation flow. Removing quarantine is a separate, advanced option:

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

PaperlessLLM creates a mode-`0600` `.master-key` in the data directory unless `PAPERLESSLLM_MASTER_KEY` is supplied. An environment-provided key must be exactly 32 bytes encoded as base64 or hexadecimal. Do not rotate or delete that key while encrypted secrets remain in the database.

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

## Stop a desktop-launched service

Closing the browser does not stop PaperlessLLM; the worker intentionally remains active. Before upgrading or uninstalling a copy started with `launch` or by double-clicking, stop the `paperless-llm` process in macOS Activity Monitor or the Linux system monitor. On Linux, the equivalent terminal command is:

```sh
pkill -TERM -x paperless-llm
```

For a managed service, use `launchctl bootout` or `systemctl --user stop paperless-llm.service` instead. Double-clicking the app or choosing the application-menu entry starts it again.

## Initial configuration

The setup wizard needs:

1. A reachable Paperless-ngx v3 URL and API token. PaperlessLLM checks API v10 compatibility.
2. An OpenAI, Claude, LM Studio, or custom OpenAI-compatible provider and model.
3. Processing mode, enabled AI functions, taxonomy restrictions, and optional trigger tags.
4. Per-document token limits and any global/provider daily or monthly budgets.

Keep the service on `127.0.0.1` for a single-machine installation. Before making it reachable from another computer, configure the administrator password and follow [Reverse proxy and LAN access](reverse-proxy.md).

## Upgrade and uninstall

Back up the data directory before an upgrade. Stop the service, replace only the executable, and start it again; embedded database migrations run at startup. Database migrations are forward-only, so keep the matching backup until the new version has been verified.

Uninstalling the executable does not remove configuration or statistics. A Linux application-menu installation can be removed with:

```sh
rm "$HOME/.local/bin/paperless-llm"
rm "${XDG_DATA_HOME:-$HOME/.local/share}/applications/io.paperlessllm.PaperlessLLM.desktop"
```

Remove a separately created `PaperlessLLM.desktop` shortcut from the desktop in the usual file-manager way. Delete the data directory only when its configuration, statistics, and encrypted credentials are no longer needed.
