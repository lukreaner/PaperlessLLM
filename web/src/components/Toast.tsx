import { createContext, useCallback, useContext, useMemo, useState, type ReactNode } from "react";

type ToastTone = "success" | "error" | "info";
interface ToastMessage { id: number; text: string; tone: ToastTone }
interface ToastContextValue { showToast: (text: string, tone?: ToastTone) => void }

const ToastContext = createContext<ToastContextValue>({ showToast: () => undefined });

export function ToastProvider({ children }: { children: ReactNode }) {
  const [messages, setMessages] = useState<ToastMessage[]>([]);
  const showToast = useCallback((text: string, tone: ToastTone = "success") => {
    const id = Date.now() + Math.random();
    setMessages((current) => [...current, { id, text, tone }]);
    window.setTimeout(() => setMessages((current) => current.filter((item) => item.id !== id)), 4400);
  }, []);
  const value = useMemo(() => ({ showToast }), [showToast]);

  return (
    <ToastContext.Provider value={value}>
      {children}
      <div className="toast-region" aria-live="polite">
        {messages.map((message) => (
          <button key={message.id} className={`toast toast--${message.tone}`} onClick={() => setMessages((current) => current.filter((item) => item.id !== message.id))}>
            <span>{message.tone === "success" ? "✓" : message.tone === "error" ? "!" : "i"}</span>
            {message.text}
          </button>
        ))}
      </div>
    </ToastContext.Provider>
  );
}

export const useToast = () => useContext(ToastContext);
