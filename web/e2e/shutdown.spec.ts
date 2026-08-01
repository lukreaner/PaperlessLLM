import { expect, test } from "@playwright/test";
import { defaultSettings } from "../src/api/defaults";

test("dashboard quit confirms the action, sends the protected header, and shows the stopped screen", async ({ page }) => {
  let shutdownHeader: string | undefined;
  const settings = structuredClone(defaultSettings);
  settings.paperless = { base_url: "https://paperless.example", api_token_configured: true };
  const status = {
    version: "0.1.0",
    setup_required: false,
    worker: { state: "running", last_scan_at: null, next_scan_at: null, active_job_id: null },
    paperless: { state: "connected", message: "Paperless connected" },
    provider: { state: "connected", message: "Provider connected" },
    queue: { queued: 0, analyzing: 0, awaiting_review: 0, applying: 0, completed: 0, budget_blocked: 0, failed: 0, rejected: 0 },
    auth: { required: false, authenticated: true, password_configured: false },
  };

  await page.route("http://127.0.0.1:4173/api/**", async (route) => {
    const request = route.request();
    const path = new URL(request.url()).pathname;
    if (path === "/api/bootstrap") {
      return route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ status, settings, providers: [], catalogs: undefined }) });
    }
    if (path === "/api/jobs") {
      return route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ items: [], total: 0, limit: 6, offset: 0 }) });
    }
    if (path === "/api/stats/summary") {
      return route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({
        range: "month",
        totals: { input_tokens: 0, output_tokens: 0, reasoning_tokens: 0, cache_read_tokens: 0, cache_write_tokens: 0, total_tokens: 0, requests: 0, failed_requests: 0, estimated_requests: 0 },
        budgets: { daily_used: 0, daily_limit: null, monthly_used: 0, monthly_limit: null, warning_percent: 80, timezone: "Europe/Berlin" },
        by_provider: [],
        by_function: [],
      }) });
    }
    if (path === "/api/shutdown") {
      shutdownHeader = request.headers()["x-paperlessllm-action"];
      return route.fulfill({ status: 202, contentType: "application/json", body: JSON.stringify({ ok: true }) });
    }
    return route.fulfill({ status: 500, contentType: "application/json", body: JSON.stringify({ message: `Unhandled mock endpoint ${request.method()} ${path}` }) });
  });

  await page.goto("/");
  await page.getByRole("button", { name: "Quit PaperlessLLM" }).click();
  await expect(page.getByRole("dialog", { name: "Quit PaperlessLLM?" })).toBeVisible();
  await expect(page.getByText(/settings, history, and Paperless documents are kept/)).toBeVisible();
  await page.getByRole("button", { name: "Quit PaperlessLLM", exact: true }).last().click();

  await expect(page.getByRole("heading", { name: "PaperlessLLM has stopped" })).toBeVisible();
  await expect(page.getByText(/double-click/)).toBeVisible();
  expect(shutdownHeader).toBe("shutdown");
});
