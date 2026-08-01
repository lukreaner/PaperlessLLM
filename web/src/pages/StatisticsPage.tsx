import { useMemo, useState } from "react";
import { api } from "../api/client";
import { Badge, Card, ErrorState, PageHeader, ProgressBar, Skeleton } from "../components/UI";
import { budgetPercent, formatTokens, titleCase } from "../lib/format";
import { useAsync } from "../lib/hooks";

const ranges = [{ value: "7d", label: "7 days" }, { value: "30d", label: "30 days" }, { value: "12m", label: "12 months" }, { value: "all", label: "All time" }];

export function StatisticsPage() {
  const [range, setRange] = useState("30d");
  const summary = useAsync(() => api.statsSummary(range), [range]);
  const series = useAsync(() => api.statsSeries(range), [range]);
  const maxPoint = useMemo(() => Math.max(1, ...(series.data?.points.map((point) => point.total_tokens) ?? [1])), [series.data]);

  if (summary.error || series.error) return <div className="page"><PageHeader eyebrow="Usage intelligence" title="Statistics" /><ErrorState message={summary.error ?? series.error ?? "Usage statistics are unavailable."} onRetry={() => { void summary.refresh(); void series.refresh(); }} /></div>;
  const totals = summary.data?.totals;

  return (
    <div className="page">
      <PageHeader eyebrow="Usage intelligence" title="Statistics" description="Understand exactly where your AI tokens go." actions={<div className="range-picker">{ranges.map((item) => <button key={item.value} className={range === item.value ? "is-active" : ""} onClick={() => setRange(item.value)}>{item.label}</button>)}</div>} />
      <div className="stats-metrics">
        <StatMetric label="Total tokens" value={summary.loading ? undefined : formatTokens(totals?.total_tokens ?? 0)} note={`${totals?.requests ?? 0} requests`} tone="accent" />
        <StatMetric label="Input tokens" value={summary.loading ? undefined : formatTokens(totals?.input_tokens ?? 0)} note="Document context" tone="blue" />
        <StatMetric label="Output tokens" value={summary.loading ? undefined : formatTokens(totals?.output_tokens ?? 0)} note="Includes reasoning" tone="violet" />
        <StatMetric label="Cache savings" value={summary.loading ? undefined : formatTokens(totals?.cache_read_tokens ?? 0)} note="Cache-read tokens" tone="green" />
      </div>
      <div className="stats-grid">
        <Card className="usage-chart-card">
          <div className="card-heading"><div><span className="eyebrow">Token volume</span><h2>Usage over time</h2></div>{totals && (totals.estimated_requests > 0 || (totals.unknown_requests ?? 0) > 0) && <Badge tone="warning">{totals.estimated_requests + (totals.unknown_requests ?? 0)} estimated or unknown</Badge>}</div>
          {series.loading ? <div className="chart-skeleton"><Skeleton height={210} /></div> : series.error ? <p className="muted">Couldn’t load usage history.</p> : !series.data?.points.length ? <div className="chart-empty"><span>▥</span><p>Usage appears here after the first AI request.</p></div> : (
            <div className="bar-chart" role="img" aria-label="Token usage over time">
              <div className="bar-chart__plot">{series.data.points.map((point) => {
                const outputHeight = (point.output_tokens / maxPoint) * 100;
                const inputHeight = (point.input_tokens / maxPoint) * 100;
                const cacheHeight = ((point.cache_read_tokens + point.cache_write_tokens) / maxPoint) * 100;
                return <div className="bar-column" key={point.period} title={`${point.period}: ${point.total_tokens.toLocaleString()} tokens`}><div className="bar-column__bars"><span className="bar bar--cache" style={{ height: `${cacheHeight}%` }} /><span className="bar bar--output" style={{ height: `${outputHeight}%` }} /><span className="bar bar--input" style={{ height: `${inputHeight}%` }} /></div><span>{compactPeriod(point.period, series.data!.points.length)}</span></div>;
              })}</div>
              <div className="chart-legend"><span><i className="legend-dot legend-dot--input" />Input</span><span><i className="legend-dot legend-dot--output" />Output</span><span><i className="legend-dot legend-dot--cache" />Cache</span><small>Reasoning is included in output, never added twice.</small></div>
            </div>
          )}
        </Card>
        <Card>
          <div className="card-heading"><div><span className="eyebrow">Current cycle</span><h2>Budget health</h2></div></div>
          {summary.loading ? <div className="budget-skeleton"><Skeleton /><Skeleton /><Skeleton /></div> : summary.data && <div className="large-budget-list">
            <BudgetGauge label="Daily budget" used={summary.data.budgets.daily_used} limit={summary.data.budgets.daily_limit} warning={summary.data.budgets.warning_percent} />
            <BudgetGauge label="Monthly budget" used={summary.data.budgets.monthly_used} limit={summary.data.budgets.monthly_limit} warning={summary.data.budgets.warning_percent} />
            <p>Resets in <strong>{summary.data.budgets.timezone}</strong>. Calls stop at 100% and blocked jobs resume after reset.</p>
          </div>}
        </Card>
      </div>
      <div className="breakdown-grid">
        <Card>
          <div className="card-heading"><div><span className="eyebrow">Models</span><h2>By provider</h2></div></div>
          {summary.loading ? <BreakdownSkeleton /> : !summary.data?.by_provider.length ? <p className="muted">No provider usage in this range.</p> : <BreakdownTable rows={summary.data.by_provider.map((row) => ({ label: row.provider, detail: row.model, value: row.total_tokens, meta: `${row.requests} requests` }))} />}
        </Card>
        <Card>
          <div className="card-heading"><div><span className="eyebrow">Work performed</span><h2>By function</h2></div></div>
          {summary.loading ? <BreakdownSkeleton /> : !summary.data?.by_function.length ? <p className="muted">No function usage in this range.</p> : <BreakdownTable rows={summary.data.by_function.map((row) => ({ label: titleCase(row.function), detail: `${row.documents} documents`, value: row.total_tokens }))} />}
        </Card>
      </div>
      <Card className="token-detail-card">
        <div><span className="eyebrow">Detailed accounting</span><h2>Token categories</h2><p>Reported usage is preferred. When a provider omits usage, PaperlessLLM records a visible estimate instead of zero.</p></div>
        <div className="token-detail-grid"><TokenDetail label="Input" value={totals?.input_tokens} /><TokenDetail label="Cache read" value={totals?.cache_read_tokens} /><TokenDetail label="Cache write" value={totals?.cache_write_tokens} /><TokenDetail label="Output" value={totals?.output_tokens} /><TokenDetail label="Reasoning / thinking" value={totals?.reasoning_tokens} subset /><TokenDetail label="Failed requests" value={totals?.failed_requests} plain /></div>
        {totals && <div className="capability-row" aria-label="Usage source breakdown"><Badge tone="success">{totals.reported_requests ?? 0} reported</Badge><Badge tone="info">{totals.preflight_counted_requests ?? 0} preflight-counted</Badge><Badge tone="warning">{totals.estimated_requests} estimated</Badge><Badge tone={(totals.unknown_requests ?? 0) > 0 ? "danger" : "neutral"}>{totals.unknown_requests ?? 0} unknown</Badge></div>}
      </Card>
    </div>
  );
}

