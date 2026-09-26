import { Icon } from '../../components/Icon';
import { semanticReady, type AiStatus, type Fallback, type SearchOutcome, type SparkSummary } from '../../services/types';
import { SparkRow } from '../sparks/SparkRow';
import { BestMatchCard } from './BestMatchCard';
import type { SearchState } from '../../app/useSearch';

interface ResultRegionProps {
  state: SearchState;
  pendingVisible: boolean;
  /** The search is taking long enough that local intelligence is probably loading. */
  slow: boolean;
  ai: AiStatus;
  copiedId: number | null;
  onCopy: (spark: SparkSummary) => void;
  onToggleFavorite: (spark: SparkSummary) => void;
  onEdit: (spark: SparkSummary) => void;
  onDelete: (spark: SparkSummary) => void;
  onAdd: () => void;
}

const FALLBACK_TEXT: Record<Fallback, string> = {
  offline: 'Standard search · local intelligence offline',
  noEmbeddingModel: 'Standard search · no local embedding model installed',
  indexing: 'Standard search · local intelligence is still indexing',
  timedOut: "Standard search · local intelligence didn't answer in time",
  failed: 'Standard search · local intelligence unavailable',
};

/** Every result says which retrieval produced it; the mode never changes silently. */
function ModeLine({ outcome }: { outcome: SearchOutcome }) {
  if (outcome.mode === 'semantic') {
    return (
      <p className="result-mode is-semantic">
        <Icon name="sparkle" size={13} />
        Matched by local intelligence{outcome.partiallyIndexed ? ' · still indexing some Sparks' : ''}
      </p>
    );
  }
  return <p className="result-mode">{FALLBACK_TEXT[outcome.fallback ?? 'offline']}</p>;
}

function Pending({ waking }: { waking: boolean }) {
  return (
    <div className="match is-pending" aria-busy="true">
      <p className="pending-line">
        <span className="spinner" aria-hidden="true" />
        {waking ? 'Waking up local intelligence… the first search can take a few seconds' : 'Finding your Spark…'}
      </p>
      <div className="skeleton is-title" />
      <div className="skeleton is-line" />
      <div className="skeleton is-line" style={{ width: '82%' }} />
      <div className="skeleton is-button" />
    </div>
  );
}

function NoMatch({
  outcome,
  copiedId,
  onCopy,
  onToggleFavorite,
  onAdd,
}: {
  outcome: SearchOutcome;
  copiedId: number | null;
  onCopy: (spark: SparkSummary) => void;
  onToggleFavorite: (spark: SparkSummary) => void;
  onAdd: () => void;
}) {
  const hasCandidates = outcome.alternatives.length > 0;
  return (
    <div className="no-match" role="status">
      <div className="no-match-head">
        <Icon name="search" size={17} />
        <div>
          <h2 className="no-match-title">No strong match</h2>
          <p className="no-match-text">
            {hasCandidates
              ? "Nothing fits that goal confidently. These are the closest Sparks in your library."
              : "Nothing in your library fits that goal yet."}
          </p>
        </div>
      </div>
      {hasCandidates && (
        <>
          <p className="no-match-caption">Closest Sparks</p>
          <ul className="no-match-list spark-rows">
            {outcome.alternatives.map((spark) => (
              <SparkRow
                key={spark.id}
                spark={spark}
                copied={copiedId === spark.id}
                onCopy={onCopy}
                onToggleFavorite={onToggleFavorite}
              />
            ))}
          </ul>
        </>
      )}
      <button type="button" className="button is-quiet no-match-add" onClick={onAdd}>
        <Icon name="plus" size={16} /> Save a new Spark for this goal
      </button>
    </div>
  );
}

/** Everything between the goal input and Favorites. Keeps the shell stable:
 *  searching, results, and misses all render in this one region. */
export function ResultRegion(props: ResultRegionProps) {
  const { state, pendingVisible, slow, ai, copiedId, onCopy, onToggleFavorite, onEdit, onDelete, onAdd } = props;

  if (state.status === 'idle') return null;

  if (state.status === 'error') {
    return (
      <section className="result-region" aria-live="polite">
        <div className="notice is-error" role="alert">
          <Icon name="alert" size={17} />
          <div className="notice-body">{state.message}</div>
        </div>
      </section>
    );
  }

  const outcome = state.status === 'done' ? state.outcome : state.previous;
  const showPending = state.status === 'searching' && (pendingVisible || !outcome);

  return (
    <section className="result-region" aria-live="polite" aria-label="Best Match">
      {showPending ? (
        pendingVisible ? <Pending waking={slow && semanticReady(ai)} /> : null
      ) : outcome ? (
        <div style={{ opacity: state.status === 'searching' ? 0.6 : 1, transition: 'opacity 120ms' }}>
          <ModeLine outcome={outcome} />
          {outcome.best ? (
            <BestMatchCard
              key={outcome.best.id}
              spark={outcome.best}
              copied={copiedId === outcome.best.id}
              onCopy={onCopy}
              onToggleFavorite={onToggleFavorite}
              onEdit={onEdit}
              onDelete={onDelete}
            />
          ) : (
            <NoMatch
              outcome={outcome}
              copiedId={copiedId}
              onCopy={onCopy}
              onToggleFavorite={onToggleFavorite}
              onAdd={onAdd}
            />
          )}
        </div>
      ) : null}
    </section>
  );
}
