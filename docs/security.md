# Credential and repository security

PaperlessLLM treats credentials and document metadata as local application data, not source code. The safeguards in this document are defense in depth; no ignore rule or scanner makes it safe to deliberately commit a credential.

## Where credentials are stored

- On macOS, each credential save first attempts Keychain. If that individual operation fails, PaperlessLLM stores that value in the encrypted-SQLite fallback. A single installation can therefore contain some values in Keychain and others in SQLite; do not assume one backend applies to every saved value.
- On Windows, PaperlessLLM requires Windows Credential Manager for every saved Paperless/provider credential. It fails closed if that store is unavailable and does not create or use the encrypted-SQLite/master-key fallback.
- On Linux, credential values are AES-GCM encrypted in SQLite. macOS uses the same fallback as described above. On those platforms the encryption key comes from `PAPERLESSLLM_MASTER_KEY` or a generated mode-`0600` `.master-key` file.
- Saved credential values and custom provider headers are never returned by the settings API. The dashboard receives only configured/not-configured indicators.
- The default data directories are outside the source tree. On Windows, PaperlessLLM rejects data-directory reparse points and applies a protected ACL that grants full access only to the owning user, local Administrators, and SYSTEM; private child files receive the corresponding non-inheriting ACL. On macOS and Linux, PaperlessLLM tightens the data directory to mode `0700` and the SQLite database, existing sidecars, and generated `.master-key` to mode `0600`. Keep custom `--data-dir` locations outside every Git checkout as well.

On Linux and for the macOS fallback, the generated `.master-key` is stored in the same private data directory as SQLite. Encryption protects a database copied without its key; it does not protect against compromise of the service account, the complete data directory, or a backup containing both key and ciphertext. `PAPERLESSLLM_MASTER_KEY` must be supplied through a credential manager or equivalently protected environment source, never a committed `.env` file, a shell-history entry, or a world-readable service definition.

The database, its SQLite sidecars, `.master-key`, runtime lock/instance files, logs, secret-scanner reports, nested environment files, common private-key extensions, and documented backup names are ignored repository-wide. Ignore rules cover only recognized names and untracked files: an arbitrarily named archive or an already tracked file can still be committed. Credential-bearing runtime data and backups must remain outside the repository regardless of filename or whether `git add -f` would be necessary.

The repository deliberately permits `*.env.example`, `*.env.sample`, and `*.env.template`. Those files must contain placeholders only. CI rejects tracked `*.key`, `*.pem`, `*.p12`, and `*.pfx` files unconditionally; adding an intentional public certificate or test fixture requires a reviewed change to that policy, not merely a forced Git add.

## Provider data flow

Document OCR text, enabled taxonomy context, the enforced analysis instructions, and—when document-file mode is enabled—a supported document rendition are sent to the active provider endpoint. Review mode delays Paperless metadata writes; it does not delay or prevent that provider request. A hosted provider may retain or process requests according to its own account settings and terms. Use a hosted preset only for documents you are permitted to transmit, or keep the provider URL on a trusted local network with LM Studio or a custom compatible server.

Changing a saved provider preset or base URL changes the credential's destination. PaperlessLLM therefore requires the API key to be entered again and does not carry custom secret headers to the new destination. Provider HTTP redirects are rejected rather than forwarding credentials to a second URL.

## Backups and local compromise

The backup commands in [backup and restore](backup-and-restore.md) produce compressed archives, not encrypted archives. A generated `.master-key` is included with the fallback ciphertext, so that archive is sufficient to decrypt fallback credentials. It also contains private operational data such as document titles, review suggestions, custom prompts, errors, and statistics. On Unix, create it under a restrictive `umask` and keep it mode `0600` in a mode-`0700` directory; on every platform, encrypt it before copying it to removable, cloud, or shared storage.

Windows Credential Manager and Keychain-backed macOS values are not in a data-directory archive. Plan to re-enter Windows credentials after a restore on another account or computer. Because the macOS fallback is selected per operation, always preserve any data-directory `.master-key` and separately include Keychain recovery or credential re-entry in the macOS plan. If `PAPERLESSLLM_MASTER_KEY` is external, keep it in a credential vault separate from the archive.

## Dashboard diagnostic logs

The `/api/logs` endpoint remains inside the protected API router and requires an authenticated session whenever LAN authentication is enabled. It reads only the fixed current desktop-log filename inside the configured data directory, rejects symlinks and non-files, and bounds each read to the newest 128 KiB and 500 entries. It never reads the previous or temporary rotation generation. Responses use `Cache-Control: no-store`. A version marker prevents an older, unstructured log from being exposed after an upgrade.

Desktop children write through a dedicated bounded writer rather than inherited stdout/stderr descriptors. The current log rotates at roughly 4 MiB and one previous generation is retained, for roughly 8 MiB total diagnostic content. Existing versioned generations are bounded before reuse; unversioned legacy current, previous, or recovery logs are deleted instead of deliberately retaining output that predates the safe logging contract. The fixed current, previous, temporary, and lock targets reject symlinks; Windows additionally rejects reparse points and protects files with its private ACL, while Unix files use mode `0600`. Rotation uses same-directory recovery files and renames, and an interrupted current-file handoff is completed on the next launch without selecting an alternate path. Desktop logging uses a fixed `paperless_llm=info` filter and deliberately ignores `RUST_LOG`, preventing dependency debug traces from persisting headers or bodies. Foreground `serve` processes continue to log to the console and may honor `RUST_LOG` for interactive troubleshooting.

