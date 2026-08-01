import { useState } from "react";
import { api } from "../api/client";
import type { ActiveJob, Job, JobState } from "../api/types";
import { useApp } from "../App";
import { useToast } from "../components/Toast";
import { Badge, Button, Card, ErrorState, PageHeader, ProgressBar, Skeleton, StatusDot } from "../components/UI";
import { WorkerControls } from "../components/WorkerControls";
import { WorkerLiveStatus } from "../components/WorkerLiveStatus";
import { budgetPercent, formatRelativeTime, formatTokens, titleCase } from "../lib/format";
import { useAsync, usePollingStale, useVisiblePolling } from "../lib/hooks";
import { Link } from "../lib/router";

const stateTone: Record<JobState, "neutral" | "success" | "warning" | "danger" | "info" | "accent"> = {
  queued: "neutral", analyzing: "accent", awaiting_review: "warning", applying: "info", completed: "success", budget_blocked: "warning", failed: "danger", rejected: "neutral",
};

export function OverviewPage() {
  const { status, settings, providers, statusLastSuccessAt, statusPollError, refreshStatus } = useApp();
  const [scanning, setScanning] = useState(false);
  const { showToast } = useToast();
  const stats = useAsync(() => api.statsSummary("month"), []);
  const jobs = useAsync(() => api.jobs({ limit: 6, offset: 0 }), []);

  useVisiblePolling(jobs.refreshSilently, 2_000);
  useVisiblePolling(stats.refreshSilently, 15_000);
  const jobsStale = usePollingStale(jobs.lastSuccessAt, jobs.backgroundError);
  const statusStale = usePollingStale(statusLastSuccessAt, statusPollError);

  async function scan() {
    setScanning(true);
    try {
      const result = await api.scan();
      showToast(result.message ?? `${result.queued} document${result.queued === 1 ? "" : "s"} queued`);
      await Promise.all([refreshStatus(), jobs.refresh(), stats.refresh()]);
    } catch (caught) {
      showToast(caught instanceof Error ? caught.message : "Scan failed", "error");
    } finally {
      setScanning(false);
    }
  }

  const queueTotal = Object.entries(status.queue).filter(([state]) => !["completed", "failed", "rejected"].includes(state)).reduce((sum, [, count]) => sum + count, 0);
  const daily = stats.data?.budgets;
  const monthlyPercent = daily ? budgetPercent(daily.monthly_used, daily.monthly_limit) : 0;
  const dailyPercent = daily ? budgetPercent(daily.daily_used, daily.daily_limit) : 0;
  const activeProvider = providers.find((provider) => provider.id === settings.active_provider_id || provider.active);
  const paused = status.worker.paused || status.worker.state === "paused";
  const activeJobs = status.worker.active_jobs?.length ? status.worker.active_jobs : status.worker.active_job ? [status.worker.active_job] : [];
  const activeJobCount = activeJobs.length;
  const activeQueueDetail = activeJobCount > 1
    ? `${paused ? "Finishing" : "Working on"} ${activeJobCount} documents in parallel`
    : activeJobs[0]
      ? `${paused ? "Finishing" : "Working on"} document #${activeJobs[0].document_id}`
      : status.worker.scan_progress
        ? `${paused ? "Finishing scan" : "Scanning"} · ${status.worker.scan_progress.discovered.toLocaleString()} discovered`
        : paused ? "Processing is paused" : "Worker is ready";
  const workerHealthDetail = statusStale
    ? "Live status unavailable — reconnecting"
    : activeJobCount > 1
      ? `${activeJobCount} documents are being analyzed in parallel${paused ? "; pausing afterward" : ""}`
      : activeJobs[0]
        ? `Document #${activeJobs[0].document_id} is ${titleCase(activeJobs[0].state)}${paused ? "; pausing afterward" : ""}`
        : status.worker.scan_progress
          ? `${titleCase(status.worker.scan_progress.trigger)} scan: ${status.worker.scan_progress.discovered.toLocaleString()} discovered, ${status.worker.scan_progress.queued.toLocaleString()} queued${paused ? "; pausing afterward" : ""}`
          : paused
            ? "Paused — queued documents are preserved"
            : status.worker.next_scan_at ? `Next scan ${formatRelativeTime(status.worker.next_scan_at)}` : "Ready for a manual scan";

  return (
    <div className="page">
      <PageHeader eyebrow="Control room" title="Overview" description="Your document pipeline at a glance." actions={<div className="page-actions"><WorkerControls onChanged={jobs.refreshSilently} /><Button loading={scanning} disabled={paused} title={paused ? "Continue processing before starting a scan" : undefined} onClick={() => void scan()}><span aria-hidden="true">↻</span> Scan now</Button></div>} />

      {(status.queue.budget_blocked ?? 0) > 0 && (
        <div className="budget-banner"><span>!</span><div><strong>{status.queue.budget_blocked} job{status.queue.budget_blocked === 1 ? " is" : "s are"} waiting for a token budget reset</strong><p>Processing resumes automatically after reset or when you raise the limit.</p></div><Link to="/settings?section=limits">Review limits</Link></div>
      )}

      <WorkerLiveStatus worker={status.worker} lastSuccessAt={statusLastSuccessAt} pollError={statusPollError} />
      {activeJobCount > 1 && <ActiveDocuments jobs={activeJobs} paused={paused} />}

      <div className="metric-grid">
        <MetricCard label="Active queue" value={queueTotal.toLocaleString()} meta={activeQueueDetail} mark="Q" tone="accent" />
        <MetricCard label="Awaiting review" value={(status.queue.awaiting_review ?? 0).toLocaleString()} meta={status.queue.awaiting_review ? "Your decision needed" : "All caught up"} mark="R" tone="gold" link="/review" />
        <MetricCard label="Tokens this month" value={stats.loading ? undefined : stats.error ? "—" : formatTokens(stats.data?.totals.total_tokens ?? 0)} meta={stats.data ? `${stats.data.totals.requests.toLocaleString()} AI requests` : "Usage unavailable"} mark="T" tone="violet" />
        <MetricCard label="Completed" value={(status.queue.completed ?? 0).toLocaleString()} meta="In recorded history" mark="✓" tone="green" />
      </div>

      <div className="overview-grid">
        <Card className="pipeline-card" padded={false}>
          <div className="card-heading card-heading--padded"><div><span className="eyebrow">Recent activity</span><h2>Document pipeline <span className={`live-label ${jobsStale ? "live-label--stale" : ""}`}><i />{jobsStale ? "Reconnecting" : "Live"}</span></h2></div><Link className="text-link" to="/activity">View all →</Link></div>
          {jobs.loading ? <TableSkeleton /> : jobs.error ? <div className="inset-error"><p>{jobs.error}</p><Button size="sm" variant="ghost" onClick={() => void jobs.refresh()}>Retry</Button></div> : !jobs.data?.items.length ? (
            <div className="pipeline-empty"><span>◎</span><h3>No documents processed yet</h3><p>Run a scan to discover documents that match your processing rules.</p><Button variant="secondary" size="sm" onClick={() => void scan()}>Run first scan</Button></div>
          ) : (
            <div className="activity-list">{jobs.data.items.map((job) => <ActivityRow key={job.id} job={job} />)}</div>
          )}
        </Card>

        <div className="overview-side">
          <Card>
            <div className="card-heading"><div><span className="eyebrow">Connections</span><h2>System health</h2></div><button className="icon-button" title="Refresh status" onClick={() => void refreshStatus()}>↻</button></div>
            <div className="health-list">
              <HealthRow label="Paperless-ngx" detail={status.paperless.message ?? settings.paperless.base_url} state={status.paperless.state} />
              <HealthRow label={activeProvider?.name ?? "AI provider"} detail={activeProvider?.model || status.provider.message || "Not configured"} state={status.provider.state} />
              <HealthRow label="Processing worker" detail={workerHealthDetail} state={statusStale ? "disconnected" : status.worker.state === "running" || status.worker.state === "scanning" ? "connected" : paused ? "degraded" : "disconnected"} />
            </div>
          </Card>
          <Card>
            <div className="card-heading"><div><span className="eyebrow">Token guardrails</span><h2>Budget</h2></div><Link className="text-link" to="/statistics">Details →</Link></div>
            {stats.loading ? <div className="budget-skeleton"><Skeleton /><Skeleton width="75%" /></div> : stats.error ? <p className="muted">Usage data is unavailable.</p> : (
              <div className="budget-list">
                <BudgetRow label="Today" used={daily?.daily_used ?? 0} limit={daily?.daily_limit ?? null} percent={dailyPercent} warning={daily?.warning_percent ?? 80} />
                <BudgetRow label="This month" used={daily?.monthly_used ?? 0} limit={daily?.monthly_limit ?? null} percent={monthlyPercent} warning={daily?.warning_percent ?? 80} />
                {!daily?.daily_limit && !daily?.monthly_limit && <p className="budget-note">No token budget configured. Per-document limits still apply.</p>}
              </div>
            )}
          </Card>
        </div>
      </div>
    </div>
  );
}

