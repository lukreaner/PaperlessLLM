import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { Job } from "../api/types";
import { ActivityTableRow } from "./ActivityPage";

const baseJob: Job = {
  id: "job-1",
  document_id: 2388,
  document_title: "Example document",
  state: "failed",
  attempt: 1,
  provider_name: "OpenAI profile",
  provider_kind: "openai",
  api_family: "responses",
  model: "example-model",
  functions: ["tags", "title"],
  created_at: "2026-08-01T12:00:00Z",
  updated_at: "2026-08-01T12:00:01Z",
};

function renderRow(job: Job) {
  render(
    <table><tbody><ActivityTableRow
      job={job}
      expanded
      retrying={false}
      onExpand={vi.fn()}
      onRetry={vi.fn()}
      onRemove={vi.fn()}
    /></tbody></table>,
  );
}

describe("ActivityTableRow diagnostics", () => {
  it("makes a zero-request preflight failure explicit", () => {
    renderRow({
      ...baseJob,
      error_code: "validation_failed",
      failure_phase: "preflight",
      error: "AI request preflight failed. Check document file input, token limits, the selected model, and processing settings.",
    });

    expect(screen.getByRole("alert")).toHaveTextContent("request preflight failed");
    expect(screen.getByRole("alert")).toHaveTextContent("validation_failed");
    expect(screen.getByRole("alert")).toHaveTextContent("AI request preflight");
    expect(screen.getByText("No AI inference recorded · 0 inference attempts", { selector: "strong" })).toBeInTheDocument();
    expect(screen.getByText("OpenAI", { selector: "strong" })).toBeInTheDocument();
    expect(screen.getByText("Responses API", { selector: "strong" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Open safe logs" })).toHaveAttribute("href", "/logs");
  });

  it("shows the latest provider attempt without an upstream response body", () => {
    renderRow({
      ...baseJob,
      attempt: 2,
      error_code: "provider_remote",
      failure_phase: "provider",
      error: "The AI provider returned HTTP 429.",
      upstream_status: 429,
      provider_operation: "repair",
      provider_attempt: 2,
      provider_status: "failed",
      provider_latency_ms: 321,
    });

    expect(screen.getByText("Attempt 2 · Repair · Failed · 321 ms", { selector: "strong" })).toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent("provider_remote");
    expect(screen.getByRole("alert")).toHaveTextContent("Upstream HTTP 429");
    expect(screen.queryByText(/response body/i)).not.toBeInTheDocument();
  });

  it("keeps an earlier inference visible when a later repair preflight fails", () => {
    renderRow({
      ...baseJob,
      error_code: "validation_failed",
      failure_phase: "preflight",
      error: "AI request preflight failed. Check document file input, token limits, the selected model, and processing settings.",
      provider_operation: "analysis",
      provider_attempt: 1,
      provider_status: "failed",
      provider_latency_ms: 111,
    });

    expect(screen.getByText("Attempt 1 · Analysis · Failed · 111 ms", { selector: "strong" })).toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent("AI request preflight");
    expect(screen.queryByText(/0 inference attempts/i)).not.toBeInTheDocument();
  });
});
