import { useState, type FormEvent } from "react";
import { api } from "../api/client";
import { Logo } from "../components/Logo";
import { Alert, Button, Field, Input } from "../components/UI";

export function LoginPage({ onAuthenticated }: { onAuthenticated: () => void }) {
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();

  async function submit(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    setError(undefined);
    try {
      await api.login(password);
      onAuthenticated();
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Sign in failed.");
    } finally {
      setBusy(false);
    }
  }

  return (
    <main className="auth-page">
      <div className="auth-page__ambient auth-page__ambient--one" />
      <div className="auth-page__ambient auth-page__ambient--two" />
      <form className="auth-card" onSubmit={submit}>
        <Logo />
        <div className="auth-card__heading"><span className="eyebrow">Private workspace</span><h1>Welcome back</h1><p>Sign in to manage document processing.</p></div>
        {error && <Alert tone="danger">{error}</Alert>}
        <Field label="Administrator password">
          <Input type="password" value={password} autoFocus autoComplete="current-password" onChange={(event) => setPassword(event.target.value)} />
        </Field>
        <Button type="submit" loading={busy} disabled={!password}>Sign in</Button>
        <p className="auth-card__foot">Credentials stay on this PaperlessLLM instance.</p>
      </form>
    </main>
  );
}
