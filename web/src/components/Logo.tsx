export function Logo({ compact = false }: { compact?: boolean }) {
  return (
    <div className={`logo ${compact ? "logo--compact" : ""}`} aria-label="PaperlessLLM">
      <span className="logo__mark" aria-hidden="true"><span>PL</span></span>
      {!compact && <span className="logo__name">Paperless<span>LLM</span></span>}
    </div>
  );
}
