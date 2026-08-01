import { useCallback, useEffect, useRef, useState } from "react";

export interface AsyncState<T> {
  data?: T;
  loading: boolean;
  error?: string;
  refresh: () => Promise<void>;
  setData: React.Dispatch<React.SetStateAction<T | undefined>>;
}

export function useAsync<T>(load: () => Promise<T>, deps: React.DependencyList = []): AsyncState<T> {
  const [data, setData] = useState<T>();
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string>();
  const mounted = useRef(true);
  const requestSequence = useRef(0);

  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);

  const refresh = useCallback(async () => {
    const request = ++requestSequence.current;
    setLoading(true);
    setError(undefined);
    try {
      const result = await load();
      if (mounted.current && request === requestSequence.current) setData(result);
    } catch (caught) {
      if (mounted.current && request === requestSequence.current) setError(caught instanceof Error ? caught.message : "Something went wrong.");
    } finally {
      if (mounted.current && request === requestSequence.current) setLoading(false);
    }
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);

  useEffect(() => { void refresh(); }, [refresh]);

  return { data, loading, error, refresh, setData };
}
