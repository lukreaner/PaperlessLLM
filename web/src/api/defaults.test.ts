import { describe, expect, it } from "vitest";
import { defaultSettings, providerPresets } from "./defaults";

describe("safe defaults", () => {
  it("omits sampling for all provider presets", () => {
    expect(Object.values(providerPresets).every((provider) => provider.temperature === null)).toBe(true);
    expect(providerPresets.openai.transport).toBe("responses");
    expect(providerPresets.claude.transport).toBe("anthropic_messages");
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
  });
});
