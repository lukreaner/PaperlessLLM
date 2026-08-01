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
});
