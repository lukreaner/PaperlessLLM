import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useAsync, usePollingStale, useVisiblePolling } from "./hooks";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

afterEach(() => vi.useRealTimers());

describe("useAsync", () => {
  it("ignores a stale response after dependencies start a newer request", async () => {
    const first = deferred<string>();
    const second = deferred<string>();
    const load = vi.fn((key: string) => key === "first" ? first.promise : second.promise);
    const { result, rerender } = renderHook(
      ({ key }) => useAsync(() => load(key), [key]),
      { initialProps: { key: "first" } },
    );

    await waitFor(() => expect(load).toHaveBeenCalledWith("first"));
    rerender({ key: "second" });
    await waitFor(() => expect(load).toHaveBeenCalledWith("second"));

    await act(async () => second.resolve("newest"));
    await waitFor(() => expect(result.current.data).toBe("newest"));

    await act(async () => first.resolve("stale"));
    expect(result.current.data).toBe("newest");
    expect(result.current.loading).toBe(false);
    expect(result.current.error).toBeUndefined();
  });

  it("ignores a stale error after a newer request succeeds", async () => {
    const first = deferred<string>();
    const second = deferred<string>();
    const { result, rerender } = renderHook(
      ({ key }) => useAsync(() => key === "first" ? first.promise : second.promise, [key]),
      { initialProps: { key: "first" } },
    );

    rerender({ key: "second" });
    await act(async () => second.resolve("ready"));
    await waitFor(() => expect(result.current.data).toBe("ready"));
    await act(async () => first.reject(new Error("too late")));

    expect(result.current.data).toBe("ready");
    expect(result.current.error).toBeUndefined();
  });

  it("keeps slow background refreshes single-flight and lets the response win", async () => {
    const initial = deferred<string>();
    const background = deferred<string>();
    const load = vi.fn()
      .mockImplementationOnce(() => initial.promise)
      .mockImplementationOnce(() => background.promise);
    const { result } = renderHook(() => useAsync(load, []));

    await act(async () => initial.resolve("initial"));
    await waitFor(() => expect(result.current.data).toBe("initial"));

    let first!: Promise<void>;
    let second!: Promise<void>;
    act(() => {
      first = result.current.refreshSilently();
      second = result.current.refreshSilently();
    });
    expect(first).toBe(second);
    expect(load).toHaveBeenCalledTimes(2);

    await act(async () => background.resolve("fresh"));
    await first;
    expect(result.current.data).toBe("fresh");
    expect(result.current.lastSuccessAt).toBeTypeOf("number");
  });

  it("preserves live data and exposes a background refresh failure", async () => {
    const initial = deferred<string>();
    const background = deferred<string>();
    const load = vi.fn()
      .mockImplementationOnce(() => initial.promise)
      .mockImplementationOnce(() => background.promise);
    const { result } = renderHook(() => useAsync(load, []));

    await act(async () => initial.resolve("known-good"));
    await waitFor(() => expect(result.current.data).toBe("known-good"));

    const refresh = result.current.refreshSilently();
    await act(async () => background.reject(new Error("service unavailable")));
    await refresh;

    expect(result.current.data).toBe("known-good");
    expect(result.current.error).toBeUndefined();
    expect(result.current.backgroundError).toBe("service unavailable");
  });
});

describe("useVisiblePolling", () => {
  it("does not overlap a slow polling request", async () => {
    vi.useFakeTimers();
    const first = deferred<void>();
    const poll = vi.fn()
      .mockImplementationOnce(() => first.promise)
      .mockResolvedValue(undefined);
    const { unmount } = renderHook(() => useVisiblePolling(poll, 1_000));

    await act(async () => vi.advanceTimersByTimeAsync(1_000));
    expect(poll).toHaveBeenCalledTimes(1);
    await act(async () => vi.advanceTimersByTimeAsync(5_000));
    expect(poll).toHaveBeenCalledTimes(1);

    await act(async () => first.resolve());
    await act(async () => vi.advanceTimersByTimeAsync(1_000));
    expect(poll).toHaveBeenCalledTimes(2);
    unmount();
  });
});

describe("usePollingStale", () => {
  it("marks a silent or failed live feed as stale", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-08-01T10:00:00Z"));
    const successfulAt = Date.now();
    const { result, rerender } = renderHook(
      ({ error }) => usePollingStale(successfulAt, error, 1_000),
      { initialProps: { error: undefined as string | undefined } },
    );

    expect(result.current).toBe(false);
    act(() => vi.advanceTimersByTime(2_000));
    expect(result.current).toBe(true);

    vi.setSystemTime(successfulAt);
    rerender({ error: "connection lost" });
    expect(result.current).toBe(true);
  });
});
