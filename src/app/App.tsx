import { useCallback, useEffect, useRef, useState } from 'react';
import { Icon } from '../components/Icon';
import { ToastView, useToast } from '../components/Toast';
import { SparkEditor, type EditorMode } from '../features/add-spark/SparkEditor';
import { Favorites } from '../features/favorites/Favorites';
import { GoalInput } from '../features/search/GoalInput';
import { ResultRegion } from '../features/search/ResultRegion';
import { SettingsPanel } from '../features/settings/SettingsPanel';
import { Welcome } from '../features/settings/Welcome';
import { api, EVENTS, listen, toApiError } from '../services/api';
import type { AiStatus, LibraryInfo, SparkSummary } from '../services/types';
import { Footer } from './Footer';
import { Header } from './Header';
import { EmptyLibrary, HotkeyUnavailable, LibraryUnavailable } from './StateCards';
import { useSearch } from './useSearch';
import { useSnapshot } from './useSnapshot';

type Overlay = { kind: 'settings' } | EditorMode | null;

const OFFLINE_AI: AiStatus = {
  state: 'checking',
  embedModel: null,
  semanticModel: 'qwen3-embedding:8b-q8_0',
  chatModel: null,
  chatModelsTooLarge: false,
  indexed: 0,
  total: 0,
  indexing: false,
};

/** After a successful copy an unpinned panel collapses back into its bay,
 *  returning focus to the app the user was working in (ready for Ctrl+V). */
const COLLAPSE_AFTER_COPY_MS = 650;
const COPIED_FEEDBACK_MS = 1800;
/** The panel's maximum height (platform.rs MAX_HEIGHT): overlays get it all. */
const FULL_HEIGHT = 900;

/** Puts keyboard focus inside the top-most overlay (welcome, Settings, editor):
 *  on its `data-autofocus` control if it has one, else on the overlay itself.
 *  Returns false when no overlay is open. */
function focusTopOverlay(): boolean {
  const overlays = document.querySelectorAll<HTMLElement>('.overlay');
  const top = overlays[overlays.length - 1];
  if (!top) return false;
  if (!top.contains(document.activeElement)) (top.querySelector<HTMLElement>('[data-autofocus]') ?? top).focus();
  return true;
}

