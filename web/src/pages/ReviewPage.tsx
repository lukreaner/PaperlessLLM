import { useEffect, useId, useMemo, useState } from "react";
import { api } from "../api/client";
import type { CatalogItem, FieldValue, ReviewItem, ReviewSuggestion } from "../api/types";
import { useApp } from "../App";
import { useToast } from "../components/Toast";
import { Alert, Badge, Button, Card, ConfirmDialog, EmptyState, ErrorState, Input, LoadingState, PageHeader, Select, Textarea } from "../components/UI";
import { formatDateTime, formatTokens } from "../lib/format";
import { useAsync } from "../lib/hooks";

export function ReviewPage() {
  const reviews = useAsync(() => api.reviews(), []);
  const { catalogs, settings, refreshStatus } = useApp();
  const [selectedId, setSelectedId] = useState<string>();
  const [edited, setEdited] = useState<ReviewSuggestion>();
  const [busyAction, setBusyAction] = useState<"approve" | "reject">();
  const [confirmReject, setConfirmReject] = useState(false);
  const { showToast } = useToast();

  const selected = useMemo(() => reviews.data?.items.find((item) => item.id === selectedId) ?? reviews.data?.items[0], [reviews.data, selectedId]);
  useEffect(() => {
    if (!selected) { setEdited(undefined); return; }
    setSelectedId(selected.id);
    setEdited(structuredClone(selected.suggested));
  }, [selected?.id]);

  async function finish(action: "approve" | "reject") {
    if (!selected || !edited) return;
    setBusyAction(action);
    try {
      if (action === "approve") await api.approveReview(selected.id, edited);
      else await api.rejectReview(selected.id);
      showToast(action === "approve" ? "Suggestions applied to Paperless" : "Suggestion rejected");
      setConfirmReject(false);
      setSelectedId(undefined);
      await Promise.all([reviews.refresh(), refreshStatus()]);
    } catch (caught) {
      showToast(caught instanceof Error ? caught.message : `Could not ${action} suggestion`, "error");
    } finally {
      setBusyAction(undefined);
    }
  }

  if (reviews.loading) return <div className="page"><PageHeader eyebrow="Human in the loop" title="Review" description="Approve AI suggestions before they reach Paperless." /><Card><LoadingState label="Loading review queue" /></Card></div>;
  if (reviews.error) return <div className="page"><PageHeader eyebrow="Human in the loop" title="Review" /><ErrorState message={reviews.error} onRetry={() => void reviews.refresh()} /></div>;
  if (!reviews.data?.items.length) return (
    <div className="page"><PageHeader eyebrow="Human in the loop" title="Review" description="Approve AI suggestions before they reach Paperless." />
      <Card><EmptyState mark="✓" title="The review queue is clear" description="There are no suggestions waiting for you. New items appear here when review mode is enabled." action={<Button variant="secondary" onClick={() => void reviews.refresh()}>Check again</Button>} /></Card>
    </div>
  );

  return (
    <div className="page page--review">
      <PageHeader eyebrow="Human in the loop" title="Review" description={`${reviews.data.total} suggestion${reviews.data.total === 1 ? "" : "s"} waiting for a decision.`} />
      <div className="review-layout">
        <Card className="review-queue" padded={false}>
          <div className="review-queue__header"><span>Queue</span><Badge tone="warning">{reviews.data.total}</Badge></div>
          <div className="review-queue__items">
            {reviews.data.items.map((item) => <ReviewQueueItem key={item.id} item={item} selected={item.id === selected?.id} onClick={() => setSelectedId(item.id)} />)}
          </div>
        </Card>
        {selected && edited && (
          <Card className="review-editor" padded={false}>
            <header className="review-editor__header">
              <div><span className="eyebrow">Document #{selected.document_id}</span><h2>{selected.document_title}</h2><p>Analyzed {formatDateTime(selected.created_at)}{selected.token_total ? ` · ${formatTokens(selected.token_total)} tokens` : ""}</p></div>
              {selected.document_url && <a href={selected.document_url} target="_blank" rel="noreferrer" className="button button--secondary button--sm">Open in Paperless ↗</a>}
            </header>
            {selected.warnings.length > 0 && <div className="review-warnings">{selected.warnings.map((warning) => <Alert tone="warning" key={warning}>{warning}</Alert>)}</div>}
            <div className="review-comparison">
              <ComparisonHeader />
              {Object.prototype.hasOwnProperty.call(selected.suggested, "title") && <ComparisonRow label="Title" current={selected.current.title} input={<Input value={edited.title ?? ""} onChange={(event) => setEdited({ ...edited, title: event.target.value })} />} />}
              {Object.prototype.hasOwnProperty.call(selected.suggested, "correspondent") && <ComparisonRow label="Correspondent" current={selected.current.correspondent} input={<ReviewTaxonomyEditor label="Suggested correspondent" value={edited.correspondent} choices={catalogs?.correspondents.map((item) => item.name) ?? []} allowNew={!settings.processing.restrict_existing_correspondents} onChange={(correspondent) => setEdited({ ...edited, correspondent })} />} />}
              {Object.prototype.hasOwnProperty.call(selected.suggested, "document_type") && <ComparisonRow label="Document type" current={selected.current.document_type} input={<ReviewTaxonomyEditor label="Suggested document type" value={edited.document_type} choices={catalogs?.document_types.map((item) => item.name) ?? []} allowNew={!settings.processing.restrict_existing_document_types} onChange={(document_type) => setEdited({ ...edited, document_type })} />} />}
              {Object.prototype.hasOwnProperty.call(selected.suggested, "tags") && <ComparisonRow label="Tags" current={(selected.current.tags ?? []).join(", ")} input={<TagEditor tags={edited.tags ?? []} onChange={(tags) => setEdited({ ...edited, tags })} choices={catalogs?.tags.map((item) => item.name) ?? []} />} />}
              {edited.custom_fields && Object.entries(edited.custom_fields).map(([name, value]) => {
                const definition = catalogs?.custom_fields.find((field) => field.name.trim().toLocaleLowerCase() === name.trim().toLocaleLowerCase());
                return <ComparisonRow key={name} label={name} current={displayCustomValue(definition, selected.current.custom_fields?.[name])} input={<div className="custom-review-editor"><CustomValueEditor definition={definition} value={value} onChange={(next) => setEdited({ ...edited, custom_fields: { ...edited.custom_fields, [name]: next } })} /><Button size="sm" variant="ghost" onClick={() => { const custom_fields = { ...edited.custom_fields }; delete custom_fields[name]; setEdited({ ...edited, custom_fields }); }}>Omit</Button></div>} />;
              })}
            </div>
            <footer className="review-editor__footer">
              <div><strong>Applying is conservative</strong><span>Paperless is re-fetched and unrelated metadata is preserved.</span></div>
              <div><Button variant="ghost" onClick={() => setConfirmReject(true)} disabled={Boolean(busyAction)}>Reject</Button><Button loading={busyAction === "approve"} disabled={Boolean(busyAction)} onClick={() => void finish("approve")}>Approve & apply</Button></div>
            </footer>
          </Card>
        )}
      </div>
      <ConfirmDialog open={confirmReject} title="Reject this suggestion?" description="The AI usage remains in statistics, but no metadata will be written to Paperless." confirmLabel="Reject suggestion" danger busy={busyAction === "reject"} onCancel={() => setConfirmReject(false)} onConfirm={() => void finish("reject")} />
    </div>
  );
}