function compactPeriod(value: string, count: number): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return new Intl.DateTimeFormat(undefined, count > 40 ? { month: "short" } : { month: "short", day: "numeric" }).format(date);
}

function StatMetric({ label, value, note, tone }: { label: string; value?: string; note: string; tone: string }) { return <div className={`stat-metric stat-metric--${tone}`}><span>{label}</span>{value === undefined ? <Skeleton width="88px" height={32} /> : <strong>{value}</strong>}<small>{note}</small></div>; }

function BudgetGauge({ label, used, limit, warning }: { label: string; used: number; limit: number | null; warning: number }) {
  const percent = budgetPercent(used, limit); const tone = percent >= 100 ? "danger" : percent >= warning ? "warning" : "accent";
  return <div className="large-budget"><div><strong>{label}</strong><span>{limit ? `${Math.round(percent)}%` : "Unlimited"}</span></div><ProgressBar value={percent} tone={tone} label={label} /><p>{formatTokens(used)} used{limit ? ` of ${formatTokens(limit)}` : ""}</p></div>;
}

function BreakdownTable({ rows }: { rows: Array<{ label: string; detail: string; value: number; meta?: string }> }) {
  const max = Math.max(1, ...rows.map((row) => row.value));
  return <div className="breakdown-table">{rows.map((row) => <div className="breakdown-row" key={`${row.label}-${row.detail}`}><div><strong>{row.label}</strong><span>{row.detail}</span></div><div className="breakdown-row__value"><strong>{formatTokens(row.value)}</strong>{row.meta && <small>{row.meta}</small>}</div><div className="breakdown-row__bar"><span style={{ width: `${(row.value / max) * 100}%` }} /></div></div>)}</div>;
}

function BreakdownSkeleton() { return <div className="breakdown-table">{[1, 2, 3].map((item) => <div key={item} className="breakdown-row"><Skeleton width="45%" /><Skeleton width="60px" /></div>)}</div>; }
function TokenDetail({ label, value, subset, plain }: { label: string; value?: number; subset?: boolean; plain?: boolean }) { return <div><span>{label}{subset && <sup>subset</sup>}</span><strong>{value === undefined ? "—" : plain ? value.toLocaleString() : formatTokens(value)}</strong></div>; }
