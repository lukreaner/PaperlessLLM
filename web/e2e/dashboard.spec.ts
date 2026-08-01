import { expect, test, type Page, type Route } from "@playwright/test";
import { defaultSettings } from "../src/api/defaults";
import type { AppSettings, BootstrapResponse, Catalogs, ProviderProfile } from "../src/api/types";

const now = "2026-08-01T10:00:00Z";
const catalogs: Catalogs = {
  tags: [{ id: 1, name: "Invoices" }, { id: 2, name: "Needs review" }],
  correspondents: [{ id: 3, name: "Acme GmbH" }],
  document_types: [{ id: 4, name: "Invoice" }],
  custom_fields: [{ id: 5, name: "Department", data_type: "select", extra_data: { select_options: [{ id: "ops", label: "Operations" }, { id: "sales", label: "Sales" }] } }],
  fetched_at: now,
};

const provider: ProviderProfile = {
  id: "provider-1",
  name: "LM Studio",
  kind: "lmstudio",
  base_url: "http://localhost:1234/v1",
  model: "local-model",
  transport: "responses",
  chat_output_token_field: "max_tokens",
  api_key_configured: false,
  active: true,
  temperature: null,
  last_test_ok: true,
  capabilities: { structured_output: false, reasoning: true, temperature: true, file_pdf: false, file_images: true, token_counting: "estimated" },
};

function status(setupRequired = false, authenticated = true) {
  return {
    version: "0.1.0",
    setup_required: setupRequired,
    worker: { state: "running" as const, last_scan_at: now, next_scan_at: now },
    paperless: { state: "connected" as const, message: "Paperless connected" },
    provider: { state: "connected" as const, message: "Provider connected" },
    queue: { queued: 0, analyzing: 0, awaiting_review: 1, applying: 0, completed: 8, budget_blocked: 1, failed: 0, rejected: 0 },
    auth: { required: false, authenticated, password_configured: false },
  };
}

function configuredBootstrap(overrides: Partial<BootstrapResponse> = {}): BootstrapResponse {
  const settings: AppSettings = structuredClone(defaultSettings);
  settings.paperless = { base_url: "https://paperless.example", api_token_configured: true };
  settings.active_provider_id = provider.id;
  return { status: status(), settings, providers: [provider], catalogs, ...overrides };
}

async function json(route: Route, body: unknown, statusCode = 200) {
  await route.fulfill({ status: statusCode, contentType: "application/json", body: JSON.stringify(body) });
}

async function installConfiguredApi(page: Page) {
  let reviewOpen = true;
  let approvedBody: unknown;
  let approveCalls = 0;
  let rejectCalls = 0;
  const bootstrap = configuredBootstrap();
  bootstrap.settings.processing.restrict_existing_correspondents = false;
  bootstrap.settings.processing.restrict_existing_document_types = false;
  await page.route("http://127.0.0.1:4173/api/**", async (route) => {
    const request = route.request();
    const path = new URL(request.url()).pathname;
    if (path === "/api/bootstrap") return json(route, bootstrap);
    if (path === "/api/status") return json(route, status());
    if (path === "/api/scan") return json(route, { discovered: 2, queued: 1, skipped: 1, message: "1 document queued" });
    if (path === "/api/jobs") return json(route, { items: [{ id: "job-1", document_id: 42, document_title: "August invoice", document_url: "https://paperless.example/documents/42/details", state: "budget_blocked", attempt: 1, provider_name: "LM Studio", model: "local-model", functions: ["tags", "title", "custom_fields"], created_at: now, updated_at: now, token_total: 1234 }], total: 1, limit: 50, offset: 0 });
    if (path === "/api/reviews" && reviewOpen) return json(route, { items: [{ id: "review-1", job_id: "job-1", document_id: 42, document_title: "August invoice", created_at: now, current: { title: "", tags: ["Needs review"], custom_fields: { Department: "ops" } }, suggested: { title: "Invoice August 2026", correspondent: "AI Sender", document_type: "AI Classification", tags: ["Invoices"], custom_fields: { Department: "sales" } }, warnings: ["OCR text was truncated"], token_total: 1234 }], total: 1 });
    if (path === "/api/reviews") return json(route, { items: [], total: 0 });
    if (path === "/api/reviews/review-1/approve") { approveCalls += 1; approvedBody = request.postDataJSON(); reviewOpen = false; return json(route, { ok: true }); }
    if (path === "/api/reviews/review-1/reject") { rejectCalls += 1; reviewOpen = false; return json(route, { ok: true }); }
    if (path === "/api/stats/summary") return json(route, { range: "30d", totals: { input_tokens: 9000, output_tokens: 2000, reasoning_tokens: 500, cache_read_tokens: 1000, cache_write_tokens: 0, total_tokens: 11000, requests: 4, failed_requests: 1, estimated_requests: 2 }, budgets: { daily_used: 10000, daily_limit: 10000, monthly_used: 11000, monthly_limit: 50000, warning_percent: 80, timezone: "Europe/Berlin" }, by_provider: [{ provider: "lmstudio", model: "local-model", total_tokens: 11000, requests: 4 }], by_function: [{ function: "title", total_tokens: 11000, documents: 3 }] });
    if (path === "/api/stats/series") return json(route, { range: "30d", points: [{ period: now, input_tokens: 9000, output_tokens: 2000, reasoning_tokens: 500, cache_read_tokens: 1000, cache_write_tokens: 0, total_tokens: 11000, requests: 4, failed_requests: 1, estimated_requests: 2 }] });
    return json(route, { message: `Unhandled mock endpoint ${request.method()} ${path}` }, 500);
  });
  return { approved: () => approvedBody, approveCalls: () => approveCalls, rejectCalls: () => rejectCalls };
}

