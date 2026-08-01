import type { WorkerStatus } from "../api/types";
import { formatRelativeTime, titleCase } from "../lib/format";
import { usePollingStale } from "../lib/hooks";
import { Link } from "../lib/router";

export function WorkerLiveStatus({ worker, lastSuccessAt, pollError, showLink = true }: { worker: WorkerStatus; lastSuccessAt?: number; pollError?: string; showLink?: boolean }) {
  const paused = worker.paused || worker.state === "paused";
  const active = worker.active_job;
  const scan = worker.scan_progress;
  const pausing = paused && Boolean(active || scan);
  const stale = usePollingStale(lastSuccessAt, pollError);
  const lastUpdate = lastSuccessAt ? new Date(lastSuccessAt).toISOString() : undefined;
  return (
    <div className={`live-worker ${stale ? "live-worker--stale" : paused ? "live-worker--paused" : active || scan ? "live-worker--active" : ""}`} aria-live="polite">
      <span className="live-worker__pulse" aria-hidden="true" />
      <div>
        <span>{stale ? "Live updates interrupted" : pausing ? "Pausing safely" : paused ? "Processing paused" : scan ? "Scanning Paperless" : active ? "Working now" : "Worker live"}</span>
        <strong>{stale ? `Last known: ${active ? active.document_title || `Document #${active.document_id}` : scan ? `${scan.discovered} documents discovered` : paused ? "processing paused" : "no active document"}` : active ? active.document_title || `Document #${active.document_id}` : scan ? `${scan.discovered} discovered · ${scan.queued} queued` : paused ? "Waiting documents will stay queued until you continue." : "No document is currently being processed."}</strong>
        {stale
          ? <small>{lastUpdate ? `Last successful update ${formatRelativeTime(lastUpdate)}` : "No successful live update"} · reconnecting automatically</small>
          : active
            ? <small>Document #{active.document_id} · {titleCase(active.state)} · updated {formatRelativeTime(active.updated_at)}{pausing ? " · no new job will start" : ""}</small>
            : scan && <small>{titleCase(scan.trigger)} scan · started {formatRelativeTime(scan.started_at)}{pausing ? " · no job will start after this scan" : ""}</small>}
      </div>
      {showLink && <Link className="text-link" to="/activity">Open live activity →</Link>}
    </div>
  );
}
