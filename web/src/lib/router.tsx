import { createContext, useCallback, useContext, useEffect, useMemo, useState, type AnchorHTMLAttributes, type MouseEvent, type ReactNode } from "react";

interface LocationState { pathname: string; search: string }
interface RouterValue { location: LocationState; navigate: (to: string, options?: { replace?: boolean }) => void }
const RouterContext = createContext<RouterValue | null>(null);

function browserLocation(): LocationState {
  return { pathname: window.location.pathname, search: window.location.search };
}

export function Router({ children }: { children: ReactNode }) {
  const [location, setLocation] = useState(browserLocation);
  useEffect(() => {
    const update = () => setLocation(browserLocation());
    window.addEventListener("popstate", update);
    return () => window.removeEventListener("popstate", update);
  }, []);
  const navigate = useCallback((to: string, options?: { replace?: boolean }) => {
    const target = new URL(to, window.location.href);
    if (target.origin !== window.location.origin) { window.location.assign(target); return; }
    window.history[options?.replace ? "replaceState" : "pushState"]({}, "", `${target.pathname}${target.search}${target.hash}`);
    setLocation(browserLocation());
    window.scrollTo({ top: 0, behavior: "instant" });
  }, []);
  const value = useMemo(() => ({ location, navigate }), [location, navigate]);
  return <RouterContext.Provider value={value}>{children}</RouterContext.Provider>;
}

function useRouter() {
  const value = useContext(RouterContext);
  if (!value) throw new Error("Router hooks must be used inside Router");
  return value;
}

export const useLocation = () => useRouter().location;
export const useNavigate = () => useRouter().navigate;

export function Link({ to, onClick, children, ...props }: AnchorHTMLAttributes<HTMLAnchorElement> & { to: string }) {
  const navigate = useNavigate();
  function follow(event: MouseEvent<HTMLAnchorElement>) {
    onClick?.(event);
    if (event.defaultPrevented || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey || props.target) return;
    event.preventDefault();
    navigate(to);
  }
  return <a {...props} href={to} onClick={follow}>{children}</a>;
}

export function Navigate({ to, replace = true }: { to: string; replace?: boolean }) {
  const navigate = useNavigate();
  useEffect(() => navigate(to, { replace }), [navigate, replace, to]);
  return null;
}
