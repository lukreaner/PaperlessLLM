import { useMemo, useState } from "react";
import { api } from "../api/client";
import type { DiagnosticLogEntry } from "../api/types";
import { Alert, Button, Card, EmptyState, ErrorState, Input, LoadingState, PageHeader, Select, StatusDot } from "../components/UI";
import { formatDateTime } from "../lib/format";
import { useAsync, usePollingStale, useVisiblePolling } from "../lib/hooks";

const levels = ["ALL", "INFO", "WARN", "ERROR", "DEBUG", "TRACE"] as const;
type LevelFilter = typeof levels[number];
const AUTO_REFRESH_MS = 5_000;

export function filterDiagnosticEntries(entries: DiagnosticLogEntry[], level: LevelFilter, target: string, query: string) {
  const normalize = (value: string) => value.toLocaleLowerCase().replace(/[._-]+/g, " ");
  const needle = normalize(query.trim());
  return entries.filter((entry) => {
    if (level !== "ALL" && entry.level !== level) return false;
    if (target && entry.target !== target) return false;
    if (!needle) return true;
    const searchable = [
      entry.timestamp,
      entry.level,
      entry.event,
      entry.target,
      ...Object.entries(entry.fields).flatMap(([key, value]) => [key, value]),
    ].join(" ");
    return normalize(searchable).includes(needle);
  });
}

function eventLabel(event: string) {
  return event.split(/[._]/).map((part) => part.charAt(0).toUpperCase() + part.slice(1)).join(" · ");
}

function fieldLabel(field: string) {
  return field.replaceAll("_", " ");
}

function workerLabel(
  state: "running" | "scanning" | "pausing" | "paused" | "error" | "unknown",
  active: boolean,
  scanDiscovered?: number,
  scanQueued?: number,
) {
  if (state === "paused") return "Paused";
  if (state === "pausing") {
    return scanDiscovered === undefined || scanQueued === undefined
      ? "Finishing current document"
      : `Finishing current scan · ${scanDiscovered.toLocaleString()} discovered, ${scanQueued.toLocaleString()} queued`;
  }
  if (state === "scanning") {
    return scanDiscovered === undefined || scanQueued === undefined
      ? "Scanning Paperless"
      : `Scanning Paperless · ${scanDiscovered.toLocaleString()} discovered, ${scanQueued.toLocaleString()} queued`;
  }
  if (state === "error") return "Needs attention";
  if (active) return "Processing a document";
  return state === "running" ? "Ready" : "State unavailable";
}

