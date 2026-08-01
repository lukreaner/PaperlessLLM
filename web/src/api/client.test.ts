import { afterEach, describe, expect, it, vi } from "vitest";
import { api } from "./client";

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("desktop service actions", () => {
  it("sends the explicit non-simple confirmation header when shutting down", async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response(JSON.stringify({ ok: true }), {
      status: 202,
      headers: { "Content-Type": "application/json" },
    }));
    vi.stubGlobal("fetch", fetchMock);

    await expect(api.shutdown()).resolves.toEqual({ ok: true });
    expect(fetchMock).toHaveBeenCalledOnce();
    const [path, options] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(path).toBe("/api/shutdown");
    expect(options.method).toBe("POST");
    expect(options.cache).toBe("no-store");
    expect(new Headers(options.headers).get("X-PaperlessLLM-Action")).toBe("shutdown");
  });

  it("aborts a hung live-status request so polling can recover", async () => {
    vi.useFakeTimers();
    const fetchMock = vi.fn().mockImplementation((_path: string, options: RequestInit) => new Promise((_resolve, reject) => {
      options.signal?.addEventListener("abort", () => reject(new DOMException("aborted", "AbortError")));
    }));
    vi.stubGlobal("fetch", fetchMock);

    const outcome = expect(api.status()).rejects.toThrow("did not answer in time");
    await vi.advanceTimersByTimeAsync(10_000);
    await outcome;
    expect(fetchMock).toHaveBeenCalledOnce();
  });
});