test("first-run wizard tests both connections and reaches the dashboard", async ({ page }) => {
  let setupDone = false;
  let settings: AppSettings = structuredClone(defaultSettings);
  let savedProvider: ProviderProfile | undefined;
  await page.route("http://127.0.0.1:4173/api/**", async (route) => {
    const request = route.request();
    const path = new URL(request.url()).pathname;
    if (path === "/api/bootstrap") return json(route, setupDone ? configuredBootstrap({ settings, providers: savedProvider ? [savedProvider] : [] }) : { status: status(true), settings, providers: [], catalogs });
    if (path === "/api/paperless/test") return json(route, { ok: true, version: "3.2.1", api_version: 10, document_count: 127, message: "Connected" });
    if (path === "/api/paperless/catalogs") return json(route, catalogs);
    if (path === "/api/settings" && request.method() === "PUT") {
      settings = request.postDataJSON() as AppSettings;
      settings.paperless = { ...settings.paperless, api_token: undefined, api_token_configured: true };
      if (settings.active_provider_id) setupDone = true;
      return json(route, settings);
    }
    if (path === "/api/providers" && request.method() === "POST") {
      savedProvider = { ...(request.postDataJSON() as ProviderProfile), id: "provider-1", last_test_ok: undefined };
      return json(route, savedProvider, 201);
    }
    if (path === "/api/providers/provider-1" && request.method() === "PUT") {
      savedProvider = { ...(request.postDataJSON() as ProviderProfile), id: "provider-1", last_test_ok: undefined };
      return json(route, savedProvider);
    }
    if (path === "/api/providers/provider-1/test") {
      savedProvider = { ...(savedProvider ?? provider), last_test_ok: true, capabilities: provider.capabilities };
      return json(route, { ok: true, message: "Provider connection succeeded", latency_ms: 12, capabilities: provider.capabilities });
    }
    if (path === "/api/jobs") return json(route, { items: [], total: 0, limit: 6, offset: 0 });
    if (path === "/api/stats/summary") return json(route, { range: "month", totals: { input_tokens: 0, output_tokens: 0, reasoning_tokens: 0, cache_read_tokens: 0, cache_write_tokens: 0, total_tokens: 0, requests: 0, failed_requests: 0, estimated_requests: 0 }, budgets: { daily_used: 0, daily_limit: null, monthly_used: 0, monthly_limit: null, warning_percent: 80, timezone: "Europe/Berlin" }, by_provider: [], by_function: [] });
    return json(route, { message: `Unhandled mock endpoint ${request.method()} ${path}` }, 500);
  });

  await page.goto("/");
  await page.getByRole("button", { name: "Keep it local" }).click();
  await page.getByLabel("Paperless URL").fill("https://paperless.example");
  await page.getByLabel("API token").fill("paperless-token");
  await page.getByRole("button", { name: "Test connection" }).click();
  await expect(page.getByText(/127 documents/)).toBeVisible();
  await page.getByRole("button", { name: "Continue" }).click();
  await page.getByLabel("Model ID").fill("local-model");
  await page.getByRole("button", { name: "Test provider" }).click();
  await expect(page.getByText("Connection works")).toBeVisible();
  await page.getByRole("button", { name: "Continue" }).click();
  await page.getByRole("button", { name: "Continue" }).click();
  await page.getByRole("button", { name: "Finish setup" }).click();
  await expect(page.getByRole("heading", { name: "Overview" })).toBeVisible();
});