Raw log strings are not returned to the browser. The service maps recognized lines to fixed event and subsystem names, exposes only validated operational fields, and reduces unknown messages to a generic event. It strips multiline continuations and does not include credentials, custom headers, prompts, OCR/document content, model names, local filesystem paths, or upstream response bodies. The desktop log file remains private application data and must not be committed or shared without separate review.

## Automated Git safeguards

The secret-scanning workflow runs on every push and pull request, can be started manually, and also performs a scheduled full-history scan. It:

1. checks out complete history without persisting a GitHub credential;
2. rejects tracked PaperlessLLM runtime files, environment files, key material, databases, logs, and data backups, even if someone bypassed `.gitignore` with `git add -f`;
3. downloads the pinned Gitleaks release over HTTPS and verifies its pinned SHA-256 checksum before execution;
4. scans all reachable history, including limited nested archive and encoded-content inspection;
5. fully redacts detected values from CI output and does not upload a finding report artifact.

Every reusable GitHub Action in the build, test, secret-scan, and release workflows is pinned to a full immutable commit SHA. Human-readable version comments remain beside the pins so upgrades can be reviewed deliberately, and Dependabot checks those pins weekly for reviewable updates.

## Recommended GitHub repository settings

The following are manual settings on the hosted GitHub repository or its organization/enterprise. This local documentation patch does not enable or verify them:

1. Enable GitHub Secret Protection, secret scanning, and push protection wherever the repository's plan and visibility make them available. Keep custom patterns and delegated-alert access as narrow as practical.
2. Protect the default and release branches with a branch protection rule or ruleset that requires the **Gitleaks** workflow status check before merge. Prevent ordinary users from bypassing that requirement.
3. Enable the GitHub Actions policy that requires actions to be pinned to a full-length commit SHA, if that control is available at the repository, organization, or enterprise level.
4. Review Dependabot pull requests for GitHub Actions rather than merging them automatically. Confirm the upstream release, the new full SHA, the adjacent version comment, and the workflow diff before approval.

GitHub's native scanning and push protection complement the repository workflow; neither replaces rotation after a real credential has been exposed.

The repository also includes a pinned `.pre-commit-config.yaml`. Developers who use [pre-commit](https://pre-commit.com/) can enable the local staged-change check with:

```sh
pre-commit install
```

The hook then scans staged changes automatically before each commit. It is an opt-in local check: `git commit --no-verify` bypasses it, and it does not scan unstaged files, untracked files, complete history, or enforce CI's forbidden-path list. Review the working tree yourself and rely on the protected CI check before merging.

To reproduce the complete CI content scan with Gitleaks 8.30.1 installed:

```sh
gitleaks git . \
  --log-opts='--all' \
  --redact=100 \
  --no-banner \
  --no-color \
  --verbose \
  --max-archive-depth=2 \
  --max-decode-depth=2 \
  --timeout=300
```

Before every commit or push, review `git status --short`, the staged diff, and relevant untracked files. Never add an application data directory, credential-bearing backup archive, `.env` file, `.master-key`, database, log, or private key, even when its filename is not ignored.

## If data or a credential is exposed

Treat a committed, uploaded, or otherwise disclosed credential as compromised even if the file or commit is later deleted. A database or backup exposure is also a privacy incident because non-secret metadata is stored in plaintext.

1. Contain access to the repository, artifact, backup location, or host without copying secret values into tickets or chat.
2. Revoke or rotate every affected downstream credential immediately. This includes Paperless tokens, OpenAI/Anthropic/other provider keys, and credentials carried in custom provider headers. If a backup contains both SQLite and `.master-key`, assume every fallback credential in it is readable.
3. Enter replacement credentials in PaperlessLLM and stop using the old values. If the database was exposed, reset the PaperlessLLM administrator password as well; doing so invalidates existing dashboard sessions. Assess the disclosure of document titles, suggestions, custom prompts, errors, and usage history separately because rotation cannot undo that privacy exposure.
4. Do not overwrite or delete `.master-key` while encrypted rows still depend on it. PaperlessLLM has no in-place rekey operation. Rotating downstream credentials makes disclosed old ciphertext unusable for future access; moving future fallback storage to a new master key requires a controlled reinitialization and credential re-entry.
5. Remove the sensitive data from Git history and uploaded artifacts using the hosting provider's documented process. Coordinate any required force-push with collaborators and account for forks, clones, caches, CI logs, release assets, and backup copies that can retain it.
6. Re-run the complete full-history scan and review the rewritten history before restoring normal pushes or releases.

Removing the current file or adding it to `.gitignore` is not sufficient because older Git objects and distributed copies can retain the value. Secret scanners can also miss valid credentials, so a clean scan does not replace rotation.
