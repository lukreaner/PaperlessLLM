import type {
  AppSettings,
  AppStatus,
  AuthStatus,
  BootstrapResponse,
  Catalogs,
  CatalogItem,
  CreateCustomFieldRequest,
  DiagnosticLogs,
  JobsResponse,
  PaperlessTestResult,
  ProviderProfile,
  ProviderTestResult,
  ReviewsResponse,
  ReviewSuggestion,
  ScanResult,
  StatsSeries,
  StatsSummary,
  WorkerStatus,
  CancelJobResult,
} from "./types";

let csrfToken: string | undefined;

export class ApiError extends Error {
  status: number;
  details?: unknown;

  constructor(message: string, status: number, details?: unknown) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.details = details;
  }
}

function messageFromBody(body: unknown, fallback: string): string {
  if (body && typeof body === "object") {
    const candidate = body as { message?: unknown; error?: unknown };
    if (typeof candidate.message === "string") return candidate.message;
    if (typeof candidate.error === "string") return candidate.error;
  }
  return fallback;
}

export async function apiRequest<T>(path: string, init: RequestInit = {}): Promise<T> {
  const headers = new Headers(init.headers);
  if (init.body && !(init.body instanceof FormData)) headers.set("Content-Type", "application/json");
  if (csrfToken && init.method && init.method !== "GET") headers.set("X-CSRF-Token", csrfToken);

  let response: Response;
  try {
    response = await fetch(path, { ...init, headers, cache: "no-store", credentials: "same-origin" });
  } catch {
    if (init.signal?.aborted) throw new ApiError("PaperlessLLM did not answer in time. Retrying automatically.", 0);
    throw new ApiError("PaperlessLLM is not reachable. Check that the service is running.", 0);
  }

  const contentType = response.headers.get("content-type") ?? "";
  const body = response.status === 204
    ? undefined
    : contentType.includes("application/json")
      ? await response.json().catch(() => undefined)
      : await response.text().catch(() => undefined);

  if (!response.ok) {
    const fallback = response.status === 401
      ? "Your session has expired. Sign in again."
      : `Request failed (${response.status}).`;
    throw new ApiError(messageFromBody(body, fallback), response.status, body);
  }
  return body as T;
}

function timedGet<T>(path: string, timeoutMs = 10_000): Promise<T> {
  const controller = new AbortController();
  const timer = window.setTimeout(() => controller.abort(), timeoutMs);
  return apiRequest<T>(path, { signal: controller.signal }).finally(() => window.clearTimeout(timer));
}

function json(method: string, value?: unknown): RequestInit {
  return { method, body: value === undefined ? undefined : JSON.stringify(value) };
}

function queryString(params: Record<string, string | number | undefined>): string {
  const query = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) {
    if (value !== undefined && value !== "") query.set(key, String(value));
  }
  const encoded = query.toString();
  return encoded ? `?${encoded}` : "";
}

export const api = {
  status: () => timedGet<AppStatus>("/api/status"),
  bootstrap: async () => {
    const result = await apiRequest<BootstrapResponse>("/api/bootstrap");
    csrfToken = result.status.auth.csrf_token;
    return result;
  },
  authStatus: async () => {
    const result = await apiRequest<AuthStatus>("/api/auth/status");
    csrfToken = result.csrf_token;
    return result;
  },
  login: async (password: string) => {
    const result = await apiRequest<AuthStatus>("/api/auth/login", json("POST", { password }));
    csrfToken = result.csrf_token;
    return result;
  },
  logout: async () => {
    try {
      await apiRequest<void>("/api/auth/logout", json("POST"));
    } finally {
      csrfToken = undefined;
    }
  },
  shutdown: () => apiRequest<{ ok: boolean }>("/api/shutdown", {
    method: "POST",
    headers: { "X-PaperlessLLM-Action": "shutdown" },
  }),
  setPassword: (password: string, current_password?: string) =>
    apiRequest<{ ok: boolean }>("/api/auth/password", json("PUT", { password, current_password })),
  getSettings: () => apiRequest<AppSettings>("/api/settings"),
  saveSettings: (settings: AppSettings) => apiRequest<AppSettings>("/api/settings", json("PUT", settings)),
  testPaperless: (paperless: { base_url: string; api_token?: string; custom_ca_pem?: string }) =>
    apiRequest<PaperlessTestResult>("/api/paperless/test", json("POST", paperless)),
  catalogs: () => apiRequest<Catalogs>("/api/paperless/catalogs"),
  createCustomField: (field: CreateCustomFieldRequest) =>
    apiRequest<CatalogItem>("/api/paperless/custom-fields", json("POST", field)),
  providers: () => apiRequest<ProviderProfile[]>("/api/providers"),
  createProvider: (provider: Omit<ProviderProfile, "id">) =>
    apiRequest<ProviderProfile>("/api/providers", json("POST", provider)),
  updateProvider: (provider: ProviderProfile) =>
    apiRequest<ProviderProfile>(`/api/providers/${encodeURIComponent(provider.id)}`, json("PUT", provider)),
  deleteProvider: (id: string) =>
    apiRequest<void>(`/api/providers/${encodeURIComponent(id)}`, json("DELETE")),
  providerModels: (id: string) =>
    apiRequest<{ models: string[] }>(`/api/providers/${encodeURIComponent(id)}/models`),
  testProvider: (id: string) =>
    apiRequest<ProviderTestResult>(`/api/providers/${encodeURIComponent(id)}/test`, json("POST")),
  scan: () => apiRequest<ScanResult>("/api/scan", json("POST")),
  workerStatus: () => apiRequest<WorkerStatus>("/api/worker"),
  pauseWorker: () => apiRequest<WorkerStatus>("/api/worker/pause", { method: "POST", headers: { "X-PaperlessLLM-Action": "worker-control" } }),
  resumeWorker: () => apiRequest<WorkerStatus>("/api/worker/resume", { method: "POST", headers: { "X-PaperlessLLM-Action": "worker-control" } }),
  logs: () => timedGet<DiagnosticLogs>("/api/logs"),
  jobs: (params: { state?: string; q?: string; limit?: number; offset?: number } = {}) =>
    timedGet<JobsResponse>(`/api/jobs${queryString(params)}`),
  retryJob: (id: string) => apiRequest<{ ok: boolean }>(`/api/jobs/${encodeURIComponent(id)}/retry`, json("POST")),
  cancelJob: (id: string) => apiRequest<CancelJobResult>(`/api/jobs/${encodeURIComponent(id)}/cancel`, { method: "POST", headers: { "X-PaperlessLLM-Action": "worker-control" } }),
  reviews: () => apiRequest<ReviewsResponse>("/api/reviews"),
  approveReview: (id: string, suggestion: ReviewSuggestion) =>
    apiRequest<{ ok: boolean }>(`/api/reviews/${encodeURIComponent(id)}/approve`, json("POST", { suggestion })),
  rejectReview: (id: string) =>
    apiRequest<{ ok: boolean }>(`/api/reviews/${encodeURIComponent(id)}/reject`, json("POST")),
  statsSummary: (range: string) => apiRequest<StatsSummary>(`/api/stats/summary${queryString({ range })}`),
  statsSeries: (range: string) => apiRequest<StatsSeries>(`/api/stats/series${queryString({ range })}`),
};
