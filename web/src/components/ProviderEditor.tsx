import { useEffect, useMemo, useState } from "react";
import { api } from "../api/client";
import { providerPresets } from "../api/defaults";
import type { ProviderKind, ProviderProfile, ProviderTestResult, ProviderTransport } from "../api/types";
import { useToast } from "./Toast";
import { Alert, Badge, Button, Field, Input, Select, Toggle } from "./UI";

export function newProvider(kind: ProviderKind): ProviderProfile {
  return { ...providerPresets[kind], id: "" };
}

export function ProviderEditor({ provider, onChange, onSaved, showActive = true }: {
  provider: ProviderProfile;
  onChange: (provider: ProviderProfile) => void;
  onSaved: (provider: ProviderProfile) => void;
  showActive?: boolean;
}) {
  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState(false);
  const [loadingModels, setLoadingModels] = useState(false);
  const [models, setModels] = useState<string[]>([]);
  const [result, setResult] = useState<ProviderTestResult>();
  const [error, setError] = useState<string>();
  const { showToast } = useToast();
  const update = <K extends keyof ProviderProfile>(key: K, value: ProviderProfile[K]) => {
    const invalidatesTest = ["base_url", "api_key", "model", "transport", "reasoning_effort", "temperature", "headers", "max_output_tokens"].includes(key);
    if (invalidatesTest) {
      setResult(undefined);
      onChange({ ...provider, [key]: value, capabilities: undefined, last_test_ok: undefined, last_tested_at: undefined });
    } else {
      onChange({ ...provider, [key]: value });
    }
  };

  useEffect(() => {
    setModels([]);
    setResult(undefined);
    setError(undefined);
  }, [provider.id]);

  const supportsReasoning = provider.kind === "openai" || provider.kind === "claude" || provider.capabilities?.reasoning;
  const supportsTemperature = provider.kind === "custom" || provider.kind === "lmstudio" || provider.capabilities?.temperature;

  async function save(): Promise<ProviderProfile | undefined> {
    setSaving(true);
    setError(undefined);
    try {
      const { id: _id, ...newProfile } = provider;
      const saved = provider.id
        ? await api.updateProvider(provider)
        : await api.createProvider(newProfile);
      onSaved(saved);
      showToast(`${saved.name} saved`);
      return saved;
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Could not save provider.");
    } finally {
      setSaving(false);
    }
  }

  async function ensureSaved(): Promise<ProviderProfile | undefined> {
    if (provider.id) {
      const saved = await api.updateProvider(provider);
      onSaved(saved);
      return saved;
    }
    return save();
  }

  async function loadModels() {
    setLoadingModels(true);
    setError(undefined);
    try {
      const saved = await ensureSaved();
      if (!saved) return;
      const response = await api.providerModels(saved.id);
      setModels(response.models);
      if (response.models.length === 1 && !saved.model) {
        const changed = { ...saved, model: response.models[0] };
        onChange(changed);
        onSaved(changed);
      }
      if (!response.models.length) setError("The provider returned no models. You can enter a model ID manually.");
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Could not load models.");
    } finally {
      setLoadingModels(false);
    }
  }

  async function test() {
    setTesting(true);
    setError(undefined);
    setResult(undefined);
    try {
      const saved = await ensureSaved();
      if (!saved) return;
      const response = await api.testProvider(saved.id);
      setResult(response);
      const updated = { ...saved, capabilities: response.capabilities, last_test_ok: response.ok, last_tested_at: new Date().toISOString() };
      onChange(updated);
      onSaved(updated);
    } catch (caught) {
      onChange({ ...provider, capabilities: undefined, last_test_ok: false, last_tested_at: new Date().toISOString() });
      setError(caught instanceof Error ? caught.message : "Provider test failed.");
    } finally {
      setTesting(false);
    }
  }

  const headerRows = useMemo(() => Object.entries(provider.headers ?? {}), [provider.headers]);
  function changeHeader(index: number, key: string, value: string) {
    const rows = [...headerRows];
    rows[index] = [key, value];
    update("headers", Object.fromEntries(rows.filter(([name]) => name.trim())));
  }

  return (
    <div className="provider-editor">
      {error && <Alert tone="danger">{error}</Alert>}
      {result && <Alert tone={result.ok ? "success" : "danger"} title={result.ok ? "Connection works" : "Connection failed"}><p>{result.message}{result.latency_ms ? ` (${result.latency_ms} ms)` : ""}</p></Alert>}
      <div className="form-grid form-grid--two">
        <Field label="Profile name" required><Input value={provider.name} onChange={(event) => update("name", event.target.value)} /></Field>
        <Field label="Provider preset" required>
          <Select value={provider.kind} onChange={(event) => {
            const kind = event.target.value as ProviderKind;
            onChange({ ...newProvider(kind), id: provider.id, active: provider.active, name: providerPresets[kind].name });
          }}>
            <option value="openai">OpenAI</option><option value="claude">Claude</option><option value="lmstudio">LM Studio</option><option value="custom">Custom OpenAI-compatible</option>
          </Select>
        </Field>
      </div>
      <Field label="Base URL" required hint={provider.kind === "lmstudio" ? "LM Studio typically runs locally on port 1234." : "Requests are sent directly from this PaperlessLLM service."}>
        <Input type="url" value={provider.base_url} onChange={(event) => update("base_url", event.target.value)} placeholder="https://…" />
      </Field>
      <Field label="API key" hint={provider.api_key_configured ? "A key is saved. Leave blank to keep it." : provider.kind === "lmstudio" ? "Usually not required for LM Studio." : "Stored securely; never returned to this browser."}>
        <Input type="password" value={provider.api_key ?? ""} onChange={(event) => update("api_key", event.target.value)} autoComplete="new-password" placeholder={provider.api_key_configured ? "••••••••••••" : "Enter API key"} />
      </Field>
      <div className="form-grid form-grid--model">
        <Field label="Model ID" required>
          <Input list={`models-${provider.id || "new"}`} value={provider.model} onChange={(event) => update("model", event.target.value)} placeholder="Choose or enter a model ID" />
          <datalist id={`models-${provider.id || "new"}`}>{models.map((model) => <option key={model} value={model} />)}</datalist>
        </Field>
        <Button type="button" variant="secondary" loading={loadingModels} onClick={() => void loadModels()}>Discover models</Button>
      </div>
      {(provider.kind === "lmstudio" || provider.kind === "custom") && (
        <div className="form-grid form-grid--two"><Field label="API transport">
          <Select value={provider.transport} onChange={(event) => update("transport", event.target.value as ProviderTransport)}>
            <option value="responses">Responses API</option><option value="chat_completions">Chat Completions</option>
          </Select>
        </Field>{provider.transport === "chat_completions" && <Field label="Generated-token field" hint="Choose the field expected by this compatible server."><Select value={provider.chat_output_token_field ?? "max_tokens"} onChange={(event) => update("chat_output_token_field", event.target.value as ProviderProfile["chat_output_token_field"])}><option value="max_tokens">max_tokens</option><option value="max_completion_tokens">max_completion_tokens</option></Select></Field>}</div>
      )}
      {supportsReasoning && (
        <Field label={provider.kind === "claude" ? "Thinking effort" : "Reasoning effort"} hint="Sent only when the model and provider support it.">
          <Select value={provider.reasoning_effort ?? ""} onChange={(event) => update("reasoning_effort", (event.target.value || undefined) as ProviderProfile["reasoning_effort"])}>
            <option value="">Provider default (omit field)</option><option value="none">None</option>{provider.kind !== "claude" && <option value="minimal">Minimal</option>}<option value="low">Low</option><option value="medium">Medium</option><option value="high">High</option><option value="xhigh">Extra high</option><option value="max">Maximum</option>
          </Select>
        </Field>
      )}
      <div className="setting-list setting-list--flush">
        <Toggle checked={provider.temperature !== null && provider.temperature !== undefined} disabled={!supportsTemperature} onChange={(enabled) => update("temperature", enabled ? 0.3 : null)} label="Send temperature" description={supportsTemperature ? "Optional. Leave off for reasoning models and providers that reject it." : "Disabled until the provider reports temperature support."} />
        {provider.temperature !== null && provider.temperature !== undefined && supportsTemperature && <Field label="Temperature"><Input type="number" min="0" max="2" step="0.1" value={provider.temperature} onChange={(event) => update("temperature", event.target.value === "" ? null : Number(event.target.value))} /></Field>}
      </div>
      {provider.kind === "custom" && (
        <div>
          <div className="inline-heading"><strong>Custom request headers</strong><span>Optional</span></div>
          {provider.headers_configured && provider.headers === undefined && <Alert tone="info"><p>Custom headers are stored securely. Their values are hidden; choose replace to enter a complete new set or clear to remove them.</p><div className="form-actions"><Button type="button" variant="ghost" size="sm" onClick={() => update("headers", {})}>Replace stored headers</Button><Button type="button" variant="danger" size="sm" onClick={() => update("headers", {})}>Clear stored headers</Button></div></Alert>}
          {provider.headers !== undefined && <div className="header-editor">
            {headerRows.map(([key, value], index) => (
              <div className="header-row" key={`${key}-${index}`}>
                <Input aria-label={`Header ${index + 1} name`} value={key} onChange={(event) => changeHeader(index, event.target.value, value)} placeholder="Header name" />
                <Input aria-label={`Header ${index + 1} value`} value={value} onChange={(event) => changeHeader(index, key, event.target.value)} placeholder="Value" />
                <button type="button" aria-label="Remove header" onClick={() => update("headers", Object.fromEntries(headerRows.filter((_, row) => row !== index)))}>×</button>
              </div>
            ))}
            <Button type="button" variant="ghost" size="sm" onClick={() => update("headers", { ...provider.headers, [`X-Custom-${headerRows.length + 1}`]: "" })}>+ Add header</Button>
            {provider.headers_configured && <p className="muted">Saving replaces the complete stored header set. Saving an empty set clears it.</p>}
          </div>}
        </div>
      )}
      {provider.capabilities && (
        <div className="capability-row" aria-label="Provider capabilities">
          <Badge tone={provider.capabilities.structured_output ? "success" : "warning"}>{provider.capabilities.structured_output ? "Structured output" : "Validated JSON fallback"}</Badge>
          {provider.capabilities.reasoning && <Badge tone="info">Reasoning</Badge>}
          {provider.capabilities.file_pdf && <Badge tone="neutral">PDF input</Badge>}
          {provider.capabilities.file_images && <Badge tone="neutral">Image input</Badge>}
          <Badge tone="neutral">{provider.capabilities.token_counting} token count</Badge>
        </div>
      )}
      {showActive && <Toggle checked={provider.active} onChange={(next) => update("active", next)} label="Use this provider for processing" description="Only one provider profile can be active at a time." />}
      <p className="muted">Provider Test performs one or two small inference calls. Each call is budgeted separately, and its token usage is recorded in Statistics.</p>
      <div className="form-actions">
        <Button type="button" variant="secondary" loading={testing} onClick={() => void test()} disabled={!provider.base_url || !provider.model}>Test provider</Button>
        <Button type="button" loading={saving} onClick={() => void save()} disabled={!provider.name || !provider.base_url}>Save provider</Button>
      </div>
    </div>
  );
}