test("review, budget pause, activity, scan, and statistics flows work", async ({ page }) => {
  const mock = await installConfiguredApi(page);
  await page.goto("/");
  await expect(page.getByText(/waiting for a token budget reset/)).toBeVisible();
  await page.getByRole("button", { name: /Scan now/ }).click();
  await expect(page.getByText("1 document queued")).toBeVisible();

  await page.locator(".sidebar__nav").getByRole("link", { name: /Review/ }).click();
  const titleRow = page.locator(".comparison-row").filter({ hasText: "Title" });
  await titleRow.locator("input").fill("Approved invoice title");
  const departmentRow = page.locator(".comparison-row").filter({ hasText: "Department" });
  await departmentRow.locator("select").selectOption({ label: "Sales" });
  await page.getByRole("combobox", { name: "Suggested correspondent" }).fill("Corrected New Sender");
  await page.getByRole("combobox", { name: "Suggested document type" }).fill("Corrected New Type");
  await page.getByRole("button", { name: "Approve & apply" }).click();
  await expect(page.getByRole("heading", { name: "The review queue is clear" })).toBeVisible();
  expect(mock.approved()).toMatchObject({ suggestion: { title: "Approved invoice title", correspondent: "Corrected New Sender", document_type: "Corrected New Type", custom_fields: { Department: "sales" } } });

  await page.getByRole("link", { name: "Activity" }).click();
  await expect(page.getByText("August invoice")).toBeVisible();
  await expect(page.locator(".badge", { hasText: "Budget Blocked" })).toBeVisible();
  await page.getByRole("link", { name: "Statistics" }).click();
  await expect(page.getByRole("heading", { name: "Statistics" })).toBeVisible();
  await expect(page.getByText("11.0k").first()).toBeVisible();
  await expect(page.getByText("Reasoning / thinking")).toBeVisible();
});

test("LAN login establishes CSRF state and sign out returns to login", async ({ page }) => {
  let authenticated = false;
  let logoutCsrf: string | null = null;
  await page.route("http://127.0.0.1:4173/api/**", async (route) => {
    const request = route.request();
    const path = new URL(request.url()).pathname;
    if (path === "/api/bootstrap" && !authenticated) return json(route, { message: "unauthorized" }, 401);
    if (path === "/api/auth/login") { authenticated = true; return json(route, { required: true, authenticated: true, password_configured: true, csrf_token: "csrf-test" }); }
    if (path === "/api/bootstrap") return json(route, configuredBootstrap({ status: { ...status(), auth: { required: true, authenticated: true, password_configured: true, csrf_token: "csrf-test" } } }));
    if (path === "/api/auth/logout") { logoutCsrf = request.headers()["x-csrf-token"] ?? null; authenticated = false; return route.fulfill({ status: 204 }); }
    if (path === "/api/jobs") return json(route, { items: [], total: 0, limit: 6, offset: 0 });
    if (path === "/api/stats/summary") return json(route, { range: "month", totals: { input_tokens: 0, output_tokens: 0, reasoning_tokens: 0, cache_read_tokens: 0, cache_write_tokens: 0, total_tokens: 0, requests: 0, failed_requests: 0, estimated_requests: 0 }, budgets: { daily_used: 0, daily_limit: null, monthly_used: 0, monthly_limit: null, warning_percent: 80, timezone: "Europe/Berlin" }, by_provider: [], by_function: [] });
    return json(route, {});
  });

  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Welcome back" })).toBeVisible();
  await page.getByLabel("Administrator password").fill("correct horse battery staple");
  await page.getByRole("button", { name: "Sign in" }).click();
  await expect(page.getByRole("heading", { name: "Overview" })).toBeVisible();
  await page.getByRole("button", { name: "Sign out" }).click();
  await expect(page.getByRole("heading", { name: "Welcome back" })).toBeVisible();
  expect(logoutCsrf).toBe("csrf-test");
});

