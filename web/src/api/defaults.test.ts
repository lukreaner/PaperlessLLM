import { describe, expect, it } from "vitest";
import { defaultSettings, providerPresets } from "./defaults";

describe("safe defaults", () => {
  it("omits sampling for all provider presets", () => {
    expect(Object.values(providerPresets).every((provider) => provider.temperature === null)).toBe(true);
    expect(providerPresets.openai.transport).toBe("responses");
    expect(providerPresets.claude.transport).toBe("anthropic_messages");
  });

  it("configures hosted OpenAI-compatible presets without hard-coded models", () => {
    expect(providerPresets).toMatchObject({
      mistral: {
        name: "Mistral",
        base_url: "https://api.mistral.ai/v1",
        transport: "chat_completions",
        chat_output_token_field: "max_tokens",
      },
      kimi: {
        name: "Kimi (Global)",
        base_url: "https://api.moonshot.ai/v1",
        transport: "chat_completions",
        chat_output_token_field: "max_completion_tokens",
      },
      moonshot: {
        name: "Moonshot (China)",
        base_url: "https://api.moonshot.cn/v1",
        transport: "chat_completions",
        chat_output_token_field: "max_completion_tokens",
      },
      google: {
        name: "Google Gemini",
        base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
        transport: "chat_completions",
        chat_output_token_field: "max_tokens",
      },
    });

    for (const kind of ["mistral", "kimi", "moonshot", "google"] as const) {
      expect(providerPresets[kind].model).toBe("");
      expect(providerPresets[kind].api_key_configured).toBe(false);
      expect(providerPresets[kind].temperature).toBeNull();
    }
  });

  it("preserves Paperless metadata and restricts taxonomy by default", () => {
    expect(defaultSettings.processing.restrict_existing_tags).toBe(true);
    expect(defaultSettings.processing.restrict_existing_correspondents).toBe(true);
    expect(defaultSettings.processing.restrict_existing_document_types).toBe(true);
    expect(Object.values(defaultSettings.processing.overwrite).every((enabled) => !enabled)).toBe(true);
    expect(defaultSettings.processing.processed_tag_name).toBe("ai-processed");
  });

  it("starts file input and token budgets disabled", () => {
    expect(defaultSettings.file_input.enabled).toBe(false);
    expect(defaultSettings.limits.global_budget).toEqual({ daily: null, monthly: null });
    expect(defaultSettings.limits.max_input_tokens).toBe(32_000);
    expect(defaultSettings.limits.max_output_tokens).toBe(8_000);
    expect(defaultSettings.processing.inference_concurrency).toBe(1);
  });
});
