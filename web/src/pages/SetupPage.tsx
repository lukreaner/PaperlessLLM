import { useMemo, useState } from "react";
import { api } from "../api/client";
import { defaultSettings } from "../api/defaults";
import type { AppSettings, Catalogs, PaperlessTestResult, ProviderKind, ProviderProfile } from "../api/types";
import { Logo } from "../components/Logo";
import { newProvider, ProviderEditor } from "../components/ProviderEditor";
import { ProcessingControls } from "../components/SettingsControls";
import { useToast } from "../components/Toast";
import { Alert, Button, Card, Field, Input, ProgressBar, Select, Toggle } from "../components/UI";
import { validationMessage } from "../lib/format";
import { useApp } from "../App";

const steps = ["Welcome", "Paperless", "AI provider", "Processing", "Limits"];

export function SetupPage({ onComplete }: { onComplete: () => void }) {
  const app = useApp();
  const [step, setStep] = useState(0);
  const [settings, setSettings] = useState<AppSettings>(() => ({ ...defaultSettings, ...app.settings }));
  const [provider, setProvider] = useState<ProviderProfile>(() => app.providers[0] ?? newProvider("openai"));
  const [catalogs, setCatalogs] = useState<Catalogs>(app.catalogs ?? { tags: [], correspondents: [], document_types: [], custom_fields: [] });
  const [paperlessResult, setPaperlessResult] = useState<PaperlessTestResult>();
  const [paperlessTesting, setPaperlessTesting] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [adminPassword, setAdminPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const { showToast } = useToast();

  const canContinue = useMemo(() => {
    if (step === 1) return paperlessResult?.ok === true;
    if (step === 2) return Boolean(provider.id && provider.model && provider.last_test_ok);
    if (step === 3) return Object.values(settings.processing.functions).some(Boolean) && (!settings.processing.process_only_tagged || settings.processing.trigger_tag_ids.length > 0);
    if (step === 4 && settings.security.lan_enabled) return adminPassword.length >= 12 && adminPassword === confirmPassword && settings.security.reverse_proxy_tls;
    return true;
  }, [adminPassword, confirmPassword, paperlessResult, provider, settings, step]);

  async function testPaperless() {
    const invalid = validationMessage(settings.paperless);
    if (invalid) { setError(invalid); return; }
    setPaperlessTesting(true);
    setError(undefined);
    setPaperlessResult(undefined);
    try {
      const result = await api.testPaperless({
        base_url: settings.paperless.base_url,
        api_token: settings.paperless.api_token,
        custom_ca_pem: settings.paperless.custom_ca_pem,
      });
      setPaperlessResult(result);
      if (result.ok) {
        const saved = await api.saveSettings(settings);
        setSettings(saved);
        const loaded = await api.catalogs();
        setCatalogs(loaded);
        app.updateCatalogs(loaded);
      }
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Connection test failed.");
    } finally {
      setPaperlessTesting(false);
    }
  }

  async function complete() {
    setBusy(true);
    setError(undefined);
    try {
      const nextSettings = { ...settings, active_provider_id: provider.id };
      // Establish the credential before enabling LAN authentication so there
      // is never a persisted, remotely reachable configuration without a
      // password.
      if (nextSettings.security.lan_enabled && adminPassword) await api.setPassword(adminPassword);
      const saved = await api.saveSettings(nextSettings);
      app.updateSettings(saved);
      showToast("PaperlessLLM is ready");
      onComplete();
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Could not finish setup.");
    } finally {
      setBusy(false);
    }
  }

  function next() {
    if (step === 4) { void complete(); return; }
    setStep((current) => Math.min(4, current + 1));
  }

  return (
    <main className="setup-page">
      <header className="setup-topbar"><Logo /><span>First-run setup</span></header>
      <div className="setup-layout">
        <aside className="setup-steps" aria-label="Setup progress">
          <div><span className="eyebrow">Quick setup</span><h1>Your documents,<br />intelligently filed.</h1><p>Connect Paperless and an AI provider. Nothing leaves this Mac except requests to services you choose.</p></div>
          <ol>{steps.map((label, index) => <li key={label} className={index === step ? "is-active" : index < step ? "is-complete" : ""}><span>{index < step ? "✓" : index + 1}</span>{label}</li>)}</ol>
          <ProgressBar value={((step + 1) / steps.length) * 100} label={`Step ${step + 1} of ${steps.length}`} />
        </aside>
        <section className="setup-content">
          {error && <Alert tone="danger">{error}</Alert>}
          {step === 0 && <WelcomeStep onChoose={(kind) => { setProvider(newProvider(kind)); setStep(1); }} />}
          {step === 1 && (
            <div className="setup-panel">
              <div className="setup-heading"><span className="eyebrow">Step 2 of 5</span><h2>Connect Paperless-ngx</h2><p>PaperlessLLM requires Paperless-ngx v3 and API v10. Your token is stored securely and never shown again.</p></div>
              <Card>
                <div className="form-stack">
                  <Field label="Paperless URL" required hint="The base address you use to open Paperless, without /api."><Input type="url" autoFocus value={settings.paperless.base_url} onChange={(event) => { setSettings({ ...settings, paperless: { ...settings.paperless, base_url: event.target.value } }); setPaperlessResult(undefined); }} placeholder="https://paperless.example.com" /></Field>
                  <Field label="API token" required hint={settings.paperless.api_token_configured ? "A token is already saved; leave blank to keep it." : "Find this in Paperless under My Profile → API Token."}><Input type="password" value={settings.paperless.api_token ?? ""} onChange={(event) => { setSettings({ ...settings, paperless: { ...settings.paperless, api_token: event.target.value } }); setPaperlessResult(undefined); }} autoComplete="new-password" placeholder="Paste Paperless token" /></Field>
                  <details className="details-box"><summary>Private certificate authority</summary><Field label="Custom CA certificate (PEM)" hint="Use this for a private CA. TLS verification is never disabled."><textarea className="input textarea code-input" rows={5} value={settings.paperless.custom_ca_pem ?? ""} onChange={(event) => { setPaperlessResult(undefined); setSettings({ ...settings, paperless: { ...settings.paperless, custom_ca_pem: event.target.value } }); }} placeholder="-----BEGIN CERTIFICATE-----" /></Field></details>
                  {paperlessResult && <Alert tone={paperlessResult.ok ? "success" : "danger"} title={paperlessResult.ok ? "Connected" : "Connection failed"}><p>{paperlessResult.message}</p>{paperlessResult.ok && <p className="result-meta">Paperless {paperlessResult.version} · API v{paperlessResult.api_version}{paperlessResult.document_count !== undefined ? ` · ${paperlessResult.document_count.toLocaleString()} documents` : ""}</p>}</Alert>}
                  <Button variant="secondary" loading={paperlessTesting} onClick={() => void testPaperless()}>Test connection</Button>
                </div>
              </Card>
            </div>
          )}
          {step === 2 && (
            <div className="setup-panel setup-panel--wide">
              <div className="setup-heading"><span className="eyebrow">Step 3 of 5</span><h2>Choose your AI</h2><p>Use a hosted model or keep inference local with LM Studio. Unsupported options are omitted from requests automatically.</p></div>
              <Card><ProviderEditor provider={provider} onChange={setProvider} onSaved={(saved) => { setProvider(saved); app.updateProviders([saved, ...app.providers.filter((item) => item.id !== saved.id)]); }} showActive={false} /></Card>
            </div>
          )}
          {step === 3 && (
            <div className="setup-panel setup-panel--wide">
              <div className="setup-heading"><span className="eyebrow">Step 4 of 5</span><h2>Decide what AI can change</h2><p>The defaults add metadata conservatively and apply automatically. Turn on review mode to approve each document first.</p></div>
              <Card>
                <div className="setting-list setup-mode-choice"><Toggle checked={settings.processing.review_mode} onChange={(review_mode) => setSettings({ ...settings, processing: { ...settings.processing, review_mode } })} label="Review suggestions before applying" description="Paperless is not changed until you approve a suggestion." /></div>
                <ProcessingControls value={settings.processing} tags={catalogs.tags} customFields={catalogs.custom_fields} onChange={(processing) => setSettings({ ...settings, processing })} compact />
              </Card>
            </div>
          )}
          {step === 4 && (
            <div className="setup-panel setup-panel--wide">
              <div className="setup-heading"><span className="eyebrow">Step 5 of 5</span><h2>Set sensible limits</h2><p>Hard limits stop new AI calls before they exceed your budget. Leave daily or monthly budgets blank to disable them.</p></div>
              <Card>
                <div className="form-grid form-grid--two">
                  <Field label="Maximum input tokens / document"><Input type="number" min="1000" value={settings.limits.max_input_tokens} onChange={(event) => setSettings({ ...settings, limits: { ...settings.limits, max_input_tokens: Number(event.target.value) } })} /></Field>
                  <Field label="Maximum generated tokens"><Input type="number" min="256" value={settings.limits.max_output_tokens} onChange={(event) => setSettings({ ...settings, limits: { ...settings.limits, max_output_tokens: Number(event.target.value) } })} /></Field>
                  <Field label="Daily token budget" hint="Optional"><Input type="number" min="0" value={settings.limits.global_budget.daily ?? ""} onChange={(event) => setSettings({ ...settings, limits: { ...settings.limits, global_budget: { ...settings.limits.global_budget, daily: event.target.value ? Number(event.target.value) : null } } })} placeholder="Unlimited" /></Field>
                  <Field label="Monthly token budget" hint="Optional"><Input type="number" min="0" value={settings.limits.global_budget.monthly ?? ""} onChange={(event) => setSettings({ ...settings, limits: { ...settings.limits, global_budget: { ...settings.limits.global_budget, monthly: event.target.value ? Number(event.target.value) : null } } })} placeholder="Unlimited" /></Field>
                  <Field label="Reset timezone"><Input value={settings.limits.reset_timezone} onChange={(event) => setSettings({ ...settings, limits: { ...settings.limits, reset_timezone: event.target.value } })} /></Field>
                  <Field label="Polling interval (minutes)"><Input type="number" min="1" value={settings.processing.poll_interval_minutes} onChange={(event) => setSettings({ ...settings, processing: { ...settings.processing, poll_interval_minutes: Number(event.target.value) } })} /></Field>
                </div>
                <div className="setting-list setup-security">
                  <Toggle checked={settings.security.lan_enabled} onChange={(lan_enabled) => setSettings({ ...settings, security: { ...settings.security, lan_enabled } })} label="Require authentication for network access" description="Use an HTTPS reverse proxy; the listener address remains controlled by the --bind startup option." />
                  {settings.security.lan_enabled && <>
                    <Toggle checked={settings.security.reverse_proxy_tls} onChange={(reverse_proxy_tls) => setSettings({ ...settings, security: { ...settings.security, reverse_proxy_tls } })} label="HTTPS reverse proxy is configured" description="Confirm that TLS terminates before traffic reaches PaperlessLLM." />
                    <div className="form-grid form-grid--two"><Field label="Administrator password" hint="At least 12 characters"><Input type="password" value={adminPassword} onChange={(event) => setAdminPassword(event.target.value)} autoComplete="new-password" /></Field><Field label="Confirm password"><Input type="password" value={confirmPassword} onChange={(event) => setConfirmPassword(event.target.value)} autoComplete="new-password" /></Field></div>
                    {confirmPassword && adminPassword !== confirmPassword && <p className="inline-error">Passwords do not match.</p>}
                  </>}
                </div>
              </Card>
            </div>
          )}
          {step > 0 && (
            <footer className="setup-actions">
              <Button variant="ghost" onClick={() => { setError(undefined); setStep((current) => current - 1); }}>Back</Button>
              <span>Step {step + 1} of {steps.length}</span>
              <Button onClick={next} disabled={!canContinue} loading={busy}>{step === 4 ? "Finish setup" : "Continue"}</Button>
            </footer>
          )}
        </section>
      </div>
    </main>
  );
}

function WelcomeStep({ onChoose }: { onChoose: (kind: ProviderKind) => void }) {
  return (
    <div className="welcome-step">
      <span className="welcome-step__kicker">Private document intelligence</span>
      <h2>Give Paperless<br /><em>a careful second brain.</em></h2>
      <p>Classify, title, and enrich every document using the AI provider you trust—while staying in control of changes and token usage.</p>
      <div className="welcome-features">
        <div><span>01</span><strong>Conservative by design</strong><p>Existing metadata stays untouched unless you explicitly allow overwrites.</p></div>
        <div><span>02</span><strong>Any model you choose</strong><p>OpenAI, Claude, LM Studio, and OpenAI-compatible servers.</p></div>
        <div><span>03</span><strong>Know every token</strong><p>Track exact or estimated usage and enforce daily and monthly budgets.</p></div>
      </div>
      <div className="welcome-actions">
        <Button onClick={() => onChoose("openai")}>Start with OpenAI</Button>
        <Button variant="secondary" onClick={() => onChoose("claude")}>Use Claude</Button>
        <Button variant="secondary" onClick={() => onChoose("lmstudio")}>Keep it local</Button>
      </div>
    </div>
  );
}
