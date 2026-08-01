import { describe, expect, it } from "vitest";
import { credentialDestination, newProvider } from "./ProviderEditor";

describe("credentialDestination", () => {
  it("canonicalizes harmless URL formatting without crossing provider presets", () => {
    const mistral = newProvider("mistral");
    expect(credentialDestination({
      ...mistral,
      base_url: "HTTPS://API.MISTRAL.AI:443/v1/",
    })).toBe(credentialDestination(mistral));

    expect(credentialDestination({ ...mistral, kind: "google" }))
      .not.toBe(credentialDestination(mistral));
  });
});
