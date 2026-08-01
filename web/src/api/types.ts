export type ProviderKind =
  | "openai"
  | "claude"
  | "mistral"
  | "kimi"
  | "moonshot"
  | "google"
  | "lmstudio"
  | "custom";
export type ProviderTransport = "responses" | "chat_completions" | "anthropic_messages";
export type ProviderApiFamily = "responses" | "chat_completions" | "messages";
export type JobState =
  | "queued"
  | "analyzing"
  | "awaiting_review"
  | "applying"
  | "completed"
  | "budget_blocked"
  | "failed"
  | "rejected";

export interface AuthStatus {
  required: boolean;
  authenticated: boolean;
  password_configured: boolean;
  csrf_token?: string;
}

export interface ConnectionHealth {
  state: "connected" | "disconnected" | "degraded" | "unconfigured";
  message?: string;
  checked_at?: string;
}

export interface ActiveJob {
  id: string;
  document_id: number;
  document_title: string;
  state: JobState;
  updated_at: string;
}

export interface ScanProgress {
  trigger: "manual" | "scheduled";
  discovered: number;
  queued: number;
  started_at: string;
}

export interface WorkerStatus {
  state: "running" | "scanning" | "pausing" | "paused" | "error";
  paused: boolean;
  last_scan_at?: string;
  next_scan_at?: string;
  active_job_id?: string;
  active_job?: ActiveJob;
  active_job_ids?: string[];
  active_jobs?: ActiveJob[];
  scan_progress?: ScanProgress;
}

export interface AppStatus {
  version: string;
  setup_required: boolean;
  worker: WorkerStatus;
  paperless: ConnectionHealth;
  provider: ConnectionHealth;
  queue: Record<JobState, number>;
  auth: AuthStatus;
}

export interface PaperlessSettings {
  base_url: string;
  api_token_configured: boolean;
  api_token?: string;
  custom_ca_pem?: string;
}

export interface ProviderCapabilities {
  structured_output: boolean;
  reasoning: boolean;
  temperature: boolean;
  file_pdf: boolean;
  file_images: boolean;
  token_counting: "exact" | "estimated" | "unknown";
}

export interface ProviderProfile {
  id: string;
  name: string;
  kind: ProviderKind;
  base_url: string;
  model: string;
  transport: ProviderTransport;
  chat_output_token_field?: "max_tokens" | "max_completion_tokens";
  api_key_configured: boolean;
  api_key?: string;
  headers?: Record<string, string>;
  headers_configured?: boolean;
  reasoning_effort?: "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max";
  temperature?: number | null;
  max_output_tokens?: number;
  active: boolean;
  capabilities?: ProviderCapabilities;
  last_tested_at?: string;
  last_test_ok?: boolean;
}

export interface CatalogItem {
  id: number;
  name: string;
  slug?: string;
  color?: string;
  data_type?: CustomFieldType;
  extra_data?: unknown;
}

export type CustomFieldType =
  | "string"
  | "long_text"
  | "url"
  | "date"
  | "boolean"
  | "integer"
  | "float"
  | "monetary"
  | "select";

export interface Catalogs {
  tags: CatalogItem[];
  correspondents: CatalogItem[];
  document_types: CatalogItem[];
  custom_fields: CatalogItem[];
  fetched_at?: string;
}

export interface CreateCustomFieldRequest {
  name: string;
  data_type: CustomFieldType;
  extra_data?: Record<string, unknown>;
}

export interface FunctionSettings {
  tags: boolean;
  correspondent: boolean;
  document_type: boolean;
  title: boolean;
  custom_fields: boolean;
}

export interface OverwriteSettings {
  tags: boolean;
  correspondent: boolean;
  document_type: boolean;
  title: boolean;
  custom_fields: boolean;
}

export interface ProcessingSettings {
  poll_interval_minutes: number;
  max_documents_per_scan: number | null;
  inference_concurrency: number;
  review_mode: boolean;
  use_existing_correspondents_and_tags: boolean;
  process_only_tagged: boolean;
  trigger_tag_ids: number[];
  trigger_match: "any" | "all";
  restrict_existing_tags: boolean;
  restrict_existing_correspondents: boolean;
  restrict_existing_document_types: boolean;
  add_processed_tag: boolean;
  processed_tag_name: string;
  functions: FunctionSettings;
  overwrite: OverwriteSettings;
  custom_field_ids: number[];
  custom_prompt_mode: "safe" | "expert";
  custom_prompt: string;
}

export interface FileInputSettings {
  enabled: boolean;
  rendition: "archive" | "original";
  max_megabytes: number;
  max_pages: number;
  ocr_excerpt_tokens: number;
}

