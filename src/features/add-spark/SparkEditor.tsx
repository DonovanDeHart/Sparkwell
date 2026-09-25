import { useCallback, useEffect, useRef, useState, type KeyboardEvent } from 'react';
import { Icon } from '../../components/Icon';
import { IconButton } from '../../components/IconButton';
import { Toggle } from '../../components/Toggle';
import { api, toApiError } from '../../services/api';
import { smartAddReady, type AiStatus, type SparkSummary } from '../../services/types';
import './editor.css';

const MAX_TAGS = 8;

export type EditorMode = { kind: 'add' } | { kind: 'edit'; id: number };

interface SparkEditorProps {
  mode: EditorMode;
  ai: AiStatus;
  onClose: () => void;
  onSaved: (spark: SparkSummary, created: boolean) => void;
  onDeleted: (id: number, title: string) => void;
}

interface Draft {
  body: string;
  title: string;
  summary: string;
  tags: string[];
  favorite: boolean;
}

const EMPTY: Draft = { body: '', title: '', summary: '', tags: [], favorite: false };

function addTags(existing: string[], raw: string): string[] {
  const next = [...existing];
  for (const piece of raw.split(',')) {
    const tag = piece.trim().replace(/^#/, '').replace(/\s+/g, ' ').slice(0, 32);
    if (!tag || next.some((t) => t.toLowerCase() === tag.toLowerCase())) continue;
    if (next.length >= MAX_TAGS) break;
    next.push(tag);
  }
  return next;
}

/** Add New Spark / Edit Spark, as an in-panel drawer. */
export function SparkEditor({ mode, ai, onClose, onSaved, onDeleted }: SparkEditorProps) {
  const isEdit = mode.kind === 'edit';
  const [draft, setDraft] = useState<Draft>(EMPTY);
  const [initial, setInitial] = useState<Draft>(EMPTY);
  const [tagText, setTagText] = useState('');
  const [loading, setLoading] = useState(isEdit);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [duplicateOf, setDuplicateOf] = useState<string | null>(null);
  const [confirmDiscard, setConfirmDiscard] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [drafting, setDrafting] = useState(false);
  const [draftNote, setDraftNote] = useState<string | null>(null);
  const draftRequest = useRef(0);
  const bodyRef = useRef<HTMLTextAreaElement>(null);
  const draftRef = useRef(draft);
  draftRef.current = draft;

  const aiReady = smartAddReady(ai);

  useEffect(() => {
    if (mode.kind !== 'edit') {
      bodyRef.current?.focus();
      return;
    }
    let cancelled = false;
    api
      .getSpark(mode.id)
      .then((spark) => {
        if (cancelled) return;
        const loaded = {
          body: spark.body,
          title: spark.title,
          summary: spark.summary,
          tags: spark.tags,
          favorite: spark.favorite,
        };
        setDraft(loaded);
        setInitial(loaded);
        setLoading(false);
        requestAnimationFrame(() => bodyRef.current?.focus());
      })
      .catch((err) => {
        if (cancelled) return;
        setError(toApiError(err).message);
        setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [mode]);

  const update = (patch: Partial<Draft>) => {
    setDraft((d) => ({ ...d, ...patch }));
    setConfirmDiscard(false);
    setDuplicateOf(null);
    setError(null);
  };

  const dirty =
    tagText.trim() !== '' ||
    draft.body !== initial.body ||
    draft.title !== initial.title ||
    draft.summary !== initial.summary ||
    draft.favorite !== initial.favorite ||
    draft.tags.join('\u0000') !== initial.tags.join('\u0000');

  /** Asks local intelligence for title/summary/tags. `onlyEmpty` (automatic
   *  runs) never overwrites anything the user has typed. */
  const runSmartAdd = useCallback(
    async (body: string, onlyEmpty: boolean) => {
      if (!body.trim()) return;
      const id = ++draftRequest.current;
      const before = draftRef.current;
      setDrafting(true);
      setDraftNote(null);
      try {
        const s = await api.suggestMetadata(body);
        if (draftRequest.current !== id) return;
        setDraft((d) => ({
          ...d,
          // Fill a field if it's empty, or (manual run) if the user hasn't
          // touched it since the request started.
          title: !d.title.trim() || (!onlyEmpty && d.title === before.title) ? s.title : d.title,
          summary: !d.summary.trim() || (!onlyEmpty && d.summary === before.summary) ? s.summary : d.summary,
          tags: d.tags.length === 0 || (!onlyEmpty && d.tags === before.tags) ? s.tags : d.tags,
        }));
        setDraftNote('Details drafted locally — edit anything before saving.');
      } catch (err) {
        if (draftRequest.current !== id) return;
        setDraftNote(toApiError(err).message);
      } finally {
        if (draftRequest.current === id) setDrafting(false);
      }
    },
    [],
  );

  const cancelSmartAdd = () => {
    draftRequest.current++;
    setDrafting(false);
    setDraftNote(null);
  };

  const save = async (allowDuplicate = false) => {
    if (saving || loading) return;
    const body = draft.body.trim();
    if (!body) {
      setError('Paste or type the Spark itself before saving.');
      bodyRef.current?.focus();
      return;
    }
    const tags = addTags(draft.tags, tagText);
    setSaving(true);
    setError(null);
    try {
      const input = {
        title: draft.title,
        summary: draft.summary,
        body: draft.body,
        tags,
        favorite: draft.favorite,
        allowDuplicate,
      };
      const saved = mode.kind === 'edit' ? await api.updateSpark(mode.id, input) : await api.createSpark(input);
      draftRequest.current++;
      onSaved(saved, mode.kind === 'add');
    } catch (err) {
      const e = toApiError(err);
      if (e.kind === 'duplicate') setDuplicateOf(e.existingTitle ?? 'another Spark');
      else setError(e.message);
      setSaving(false);
    }
  };

  const remove = async () => {
    if (mode.kind !== 'edit') return;
    if (!confirmDelete) {
      setConfirmDelete(true);
      return;
    }
    try {
      await api.deleteSpark(mode.id);
      onDeleted(mode.id, draft.title);
    } catch (err) {
      setError(toApiError(err).message);
      setConfirmDelete(false);
    }
  };

  const requestClose = () => {
    if (dirty && !confirmDiscard) {
      setConfirmDiscard(true);
      return;
    }
    draftRequest.current++;
    onClose();
  };

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      requestClose();
    } else if ((e.ctrlKey || e.metaKey) && (e.key === 'Enter' || e.key.toLowerCase() === 's')) {
      e.preventDefault();
      void save();
    }
  };

  const onTagKey = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Enter' || e.key === ',') {
      if (e.ctrlKey || e.metaKey) return;
      e.preventDefault();
      if (tagText.trim()) {
        update({ tags: addTags(draft.tags, tagText) });
        setTagText('');
      }
    } else if (e.key === 'Backspace' && !tagText && draft.tags.length) {
      update({ tags: draft.tags.slice(0, -1) });
    }
  };

  const title = isEdit ? 'Edit Spark' : 'Add New Spark';

  return (
    <div className="overlay editor" role="dialog" aria-modal="true" aria-label={title} onKeyDown={onKeyDown}>
      <div className="overlay-head">
        <IconButton icon="back" label="Back" onClick={requestClose} />
        <h2 className="overlay-title">{title}</h2>
        <IconButton icon="close" label="Close" onClick={requestClose} />
      </div>

      <div className="overlay-content scroll">
        {loading ? (
          <div className="pending-line">
            <span className="spinner" aria-hidden="true" /> Loading Spark…
          </div>
        ) : (
          <div className="editor-fields">
            <div className="field">
              <label className="field-label" htmlFor="spark-body">
                Spark
                <span className="field-hint">
                  {draft.body.length > 0 ? `${draft.body.length.toLocaleString()} characters` : 'Required'}
                </span>
              </label>
              <textarea
                id="spark-body"
                ref={bodyRef}
                className="textarea editor-body selectable"
                value={draft.body}
                spellCheck={false}
                placeholder="Paste or write the full Spark — the prompt, workflow, role, or instructions you want to reuse."
                onChange={(e) => update({ body: e.target.value })}
                onPaste={(e) => {
                  const pasted = e.clipboardData.getData('text');
                  const wasEmpty = !draftRef.current.body.trim();
                  if (aiReady && wasEmpty && pasted.trim() && !draftRef.current.title.trim() && !draftRef.current.summary.trim()) {
                    // Let the paste land, then draft details from it.
                    window.setTimeout(() => void runSmartAdd(pasted, true), 0);
                  }
                }}
              />
            </div>

            {aiReady && (
              <div className="smart-add" aria-live="polite">
                {drafting ? (
                  <>
                    <span className="spinner" aria-hidden="true" />
                    <span className="smart-add-text">Drafting title, summary & tags locally…</span>
                    <button type="button" className="button is-quiet smart-add-cancel" onClick={cancelSmartAdd}>
                      Cancel
                    </button>
                  </>
                ) : (
                  <>
                    <button
                      type="button"
                      className="button is-ice smart-add-button"
                      disabled={!draft.body.trim()}
                      onClick={() => void runSmartAdd(draft.body, false)}
                    >
                      <Icon name="sparkle" size={16} /> Auto-fill details
                    </button>
                    {draftNote && <span className="smart-add-text">{draftNote}</span>}
                  </>
                )}
              </div>
            )}

            <div className="field">
              <label className="field-label" htmlFor="spark-title">
                Title
                <span className="field-hint">Leave blank to use the first line</span>
              </label>
              <input
                id="spark-title"
                className="input"
                value={draft.title}
                maxLength={120}
                placeholder="Name it by what it does — e.g. MCP Server Architect"
                onChange={(e) => update({ title: e.target.value })}
              />
            </div>

            <div className="field">
              <label className="field-label" htmlFor="spark-summary">
                Summary
              </label>
              <textarea
                id="spark-summary"
                className="textarea"
                rows={2}
                maxLength={600}
                value={draft.summary}
                placeholder="What does this Spark help you accomplish?"
                onChange={(e) => update({ summary: e.target.value })}
              />
            </div>

            <div className="field">
              <label className="field-label" htmlFor="spark-tags">
                Tags
                <span className="field-hint">Enter or comma to add</span>
              </label>
              <div className="tag-input" onClick={() => document.getElementById('spark-tags')?.focus()}>
                {draft.tags.map((tag) => (
                  <span key={tag} className="chip tag-chip">
                    {tag}
                    <button
                      type="button"
                      className="tag-remove"
                      aria-label={`Remove tag ${tag}`}
                      onClick={(e) => {
                        e.stopPropagation();
                        update({ tags: draft.tags.filter((t) => t !== tag) });
                      }}
                    >
                      <Icon name="close" size={11} strokeWidth={2.2} />
                    </button>
                  </span>
                ))}
                <input
                  id="spark-tags"
                  className="tag-input-field"
                  value={tagText}
                  disabled={draft.tags.length >= MAX_TAGS}
                  placeholder={draft.tags.length ? '' : 'e.g. MCP, Architecture'}
                  onChange={(e) => setTagText(e.target.value)}
                  onKeyDown={onTagKey}
                  onBlur={() => {
                    if (tagText.trim()) {
                      update({ tags: addTags(draft.tags, tagText) });
                      setTagText('');
                    }
                  }}
                />
              </div>
            </div>

            <div className="favorite-field">
              <span className="favorite-label">
                <Icon name={draft.favorite ? 'starFilled' : 'star'} size={17} />
                Add to Favorites
              </span>
              <Toggle
                checked={draft.favorite}
                label="Add to Favorites"
                onChange={(favorite) => update({ favorite })}
              />
            </div>

            {duplicateOf && (
              <div className="notice is-warning" role="alert">
                <Icon name="alert" size={17} />
                <div className="notice-body">
                  <span>
                    This exact Spark is already in your library as <strong>“{duplicateOf}”</strong>.
                  </span>
                  <div className="notice-actions">
                    <button type="button" className="button" onClick={() => void save(true)}>
                      Save anyway
                    </button>
                    <button type="button" className="button is-quiet" onClick={() => setDuplicateOf(null)}>
                      Keep editing
                    </button>
                  </div>
                </div>
              </div>
            )}

            {error && (
              <div className="notice is-error" role="alert">
                <Icon name="alert" size={17} />
                <div className="notice-body">{error}</div>
              </div>
            )}

            {confirmDiscard && (
              <div className="notice is-warning" role="alert">
                <Icon name="alert" size={17} />
                <div className="notice-body">
                  <span>Discard your unsaved changes?</span>
                  <div className="notice-actions">
                    <button type="button" className="button is-danger" onClick={requestClose}>
                      Discard
                    </button>
                    <button type="button" className="button is-quiet" onClick={() => setConfirmDiscard(false)}>
                      Keep editing
                    </button>
                  </div>
                </div>
              </div>
            )}
          </div>
        )}
      </div>

      <div className="overlay-foot">
        {isEdit && (
          <button
            type="button"
            className={`button ${confirmDelete ? 'is-danger' : 'is-quiet'}`}
            onClick={() => void remove()}
            onBlur={() => setConfirmDelete(false)}
          >
            <Icon name="trash" size={15} />
            {confirmDelete ? 'Confirm delete' : 'Delete'}
          </button>
        )}
        <span style={{ flex: 1 }} />
        <button type="button" className="button is-quiet" onClick={requestClose}>
          Cancel
        </button>
        <button
          type="button"
          className="button is-fire"
          onClick={() => void save()}
          disabled={saving || loading}
          title="Save (Ctrl+Enter)"
        >
          {saving ? <span className="spinner" aria-hidden="true" /> : <Icon name="check" size={16} strokeWidth={2} />}
          {isEdit ? 'Save changes' : 'Save Spark'}
        </button>
      </div>
    </div>
  );
}
