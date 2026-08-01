import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { WorkerStatus } from "../api/types";
import { WorkerLiveStatus } from "./WorkerLiveStatus";

describe("WorkerLiveStatus", () => {
  it("shows live progress while a large Paperless scan is paging", () => {
    const worker: WorkerStatus = {
      state: "scanning",
      paused: false,
      scan_progress: {
        trigger: "manual",
        discovered: 1_250,
        queued: 1_100,
        started_at: new Date().toISOString(),
      },
    };

    render(<WorkerLiveStatus worker={worker} lastSuccessAt={Date.now()} showLink={false} />);

    expect(screen.getByText("Scanning Paperless")).toBeVisible();
    expect(screen.getByText("1250 discovered · 1100 queued")).toBeVisible();
    expect(screen.getByText(/Manual scan/)).toBeVisible();
  });

  it("summarizes multiple documents being analyzed concurrently", () => {
    const updatedAt = new Date().toISOString();
    const worker: WorkerStatus = {
      state: "running",
      paused: false,
      active_job_id: "job-1",
      active_job: { id: "job-1", document_id: 41, document_title: "Invoice", state: "analyzing", updated_at: updatedAt },
      active_job_ids: ["job-1", "job-2"],
      active_jobs: [
        { id: "job-1", document_id: 41, document_title: "Invoice", state: "analyzing", updated_at: updatedAt },
        { id: "job-2", document_id: 42, document_title: "Statement", state: "analyzing", updated_at: updatedAt },
      ],
    };

    render(<WorkerLiveStatus worker={worker} lastSuccessAt={Date.now()} showLink={false} />);

    expect(screen.getByText("2 documents in parallel")).toBeVisible();
    expect(screen.getByText(/2 AI analyses in progress/)).toBeVisible();
    expect(screen.getByText(/Invoice \(#41\).*Statement \(#42\)/)).toBeVisible();
  });
});
