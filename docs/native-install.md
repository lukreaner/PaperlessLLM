# Native installation

PaperlessLLM is distributed as one native executable with the dashboard and database migrations embedded. Node.js, Rust, and a container runtime are not required on the destination computer.

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

Extract the archive, change into the extracted `paperless-llm-<version>-<target>` directory, then install the executable for the current user:

```sh
mkdir -p "$HOME/.local/bin"
install -m 0755 paperless-llm "$HOME/.local/bin/paperless-llm"
mkdir -p "$HOME/Library/Application Support/PaperlessLLM"
chmod 700 "$HOME/Library/Application Support/PaperlessLLM"
```

Run the first-start wizard in the foreground:

```sh
"$HOME/.local/bin/paperless-llm" serve \
  --data-dir "$HOME/Library/Application Support/PaperlessLLM" \
  --bind 127.0.0.1 \
  --port 8765 \
  --open-browser
```

If the browser does not open, visit `http://127.0.0.1:8765`. Stop the foreground process with Ctrl-C after setup.

Release automation signs and notarizes macOS binaries only when the project maintainers configure Apple credentials. For an unsigned release, macOS may quarantine the executable. Verify the checksum and release source before deciding whether to remove the quarantine attribute with `xattr -d com.apple.quarantine "$HOME/.local/bin/paperless-llm"`.

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

The Linux archives target musl and do not require a matching glibc version. Install for the current user:

```sh
install -d -m 0755 "$HOME/.local/bin"
install -m 0755 paperless-llm "$HOME/.local/bin/paperless-llm"
install -d -m 0700 "$HOME/.local/share/paperless-llm"
```

Run the first-start wizard:

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

## Initial configuration

The setup wizard needs:

1. A reachable Paperless-ngx v3 URL and API token. PaperlessLLM checks API v10 compatibility.
2. An OpenAI, Claude, LM Studio, or custom OpenAI-compatible provider and model.
3. Processing mode, enabled AI functions, taxonomy restrictions, and optional trigger tags.
4. Per-document token limits and any global/provider daily or monthly budgets.

Keep the service on `127.0.0.1` for a single-machine installation. Before making it reachable from another computer, configure the administrator password and follow [Reverse proxy and LAN access](reverse-proxy.md).

## Upgrade and uninstall

Back up the data directory before an upgrade. Stop the service, replace only the executable, and start it again; embedded database migrations run at startup. Database migrations are forward-only, so keep the matching backup until the new version has been verified.

Uninstalling the executable does not remove configuration or statistics. Delete the data directory only when those records and encrypted credentials are no longer needed.
