import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { api } from "../api/client";
import type { DiagnosticLogEntry, DiagnosticLogs } from "../api/types";
import { filterDiagnosticEntries, LogsPage } from "./LogsPage";

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

const worker: DiagnosticLogs["worker"] = {
  state: "running",
  active: false,
  last_scan_at: "2026-08-01T10:01:00Z",
  next_scan_at: "2026-08-01T10:06:00Z",
};

const entries: DiagnosticLogEntry[] = [
  {
    timestamp: "2026-08-01T10:00:00Z",
    level: "INFO",
    event: "service.ready",
    target: "service",
    fields: { listener: "loopback", port: "8765" },
  },
  {
    timestamp: "2026-08-01T10:01:00Z",
    level: "WARN",
    event: "paperless.scan_failed",
    target: "paperless",
    fields: {
      error_code: "paperless_pagination_origin",
      detail: "Paperless pagination was rejected because its next-page URL left the configured server origin.",
    },
  },
];

describe("LogsPage", () => {
  it("renders readable entries and refreshes on request", async () => {
    const request = vi.spyOn(api, "logs").mockResolvedValue({
      available: true,
      entries,
      truncated: false,
      updated_at: "2026-08-01T10:01:00Z",
      worker,
    });
    render(<LogsPage />);

    expect(await screen.findByText("Service · Ready")).toBeInTheDocument();
    expect(screen.getByText("Paperless · Scan · Failed")).toBeInTheDocument();
    expect(screen.getByText("WARN", { selector: "strong" })).toBeInTheDocument();
    expect(screen.getByText("Ready", { selector: ".diagnostic-worker strong" })).toBeInTheDocument();
    const listItems = screen.getAllByRole("listitem");
    expect(listItems[0]).toHaveTextContent("Paperless · Scan · Failed");
    expect(listItems[1]).toHaveTextContent("Service · Ready");
    expect(screen.getByText(/Newest first/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
    await waitFor(() => expect(request).toHaveBeenCalledTimes(2));
  });

  it("filters by level, target, and safe-field search", async () => {
    vi.spyOn(api, "logs").mockResolvedValue({
      available: true,
      entries,
      truncated: false,
      worker,
    });
    render(<LogsPage />);
    expect(await screen.findByText("Paperless · Scan · Failed")).toBeInTheDocument();

    fireEvent.change(screen.getByRole("combobox", { name: "Filter logs by level" }), { target: { value: "INFO" } });
    expect(screen.queryByText("Paperless · Scan · Failed")).not.toBeInTheDocument();
    expect(screen.getByText("Service · Ready")).toBeInTheDocument();

    fireEvent.change(screen.getByRole("combobox", { name: "Filter logs by level" }), { target: { value: "ALL" } });
    fireEvent.change(screen.getByRole("combobox", { name: "Filter logs by target" }), { target: { value: "paperless" } });
    fireEvent.change(screen.getByRole("textbox", { name: "Search logs" }), { target: { value: "pagination origin" } });
    expect(screen.getByText("Paperless · Scan · Failed")).toBeInTheDocument();
    expect(screen.queryByText("Service · Ready")).not.toBeInTheDocument();
  });

  it("polls while auto-refresh is enabled and stops when disabled", async () => {
    vi.useFakeTimers();
    const request = vi.spyOn(api, "logs").mockResolvedValue({
      available: true,
      entries: [],
      truncated: false,
      worker,
    });
    render(<LogsPage />);
    await act(async () => { await Promise.resolve(); });
    expect(request).toHaveBeenCalledTimes(1);

    await act(async () => {
      vi.advanceTimersByTime(5_000);
      await Promise.resolve();
    });
    expect(request).toHaveBeenCalledTimes(2);

    fireEvent.click(screen.getByRole("checkbox", { name: "Auto-refresh" }));
    await act(async () => {
      vi.advanceTimersByTime(10_000);
      await Promise.resolve();
    });
    expect(request).toHaveBeenCalledTimes(2);
  });

  it("shows a clear unavailable state", async () => {
    vi.spyOn(api, "logs").mockResolvedValue({
      available: false,
      entries: [],
      truncated: false,
      message: "No desktop log is available yet.",
      worker,
    });
    render(<LogsPage />);
    expect(await screen.findByText("No desktop log available")).toBeInTheDocument();
    expect(screen.getByText("No desktop log is available yet.")).toBeInTheDocument();
  });

  it("distinguishes a paused worker from an unavailable worker", async () => {
    vi.spyOn(api, "logs").mockResolvedValue({
      available: true,
      entries: [],
      truncated: false,
      worker: { ...worker, state: "paused", next_scan_at: undefined },
    });
    render(<LogsPage />);

    expect(await screen.findByText("Paused", { selector: ".diagnostic-worker strong" })).toBeInTheDocument();
    expect(screen.getByText("Not scheduled")).toBeInTheDocument();
  });

  it("shows an explicit Paperless scan state", async () => {
    vi.spyOn(api, "logs").mockResolvedValue({
      available: true,
      entries: [],
      truncated: false,
      worker: { ...worker, state: "scanning", scan_discovered: 12, scan_queued: 3 },
    });
    render(<LogsPage />);

    expect(await screen.findByText("Scanning Paperless · 12 discovered, 3 queued", { selector: ".diagnostic-worker strong" })).toBeInTheDocument();
  });

  it("keeps the last log data visible while background updates reconnect", async () => {
    vi.useFakeTimers();
    vi.spyOn(api, "logs")
      .mockResolvedValueOnce({
        available: true,
        entries,
        truncated: false,
        worker,
      })
      .mockRejectedValueOnce(new Error("local API timed out"));
    render(<LogsPage />);
    await act(async () => { await Promise.resolve(); });

    await act(async () => {
      vi.advanceTimersByTime(5_000);
      await Promise.resolve();
    });

    expect(screen.getByRole("status")).toHaveTextContent("Updates reconnecting");
    expect(screen.getByText("Service · Ready")).toBeInTheDocument();
  });
});

describe("filterDiagnosticEntries", () => {
  it("searches only the structured values supplied by the API", () => {
    expect(filterDiagnosticEntries(entries, "ALL", "", "loopback")).toEqual([entries[0]]);
    expect(filterDiagnosticEntries(entries, "WARN", "paperless", "pagination")).toEqual([entries[1]]);
    expect(filterDiagnosticEntries(entries, "ERROR", "", "")).toEqual([]);
  });
});