export function LogsPage() {
  const [level, setLevel] = useState<LevelFilter>("ALL");
  const [target, setTarget] = useState("");
  const [query, setQuery] = useState("");
  const [autoRefresh, setAutoRefresh] = useState(true);
  const logs = useAsync(() => api.logs(), []);
  useVisiblePolling(logs.refreshSilently, AUTO_REFRESH_MS, autoRefresh);
  const pollingStale = usePollingStale(logs.lastSuccessAt, logs.backgroundError);
  const reconnecting = autoRefresh && pollingStale && Boolean(logs.data);
  const updatedAt = logs.data?.updated_at;
  const newestEntries = logs.data ? [...logs.data.entries].reverse() : [];
  const targets = useMemo(
    () => [...new Set((logs.data?.entries ?? []).map((entry) => entry.target))].sort(),
    [logs.data?.entries],
  );
  const filteredEntries = filterDiagnosticEntries(newestEntries, level, target, query);
  const hasFilters = level !== "ALL" || Boolean(target) || Boolean(query.trim());

  function clearFilters() {
    setLevel("ALL");
    setTarget("");
    setQuery("");
  }

  return (
    <div className="page">
      <PageHeader
        eyebrow="Diagnostics"
        title="Logs"
        description="Structured service events for troubleshooting. Only allow-listed operational context leaves the service."
        actions={<>
          {reconnecting && <span className="diagnostic-refresh-state" role="status">Updates reconnecting</span>}
          <label className="diagnostic-auto-refresh">
            <input type="checkbox" checked={autoRefresh} onChange={(event) => setAutoRefresh(event.target.checked)} />
            <span>Auto-refresh</span>
          </label>
          <Button variant="secondary" loading={logs.loading} onClick={() => void logs.refresh()}>Refresh</Button>
        </>}
      />
      {logs.data?.worker && (
        <Card className={`diagnostic-worker diagnostic-worker--${logs.data.worker.state}`}>
          <div className="diagnostic-worker__state">
            <StatusDot state={logs.data.worker.state === "error" ? "danger" : logs.data.worker.state === "running" || logs.data.worker.state === "scanning" ? "success" : logs.data.worker.state === "paused" || logs.data.worker.state === "pausing" ? "warning" : "neutral"} />
            <div><span>Worker</span><strong>{workerLabel(logs.data.worker.state, logs.data.worker.active, logs.data.worker.scan_discovered, logs.data.worker.scan_queued)}</strong></div>
          </div>
          <div><span>Last scan</span><strong>{logs.data.worker.last_scan_at ? formatDateTime(logs.data.worker.last_scan_at) : "Not yet"}</strong></div>
          <div><span>Next scan</span><strong>{logs.data.worker.next_scan_at ? formatDateTime(logs.data.worker.next_scan_at) : "Not scheduled"}</strong></div>
        </Card>
      )}
      {logs.data?.truncated && (
        <Alert tone="info" title="Showing the newest entries">
          <p>The desktop log is capped by size and line count in this view.</p>
        </Alert>
      )}
      <Card padded={false} className="diagnostic-log-card">
        {logs.data?.available && logs.data.entries.length > 0 && (
          <div className="diagnostic-log-toolbar">
            <label className="search-field"><span aria-hidden="true">⌕</span><Input aria-label="Search logs" value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search events and safe fields…" /></label>
            <Select aria-label="Filter logs by level" value={level} onChange={(event) => setLevel(event.target.value as LevelFilter)}>
              {levels.map((item) => <option key={item} value={item}>{item === "ALL" ? "All levels" : item}</option>)}
            </Select>
            <Select aria-label="Filter logs by target" value={target} onChange={(event) => setTarget(event.target.value)}>
              <option value="">All targets</option>
              {targets.map((item) => <option key={item} value={item}>{item}</option>)}
            </Select>
          </div>
        )}
        {logs.loading && !logs.data ? (
          <LoadingState label="Loading logs" />
        ) : logs.error ? (
          <div className="inset-state"><ErrorState message={logs.error} onRetry={() => void logs.refresh()} /></div>
        ) : !logs.data?.available ? (
          <EmptyState
            mark="≡"
            title="No desktop log available"
            description={logs.data?.message ?? "Start PaperlessLLM from the app or desktop launcher to create a diagnostic log."}
            action={<Button variant="secondary" onClick={() => void logs.refresh()}>Check again</Button>}
          />
        ) : !logs.data.entries.length ? (
          <EmptyState mark="≡" title="The log is empty" description="New service events will appear here as PaperlessLLM runs." />
        ) : !filteredEntries.length ? (
          <EmptyState mark="⌕" title="No matching log entries" description="Try another search, level, or target." action={<Button variant="secondary" onClick={clearFilters}>Clear filters</Button>} />
        ) : (
          <>
            <ol className="diagnostic-log" aria-label="Recent PaperlessLLM log entries">
              {filteredEntries.map((entry, index) => (
                <li key={`${entry.timestamp}-${entry.event}-${index}`} className={`diagnostic-log__entry diagnostic-log__entry--${entry.level.toLowerCase()}`}>
                  <time dateTime={entry.timestamp} title={formatDateTime(entry.timestamp)}>{entry.timestamp}</time>
                  <strong>{entry.level}</strong>
                  <span className="diagnostic-log__target">{entry.target}</span>
                  <div className="diagnostic-log__context">
                    <code title={entry.event}>{eventLabel(entry.event)}</code>
                    {Object.keys(entry.fields).length > 0 && (
                      <dl>{Object.entries(entry.fields).map(([key, value]) => (
                        <div key={key} className={key === "detail" ? "diagnostic-log__detail" : undefined}><dt>{fieldLabel(key)}</dt><dd>{value}</dd></div>
                      ))}</dl>
                    )}
                  </div>
                </li>
              ))}
            </ol>
            <div className="diagnostic-log__footer" aria-live="polite">
              <span>Newest first · {filteredEntries.length}{hasFilters ? ` of ${logs.data.entries.length}` : ""} recent {logs.data.entries.length === 1 ? "entry" : "entries"}</span>
              <span>{updatedAt ? `Updated ${formatDateTime(updatedAt)}` : "Update time unavailable"}</span>
            </div>
          </>
        )}
      </Card>
    </div>
  );
}
