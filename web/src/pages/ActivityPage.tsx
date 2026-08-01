import { useEffect, useState } from "react";
import { api } from "../api/client";
import type { Job, JobState } from "../api/types";
import { useApp } from "../App";
import { useToast } from "../components/Toast";
import { Alert, Badge, Button, Card, ConfirmDialog, EmptyState, ErrorState, Input, LoadingState, PageHeader, Select } from "../components/UI";
import { WorkerControls } from "../components/WorkerControls";
import { WorkerLiveStatus } from "../components/WorkerLiveStatus";
import { formatDateTime, formatTokens, titleCase } from "../lib/format";
import { useAsync, usePollingStale, useVisiblePolling } from "../lib/hooks";

const states: Array<{ value: "" | JobState; label: string }> = [
  { value: "", label: "All states" }, { value: "queued", label: "Queued" }, { value: "analyzing", label: "Analyzing" }, { value: "awaiting_review", label: "Awaiting review" }, { value: "applying", label: "Applying" }, { value: "completed", label: "Completed" }, { value: "budget_blocked", label: "Budget blocked" }, { value: "failed", label: "Failed" }, { value: "rejected", label: "Rejected" },
];

const tone: Record<JobState, "neutral" | "success" | "warning" | "danger" | "info" | "accent"> = { queued: "neutral", analyzing: "accent", awaiting_review: "warning", applying: "info", completed: "success", budget_blocked: "warning", failed: "danger", rejected: "neutral" };

function providerKindLabel(kind?: Job["provider_kind"]) {
  if (!kind) return "—";
  if (kind === "openai") return "OpenAI";
  if (kind === "lmstudio") return "LM Studio";
  return titleCase(kind);
}

function apiFamilyLabel(family?: Job["api_family"]) {
  if (family === "responses") return "Responses API";
  if (family === "chat_completions") return "Chat Completions";
  if (family === "messages") return "Anthropic Messages";
  return "—";
}

function failurePhaseLabel(phase?: Job["failure_phase"]) {
  if (phase === "preflight") return "AI request preflight";
  if (phase === "provider") return "AI provider";
  if (phase === "paperless") return "Paperless API";
  if (phase === "paperless_apply") return "Paperless update";
  if (phase === "budget") return "Token budget";
  return phase === "processing" ? "Document processing" : "—";
}

function latestProviderAttempt(job: Job) {
  if (!job.provider_operation || !job.provider_status) {
    return job.failure_phase === "preflight"
      ? "No AI inference recorded · 0 inference attempts"
      : "No recorded AI request";
  }
  const latency = job.provider_latency_ms === undefined
    ? ""
    : job.provider_latency_ms < 1_000
      ? ` · ${job.provider_latency_ms} ms`
      : ` · ${(job.provider_latency_ms / 1_000).toFixed(1)} s`;
  const attempt = job.provider_attempt === undefined ? "" : `Attempt ${job.provider_attempt} · `;
  return `${attempt}${titleCase(job.provider_operation)} · ${titleCase(job.provider_status)}${latency}`;
}

