# PaperlessLLM

PaperlessLLM is a self-contained AI metadata sidecar for Paperless-ngx v3. It polls Paperless through REST API v10, analyzes documents with a selected AI provider, and either applies conservative metadata updates or holds editable suggestions for review.

The service runtime is one native executable. The Rust service, worker, SQLite migrations, and React dashboard are embedded together; Node.js is needed only when building from source. Release archives also provide native launchers: `PaperlessLLM.app` on macOS and an installable application-menu entry on Linux. PaperlessLLM does not retain document text or files.

## Highlights

- OpenAI Responses API with JSON Schema output, reasoning controls, and nullable temperature that is omitted by default
- Native Anthropic Messages API with structured output and thinking controls
- LM Studio and custom OpenAI-compatible endpoints using explicitly selected Responses or Chat Completions transports
- Model discovery plus manual model IDs; no hard-coded model allowlist
- Automatic processing or a per-document review queue
- Independent AI functions for tags, correspondent, document type, title, and typed custom fields
- Exact-match taxonomy restrictions, conservative merge defaults, and re-fetch-before-apply protection
- Daily, monthly, and lifetime token statistics with provider, model, function, cache, output, and reasoning breakdowns
- Atomic daily/monthly token budgets with provider-specific limits and reset timezones
- macOS Keychain storage or AES-GCM-encrypted Linux secret storage
- Authenticated LAN/reverse-proxy mode with Argon2id, HttpOnly sessions, CSRF protection, and restrictive browser headers
- Double-click launching through a macOS app bundle or Linux application-menu/desktop entry
- Native macOS ARM/x64 and Linux ARM/x64 release workflows; no container artifacts

## Requirements

- Paperless-ngx 3.x exposing API v10 and an API token
- One supported provider:
  - OpenAI
  - Anthropic Claude
  - LM Studio
  - A custom OpenAI-compatible Responses or Chat Completions endpoint

API keys are entered in the first-run wizard and are never returned to the browser after storage.

## Run a release

Download the archive for your platform and verify it against `SHA256SUMS` before extracting it.

On macOS, move `PaperlessLLM.app` to Applications if desired, then double-click it. On Linux, run the included current-user installer once:

```sh
./packaging/linux/install-desktop.sh
```

Then choose **PaperlessLLM** from the application menu. To also create a desktop icon that can be double-clicked, use:

```sh
./packaging/linux/install-desktop.sh --desktop-shortcut
```

Both launchers run `paperless-llm launch`, which starts the local service when needed, reuses it when already running, and opens the dashboard. The same behavior is available from a terminal:

```sh
paperless-llm launch
```

The worker remains active after the browser closes. See [native installation](docs/native-install.md#stop-a-desktop-launched-service) for stopping it before an upgrade or uninstall.

For foreground operation or service-manager integration, run:

```sh
paperless-llm serve --open-browser
```

The default dashboard is `http://127.0.0.1:8765`. Useful options are:

```text
--data-dir <PATH>   Override the local database/secrets directory
--bind <IP>         Listener address (default 127.0.0.1)
--port <PORT>       Dashboard/API port (default 8765)
--open-browser      Open the dashboard after startup
--no-open           Never open a browser
```

See [native installation](docs/native-install.md) for double-click setup, command-line installation, launchd, and systemd-user instructions.

## Build from source

Building requires a current stable Rust toolchain and Node.js 22 with npm:

```sh
cd web
npm ci
npm run build
cd ..
cargo build --release --locked
./target/release/paperless-llm serve --open-browser
```

The Vite output is compiled into the Rust binary. Node.js is not required to run the resulting executable.

## First-run behavior

The wizard verifies Paperless-ngx v3/API v10, saves a provider profile and model, configures automatic or review processing, selects the enabled AI functions, and establishes token limits. Polling defaults to every five minutes and **Scan now** is available from the Overview page.

Temperature is absent unless explicitly configured and supported by the tested adapter. Reasoning, thinking, structured-output, sampling, and output-limit fields are likewise serialized only for compatible transports. Invalid prompt-only JSON receives at most one budgeted repair request; PaperlessLLM never silently switches providers.

Document-file mode is disabled by default. When enabled, size/page/provider checks determine whether a supported PDF or image is sent; otherwise PaperlessLLM falls back to OCR text. Downloaded bytes remain in memory only for the request and are then released.

## Safety and privacy

By default PaperlessLLM adds tags, fills only empty scalar/custom fields, restricts taxonomy values to existing Paperless objects, and adds the `ai-processed` tag only as part of a successful final update. It re-fetches each document immediately before applying and preserves unrelated tags and the complete custom-field collection. Review mode performs no Paperless write before approval.

Keep the listener on loopback for single-machine use. LAN mode requires an administrator password and confirmation that HTTPS terminates at a reverse proxy. Follow [reverse proxy and LAN access](docs/reverse-proxy.md) before exposing the dashboard.

On Linux, the generated data directory is mode `0700` and its `.master-key` is mode `0600`. You can instead provide `PAPERLESSLLM_MASTER_KEY` as exactly 32 bytes encoded in base64 or hexadecimal. On macOS, saved tokens use Keychain.

## Development and verification

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
cd web
npm ci
npm test -- --run
npm run build
npx playwright install chromium
npm run test:e2e
```

The Playwright suite exercises the first-run wizard, review and statistics flows, budget-paused activity, and LAN login in Chromium. Automated provider and Paperless contract tests use mocks. Live API smoke tests are opt-in and require credentials supplied by the operator.

## Operations

- [Backup and restore](docs/backup-and-restore.md)
- [Reverse proxy and LAN access](docs/reverse-proxy.md)
- [Release process](docs/release-process.md)

## License

[MIT](LICENSE)