function ActiveDocuments({ jobs, paused }: { jobs: ActiveJob[]; paused: boolean }) {
  return (
    <section className="active-documents" aria-label="Active AI analyses">
      <div className="active-documents__heading">
        <div><span className="eyebrow">In progress</span><strong>{paused ? "Finishing active requests" : "Parallel document analysis"}</strong></div>
        <Badge tone={paused ? "warning" : "accent"}>{jobs.length} active</Badge>
      </div>
      <div className="active-documents__list">
        {jobs.map((job) => (
          <div className="active-documents__item" key={job.id}>
            <span aria-hidden="true">{job.document_title?.slice(0, 1).toUpperCase() || "D"}</span>
            <div>
              <strong>{job.document_title || `Document #${job.document_id}`}</strong>
              <small>Document #{job.document_id} · {titleCase(job.state)} · updated {formatRelativeTime(job.updated_at)}</small>
            </div>
          </div>
        ))}
      </div>
    </section>
  );
}

function MetricCard({ label, value, meta, mark, tone, link }: { label: string; value?: string; meta: string; mark: string; tone: string; link?: string }) {
  const content = <><div className={`metric-card__mark metric-card__mark--${tone}`}>{mark}</div><div><span>{label}</span>{value === undefined ? <Skeleton width="76px" height={36} /> : <strong>{value}</strong>}<small>{meta}</small></div></>;
  return link ? <Link className="metric-card metric-card--link" to={link}>{content}</Link> : <div className="metric-card">{content}</div>;
}

