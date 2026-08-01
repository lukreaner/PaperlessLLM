import { useEffect, useState, type ReactNode } from "react";
import type { AppStatus } from "../api/types";
import { formatRelativeTime } from "../lib/format";
import { Link, useLocation } from "../lib/router";
import { Logo } from "./Logo";
import { Badge, StatusDot } from "./UI";

const navItems = [
  { path: "/", label: "Overview", mark: "⌂" },
  { path: "/review", label: "Review", mark: "✓" },
  { path: "/activity", label: "Activity", mark: "↻" },
  { path: "/statistics", label: "Statistics", mark: "▥" },
  { path: "/logs", label: "Logs", mark: "≡" },
  { path: "/settings", label: "Settings", mark: "⚙" },
];

function connectionTone(state: string): "success" | "warning" | "danger" | "neutral" {
  if (state === "connected" || state === "running" || state === "scanning") return "success";
  if (state === "degraded" || state === "paused" || state === "pausing") return "warning";
  if (state === "disconnected" || state === "error") return "danger";
  return "neutral";
}

export function Layout({ children, status, onRefreshStatus, onLogout, onQuit }: {
  children: ReactNode;
  status?: AppStatus;
  onRefreshStatus: () => void;
  onLogout: () => void;
  onQuit: () => void;
}) {
  const [mobileOpen, setMobileOpen] = useState(false);
  const location = useLocation();
  useEffect(() => setMobileOpen(false), [location.pathname]);

  const reviewCount = status?.queue.awaiting_review ?? 0;
  return (
    <div className="app-shell">
      <aside className={`sidebar ${mobileOpen ? "is-open" : ""}`}>
        <div className="sidebar__top"><Logo /></div>
        <nav className="sidebar__nav" aria-label="Main navigation">
          <span className="sidebar__section-label">Workspace</span>
          {navItems.map((item) => (
            <Link key={item.path} to={item.path} className={`nav-item ${location.pathname === item.path ? "is-active" : ""}`}>
              <span className="nav-item__mark" aria-hidden="true">{item.mark}</span>
              <span>{item.label}</span>
              {item.path === "/review" && reviewCount > 0 && <span className="nav-item__count">{reviewCount}</span>}
            </Link>
          ))}
        </nav>
        <div className="sidebar__status">
          <button className="service-status" onClick={onRefreshStatus} title="Refresh status">
            <span className="service-status__pulse"><StatusDot state={connectionTone(status?.worker.state ?? "unconfigured")} /></span>
            <span>
              <strong>Worker {status?.worker.state ?? "unknown"}</strong>
              <small>{status?.worker.last_scan_at ? `Scanned ${formatRelativeTime(status.worker.last_scan_at)}` : "No scan yet"}</small>
            </span>
          </button>
          <div className="version-row">
            <span>Local & private</span>
            <Badge tone="neutral">v{status?.version ?? "—"}</Badge>
          </div>
          {status?.auth.required && <button className="sidebar-logout" onClick={onLogout}>Sign out</button>}
          <button className="sidebar-quit" onClick={onQuit}>
            <span aria-hidden="true">⏻</span>
            Quit PaperlessLLM
          </button>
        </div>
      </aside>
      {mobileOpen && <button className="sidebar-scrim" aria-label="Close navigation" onClick={() => setMobileOpen(false)} />}
      <main className="main-content">
        <header className="mobile-header">
          <button className="mobile-menu" aria-label="Open navigation" onClick={() => setMobileOpen(true)}>☰</button>
          <Logo compact />
          <StatusDot state={connectionTone(status?.worker.state ?? "unconfigured")} />
        </header>
        <div className="main-content__inner">{children}</div>
      </main>
    </div>
  );
}
