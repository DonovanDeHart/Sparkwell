import { useCallback, useEffect, useState } from 'react';
import { api, EVENTS, listen, toApiError } from '../services/api';
import type { AiStatus, AppSnapshot, LibraryInfo } from '../services/types';

/** App-wide status (pin, hotkey, library, local intelligence), kept live via core events. */
export function useSnapshot() {
  const [snapshot, setSnapshot] = useState<AppSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setSnapshot(await api.getAppSnapshot());
      setError(null);
    } catch (err) {
      setError(toApiError(err).message);
    }
  }, []);

  const patch = useCallback((partial: Partial<AppSnapshot>) => {
    setSnapshot((s) => (s ? { ...s, ...partial } : s));
  }, []);

  useEffect(() => {
    void refresh();
    const subscriptions = [
      listen<AiStatus>(EVENTS.aiStatus, (ai) => setSnapshot((s) => (s ? { ...s, ai } : s))),
      listen<LibraryInfo>(EVENTS.libraryChanged, (library) => setSnapshot((s) => (s ? { ...s, library } : s))),
    ];
    return () => {
      subscriptions.forEach((p) => void p.then((unlisten) => unlisten()).catch(() => undefined));
    };
  }, [refresh]);

  return { snapshot, error, refresh, patch };
}