export function ActivityPage() {
  const [state, setState] = useState("");
  const [query, setQuery] = useState("");
  const [search, setSearch] = useState("");
  const [offset, setOffset] = useState(0);
  const [expanded, setExpanded] = useState<string>();
  const [retrying, setRetrying] = useState<string>();
  const [cancelTarget, setCancelTarget] = useState<Job>();
  const [cancelling, setCancelling] = useState(false);
  const limit = 20;
  const { status, statusLastSuccessAt, statusPollError, refreshStatus } = useApp();
  const { showToast } = useToast();
  const jobs = useAsync(() => api.jobs({ state, q: search, limit, offset }), [state, search, offset]);

  useEffect(() => {
    const timer = window.setTimeout(() => { setOffset(0); setSearch(query); }, 350);
    return () => window.clearTimeout(timer);
  }, [query]);

  useVisiblePolling(jobs.refreshSilently, 2_000);
  const jobsStale = usePollingStale(jobs.lastSuccessAt, jobs.backgroundError);

  useEffect(() => {
    if (!jobs.data || offset === 0) return;
    const lastPageOffset = jobs.data.total === 0 ? 0 : Math.floor((jobs.data.total - 1) / limit) * limit;
    if (offset > lastPageOffset) setOffset(lastPageOffset);
  }, [jobs.data, limit, offset]);

  async function retry(job: Job) {
    setRetrying(job.id);
    try {
      await api.retryJob(job.id);
      showToast(`Document #${job.document_id} queued again`);
      await jobs.refresh();
    } catch (caught) {
      showToast(caught instanceof Error ? caught.message : "Could not retry job", "error");
    } finally {
      setRetrying(undefined);
    }
  }

  async function removeFromQueue() {
    if (!cancelTarget) return;
    setCancelling(true);
    try {
      await api.cancelJob(cancelTarget.id);
      showToast(`Document #${cancelTarget.document_id} removed from the queue`);
      setCancelTarget(undefined);
      await Promise.all([jobs.refresh(), refreshStatus()]);
    } catch (caught) {
      showToast(caught instanceof Error ? caught.message : "Could not remove the queued job", "error");
    } finally {
      setCancelling(false);
    }
  }

  return (
    <div className="page">
      <PageHeader eyebrow="Audit trail" title="Activity" description="Every document run, decision, and error in one place." actions={<div className="page-actions"><WorkerControls onChanged={jobs.refreshSilently} /><Button variant="secondary" onClick={() => void jobs.refresh()}>Refresh</Button></div>} />
      <WorkerLiveStatus worker={status.worker} lastSuccessAt={statusLastSuccessAt} pollError={statusPollError} showLink={false} />
      {jobs.data && jobsStale && (
        <Alert tone="warning" title="Activity updates are reconnecting">
          <p>Showing the last successful activity snapshot{jobs.lastSuccessAt ? ` from ${formatDateTime(new Date(jobs.lastSuccessAt).toISOString())}` : ""}. PaperlessLLM will retry automatically.</p>
        </Alert>
      )}
      <Card padded={false} className="activity-table-card">
        <div className="table-toolbar">
          <label className="search-field"><span aria-hidden="true">⌕</span><Input aria-label="Search activity" value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search title or document ID…" /></label>
          <Select aria-label="Filter by state" value={state} onChange={(event) => { setOffset(0); setState(event.target.value); }}>{states.map((item) => <option key={item.value} value={item.value}>{item.label}</option>)}</Select>
        </div>
        {jobs.loading ? <LoadingState label="Loading activity" /> : jobs.error ? <div className="inset-state"><ErrorState message={jobs.error} onRetry={() => void jobs.refresh()} /></div> : !jobs.data?.items.length ? (
          <EmptyState mark="◎" title={search || state ? "No matching activity" : "No activity yet"} description={search || state ? "Try a different search or state filter." : "Jobs appear here after PaperlessLLM discovers a document."} action={(search || state) && <Button variant="secondary" onClick={() => { setQuery(""); setSearch(""); setState(""); }}>Clear filters</Button>} />
        ) : (
          <>
            <div className="data-table-wrap">
              <table className="data-table activity-table">
                <thead><tr><th>Document</th><th>Status</th><th>Provider / model</th><th>Usage</th><th>Updated</th><th><span className="visually-hidden">Actions</span></th></tr></thead>
                <tbody>{jobs.data.items.map((job) => <ActivityTableRow key={job.id} job={job} expanded={expanded === job.id} retrying={retrying === job.id} onExpand={() => setExpanded(expanded === job.id ? undefined : job.id)} onRetry={() => void retry(job)} onRemove={() => setCancelTarget(job)} />)}</tbody>
              </table>
            </div>
            <div className="pagination"><span>Showing {offset + 1}–{Math.min(offset + limit, jobs.data.total)} of {jobs.data.total}</span><div><Button size="sm" variant="ghost" disabled={offset === 0} onClick={() => setOffset(Math.max(0, offset - limit))}>Previous</Button><Button size="sm" variant="ghost" disabled={offset + limit >= jobs.data.total} onClick={() => setOffset(offset + limit)}>Next</Button></div></div>
          </>
        )}
      </Card>
      <ConfirmDialog
        open={Boolean(cancelTarget)}
        title="Remove this document from the queue?"
        description={cancelTarget ? `${cancelTarget.document_title || `Document #${cancelTarget.document_id}`} will not be processed unless you manually reprocess it or Paperless receives a new file version. Its activity record and any recorded usage are kept.` : "The queued document will be removed."}
        confirmLabel="Remove from queue"
        danger
        busy={cancelling}
        onCancel={() => setCancelTarget(undefined)}
        onConfirm={() => void removeFromQueue()}
      />
    </div>
  );
}

export function ActivityTableRow({ job, expanded, retrying, onExpand, onRetry, onRemove }: { job: Job; expanded: boolean; retrying: boolean; onExpand: () => void; onRetry: () => void; onRemove: () => void }) {
  const retryable = job.state === "failed" || job.state === "rejected" || job.state === "completed";
  const removable = job.state === "queued" || job.state === "budget_blocked";
  return <>
    <tr className={expanded ? "is-expanded" : ""}>
      <td><div className="table-document"><span className="document-mark">{job.document_title?.slice(0, 1).toUpperCase() || "D"}</span><div><strong>{job.document_title || `Document #${job.document_id}`}</strong><span>#{job.document_id} · attempt {job.attempt}{job.truncated ? " · input truncated" : ""}</span></div></div></td>
      <td><Badge tone={tone[job.state]}>{titleCase(job.state)}</Badge></td>
      <td><div className="stacked-cell"><span>{job.provider_name ?? "—"}</span><small>{job.model ?? ""}</small></div></td>
      <td>{job.token_total !== undefined ? formatTokens(job.token_total) : "—"}</td>
      <td><time dateTime={job.updated_at}>{formatDateTime(job.updated_at)}</time></td>
      <td><button className="row-menu" aria-label={expanded ? "Hide job details" : "Show job details"} onClick={onExpand}>{expanded ? "−" : "•••"}</button></td>
    </tr>
    {expanded && <tr className="expanded-row"><td colSpan={6}>
      <div className="job-detail">
        <div><span>Enabled functions</span><strong>{job.functions.length ? job.functions.map(titleCase).join(", ") : "None recorded"}</strong></div>
        <div><span>Created</span><strong>{formatDateTime(job.created_at)}</strong></div>
        <div><span>Job attempt</span><strong>{job.attempt}</strong></div>
        <div><span>Provider adapter</span><strong>{providerKindLabel(job.provider_kind)}</strong></div>
        <div><span>API family</span><strong>{apiFamilyLabel(job.api_family)}</strong></div>
        <div><span>Latest AI request</span><strong>{latestProviderAttempt(job)}</strong></div>
        {job.error && <div className="job-error" role="alert">
          <span>Safe failure detail</span>
          <strong>{job.error}</strong>
          <div className="job-error__meta">
            {job.error_code && <code>{job.error_code}</code>}
            {job.failure_phase && <small>{failurePhaseLabel(job.failure_phase)}</small>}
            {job.upstream_status !== undefined && <small>Upstream HTTP {job.upstream_status}</small>}
          </div>
        </div>}
        <div className="job-detail__actions">
          {job.error && <a href="/logs" className="button button--ghost button--sm">Open safe logs</a>}
          {job.document_url && <a href={job.document_url} target="_blank" rel="noreferrer" className="button button--ghost button--sm">Open in Paperless ↗</a>}
          {removable && <Button size="sm" variant="danger" onClick={onRemove}>Remove from queue</Button>}
          {retryable && <Button size="sm" variant="secondary" loading={retrying} onClick={onRetry}>Reprocess</Button>}
        </div>
      </div>
    </td></tr>}
  </>;
}