function ReviewQueueItem({ item, selected, onClick }: { item: ReviewItem; selected: boolean; onClick: () => void }) {
  const changeCount = Object.keys(item.suggested).length;
  return <button className={`review-item ${selected ? "is-selected" : ""}`} onClick={onClick}><span className="document-mark">{item.document_title.slice(0, 1).toUpperCase()}</span><span><strong>{item.document_title}</strong><small>{changeCount} suggested change{changeCount === 1 ? "" : "s"} · {formatDateTime(item.created_at)}</small></span><span aria-hidden="true">›</span></button>;
}

function ComparisonHeader() {
  return <div className="comparison-header"><span>Field</span><span>Current in Paperless</span><span>AI suggestion · editable</span></div>;
}

function ComparisonRow({ label, current, input }: { label: string; current: unknown; input: React.ReactNode }) {
  return <div className="comparison-row"><strong>{label}</strong><div className="comparison-row__current">{current === null || current === undefined || current === "" ? <span className="muted">Empty</span> : String(current)}</div><div>{input}</div></div>;
}

export function ReviewTaxonomyEditor({ label, value, choices, allowNew, onChange }: {
  label: string;
  value?: string | null;
  choices: string[];
  allowNew: boolean;
  onChange: (value: string | null) => void;
}) {
  const listId = `review-taxonomy-${useId().replaceAll(":", "")}`;
  if (allowNew) {
    return <><Input aria-label={label} list={listId} value={value ?? ""} onChange={(event) => onChange(event.target.value || null)} placeholder="Choose or enter a new name" /><datalist id={listId}>{choices.map((choice) => <option key={choice} value={choice} />)}</datalist></>;
  }
  const isCatalogChoice = value && choices.some((choice) => choice.trim().toLocaleLowerCase() === value.trim().toLocaleLowerCase());
  return <Select aria-label={label} value={value ?? ""} onChange={(event) => onChange(event.target.value || null)}><option value="">None</option>{choices.map((choice) => <option key={choice} value={choice}>{choice}</option>)}{value && !isCatalogChoice && <option value={value}>{value} (unavailable)</option>}</Select>;
}

