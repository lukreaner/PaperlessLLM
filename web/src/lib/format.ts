export function formatTokens(value: number | null | undefined): string {
  if (value === null || value === undefined) return "—";
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(value >= 10_000_000 ? 0 : 1)}M`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(value >= 100_000 ? 0 : 1)}k`;
  return new Intl.NumberFormat("en").format(value);
}

export function formatDateTime(value?: string): string {
  if (!value) return "Never";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(date);
}

export function formatRelativeTime(value?: string, now = Date.now()): string {
  if (!value) return "Never";
  const timestamp = new Date(value).getTime();
  if (Number.isNaN(timestamp)) return value;
  const seconds = Math.round((timestamp - now) / 1000);
  const formatter = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });
  if (Math.abs(seconds) < 60) return formatter.format(seconds, "second");
  const minutes = Math.round(seconds / 60);
  if (Math.abs(minutes) < 60) return formatter.format(minutes, "minute");
  const hours = Math.round(minutes / 60);
  if (Math.abs(hours) < 24) return formatter.format(hours, "hour");
  return formatter.format(Math.round(hours / 24), "day");
}

export function budgetPercent(used: number, limit: number | null): number {
  if (!limit || limit <= 0) return 0;
  return Math.min(100, Math.max(0, (used / limit) * 100));
}

export function titleCase(value: string): string {
  return value.replaceAll("_", " ").replace(/\b\w/g, (letter) => letter.toUpperCase());
}

export function initials(value: string): string {
  return value.split(/\s+/).filter(Boolean).slice(0, 2).map((word) => word[0]).join("").toUpperCase();
}

export function validationMessage(settings: { base_url: string; model?: string }): string | null {
  if (!settings.base_url.trim()) return "Enter a server URL.";
  try {
    const url = new URL(settings.base_url);
    if (!url.protocol.startsWith("http")) return "The URL must use HTTP or HTTPS.";
  } catch {
    return "Enter a valid server URL.";
  }
  if (settings.model !== undefined && !settings.model.trim()) return "Choose or enter a model.";
  return null;
}
