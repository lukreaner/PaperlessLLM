# Backup and restore

PaperlessLLM keeps configuration, processing history, review suggestions, token statistics, and encrypted credentials in its data directory. It does not retain document text or source files, and a PaperlessLLM backup is not a backup of Paperless-ngx.

The default data directories are:

- macOS: `$HOME/Library/Application Support/PaperlessLLM`
- Linux: `$XDG_DATA_HOME/paperless-llm`, or `$HOME/.local/share/paperless-llm` when `XDG_DATA_HOME` is unset

The SQLite database uses write-ahead logging. Stop PaperlessLLM before copying the directory so the database, WAL, and encrypted-secret key form one consistent snapshot.

The examples below create compressed `.tar.gz` files; compression is not encryption. A fallback backup normally contains both encrypted credential rows and the `.master-key` that decrypts them, plus plaintext document titles, suggestions, custom prompts, errors, and statistics. Treat the resulting archive like a credential: keep it mode `0600` in a mode-`0700` directory and encrypt it before moving it to removable, cloud, or shared storage.

## Back up on Linux

For a systemd user installation:

```sh
systemctl --user stop paperless-llm.service
install -d -m 0700 "$HOME/backups"
chmod 700 "$HOME/backups"
umask 077
backup_path="$HOME/backups/paperless-llm-$(date +%Y%m%d-%H%M%S).tar.gz"
tar -C "$HOME/.local/share" \
  -czf "$backup_path" \
  paperless-llm
chmod 600 "$backup_path"
systemctl --user start paperless-llm.service
```

The generated `.master-key` is included when it lives in the data directory. This makes the archive self-contained for fallback restoration but also means encryption does not protect it from someone who obtains the whole archive. If the service instead receives `PAPERLESSLLM_MASTER_KEY` from an environment or secrets manager, back up that value separately in a credential vault. A database without its original master key cannot decrypt saved Paperless or provider tokens.

## Back up on macOS

Unload the launch agent, archive the directory, and load the agent again:

```sh
launchctl bootout "gui/$(id -u)" \
  "$HOME/Library/LaunchAgents/io.paperlessllm.paperless-llm.plist"
install -d -m 0700 "$HOME/Backups"
chmod 700 "$HOME/Backups"
umask 077
backup_path="$HOME/Backups/paperless-llm-$(date +%Y%m%d-%H%M%S).tar.gz"
tar -C "$HOME/Library/Application Support" \
  -czf "$backup_path" \
  PaperlessLLM
chmod 600 "$backup_path"
launchctl bootstrap "gui/$(id -u)" \
  "$HOME/Library/LaunchAgents/io.paperlessllm.paperless-llm.plist"
```

Keychain-backed values are not contained in this archive. Include the normal encrypted macOS/Keychain backup in the recovery plan, or be prepared to re-enter Paperless and provider tokens after restoring on a different Mac. Keychain fallback happens per credential operation, so one installation may also have encrypted values in SQLite. Always preserve any data-directory `.master-key` with the database; the archive command above does so.

## Restore

Use the same PaperlessLLM version that created the backup, or a newer version. A newer binary may migrate the restored database; that migrated copy cannot necessarily be opened by an older binary.

1. Stop PaperlessLLM.
2. Move the current data directory aside instead of deleting it, and keep that copy private.
3. Decrypt the backup if necessary, then extract it into the parent directory from a private working location.
4. Restore the external master key or Keychain entries, if applicable.
5. Ensure the data directory is owned by the service user and has mode `0700`; the generated `.master-key` must have mode `0600`.
6. Start PaperlessLLM and check `http://127.0.0.1:8765/healthz`, the dashboard connection status, recent jobs, and token totals.

Example Linux restore:

```sh
systemctl --user stop paperless-llm.service
mv "$HOME/.local/share/paperless-llm" \
  "$HOME/.local/share/paperless-llm.before-restore"
tar -C "$HOME/.local/share" -xzf paperless-llm-backup.tar.gz
chmod 700 "$HOME/.local/share/paperless-llm"
if test -f "$HOME/.local/share/paperless-llm/.master-key"; then
  chmod 600 "$HOME/.local/share/paperless-llm/.master-key"
fi
systemctl --user start paperless-llm.service
curl --fail http://127.0.0.1:8765/healthz
```

If validation fails, stop the service, move the restored directory aside, and move `paperless-llm.before-restore` back into place.

## Backup policy

Take a backup before every upgrade and on a schedule appropriate for the value of review decisions and token statistics. Retain at least one snapshot from the previous application version. Encrypt archives at rest and in transit, keep external master keys in a separate credential vault, and periodically test a restore on a private non-production machine. Delete plaintext staging copies securely according to the storage medium's capabilities and retention policy.