function ActivityRow({ job }: { job: Job }) {
  return (
    <div className="activity-row">
      <div className="document-mark">{job.document_title?.slice(0, 1).toUpperCase() || "D"}</div>
      <div className="activity-row__document"><strong>{job.document_title || `Document #${job.document_id}`}</strong><span>#{job.document_id} · {formatRelativeTime(job.updated_at)}</span></div>
      <div className="activity-row__provider"><span>{job.provider_name ?? "—"}</span><small>{job.model ?? ""}</small></div>
      <Badge tone={stateTone[job.state]}>{titleCase(job.state)}</Badge>
    </div>
  );
}

function HealthRow({ label, detail, state }: { label: string; detail: string; state: string }) {
  const tone = state === "connected" ? "success" : state === "degraded" ? "warning" : state === "unconfigured" ? "neutral" : "danger";
  return <div className="health-row"><StatusDot state={tone} /><div><strong>{label}</strong><span>{detail}</span></div><Badge tone={tone === "danger" ? "danger" : tone === "success" ? "success" : "warning"}>{state}</Badge></div>;
}

function BudgetRow({ label, used, limit, percent, warning }: { label: string; used: number; limit: number | null; percent: number; warning: number }) {
  const tone = percent >= 100 ? "danger" : percent >= warning ? "warning" : "accent";
  return <div className="budget-row"><div><strong>{label}</strong><span>{formatTokens(used)} / {limit ? formatTokens(limit) : "unlimited"}</span></div><ProgressBar value={percent} tone={tone} label={`${label} token budget`} /></div>;
}

function TableSkeleton() {
  return <div className="activity-list">{[1, 2, 3, 4].map((item) => <div key={item} className="activity-row"><Skeleton width="42px" height={42} /><div className="activity-row__document"><Skeleton width="70%" /><Skeleton width="40%" height={11} /></div><Skeleton width="72px" /></div>)}</div>;
}
