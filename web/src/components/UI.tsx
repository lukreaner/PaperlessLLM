import type { ButtonHTMLAttributes, InputHTMLAttributes, ReactNode, SelectHTMLAttributes, TextareaHTMLAttributes } from "react";

export function Button({
  variant = "primary",
  size = "md",
  loading,
  children,
  className = "",
  disabled,
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: "primary" | "secondary" | "ghost" | "danger";
  size?: "sm" | "md";
  loading?: boolean;
}) {
  return (
    <button className={`button button--${variant} button--${size} ${className}`} disabled={disabled || loading} {...props}>
      {loading && <span className="button__spinner" aria-hidden="true" />}
      {children}
    </button>
  );
}

export function Card({ children, className = "", padded = true }: { children: ReactNode; className?: string; padded?: boolean }) {
  return <section className={`card ${padded ? "card--padded" : ""} ${className}`}>{children}</section>;
}

export function PageHeader({
  eyebrow,
  title,
  description,
  actions,
}: {
  eyebrow?: string;
  title: string;
  description?: string;
  actions?: ReactNode;
}) {
  return (
    <header className="page-header">
      <div>
        {eyebrow && <div className="eyebrow">{eyebrow}</div>}
        <h1>{title}</h1>
        {description && <p>{description}</p>}
      </div>
      {actions && <div className="page-header__actions">{actions}</div>}
    </header>
  );
}

export function Field({ label, hint, error, required, children, className = "" }: {
  label: string;
  hint?: string;
  error?: string;
  required?: boolean;
  children: ReactNode;
  className?: string;
}) {
  return (
    <label className={`field ${className}`}>
      <span className="field__label">{label}{required && <span className="required"> *</span>}</span>
      {children}
      {error ? <span className="field__error">{error}</span> : hint ? <span className="field__hint">{hint}</span> : null}
    </label>
  );
}

export function Input(props: InputHTMLAttributes<HTMLInputElement>) {
  return <input className={`input ${props.className ?? ""}`} {...props} />;
}

export function Select(props: SelectHTMLAttributes<HTMLSelectElement>) {
  return <select className={`input select ${props.className ?? ""}`} {...props} />;
}

export function Textarea(props: TextareaHTMLAttributes<HTMLTextAreaElement>) {
  return <textarea className={`input textarea ${props.className ?? ""}`} {...props} />;
}

export function Toggle({ checked, onChange, label, description, disabled = false }: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: string;
  description?: string;
  disabled?: boolean;
}) {
  return (
    <label className={`toggle-row ${disabled ? "is-disabled" : ""}`}>
      <span className="toggle-row__copy">
        <span className="toggle-row__label">{label}</span>
        {description && <span className="toggle-row__description">{description}</span>}
      </span>
      <input className="visually-hidden" type="checkbox" checked={checked} disabled={disabled} onChange={(event) => onChange(event.target.checked)} />
      <span className="toggle" aria-hidden="true"><span /></span>
    </label>
  );
}

export function Badge({ children, tone = "neutral" }: {
  children: ReactNode;
  tone?: "neutral" | "success" | "warning" | "danger" | "info" | "accent";
}) {
  return <span className={`badge badge--${tone}`}>{children}</span>;
}

export function StatusDot({ state }: { state: "success" | "warning" | "danger" | "neutral" }) {
  return <span className={`status-dot status-dot--${state}`} aria-hidden="true" />;
}

export function Alert({ children, tone = "info", title }: { children: ReactNode; tone?: "info" | "success" | "warning" | "danger"; title?: string }) {
  return (
    <div className={`alert alert--${tone}`} role={tone === "danger" ? "alert" : "status"}>
      <span className="alert__mark" aria-hidden="true">{tone === "success" ? "✓" : tone === "warning" ? "!" : tone === "danger" ? "×" : "i"}</span>
      <div>{title && <strong>{title}</strong>}{children}</div>
    </div>
  );
}

export function LoadingState({ label = "Loading" }: { label?: string }) {
  return (
    <div className="loading-state" role="status">
      <span className="loader" aria-hidden="true" />
      <span>{label}…</span>
    </div>
  );
}

export function ErrorState({ message, onRetry }: { message: string; onRetry?: () => void }) {
  return (
    <Card className="empty-state">
      <span className="empty-state__mark empty-state__mark--error" aria-hidden="true">!</span>
      <h2>Couldn’t load this view</h2>
      <p>{message}</p>
      {onRetry && <Button variant="secondary" onClick={onRetry}>Try again</Button>}
    </Card>
  );
}

export function EmptyState({ title, description, action, mark = "•" }: { title: string; description: string; action?: ReactNode; mark?: string }) {
  return (
    <div className="empty-state">
      <span className="empty-state__mark" aria-hidden="true">{mark}</span>
      <h2>{title}</h2>
      <p>{description}</p>
      {action}
    </div>
  );
}

export function ProgressBar({ value, tone = "accent", label }: { value: number; tone?: "accent" | "warning" | "danger"; label?: string }) {
  const bounded = Math.max(0, Math.min(100, value));
  return (
    <div className="progress" aria-label={label} role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(bounded)}>
      <span className={`progress__fill progress__fill--${tone}`} style={{ width: `${bounded}%` }} />
    </div>
  );
}

export function Skeleton({ width = "100%", height = 16 }: { width?: string; height?: number }) {
  return <span className="skeleton" style={{ width, height }} aria-hidden="true" />;
}

export function ConfirmDialog({ open, title, description, confirmLabel, danger, busy, onCancel, onConfirm }: {
  open: boolean;
  title: string;
  description: string;
  confirmLabel: string;
  danger?: boolean;
  busy?: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  if (!open) return null;
  return (
    <div className="modal-backdrop" role="presentation" onMouseDown={(event) => event.target === event.currentTarget && onCancel()}>
      <div className="modal" role="dialog" aria-modal="true" aria-labelledby="confirm-title">
        <h2 id="confirm-title">{title}</h2>
        <p>{description}</p>
        <div className="modal__actions">
          <Button variant="ghost" onClick={onCancel} disabled={busy}>Cancel</Button>
          <Button variant={danger ? "danger" : "primary"} loading={busy} onClick={onConfirm}>{confirmLabel}</Button>
        </div>
      </div>
    </div>
  );
}