test("rejecting a review records the decision without sending an approval", async ({ page }) => {
  const mock = await installConfiguredApi(page);
  await page.goto("/review");

  await page.getByRole("button", { name: "Reject" }).click();
  await expect(page.getByRole("dialog", { name: "Reject this suggestion?" })).toBeVisible();
  await page.getByRole("button", { name: "Reject suggestion" }).click();

  await expect(page.getByRole("heading", { name: "The review queue is clear" })).toBeVisible();
  expect(mock.rejectCalls()).toBe(1);
  expect(mock.approveCalls()).toBe(0);
  expect(mock.approved()).toBeUndefined();
});

test("settings preserve an invalid draft and persist it after validation succeeds", async ({ page }) => {
  let settings = configuredBootstrap().settings;
  const submitted: AppSettings[] = [];
  await page.route("http://127.0.0.1:4173/api/**", async (route) => {
    const request = route.request();
    const path = new URL(request.url()).pathname;
    if (path === "/api/bootstrap") return json(route, configuredBootstrap({ settings }));
    if (path === "/api/settings" && request.method() === "PUT") {
      const candidate = request.postDataJSON() as AppSettings;
      submitted.push(candidate);
      if (candidate.limits.max_input_tokens < 1000) return json(route, { message: "maximum input tokens is outside the supported range" }, 400);
      settings = candidate;
      return json(route, settings);
    }
    if (path === "/api/status") return json(route, status());
    return json(route, { message: `Unhandled mock endpoint ${request.method()} ${path}` }, 500);
  });

  await page.goto("/settings?section=limits");
  const maximumInput = page.getByLabel("Maximum input tokens");
  await maximumInput.fill("100");
  await page.getByRole("button", { name: "Save changes" }).first().click();
  await expect(page.getByText("maximum input tokens is outside the supported range")).toBeVisible();
  await expect(maximumInput).toHaveValue("100");

  await maximumInput.fill("64000");
  await page.getByRole("button", { name: "Save changes" }).first().click();
  await expect(page.getByText("Settings saved")).toBeVisible();
  await expect(page.getByRole("button", { name: "Saved", exact: true })).toBeDisabled();
  expect(submitted).toHaveLength(2);
  expect(settings.limits.max_input_tokens).toBe(64000);
});

test("processing mode persists and a larger token budget resumes a blocked job", async ({ page }) => {
  let settings = configuredBootstrap().settings;
  settings.limits.global_budget = { daily: 10_000, monthly: 50_000 };
  let jobState: "budget_blocked" | "queued" = "budget_blocked";
  const persistedReviewModes: boolean[] = [];
  const currentStatus = () => ({
    ...status(),
    queue: {
      ...status().queue,
      queued: jobState === "queued" ? 1 : 0,
      budget_blocked: jobState === "budget_blocked" ? 1 : 0,
    },
  });
  await page.route("http://127.0.0.1:4173/api/**", async (route) => {
    const request = route.request();
    const path = new URL(request.url()).pathname;
    if (path === "/api/bootstrap") return json(route, configuredBootstrap({ settings, status: currentStatus() }));
    if (path === "/api/status") return json(route, currentStatus());
    if (path === "/api/jobs") return json(route, {
      items: [{ id: "job-budget", document_id: 77, document_title: "Budget-limited invoice", state: jobState, attempt: 1, provider_name: "LM Studio", model: "local-model", functions: ["title"], created_at: now, updated_at: now }],
      total: 1,
      limit: 20,
      offset: 0,
    });
    if (path === "/api/settings" && request.method() === "PUT") {
      const previousDailyLimit = settings.limits.global_budget.daily ?? 0;
      settings = request.postDataJSON() as AppSettings;
      persistedReviewModes.push(settings.processing.review_mode);
      if ((settings.limits.global_budget.daily ?? 0) > previousDailyLimit) jobState = "queued";
      return json(route, settings);
    }
    return json(route, { message: `Unhandled mock endpoint ${request.method()} ${path}` }, 500);
  });

  await page.goto("/activity");
  await expect(page.locator(".badge", { hasText: "Budget Blocked" })).toBeVisible();
  await page.getByRole("link", { name: "Settings" }).click();
  await page.getByRole("button", { name: /Processing/ }).click();

  const applyBehavior = page.getByLabel("Apply behavior");
  await expect(applyBehavior).toHaveValue("automatic");
  await applyBehavior.selectOption("review");
  await page.getByRole("button", { name: "Save changes" }).first().click();
  await expect.poll(() => persistedReviewModes).toEqual([true]);
  await expect(applyBehavior).toHaveValue("review");

  await applyBehavior.selectOption("automatic");
  await page.getByRole("button", { name: "Save changes" }).first().click();
  await expect.poll(() => persistedReviewModes).toEqual([true, false]);
  await expect(applyBehavior).toHaveValue("automatic");

  await page.getByRole("button", { name: /Limits & Files/ }).click();
  await page.getByLabel("All providers daily budget").fill("20000");
  await page.getByRole("button", { name: "Save changes" }).first().click();
  await expect.poll(() => jobState).toBe("queued");

  await page.getByRole("link", { name: "Activity" }).click();
  await expect(page.locator(".badge", { hasText: "Queued" })).toBeVisible();
  await expect(page.locator(".badge", { hasText: "Budget Blocked" })).toHaveCount(0);
  expect(settings.processing.review_mode).toBe(false);
  expect(settings.limits.global_budget.daily).toBe(20_000);
});

