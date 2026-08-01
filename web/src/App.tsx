import { createContext, useCallback, useContext, useEffect, useState } from "react";
import { api, ApiError } from "./api/client";
import type { AppSettings, AppStatus, BootstrapResponse, Catalogs, ProviderProfile } from "./api/types";
import { Layout } from "./components/Layout";
import { Button, ErrorState, LoadingState } from "./components/UI";
import { Logo } from "./components/Logo";
import { ActivityPage } from "./pages/ActivityPage";
import { LoginPage } from "./pages/LoginPage";
import { OverviewPage } from "./pages/OverviewPage";
import { ReviewPage } from "./pages/ReviewPage";
import { SettingsPage } from "./pages/SettingsPage";
import { SetupPage } from "./pages/SetupPage";
import { StatisticsPage } from "./pages/StatisticsPage";
import { Navigate, useLocation, useNavigate } from "./lib/router";

interface AppContextValue {
  status: AppStatus;
  settings: AppSettings;
  providers: ProviderProfile[];
  catalogs?: Catalogs;
  refreshStatus: () => Promise<void>;
  refreshBootstrap: () => Promise<void>;
  updateSettings: (settings: AppSettings) => void;
  updateProviders: (providers: ProviderProfile[]) => void;
  updateCatalogs: (catalogs: Catalogs) => void;
}

const AppContext = createContext<AppContextValue | null>(null);
export function useApp() {
  const value = useContext(AppContext);
  if (!value) throw new Error("useApp must be used inside the application shell");
  return value;
}

function StartupError({ message, onRetry }: { message: string; onRetry: () => void }) {
  return (
    <div className="startup-screen">
      <Logo />
      <div className="startup-card"><ErrorState message={message} onRetry={onRetry} /></div>
      <p className="startup-help">The dashboard talks only to the local PaperlessLLM service.</p>
    </div>
  );
}

function SetupComplete({ onComplete }: { onComplete: () => Promise<void> }) {
  const navigate = useNavigate();
  return <SetupPage onComplete={async () => { await onComplete(); navigate("/"); }} />;
}

export default function App() {
  const location = useLocation();
  const [bootstrap, setBootstrap] = useState<BootstrapResponse>();
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string>();
  const [needsLogin, setNeedsLogin] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    setError(undefined);
    try {
      const result = await api.bootstrap();
      setBootstrap(result);
      setNeedsLogin(result.status.auth.required && !result.status.auth.authenticated);
    } catch (caught) {
      if (caught instanceof ApiError && caught.status === 401) {
        setNeedsLogin(true);
      } else {
        setError(caught instanceof Error ? caught.message : "Unable to start PaperlessLLM.");
      }
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { void load(); }, [load]);

  const refreshStatus = useCallback(async () => {
    try {
      const status = await api.status();
      setBootstrap((current) => current ? { ...current, status } : current);
    } catch (caught) {
      if (caught instanceof ApiError && caught.status === 401) setNeedsLogin(true);
    }
  }, []);

  const logout = useCallback(async () => {
    try {
      await api.logout();
    } finally {
      setNeedsLogin(true);
      setBootstrap((current) => current ? { ...current, status: { ...current.status, auth: { ...current.status.auth, authenticated: false, csrf_token: undefined } } } : current);
    }
  }, []);

  useEffect(() => {
    if (!bootstrap || bootstrap.status.setup_required || needsLogin) return;
    const interval = window.setInterval(() => void refreshStatus(), 30_000);
    return () => window.clearInterval(interval);
  }, [bootstrap, needsLogin, refreshStatus]);

  if (loading && !bootstrap) return <div className="startup-screen"><Logo /><LoadingState label="Opening your workspace" /></div>;
  if (error && !bootstrap) return <StartupError message={error} onRetry={() => void load()} />;
  if (needsLogin) return <LoginPage onAuthenticated={() => { setNeedsLogin(false); void load(); }} />;
  if (!bootstrap) return <StartupError message="The service returned an empty response." onRetry={() => void load()} />;

  const value: AppContextValue = {
    status: bootstrap.status,
    settings: bootstrap.settings,
    providers: bootstrap.providers,
    catalogs: bootstrap.catalogs,
    refreshStatus,
    refreshBootstrap: load,
    updateSettings: (settings) => setBootstrap((current) => current ? { ...current, settings } : current),
    updateProviders: (providers) => setBootstrap((current) => current ? { ...current, providers } : current),
    updateCatalogs: (catalogs) => setBootstrap((current) => current ? { ...current, catalogs } : current),
  };

  if (bootstrap.status.setup_required) {
    return (
      <AppContext.Provider value={value}>
        {location.pathname === "/setup" ? <SetupComplete onComplete={load} /> : <Navigate to="/setup" replace />}
      </AppContext.Provider>
    );
  }

  return (
    <AppContext.Provider value={value}>
      <Layout status={bootstrap.status} onRefreshStatus={() => void refreshStatus()} onLogout={() => void logout()}>
        <AppRoute pathname={location.pathname} />
      </Layout>
    </AppContext.Provider>
  );
}

function AppRoute({ pathname }: { pathname: string }) {
  if (pathname === "/") return <OverviewPage />;
  if (pathname === "/review") return <ReviewPage />;
  if (pathname === "/activity") return <ActivityPage />;
  if (pathname === "/statistics") return <StatisticsPage />;
  if (pathname === "/settings") return <SettingsPage />;
  if (pathname === "/setup") return <Navigate to="/settings" replace />;
  return <NotFound />;
}

function NotFound() {
  const navigate = useNavigate();
  return <div className="not-found"><span>404</span><h1>That page isn’t here</h1><p>The document trail goes cold at this URL.</p><Button onClick={() => navigate("/")}>Back to overview</Button></div>;
}
