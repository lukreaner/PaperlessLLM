import type { CatalogItem, FunctionSettings, OverwriteSettings, ProcessingSettings } from "../api/types";
import { Field, Input, Select, Textarea, Toggle } from "./UI";

export const functionDescriptions: Array<{ key: keyof FunctionSettings; label: string; description: string }> = [
  { key: "tags", label: "Tags Assignment", description: "Automatically assign relevant tags to documents." },
  { key: "correspondent", label: "Correspondent Detection", description: "Identify document senders automatically." },
  { key: "document_type", label: "Document Type Classification", description: "Determine the type of document automatically." },
  { key: "title", label: "Title Generation", description: "Generate meaningful, searchable document titles." },
  { key: "custom_fields", label: "Custom Fields", description: "Extract values for selected Paperless custom fields." },
];

export function WorkflowModeControl({ reviewMode, savedReviewMode, activationContext = "settings", onChange }: {
  reviewMode: boolean;
  savedReviewMode?: boolean;
  activationContext?: "settings" | "setup";
  onChange: (reviewMode: boolean) => void;
}) {
  const selectedLabel = reviewMode ? "Review mode" : "Automatic mode";
  const savedLabel = savedReviewMode ? "Review mode" : "Automatic mode";
  const hasUnsavedModeChange = savedReviewMode !== undefined && savedReviewMode !== reviewMode;
  const status = activationContext === "setup"
    ? {
        title: `${selectedLabel} selected.`,
        detail: `It will become active when you finish setup.`,
      }
    : hasUnsavedModeChange
      ? {
          title: `${selectedLabel} selected — save changes to activate it.`,
          detail: `${savedLabel} remains active until you save.`,
        }
      : {
          title: `${selectedLabel} is active.`,
          detail: reviewMode
            ? "Every successful suggestion will wait for your approval."
            : "Successful suggestions will be applied without approval.",
        };

  return (
    <div className="workflow-mode-control">
      <div className="workflow-mode-options" role="radiogroup" aria-label="Apply behavior">
        <button type="button" role="radio" aria-checked={!reviewMode} className={!reviewMode ? "is-selected" : ""} onClick={() => onChange(false)}>
          <span className="workflow-mode-option__mark" aria-hidden="true">A</span>
          <span><strong>Automatic</strong><small>Apply every successful AI result to Paperless immediately.</small></span>
        </button>
        <button type="button" role="radio" aria-checked={reviewMode} className={reviewMode ? "is-selected" : ""} onClick={() => onChange(true)}>
          <span className="workflow-mode-option__mark" aria-hidden="true">R</span>
          <span><strong>Review before applying</strong><small>Hold suggestions in Review; Paperless stays unchanged until you approve.</small></span>
        </button>
      </div>
      <p className={`workflow-mode-status ${reviewMode ? "is-review" : "is-automatic"}`} role="status">
        <strong>{status.title}</strong>{" "}{status.detail}
      </p>
    </div>
  );
}

export function clampInferenceConcurrency(raw: string | number): number {
  const parsed = typeof raw === "number" ? raw : Number(raw);
  if (!Number.isFinite(parsed)) return 1;
  return Math.min(8, Math.max(1, Math.trunc(parsed)));
}

export function InferenceConcurrencyControl({ value, onChange }: {
  value: number;
  onChange: (value: number) => void;
}) {
  return (
    <Field label="Parallel AI analyses" hint="1 is safest for provider rate limits. Higher values process documents faster but consume quota in parallel.">
      <Input type="number" min="1" max="8" step="1" value={clampInferenceConcurrency(value)} onChange={(event) => onChange(clampInferenceConcurrency(event.target.value))} />
    </Field>
  );
}