export function App() {
  const { snapshot, error: snapshotError, refresh, patch } = useSnapshot();
  const search = useSearch();
  const { toast, show: showToast, dismiss: dismissToast } = useToast();
  const [favorites, setFavorites] = useState<SparkSummary[] | null>(null);
  const [query, setQuery] = useState('');
  const [overlay, setOverlay] = useState<Overlay>(null);
  const [copiedId, setCopiedId] = useState<number | null>(null);
  const [entering, setEntering] = useState(true);
  const [hotkeyNoticeDismissed, setHotkeyNoticeDismissed] = useState(false);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const mainRef = useRef<HTMLElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const copiedTimer = useRef<number | undefined>(undefined);
  const collapseTimer = useRef<number | undefined>(undefined);

  const pinned = snapshot?.pinned ?? false;
  const library = snapshot?.library ?? null;
  const libraryReady = library?.available ?? false;
  const ai = snapshot?.ai ?? OFFLINE_AI;
  const welcome = snapshot !== null && !snapshot.onboarded;
  const hotkeyUnavailable =
    snapshot !== null && snapshot.onboarded && !snapshot.hotkey.registered && snapshot.hotkey.error !== null;

  // Latest values for event handlers registered once.
  const live = useRef({ pinned, overlay, query, search, libraryReady, welcome });
  live.current = { pinned, overlay, query, search, libraryReady, welcome };

  const loadFavorites = useCallback(async () => {
    try {
      setFavorites(await api.listFavorites());
    } catch {
      setFavorites([]);
    }
  }, []);

  const refreshLibraryInfo = useCallback(async () => {
    try {
      patch({ library: await api.getLibraryInfo() });
    } catch {
      /* the snapshot keeps its last known state */
    }
  }, [patch]);

  useEffect(() => {
    if (libraryReady) void loadFavorites();
    else setFavorites(null);
  }, [libraryReady, library?.dir, loadFavorites]);

  const focusGoal = useCallback((select: boolean) => {
    const el = inputRef.current;
    if (!el) return;
    el.focus();
    if (select) el.select();
  }, []);

  // Window lifecycle from the core: shown (hotkey/tray) and hidden.
  useEffect(() => {
    const subs = [
      listen<boolean>(EVENTS.shown, (fresh) => {
        if (fresh) {
          setEntering(true);
          window.setTimeout(() => setEntering(false), 260);
        }
        if (!focusTopOverlay()) focusGoal(fresh);
      }),
      listen<null>(EVENTS.hidden, () => {
        window.clearTimeout(collapseTimer.current);
        setCopiedId(null);
        dismissToast();
      }),
    ];
    const t = window.setTimeout(() => setEntering(false), 260);
    return () => {
      window.clearTimeout(t);
      subs.forEach((p) => void p.then((un) => un()).catch(() => undefined));
    };
  }, [focusGoal, dismissToast]);

  // Frosted glass is decided by the core (Windows version and settings).
  const glass = snapshot?.glass ?? false;
  useEffect(() => {
    document.documentElement.toggleAttribute('data-glass', glass);
  }, [glass]);

  // The panel fits its content: report the height it needs (the core keeps
  // it within a compact range and on screen). Overlays get the full height.
  const tall = overlay !== null || welcome;
  useEffect(() => {
    const panel = panelRef.current;
    const main = mainRef.current;
    const content = contentRef.current;
    if (!panel || !main || !content || typeof ResizeObserver === 'undefined') return;
    let frame = 0;
    let reported = 0;
    const report = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        const style = getComputedStyle(main);
        const padding = parseFloat(style.paddingTop) + parseFloat(style.paddingBottom);
        const chrome = panel.offsetHeight - main.clientHeight;
        const needed = tall ? FULL_HEIGHT : Math.ceil(chrome + padding + content.offsetHeight);
        if (Math.abs(needed - reported) < 2) return;
        reported = needed;
        void api.setPanelHeight(needed).catch(() => undefined);
      });
    };
    const observer = new ResizeObserver(report);
    observer.observe(content);
    report();
    return () => {
      observer.disconnect();
      cancelAnimationFrame(frame);
    };
  }, [tall]);

  // Pending copy feedback / collapse must never outlive the shell.
  useEffect(
    () => () => {
      window.clearTimeout(copiedTimer.current);
      window.clearTimeout(collapseTimer.current);
    },
    [],
  );

  // Focus the goal input (or the first-run welcome) as soon as the shell is ready.
  const hasSnapshot = snapshot !== null;
  useEffect(() => {
    if (hasSnapshot && !focusTopOverlay()) focusGoal(false);
  }, [hasSnapshot, focusGoal]);

  const hide = useCallback(() => {
    void api.hidePanel().catch(() => undefined);
  }, []);

  const copy = useCallback(
    async (spark: SparkSummary) => {
      try {
        await api.copySpark(spark.id);
        setCopiedId(spark.id);
        window.clearTimeout(copiedTimer.current);
        copiedTimer.current = window.setTimeout(() => setCopiedId(null), COPIED_FEEDBACK_MS);
        showToast({ tone: 'success', text: `Copied “${spark.title}”` });
        if (!live.current.pinned) {
          window.clearTimeout(collapseTimer.current);
          collapseTimer.current = window.setTimeout(hide, COLLAPSE_AFTER_COPY_MS);
        }
      } catch (err) {
        showToast({ tone: 'error', text: toApiError(err).message });
      }
    },
    [showToast, hide],
  );

  const toggleFavorite = useCallback(
    async (spark: SparkSummary) => {
      try {
        const updated = await api.setFavorite(spark.id, !spark.favorite);
        search.patchSpark(updated);
        await loadFavorites();
        if (updated.favorite) {
          showToast({ tone: 'success', text: `Added “${updated.title}” to Favorites` });
        } else {
          showToast({
            tone: 'info',
            text: `Removed “${updated.title}” from Favorites`,
            action: {
              label: 'Undo',
              run: () =>
                void api
                  .setFavorite(updated.id, true)
                  .then((restored) => {
                    search.patchSpark(restored);
                    return loadFavorites();
                  })
                  .catch((err) => showToast({ tone: 'error', text: toApiError(err).message })),
            },
          });
        }
      } catch (err) {
        showToast({ tone: 'error', text: toApiError(err).message });
      }
    },
    [search, loadFavorites, showToast],
  );

  const openAdd = useCallback(() => {
    if (live.current.libraryReady) setOverlay({ kind: 'add' });
  }, []);

  const closeOverlay = useCallback(() => {
    setOverlay(null);
    requestAnimationFrame(() => {
      if (!focusTopOverlay()) focusGoal(false);
    });
  }, [focusGoal]);

  const finishWelcome = useCallback(() => {
    patch({ onboarded: true });
    requestAnimationFrame(() => focusGoal(false));
  }, [patch, focusGoal]);

  const deleteSpark = useCallback(
    async (spark: SparkSummary) => {
      try {
        await api.deleteSpark(spark.id);
        search.removeSpark(spark.id);
        await Promise.all([loadFavorites(), refreshLibraryInfo()]);
        showToast({ tone: 'info', text: `Deleted “${spark.title}”` });
      } catch (err) {
        showToast({ tone: 'error', text: toApiError(err).message });
      }
    },
    [search, loadFavorites, refreshLibraryInfo, showToast],
  );

  const onSaved = useCallback(
    (spark: SparkSummary, created: boolean) => {
      search.patchSpark(spark);
      void loadFavorites();
      void refreshLibraryInfo();
      closeOverlay();
      showToast({ tone: 'success', text: created ? `Saved “${spark.title}” to your library` : `Updated “${spark.title}”` });
    },
    [search, loadFavorites, refreshLibraryInfo, closeOverlay, showToast],
  );

  const onDeletedFromEditor = useCallback(
    (id: number, title: string) => {
      search.removeSpark(id);
      void loadFavorites();
      void refreshLibraryInfo();
      closeOverlay();
      showToast({ tone: 'info', text: `Deleted “${title || 'Spark'}”` });
    },
    [search, loadFavorites, refreshLibraryInfo, closeOverlay, showToast],
  );

  const onLibraryChanged = useCallback(
    (info: LibraryInfo) => {
      patch({ library: info });
      search.clear();
      void refresh();
    },
    [patch, search, refresh],
  );

  const togglePin = useCallback(async () => {
    try {
      patch({ pinned: await api.setPinned(!live.current.pinned) });
    } catch (err) {
      showToast({ tone: 'error', text: toApiError(err).message });
    }
  }, [patch, showToast]);

  const onQueryChange = (value: string) => {
    setQuery(value);
    // An emptied goal returns to the calm idle state (Favorites first).
    if (!value.trim() && search.state.status !== 'idle') search.clear();
  };

  // Global keyboard: Esc collapses, Ctrl+Enter copies, Ctrl+N adds, Ctrl+, settings.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented) return;
      const { overlay: ov, pinned: isPinned, query: q, search: s, welcome: firstRun } = live.current;
      const mod = e.ctrlKey || e.metaKey;
      if (e.key === 'Escape') {
        e.preventDefault();
        if (ov) {
          closeOverlay();
        } else if (!isPinned) {
          hide();
        } else if (q) {
          setQuery('');
          s.clear();
        } else {
          inputRef.current?.blur();
        }
        return;
      }
      if (ov || firstRun) return;
      if (mod && e.key === 'Enter') {
        e.preventDefault();
        const best = s.state.status === 'done' ? s.state.outcome.best : null;
        if (best && s.state.status === 'done' && s.state.outcome.query === q.trim()) void copy(best);
        else if (q.trim()) void s.run(q);
      } else if (mod && e.key.toLowerCase() === 'n') {
        e.preventDefault();
        openAdd();
      } else if (mod && e.key === ',') {
        e.preventDefault();
        setOverlay({ kind: 'settings' });
      } else if (mod && e.key.toLowerCase() === 'f') {
        e.preventDefault();
        focusGoal(true);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [closeOverlay, hide, copy, openAdd, focusGoal]);

  const hasResult =
    search.state.status === 'done' && search.state.outcome.best !== null && search.state.outcome.query === query.trim();
  const emptyLibrary = libraryReady && library?.sparkCount === 0;

  return (
    <div className="stage">
      <div ref={panelRef} className={`panel${entering ? ' is-entering' : ''}`}>
        <Header
          pinned={pinned}
          settingsOpen={overlay?.kind === 'settings'}
          onTogglePin={() => void togglePin()}
          onToggleSettings={() => (overlay?.kind === 'settings' ? closeOverlay() : setOverlay({ kind: 'settings' }))}
          onHide={hide}
        />

        <div className="body">
          <main ref={mainRef} className="main scroll">
            <div ref={contentRef}>
              <GoalInput
                value={query}
                onChange={onQueryChange}
                onSubmit={() => void search.run(query)}
                onCopyBest={() => {
                  if (search.state.status === 'done' && search.state.outcome.best) void copy(search.state.outcome.best);
                }}
                inputRef={inputRef}
                disabled={snapshot !== null && !libraryReady}
                hasResult={hasResult}
              />

              {hotkeyUnavailable && !hotkeyNoticeDismissed && (
                <HotkeyUnavailable
                  hotkey={snapshot.hotkey}
                  onChoose={() => setOverlay({ kind: 'settings' })}
                  onDismiss={() => setHotkeyNoticeDismissed(true)}
                />
              )}

              {snapshotError && !snapshot && (
                <div className="notice is-error" role="alert" style={{ marginTop: 14 }}>
                  <Icon name="alert" size={17} />
                  <div className="notice-body">{snapshotError}</div>
                </div>
              )}

              {library && !library.available ? (
                <div style={{ marginTop: 14 }}>
                  <LibraryUnavailable
                    library={library}
                    onChanged={onLibraryChanged}
                    onOpenSettings={() => setOverlay({ kind: 'settings' })}
                  />
                </div>
              ) : (
                <>
                  <ResultRegion
                    state={search.state}
                    pendingVisible={search.pendingVisible}
                    slow={search.slow}
                    ai={ai}
                    copiedId={copiedId}
                    onCopy={(s) => void copy(s)}
                    onToggleFavorite={(s) => void toggleFavorite(s)}
                    onEdit={(s) => setOverlay({ kind: 'edit', id: s.id })}
                    onDelete={(s) => void deleteSpark(s)}
                    onAdd={openAdd}
                  />
                  {emptyLibrary ? (
                    <EmptyLibrary onAdd={openAdd} />
                  ) : (
                    <Favorites
                      favorites={favorites}
                      copiedId={copiedId}
                      onCopy={(s) => void copy(s)}
                      onToggleFavorite={(s) => void toggleFavorite(s)}
                    />
                  )}
                </>
              )}
            </div>
          </main>

          <div className="dock">
            <button
              type="button"
              className="button is-fire-outline is-large is-block add-spark"
              onClick={openAdd}
              disabled={!libraryReady}
              title="Add New Spark (Ctrl+N)"
            >
              <Icon name="plus" size={18} strokeWidth={1.9} /> Add New Spark
            </button>
          </div>

          <ToastView toast={toast} onDismiss={dismissToast} />

          {welcome && (
            <Welcome hotkey={snapshot.hotkey} onHotkeyChange={(hotkey) => patch({ hotkey })} onDone={finishWelcome} />
          )}

          {overlay?.kind === 'settings' && snapshot && (
            <SettingsPanel
              snapshot={snapshot}
              onClose={closeOverlay}
              onHotkeyChange={(hotkey) => patch({ hotkey })}
              onLibraryChange={onLibraryChanged}
              onLaunchAtStartupChange={(launchAtStartup) => patch({ launchAtStartup })}
            />
          )}
          {(overlay?.kind === 'add' || overlay?.kind === 'edit') && (
            <SparkEditor
              key={overlay.kind === 'edit' ? `edit-${overlay.id}` : 'add'}
              mode={overlay}
              ai={ai}
              onClose={closeOverlay}
              onSaved={onSaved}
              onDeleted={onDeletedFromEditor}
            />
          )}
        </div>

        <Footer ai={snapshot?.ai ?? null} />
      </div>
    </div>
  );
}
