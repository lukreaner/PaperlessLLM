import { act, renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { useAsync } from "./hooks";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

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
});