export function ProcessingControls({ value, tags, customFields, onChange, compact = false }: {
  value: ProcessingSettings;
  tags: CatalogItem[];
  customFields: CatalogItem[];
  onChange: (value: ProcessingSettings) => void;
  compact?: boolean;
}) {
  const update = <K extends keyof ProcessingSettings>(key: K, next: ProcessingSettings[K]) => onChange({ ...value, [key]: next });
  const updateFunction = (key: keyof FunctionSettings, checked: boolean) => update("functions", { ...value.functions, [key]: checked });
  const updateOverwrite = (key: keyof OverwriteSettings, checked: boolean) => update("overwrite", { ...value.overwrite, [key]: checked });

  return (
    <div className="settings-stack">
      <div className="setting-list">
        <Toggle checked={value.use_existing_correspondents_and_tags} onChange={(next) => update("use_existing_correspondents_and_tags", next)} label="Use existing Correspondents and Tags?" description="Give the AI your Paperless catalogs as preferred context." />
        <Toggle checked={value.process_only_tagged} onChange={(next) => update("process_only_tagged", next)} label="Process only specific pre tagged documents?" description="Only scan documents carrying the selected trigger tags." />
      </div>

      {value.process_only_tagged && (
        <div className="sub-settings">
          <Field label="Trigger tags" hint={tags.length ? "Choose one or more existing Paperless tags." : "Connect Paperless to load tags."}>
            <div className="chip-picker">
              {tags.map((tag) => {
                const selected = value.trigger_tag_ids.includes(tag.id);
                return <button type="button" key={tag.id} className={`choice-chip ${selected ? "is-selected" : ""}`} onClick={() => update("trigger_tag_ids", selected ? value.trigger_tag_ids.filter((id) => id !== tag.id) : [...value.trigger_tag_ids, tag.id])}>{selected ? "✓ " : ""}{tag.name}</button>;
              })}
              {!tags.length && <span className="muted">No tags loaded</span>}
            </div>
          </Field>
          <Field label="Match rule">
            <Select value={value.trigger_match} onChange={(event) => update("trigger_match", event.target.value as "any" | "all")}>
              <option value="any">Any selected tag</option>
              <option value="all">All selected tags</option>
            </Select>
          </Field>
        </div>
      )}

      <div className="setting-list">
        <Toggle checked={value.restrict_existing_tags} onChange={(next) => update("restrict_existing_tags", next)} label="Restrict to Existing Tags" description="Ignore suggested tags that do not already exist in Paperless." />
        <Toggle checked={value.restrict_existing_correspondents} onChange={(next) => update("restrict_existing_correspondents", next)} label="Restrict to Existing Correspondents" description="Never create a new correspondent from an AI suggestion." />
        <Toggle checked={value.restrict_existing_document_types} onChange={(next) => update("restrict_existing_document_types", next)} label="Restrict to Existing Document Types" description="Never create a new document type from an AI suggestion." />
      </div>

      <div className="setting-list">
        <Toggle checked={value.add_processed_tag} onChange={(next) => update("add_processed_tag", next)} label="Add AI-processed tag to documents?" description="Mark documents after a successful final Paperless update." />
        {value.add_processed_tag && <Field label="AI-processed Tag Name"><Input value={value.processed_tag_name} onChange={(event) => update("processed_tag_name", event.target.value)} placeholder="ai-processed" /></Field>}
      </div>

      <div>
        <div className="settings-subheading"><h3>Enable/Disable AI Functions</h3><p>Select which functions the AI performs during document analysis.</p></div>
        <div className={`function-grid ${compact ? "function-grid--compact" : ""}`}>
          {functionDescriptions.map((item) => (
            <label key={item.key} className={`function-card ${value.functions[item.key] ? "is-selected" : ""}`}>
              <input type="checkbox" checked={value.functions[item.key]} onChange={(event) => updateFunction(item.key, event.target.checked)} />
              <span className="function-card__check" aria-hidden="true">✓</span>
              <span><strong>{item.label}</strong><small>{item.description}</small></span>
            </label>
          ))}
        </div>
      </div>

      {value.functions.custom_fields && (
        <Field label="Custom Fields" hint="Document-link fields are excluded.">
          <div className="chip-picker">
            {customFields.map((field) => {
              const selected = value.custom_field_ids.includes(field.id);
              return <button type="button" key={field.id} className={`choice-chip ${selected ? "is-selected" : ""}`} onClick={() => update("custom_field_ids", selected ? value.custom_field_ids.filter((id) => id !== field.id) : [...value.custom_field_ids, field.id])}>{selected ? "✓ " : ""}{field.name}<small>{field.data_type?.replaceAll("_", " ")}</small></button>;
            })}
            {!customFields.length && <span className="muted">No supported custom fields found</span>}
          </div>
        </Field>
      )}

      {!compact && (
        <div>
          <div className="settings-subheading"><h3>Existing metadata</h3><p>By default, PaperlessLLM fills empty values and preserves everything else.</p></div>
          <div className="overwrite-grid">
            {functionDescriptions.map((item) => (
              <Toggle key={item.key} checked={value.overwrite[item.key]} onChange={(next) => updateOverwrite(item.key, next)} label={`Overwrite ${item.label}`} description={item.key === "tags" ? "Replace tags instead of only adding new ones." : "Allow AI to replace an existing value."} />
            ))}
          </div>
        </div>
      )}

      <div>
        <div className="settings-subheading"><h3>Custom Prompt</h3><p>Tailor how documents are classified without weakening output or safety rules.</p></div>
        <div className="segmented-control" role="radiogroup" aria-label="Prompt mode">
          <button type="button" role="radio" aria-checked={value.custom_prompt_mode === "safe"} className={value.custom_prompt_mode === "safe" ? "is-active" : ""} onClick={() => update("custom_prompt_mode", "safe")}><strong>Safe mode</strong><small>Append instructions</small></button>
          <button type="button" role="radio" aria-checked={value.custom_prompt_mode === "expert"} className={value.custom_prompt_mode === "expert" ? "is-active" : ""} onClick={() => update("custom_prompt_mode", "expert")}><strong>Expert mode</strong><small>Replace task prose</small></button>
        </div>
        <Field label={value.custom_prompt_mode === "safe" ? "Additional instructions" : "Task instructions"} hint={value.custom_prompt_mode === "safe" ? "Optional. These instructions are appended to the built-in prompt." : "Safety, function boundaries, and the output schema remain enforced."}>
          <Textarea rows={compact ? 4 : 7} value={value.custom_prompt} onChange={(event) => update("custom_prompt", event.target.value)} placeholder={value.custom_prompt_mode === "safe" ? "Example: Prefer the tag “Taxes” for annual tax statements…" : "Describe how the AI should analyze each document…"} />
        </Field>
      </div>
    </div>
  );
}
