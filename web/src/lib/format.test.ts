import { describe, expect, it } from "vitest";
import { budgetPercent, formatTokens, validationMessage } from "./format";

describe("formatting and validation", () => {
  it("formats compact token counts", () => {
    expect(formatTokens(999)).toBe("999");
    expect(formatTokens(12_400)).toBe("12.4k");
    expect(formatTokens(1_500_000)).toBe("1.5M");
  });

  it("bounds budget percentages", () => {
    expect(budgetPercent(120, 100)).toBe(100);
    expect(budgetPercent(20, 100)).toBe(20);
    expect(budgetPercent(20, null)).toBe(0);
  });

  it("requires valid HTTP URLs and provider models", () => {
    expect(validationMessage({ base_url: "paperless.local" })).toMatch(/valid/i);
    expect(validationMessage({ base_url: "ftp://example.com" })).toMatch(/HTTP/i);
    expect(validationMessage({ base_url: "https://example.com", model: "" })).toMatch(/model/i);
    expect(validationMessage({ base_url: "http://localhost:1234", model: "local" })).toBeNull();
  });
});
