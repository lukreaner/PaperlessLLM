import { useState } from "react";
import { api } from "../api/client";
import type { Job, JobState } from "../api/types";
import { useApp } from "../App";
import { useToast } from "../components/Toast";
import { Badge, Button, Card, ErrorState, PageHeader, ProgressBar, Skeleton, StatusDot } from "../components/UI";
import { budgetPercent, formatRelativeTime, formatTokens, titleCase } from "../lib/format";
import { useAsync } from "../lib/hooks";
import { Link } from "../lib/router";

const stateTone: Record<JobState, "neutral" | "success" | "warning" | "danger" | "info" | "accent"> = {
  queued: "neutral", analyzing: "accent", awaiting_review: "warning", applying: "info", completed: "success", budget_blocked: "warning", failed: "danger", rejected: "neutral",
};

export function OverviewPage() {
  const { status, settings, providers, refreshStatus } = useApp();
  const [scanning, setScanning] = useState(false);
  const { showToast } = useToast();
  const stats = useAsync(() => api.statsSummary("month"), []);
  const jobs = useAsync(() => api.jobs({ limit: 6, offset: 0 }), []);

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

  return (
    <div className="page">
      <PageHeader eyebrow="Control room" title="Overview" description="Your document pipeline at a glance." actions={<Button loading={scanning} onClick={() => void scan()}><span aria-hidden="true">↻</span> Scan now</Button>} />

      {(status.queue.budget_blocked ?? 0) > 0 && (
        <div className="budget-banner"><span>!</span><div><strong>{status.queue.budget_blocked} job{status.queue.budget_blocked === 1 ? " is" : "s are"} waiting for a token budget reset</strong><p>Processing resumes automatically after reset or when you raise the limit.</p></div><Link to="/settings?section=limits">Review limits</Link></div>
      )}

      <div className="metric-grid">
        <MetricCard label="Active queue" value={queueTotal.toLocaleString()} meta={(status.queue.analyzing ?? 0) > 0 ? `${status.queue.analyzing} analyzing now` : "Worker is ready"} mark="Q" tone="accent" />
        <MetricCard label="Awaiting review" value={(status.queue.awaiting_review ?? 0).toLocaleString()} meta={status.queue.awaiting_review ? "Your decision needed" : "All caught up"} mark="R" tone="gold" link="/review" />
        <MetricCard label="Tokens this month" value={stats.loading ? undefined : stats.error ? "—" : formatTokens(stats.data?.totals.total_tokens ?? 0)} meta={stats.data ? `${stats.data.totals.requests.toLocaleString()} AI requests` : "Usage unavailable"} mark="T" tone="violet" />
        <MetricCard label="Completed" value={(status.queue.completed ?? 0).toLocaleString()} meta="In recorded history" mark="✓" tone="green" />
      </div>

      <div className="overview-grid">
        <Card className="pipeline-card" padded={false}>
          <div className="card-heading card-heading--padded"><div><span className="eyebrow">Recent activity</span><h2>Document pipeline</h2></div><Link className="text-link" to="/activity">View all →</Link></div>
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
              <HealthRow label="Processing worker" detail={status.worker.next_scan_at ? `Next scan ${formatRelativeTime(status.worker.next_scan_at)}` : "Ready for a manual scan"} state={status.worker.state === "running" ? "connected" : status.worker.state === "paused" ? "degraded" : "disconnected"} />
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