test("LAN first-run setup stores the password before enabling authentication", async ({ page }) => {
  let settings: AppSettings = structuredClone(defaultSettings);
  let savedProvider: ProviderProfile | undefined;
  let lanEnabled = false;
  let passwordSet = false;
  const finalizationOrder: string[] = [];
  await page.route("http://127.0.0.1:4173/api/**", async (route) => {
    const request = route.request();
    const path = new URL(request.url()).pathname;
    if (path === "/api/bootstrap" && lanEnabled) return json(route, { message: "unauthorized" }, 401);
    if (path === "/api/bootstrap") return json(route, { status: status(true), settings, providers: savedProvider ? [savedProvider] : [], catalogs });
    if (path === "/api/paperless/test") return json(route, { ok: true, version: "3.2.1", api_version: 10, document_count: 127, message: "Connected" });
    if (path === "/api/paperless/catalogs") return json(route, catalogs);
    if (path === "/api/auth/password") {
      passwordSet = true;
      finalizationOrder.push("password");
      return json(route, { ok: true });
    }
    if (path === "/api/settings" && request.method() === "PUT") {
      const candidate = request.postDataJSON() as AppSettings;
      if (candidate.security.lan_enabled) {
        finalizationOrder.push("LAN settings");
        if (!passwordSet) return json(route, { message: "set an administrator password before enabling LAN access" }, 409);
        lanEnabled = true;
      }
      settings = candidate;
      settings.paperless = { ...settings.paperless, api_token: undefined, api_token_configured: true };
      return json(route, settings);
    }
    if (path === "/api/providers" && request.method() === "POST") {
      savedProvider = { ...(request.postDataJSON() as ProviderProfile), id: "provider-1", last_test_ok: undefined };
      return json(route, savedProvider, 201);
    }
    if (path === "/api/providers/provider-1" && request.method() === "PUT") {
      savedProvider = { ...(request.postDataJSON() as ProviderProfile), id: "provider-1", last_test_ok: undefined };
      return json(route, savedProvider);
    }
    if (path === "/api/providers/provider-1/test") return json(route, { ok: true, message: "Provider connection succeeded", latency_ms: 12, capabilities: provider.capabilities });
    return json(route, { message: `Unhandled mock endpoint ${request.method()} ${path}` }, 500);
  });

  await page.goto("/");
  await page.getByRole("button", { name: "Keep it local" }).click();
  await page.getByLabel("Paperless URL").fill("https://paperless.example");
  await page.getByLabel("API token").fill("paperless-token");
  await page.getByRole("button", { name: "Test connection" }).click();
  await expect(page.getByText(/127 documents/)).toBeVisible();
  await page.getByRole("button", { name: "Continue" }).click();
  await page.getByLabel("Model ID").fill("local-model");
  await page.getByRole("button", { name: "Test provider" }).click();
  await expect(page.getByText("Connection works")).toBeVisible();
  await page.getByRole("button", { name: "Continue" }).click();
  await page.getByRole("button", { name: "Continue" }).click();
  await page.getByText("Require authentication for network access", { exact: true }).click();
  await expect(page.getByRole("checkbox", { name: /Require authentication for network access/ })).toBeChecked();
  await page.getByText("HTTPS reverse proxy is configured", { exact: true }).click();
  await expect(page.getByRole("checkbox", { name: /HTTPS reverse proxy is configured/ })).toBeChecked();
  await page.getByLabel("Administrator password").fill("correct horse battery staple");
  await page.getByLabel("Confirm password").fill("correct horse battery staple");
  await page.getByRole("button", { name: "Finish setup" }).click();

  await expect(page.getByRole("heading", { name: "Welcome back" })).toBeVisible();
  expect(finalizationOrder).toEqual(["password", "LAN settings"]);
});
