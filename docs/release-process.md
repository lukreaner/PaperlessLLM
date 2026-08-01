# Release process

The release workflow produces four native archives and `SHA256SUMS` from an existing semantic-version tag:

- `aarch64-apple-darwin`
- `x86_64-apple-darwin`
- `aarch64-unknown-linux-musl`
- `x86_64-unknown-linux-musl`

Each target runs on a native GitHub-hosted runner. The workflow installs and builds `web/` before compiling Rust so the dashboard is embedded in the executable, runs Rust tests, starts the release binary, and checks `/healthz`. Before tagging, main CI also runs the committed Playwright Chromium suite for setup, review, activity/statistics, budget-paused, and LAN-authentication flows. Archives contain the executable, license, operational documentation, and native service templates. No container image is built or published.

## Create a release

1. Update the `paperless-llm` Cargo package version and commit it.
2. Ensure CI is green on `main`.
3. Create and push a tag such as `v1.0.0`. The tag must exactly match the Cargo package version with a leading `v`.
4. Wait for **Release native binaries**. It creates the GitHub release and uploads all archives plus checksums.

The workflow can also be dispatched manually for an existing tag. Re-running it replaces assets on an existing GitHub release.

## Optional Apple signing and notarization

Without signing secrets, the workflow deliberately publishes unsigned macOS binaries. Configure all signing secrets to enable hardened-runtime signing:

| Repository secret | Value |
| --- | --- |
| `MACOS_CERTIFICATE_P12_BASE64` | Base64-encoded Developer ID Application `.p12` |
| `MACOS_CERTIFICATE_PASSWORD` | Password protecting the `.p12` |
| `MACOS_SIGNING_IDENTITY` | Full Developer ID Application identity |

The temporary signing keychain and certificate file are removed after each job.

Configure these additional secrets to submit signed binaries to Apple's notary service:

| Repository secret | Value |
| --- | --- |
| `APPLE_ID` | Apple developer account email |
| `APPLE_TEAM_ID` | Apple Developer team identifier |
| `APPLE_APP_PASSWORD` | App-specific password for notarytool |

Notarization is skipped unless every signing and notarization secret is present. The release remains usable when only signing is configured.

## Verify release contents

Download every asset and run:

```sh
sha256sum --check SHA256SUMS
```

For macOS signed builds, also inspect and verify both archives after extraction:

```sh
codesign --display --verbose=2 paperless-llm
codesign --verify --strict --verbose=2 paperless-llm
```

Start each binary with a temporary data directory and verify `http://127.0.0.1:8765/healthz` before announcing the release.
