import { expect, test, type Route } from "@playwright/test";
import { defaultSettings } from "../src/api/defaults";
import type { AppStatus, BootstrapResponse, JobState } from "../src/api/types";

const now = "2026-08-01T10:00:00Z";

async function json(route: Route, body: unknown, status = 200) {
  await route.fulfill({ status, contentType: "application/json", body: JSON.stringify(body) });
}

test("live activity updates and safely controls the waiting queue", async ({ page }) => {
  let paused = false;
  let activeTitle = "Incoming invoice";
  let queuedState: JobState = "queued";
  let workerHeaderChecks = 0;

  const worker = () => ({
    state: paused ? "pausing" as const : "running" as const,
    paused,
    last_scan_at: now,
    next_scan_at: paused ? undefined : now,
    active_job_id: "active-job",
    active_job_ids: ["active-job", "active-job-2"],
    active_job: {
      id: "active-job",
      document_id: 41,
      document_title: activeTitle,
      state: "analyzing" as const,
      updated_at: now,
    },
    active_jobs: [
      {
        id: "active-job",
        document_id: 41,
        document_title: activeTitle,
        state: "analyzing" as const,
        updated_at: now,
      },
      {
        id: "active-job-2",
        document_id: 42,
        document_title: "Bank statement",
        state: "analyzing" as const,
        updated_at: now,
      },
    ],
  });
  const status = (): AppStatus => ({
    version: "0.1.1",
    setup_required: false,
    worker: worker(),
    paperless: { state: "connected", message: "Paperless connected" },
    provider: { state: "connected", message: "Provider connected" },
    queue: {
      queued: queuedState === "queued" ? 1 : 0,
      analyzing: 2,
      awaiting_review: 0,
      applying: 0,
      completed: 0,
      budget_blocked: 0,
      failed: 0,
      rejected: queuedState === "rejected" ? 1 : 0,
    },
    auth: { required: false, authenticated: true, password_configured: false },
  });
  const bootstrap = (): BootstrapResponse => ({
    status: status(),
    settings: {
      ...structuredClone(defaultSettings),
      paperless: { base_url: "https://paperless.example", api_token_configured: true },
    },
    providers: [],
  });

  await page.route("http://127.0.0.1:4173/api/**", async (route) => {
    const request = route.request();
    const path = new URL(request.url()).pathname;
    if (path === "/api/bootstrap") return json(route, bootstrap());
    if (path === "/api/status") return json(route, status());
    if (path === "/api/stats/summary") return json(route, {
      range: "month",
      totals: { input_tokens: 0, output_tokens: 0, reasoning_tokens: 0, cache_read_tokens: 0, cache_write_tokens: 0, total_tokens: 0, requests: 0, failed_requests: 0, estimated_requests: 0 },
      budgets: { daily_used: 0, daily_limit: null, monthly_used: 0, monthly_limit: null, warning_percent: 80, timezone: "Europe/Berlin" },
      by_provider: [],
      by_function: [],
    });
    if (path === "/api/jobs") {
      const offset = Number(new URL(request.url()).searchParams.get("offset") ?? 0);
      const item = {
        id: "waiting-job",
        document_id: 77,
        document_title: "Queued receipt",
        state: queuedState,
        attempt: 0,
        functions: ["title"],
        created_at: now,
        updated_at: now,
        error: queuedState === "rejected" ? "Removed from the waiting queue by the administrator" : undefined,
        token_total: 0,
      };
      return json(route, {
        items: queuedState === "rejected" && offset > 0 ? [] : [item],
        total: queuedState === "queued" ? 21 : 20,
        limit: 20,
        offset,
      });
    }
    if (["/api/worker/pause", "/api/worker/resume", "/api/jobs/waiting-job/cancel"].includes(path)) {
      expect(request.headers()["x-paperlessllm-action"]).toBe("worker-control");
      workerHeaderChecks += 1;
    }
    if (path === "/api/worker/pause") {
      paused = true;
      return json(route, worker());
    }
    if (path === "/api/worker/resume") {
      paused = false;
      return json(route, worker());
    }
    if (path === "/api/jobs/waiting-job/cancel") {
      queuedState = "rejected";
      return json(route, { ok: true, job_id: "waiting-job", state: "rejected", reason: "cancelled_by_user" });
    }
    return json(route, { message: `Unhandled mock endpoint ${request.method()} ${path}` }, 500);
  });

  await page.goto("/");
  await expect(page.getByText("2 documents in parallel", { exact: true })).toBeVisible();
  await expect(page.getByText("2 active", { exact: true })).toBeVisible();
  await expect(page.getByText("Incoming invoice", { exact: true })).toBeVisible();
  await expect(page.getByText("Bank statement", { exact: true })).toBeVisible();

  activeTitle = "Updated live invoice";
  await expect(page.getByText("Updated live invoice", { exact: true })).toBeVisible({ timeout: 5_000 });

  await page.getByRole("button", { name: "Pause processing" }).click();
  await expect(page.getByText("Pausing safely")).toBeVisible();
  await expect(page.getByRole("button", { name: "Continue processing" })).toBeVisible();

  await page.getByRole("link", { name: "Activity", exact: true }).click();
  await expect(page.locator(".live-worker small")).toContainText("Updated live invoice");
  await page.getByRole("button", { name: "Next" }).click();
  await expect(page.getByText("Showing 21–21 of 21")).toBeVisible();
  await page.getByRole("button", { name: "Show job details" }).click();
  await page.getByRole("button", { name: "Remove from queue" }).click();
  await expect(page.getByRole("dialog", { name: "Remove this document from the queue?" })).toBeVisible();
  await page.getByRole("button", { name: "Remove from queue" }).last().click();
  await expect(page.getByText("Document #77 removed from the queue")).toBeVisible();
  await expect(page.locator(".badge", { hasText: "Rejected" })).toBeVisible();
  await expect(page.getByText("Showing 1–20 of 20")).toBeVisible();

  await page.getByRole("button", { name: "Continue processing" }).click();
  await expect(page.getByRole("button", { name: "Pause processing" })).toBeVisible();
  expect(workerHeaderChecks).toBe(3);
});