export interface BudgetValue {
  daily: number | null;
  monthly: number | null;
}

export interface LimitSettings {
  max_input_tokens: number;
  max_output_tokens: number;
  warning_percent: number;
  reset_timezone: string;
  global_budget: BudgetValue;
  provider_budgets: Record<string, BudgetValue>;
}

export interface SecuritySettings {
  lan_enabled: boolean;
  bind_address: string;
  session_hours: number;
  reverse_proxy_tls: boolean;
}

export interface AppSettings {
  paperless: PaperlessSettings;
  processing: ProcessingSettings;
  file_input: FileInputSettings;
  limits: LimitSettings;
  security: SecuritySettings;
  active_provider_id?: string;
}

export interface BootstrapResponse {
  status: AppStatus;
  settings: AppSettings;
  providers: ProviderProfile[];
  catalogs?: Catalogs;
}

export interface DiagnosticLogs {
  available: boolean;
  entries: DiagnosticLogEntry[];
  truncated: boolean;
  updated_at?: string;
  message?: string;
  worker: DiagnosticWorkerSnapshot;
}

export interface DiagnosticLogEntry {
  timestamp: string;
  level: "TRACE" | "DEBUG" | "INFO" | "WARN" | "ERROR";
  event: string;
  target: "service" | "dashboard" | "worker" | "paperless" | "document" | "provider" | "budget" | "api";
  fields: Record<string, string>;
}

export interface DiagnosticWorkerSnapshot {
  state: "running" | "scanning" | "pausing" | "paused" | "error" | "unknown";
  active: boolean;
  active_count: number;
  scan_discovered?: number;
  scan_queued?: number;
  last_scan_at?: string;
  next_scan_at?: string;
}

export interface PaperlessTestResult {
  ok: boolean;
  version?: string;
  api_version?: number;
  user?: string;
  document_count?: number;
  message: string;
}

export interface ProviderTestResult {
  ok: boolean;
  message: string;
  latency_ms?: number;
  capabilities?: ProviderCapabilities;
}

export interface Job {
  id: string;
  document_id: number;
  document_title: string;
  document_url?: string;
  state: JobState;
  attempt: number;
  provider_name?: string;
  provider_kind?: ProviderKind;
  api_family?: ProviderApiFamily;
  model?: string;
  functions: string[];
  created_at: string;
  updated_at: string;
  error?: string;
  error_code?: string;
  failure_phase?: "preflight" | "provider" | "paperless" | "paperless_apply" | "budget" | "processing";
  upstream_status?: number;
  provider_operation?: "analysis" | "repair" | "probe" | "token_count";
  provider_attempt?: number;
  provider_status?: "succeeded" | "failed";
  provider_latency_ms?: number;
  truncated?: boolean;
  token_total?: number;
}

export interface JobsResponse {
  items: Job[];
  total: number;
  limit: number;
  offset: number;
}

export type FieldValue = string | number | boolean | null;

export interface ReviewSuggestion {
  tags?: string[];
  correspondent?: string | null;
  document_type?: string | null;
  title?: string;
  custom_fields?: Record<string, FieldValue>;
}

export interface ReviewItem {
  id: string;
  job_id: string;
  document_id: number;
  document_title: string;
  document_url?: string;
  created_at: string;
  current: ReviewSuggestion;
  suggested: ReviewSuggestion;
  warnings: string[];
  token_total?: number;
}

export interface ReviewsResponse {
  items: ReviewItem[];
  total: number;
}

export interface UsageTotals {
  input_tokens: number;
  output_tokens: number;
  reasoning_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  total_tokens: number;
  requests: number;
  failed_requests: number;
  reported_requests?: number;
  preflight_counted_requests?: number;
  estimated_requests: number;
  unknown_requests?: number;
}

export interface BudgetStatus {
  daily_used: number;
  daily_limit: number | null;
  monthly_used: number;
  monthly_limit: number | null;
  warning_percent: number;
  timezone: string;
}

export interface StatsSummary {
  range: string;
  totals: UsageTotals;
  budgets: BudgetStatus;
  by_provider: Array<{ provider: string; model: string; total_tokens: number; requests: number }>;
  by_function: Array<{ function: string; total_tokens: number; documents: number }>;
}

export interface StatsPoint extends UsageTotals {
  period: string;
}

export interface StatsSeries {
  range: string;
  points: StatsPoint[];
}

export interface ScanResult {
  discovered: number;
  queued: number;
  skipped: number;
  message?: string;
}

export interface CancelJobResult {
  ok: boolean;
  job_id: string;
  state: "rejected";
  reason: "cancelled_by_user";
}

export interface ApiErrorBody {
  error?: string;
  message?: string;
  details?: unknown;
}
