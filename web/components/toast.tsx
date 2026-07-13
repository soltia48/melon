"use client";

import {
  createContext,
  useCallback,
  useContext,
  useRef,
  useState,
} from "react";

type ToastFn = (message: string) => void;

const ToastContext = createContext<ToastFn>(() => {});

export function useToast(): ToastFn {
  return useContext(ToastContext);
}

export function ToastProvider({ children }: { children: React.ReactNode }) {
  const [message, setMessage] = useState<string | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const toast = useCallback<ToastFn>((msg) => {
    setMessage(msg);
    if (timer.current) clearTimeout(timer.current);
    timer.current = setTimeout(() => setMessage(null), 4000);
  }, []);

  return (
    <ToastContext.Provider value={toast}>
      {children}
      {message !== null && (
        <div className="toast" role="alert" onClick={() => setMessage(null)}>
          {message}
        </div>
      )}
    </ToastContext.Provider>
  );
}
