import { useEffect, useState } from "react";
import { api } from "../api/client";
import type { Job, JobState } from "../api/types";
import { useToast } from "../components/Toast";
import { Badge, Button, Card, EmptyState, ErrorState, Input, LoadingState, PageHeader, Select } from "../components/UI";
import { formatDateTime, formatTokens, titleCase } from "../lib/format";
import { useAsync } from "../lib/hooks";

const states: Array<{ value: "" | JobState; label: string }> = [
  { value: "", label: "All states" }, { value: "queued", label: "Queued" }, { value: "analyzing", label: "Analyzing" }, { value: "awaiting_review", label: "Awaiting review" }, { value: "applying", label: "Applying" }, { value: "completed", label: "Completed" }, { value: "budget_blocked", label: "Budget blocked" }, { value: "failed", label: "Failed" }, { value: "rejected", label: "Rejected" },
];

const tone: Record<JobState, "neutral" | "success" | "warning" | "danger" | "info" | "accent"> = { queued: "neutral", analyzing: "accent", awaiting_review: "warning", applying: "info", completed: "success", budget_blocked: "warning", failed: "danger", rejected: "neutral" };

export function ActivityPage() {
  const [state, setState] = useState("");
  const [query, setQuery] = useState("");
  const [search, setSearch] = useState("");
  const [offset, setOffset] = useState(0);
  const [expanded, setExpanded] = useState<string>();
  const [retrying, setRetrying] = useState<string>();
  const limit = 20;
  const { showToast } = useToast();
  const jobs = useAsync(() => api.jobs({ state, q: search, limit, offset }), [state, search, offset]);

  useEffect(() => {
    const timer = window.setTimeout(() => { setOffset(0); setSearch(query); }, 350);
    return () => window.clearTimeout(timer);
  }, [query]);

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

  return (
    <div className="page">
      <PageHeader eyebrow="Audit trail" title="Activity" description="Every document run, decision, and error in one place." actions={<Button variant="secondary" onClick={() => void jobs.refresh()}>Refresh</Button>} />
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
                <tbody>{jobs.data.items.map((job) => <ActivityTableRow key={job.id} job={job} expanded={expanded === job.id} retrying={retrying === job.id} onExpand={() => setExpanded(expanded === job.id ? undefined : job.id)} onRetry={() => void retry(job)} />)}</tbody>
              </table>
            </div>
            <div className="pagination"><span>Showing {offset + 1}–{Math.min(offset + limit, jobs.data.total)} of {jobs.data.total}</span><div><Button size="sm" variant="ghost" disabled={offset === 0} onClick={() => setOffset(Math.max(0, offset - limit))}>Previous</Button><Button size="sm" variant="ghost" disabled={offset + limit >= jobs.data.total} onClick={() => setOffset(offset + limit)}>Next</Button></div></div>
          </>
        )}
      </Card>
    </div>
  );
}

function ActivityTableRow({ job, expanded, retrying, onExpand, onRetry }: { job: Job; expanded: boolean; retrying: boolean; onExpand: () => void; onRetry: () => void }) {
  const retryable = job.state === "failed" || job.state === "rejected" || job.state === "completed";
  return <>
    <tr className={expanded ? "is-expanded" : ""}>
      <td><div className="table-document"><span className="document-mark">{job.document_title?.slice(0, 1).toUpperCase() || "D"}</span><div><strong>{job.document_title || `Document #${job.document_id}`}</strong><span>#{job.document_id} · attempt {job.attempt}{job.truncated ? " · input truncated" : ""}</span></div></div></td>
      <td><Badge tone={tone[job.state]}>{titleCase(job.state)}</Badge></td>
      <td><div className="stacked-cell"><span>{job.provider_name ?? "—"}</span><small>{job.model ?? ""}</small></div></td>
      <td>{job.token_total !== undefined ? formatTokens(job.token_total) : "—"}</td>
      <td><time dateTime={job.updated_at}>{formatDateTime(job.updated_at)}</time></td>
      <td><button className="row-menu" aria-label={expanded ? "Hide job details" : "Show job details"} onClick={onExpand}>{expanded ? "−" : "•••"}</button></td>
    </tr>
    {expanded && <tr className="expanded-row"><td colSpan={6}><div className="job-detail"><div><span>Enabled functions</span><strong>{job.functions.length ? job.functions.map(titleCase).join(", ") : "None recorded"}</strong></div><div><span>Created</span><strong>{formatDateTime(job.created_at)}</strong></div>{job.error && <div className="job-error"><span>Error</span><code>{job.error}</code></div>}<div className="job-detail__actions">{job.document_url && <a href={job.document_url} target="_blank" rel="noreferrer" className="button button--ghost button--sm">Open in Paperless ↗</a>}{retryable && <Button size="sm" variant="secondary" loading={retrying} onClick={onRetry}>Reprocess</Button>}</div></div></td></tr>}
  </>;
}
