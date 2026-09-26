import { useCallback, useEffect, useRef, useState } from 'react';
import { api, toApiError } from '../services/api';
import type { SearchOutcome, SparkSummary } from '../services/types';

export type SearchState =
  | { status: 'idle' }
  | { status: 'searching'; query: string; previous: SearchOutcome | null }
  | { status: 'done'; outcome: SearchOutcome }
  | { status: 'error'; query: string; message: string };

/** Only show the pending skeleton if retrieval takes longer than this, so
 *  instant local searches never flash a loading state. */
const PENDING_DELAY_MS = 140;
/** A search this slow is waiting for local intelligence to load its model. */
const SLOW_DELAY_MS = 1200;

export function useSearch() {
  const [state, setState] = useState<SearchState>({ status: 'idle' });
  const [pendingVisible, setPendingVisible] = useState(false);
  const [slow, setSlow] = useState(false);
  const requestId = useRef(0);
  const pendingTimer = useRef<number | undefined>(undefined);
  const slowTimer = useRef<number | undefined>(undefined);

  const run = useCallback(async (query: string) => {
    const q = query.trim();
    const id = ++requestId.current;
    window.clearTimeout(pendingTimer.current);
    window.clearTimeout(slowTimer.current);
    setSlow(false);
    if (!q) {
      setPendingVisible(false);
      setState({ status: 'idle' });
      return;
    }
    slowTimer.current = window.setTimeout(() => {
      if (requestId.current === id) setSlow(true);
    }, SLOW_DELAY_MS);
    setState((s) => ({
      status: 'searching',
      query: q,
      previous: s.status === 'done' ? s.outcome : s.status === 'searching' ? s.previous : null,
    }));
    pendingTimer.current = window.setTimeout(() => {
      if (requestId.current === id) setPendingVisible(true);
    }, PENDING_DELAY_MS);
    try {
      const outcome = await api.searchSparks(q);
      if (requestId.current !== id) return;
      setState({ status: 'done', outcome });
    } catch (err) {
      if (requestId.current !== id) return;
      setState({ status: 'error', query: q, message: toApiError(err).message });
    } finally {
      if (requestId.current === id) {
        window.clearTimeout(pendingTimer.current);
        window.clearTimeout(slowTimer.current);
        setPendingVisible(false);
        setSlow(false);
      }
    }
  }, []);

  const clear = useCallback(() => {
    requestId.current++;
    window.clearTimeout(pendingTimer.current);
    window.clearTimeout(slowTimer.current);
    setPendingVisible(false);
    setSlow(false);
    setState({ status: 'idle' });
  }, []);

  /** Reflects edits/favorite changes in the visible result without re-searching. */
  const patchSpark = useCallback((spark: SparkSummary) => {
    const fix = (o: SearchOutcome): SearchOutcome => ({
      ...o,
      best: o.best?.id === spark.id ? spark : o.best,
      alternatives: o.alternatives.map((a) => (a.id === spark.id ? spark : a)),
    });
    setState((s) => {
      if (s.status === 'done') return { status: 'done', outcome: fix(s.outcome) };
      if (s.status === 'searching' && s.previous) return { ...s, previous: fix(s.previous) };
      return s;
    });
  }, []);

  const removeSpark = useCallback((id: number) => {
    setState((s) => {
      if (s.status !== 'done') return s;
      const o = s.outcome;
      if (o.best?.id === id) return { status: 'idle' };
      return { status: 'done', outcome: { ...o, alternatives: o.alternatives.filter((a) => a.id !== id) } };
    });
  }, []);

  useEffect(
    () => () => {
      window.clearTimeout(pendingTimer.current);
      window.clearTimeout(slowTimer.current);
    },
    [],
  );

  return { state, pendingVisible, slow, run, clear, patchSpark, removeSpark };
}