function TagEditor({ tags, choices, onChange }: { tags: string[]; choices: string[]; onChange: (tags: string[]) => void }) {
  const [draft, setDraft] = useState("");
  function add(value: string) { const next = value.trim(); if (next && !tags.some((tag) => tag.toLowerCase() === next.toLowerCase())) onChange([...tags, next]); setDraft(""); }
  return <div className="tag-editor"><div className="tag-editor__tags">{tags.map((tag) => <button key={tag} onClick={() => onChange(tags.filter((value) => value !== tag))} title={`Remove ${tag}`}>{tag} ×</button>)}</div><Input list="review-tag-options" value={draft} onChange={(event) => setDraft(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter" || event.key === ",") { event.preventDefault(); add(draft); } }} onBlur={() => add(draft)} placeholder="Add tag" /><datalist id="review-tag-options">{choices.map((tag) => <option key={tag} value={tag} />)}</datalist></div>;
}

function CustomValueEditor({ definition, value, onChange }: { definition?: CatalogItem; value: FieldValue; onChange: (value: FieldValue) => void }) {
  const type = definition?.data_type;
  if (type === "select") {
    const options = selectOptions(definition?.extra_data);
    return <Select value={JSON.stringify(value)} onChange={(event) => onChange(JSON.parse(event.target.value) as FieldValue)}>{options.map((option) => <option key={JSON.stringify(option.id)} value={JSON.stringify(option.id)}>{option.label}</option>)}</Select>;
  }
  if (type === "boolean" || typeof value === "boolean") return <Select value={String(value)} onChange={(event) => onChange(event.target.value === "true")}><option value="true">Yes</option><option value="false">No</option></Select>;
  if (type === "long_text") return <Textarea rows={3} value={value == null ? "" : String(value)} onChange={(event) => onChange(event.target.value)} />;
  if (type === "date") return <Input type="date" value={value == null ? "" : String(value)} onChange={(event) => onChange(event.target.value)} />;
  if (type === "url") return <Input type="url" value={value == null ? "" : String(value)} onChange={(event) => onChange(event.target.value)} />;
  if (type === "integer" || type === "float" || typeof value === "number") return <Input type="number" step={type === "integer" ? 1 : "any"} value={value ?? ""} onChange={(event) => onChange(event.target.value === "" ? null : Number(event.target.value))} />;
  return <Input value={value ?? ""} onChange={(event) => onChange(event.target.value)} placeholder={type === "monetary" ? "EUR123.45" : undefined} />;
}

function selectOptions(extraData: unknown): Array<{ id: string | number; label: string }> {
  const found = new Map<string, { id: string | number; label: string }>();
  function visit(value: unknown) {
    if (Array.isArray(value)) { value.forEach(visit); return; }
    if (!value || typeof value !== "object") return;
    const item = value as Record<string, unknown>;
    if ((typeof item.id === "string" || typeof item.id === "number") && (typeof item.label === "string" || typeof item.name === "string")) {
      const option = { id: item.id, label: String(item.label ?? item.name) };
      found.set(JSON.stringify(option.id), option);
    }
    Object.values(item).forEach(visit);
  }
  visit(extraData);
  return [...found.values()];
}

function displayCustomValue(definition: CatalogItem | undefined, value: FieldValue | undefined): string {
  if (value === undefined || value === null || value === "") return "—";
  if (definition?.data_type === "select") return selectOptions(definition.extra_data).find((option) => option.id === value)?.label ?? String(value);
  if (typeof value === "boolean") return value ? "Yes" : "No";
  return String(value);
}
