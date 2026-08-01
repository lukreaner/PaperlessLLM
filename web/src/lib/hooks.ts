import { useCallback, useEffect, useRef, useState } from "react";

export interface AsyncState<T> {
  data?: T;
  loading: boolean;
  error?: string;
  backgroundError?: string;
  lastSuccessAt?: number;
  refresh: () => Promise<void>;
  refreshSilently: () => Promise<void>;
  setData: React.Dispatch<React.SetStateAction<T | undefined>>;
}

export function useAsync<T>(load: () => Promise<T>, deps: React.DependencyList = []): AsyncState<T> {
  const [data, setData] = useState<T>();
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string>();
  const [backgroundError, setBackgroundError] = useState<string>();
  const [lastSuccessAt, setLastSuccessAt] = useState<number>();
  const mounted = useRef(true);
  const requestSequence = useRef(0);
  const silentInFlight = useRef<Promise<void> | undefined>(undefined);

  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);

  const run = useCallback(async (silent: boolean) => {
    const request = ++requestSequence.current;
    if (!silent) {
      setLoading(true);
      setError(undefined);
    }
    try {
      const result = await load();
      if (mounted.current && request === requestSequence.current) {
        setData(result);
        setError(undefined);
        setBackgroundError(undefined);
        setLastSuccessAt(Date.now());
      }
    } catch (caught) {
      if (mounted.current && request === requestSequence.current) {
        const message = caught instanceof Error ? caught.message : "Something went wrong.";
        if (silent) setBackgroundError(message);
        else setError(message);
      }
    } finally {
      if (mounted.current && request === requestSequence.current) setLoading(false);
    }
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);

  const refresh = useCallback(() => run(false), [run]);
  const refreshSilently = useCallback(() => {
    // Polling is intentionally single-flight. Reusing a slow request lets its
    // result win instead of continuously invalidating it on every timer tick.
    if (silentInFlight.current) return silentInFlight.current;
    const request = run(true);
    silentInFlight.current = request;
    void request.finally(() => {
      if (silentInFlight.current === request) silentInFlight.current = undefined;
    });
    return request;
  }, [run]);

  useEffect(() => { void refresh(); }, [refresh]);

  return { data, loading, error, backgroundError, lastSuccessAt, refresh, refreshSilently, setData };
}

/** Poll only while the tab is visible and schedule the next run after the
 * previous one settles. This prevents slow local API calls from piling up. */
export function useVisiblePolling(poll: () => Promise<void>, intervalMs: number, enabled = true) {
  useEffect(() => {
    if (!enabled) return;
    let cancelled = false;
    let active = false;
    let timer: number | undefined;

    const schedule = () => {
      if (cancelled || document.visibilityState !== "visible") return;
      if (timer !== undefined) window.clearTimeout(timer);
      timer = window.setTimeout(() => {
        timer = undefined;
        void run();
      }, intervalMs);
    };
    const run = async () => {
      if (cancelled || active || document.visibilityState !== "visible") return;
      active = true;
      try {
        await poll();
      } finally {
        active = false;
        schedule();
      }
    };
    const visibilityChanged = () => {
      if (timer !== undefined) window.clearTimeout(timer);
      timer = undefined;
      if (document.visibilityState === "visible") void run();
    };

    schedule();
    document.addEventListener("visibilitychange", visibilityChanged);
    return () => {
      cancelled = true;
      if (timer !== undefined) window.clearTimeout(timer);
      document.removeEventListener("visibilitychange", visibilityChanged);
    };
  }, [enabled, intervalMs, poll]);
}

export function usePollingStale(lastSuccessAt?: number, backgroundError?: string, staleAfterMs = 8_000) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), Math.min(2_000, staleAfterMs));
    return () => window.clearInterval(timer);
  }, [staleAfterMs]);
  return Boolean(backgroundError) || lastSuccessAt === undefined || now - lastSuccessAt > staleAfterMs;
}
