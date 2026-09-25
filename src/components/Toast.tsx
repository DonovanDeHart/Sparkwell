import { useCallback, useEffect, useRef, useState } from 'react';
import { Icon } from './Icon';

export interface ToastMessage {
  id: number;
  text: string;
  tone: 'success' | 'error' | 'info';
  action?: { label: string; run: () => void };
  duration?: number;
}

/** One calm toast at a time; a new message replaces the current one. */
export function useToast() {
  const [toast, setToast] = useState<ToastMessage | null>(null);
  const timer = useRef<number | undefined>(undefined);
  const nextId = useRef(1);

  const dismiss = useCallback(() => {
    window.clearTimeout(timer.current);
    setToast(null);
  }, []);

  const show = useCallback((message: Omit<ToastMessage, 'id'>) => {
    window.clearTimeout(timer.current);
    const id = nextId.current++;
    setToast({ ...message, id });
    const duration = message.duration ?? (message.action ? 5000 : message.tone === 'error' ? 4200 : 2200);
    timer.current = window.setTimeout(() => setToast((t) => (t?.id === id ? null : t)), duration);
  }, []);

  useEffect(() => () => window.clearTimeout(timer.current), []);

  return { toast, show, dismiss };
}

export function ToastView({ toast, onDismiss }: { toast: ToastMessage | null; onDismiss: () => void }) {
  return (
    <div className="toast-region" aria-live="polite" aria-atomic="true">
      {toast && (
        <div key={toast.id} className={`toast${toast.tone === 'error' ? ' is-error' : ''}`} role="status">
          <Icon name={toast.tone === 'error' ? 'alert' : toast.tone === 'success' ? 'check' : 'sparkle'} size={16} />
          <span className="toast-text">{toast.text}</span>
          {toast.action && (
            <button
              type="button"
              className="toast-action"
              onClick={() => {
                toast.action?.run();
                onDismiss();
              }}
            >
              {toast.action.label}
            </button>
          )}
        </div>
      )}
    </div>
  );
}
