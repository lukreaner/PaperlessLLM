# Release process

The release workflow produces five native archives and `SHA256SUMS` from an existing semantic-version tag:

- `aarch64-apple-darwin`
- `x86_64-apple-darwin`
- `aarch64-unknown-linux-musl`
- `x86_64-unknown-linux-musl`
- `x86_64-pc-windows-msvc`

Each target runs on a native GitHub-hosted runner. The workflow installs and builds `web/` before compiling Rust so the dashboard is embedded in the executable, runs Rust tests, starts the release binary, and checks `/healthz`. Before tagging, main CI also runs the committed Playwright Chromium suite for setup, review, activity/statistics, budget-paused, and LAN-authentication flows. Archives contain the executable, license, operational documentation, and native service templates; macOS archives also contain `PaperlessLLM.app`. The Windows ZIP contains the console/service `paperless-llm.exe`, console-free `PaperlessLLM.exe`, checksummed per-user installer/uninstaller scripts, and a quick-start guide. No container image is built or published.

## Create a release

1. Update the `paperless-llm` Cargo package version and commit it.
2. Ensure CI is green on `main`.
3. Create and push a tag such as `v1.0.0`. The tag must exactly match the Cargo package version with a leading `v`.
4. Wait for **Release native binaries**. It creates the GitHub release and uploads all archives plus checksums.

The workflow can also be dispatched manually for an existing tag. Re-running it replaces assets on an existing GitHub release.

## Optional Apple signing and notarization

Without signing secrets, the workflow publishes the standalone CLI and completed app bundle with valid ad-hoc signatures and verifies them strictly before packaging. This seals the executable plus `Info.plist` and the bundle resources so the deliverables are structurally valid, but it does not establish a trusted developer identity or satisfy notarization. Configure all signing secrets to replace the ad-hoc signatures with hardened-runtime Developer ID signatures on the standalone CLI and app bundle:

| Repository secret | Value |
| --- | --- |
| `MACOS_CERTIFICATE_P12_BASE64` | Base64-encoded Developer ID Application `.p12` |
| `MACOS_CERTIFICATE_PASSWORD` | Password protecting the `.p12` |
| `MACOS_SIGNING_IDENTITY` | Full Developer ID Application identity |

The temporary signing keychain and certificate file are removed after each job.

Configure these additional secrets to submit the signed CLI and app to Apple's notary service and staple the accepted ticket to the app:

| Repository secret | Value |
| --- | --- |
| `APPLE_ID` | Apple developer account email |
| `APPLE_TEAM_ID` | Apple Developer team identifier |
| `APPLE_APP_PASSWORD` | App-specific password for notarytool |

Notarization is skipped unless every signing and notarization secret is present. The release remains usable when only signing is configured.

## Optional Windows Authenticode signing

Without Windows signing secrets, the workflow deliberately publishes unsigned Windows executables and records that fact in the job summary. Configure both secrets to enable SHA-256 Authenticode signing with an RFC 3161 timestamp before the ZIP is assembled:

| Repository secret | Value |
| --- | --- |
| `WINDOWS_CERTIFICATE_PFX_BASE64` | Base64-encoded Windows code-signing `.pfx` |
| `WINDOWS_CERTIFICATE_PASSWORD` | Password protecting the `.pfx` |

The workflow fails closed if only one secret is configured. It signs and verifies both `PaperlessLLM.exe` and `paperless-llm.exe`, removes the imported private key and temporary certificate after the build job, and never includes the `.pfx` in an artifact. Signing the binaries does not replace verification of the release-level `SHA256SUMS` file.

## Verify release contents

Download every asset and run:

```sh
sha256sum --check SHA256SUMS
```

For every macOS build, verify the standalone CLI and the completed app after extraction:

```sh
codesign --display --verbose=2 paperless-llm
codesign --verify --strict --verbose=2 paperless-llm
codesign --verify --deep --strict --verbose=2 PaperlessLLM.app
xcrun stapler validate PaperlessLLM.app
spctl --assess --type execute --verbose=4 PaperlessLLM.app
```

The first three `codesign` checks must pass for both ad-hoc and Developer ID builds. `stapler` and `spctl` establish notarization/Gatekeeper trust and are expected to succeed only when Apple signing and notarization credentials were configured.

For a signed Windows build, inspect both executable signatures from PowerShell after extracting the ZIP:

```powershell
Get-AuthenticodeSignature .\PaperlessLLM.exe
Get-AuthenticodeSignature .\paperless-llm.exe
```

Both `Status` values must be `Valid`; an unsigned build reports `NotSigned` and must instead be authenticated through the release checksum and trusted repository/tag history.

Start each binary with a temporary data directory and verify `http://127.0.0.1:8765/healthz` before announcing the release.
