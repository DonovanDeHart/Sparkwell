import { useState } from 'react';
import { Icon } from '../components/Icon';
import { api, toApiError } from '../services/api';
import type { LibraryInfo } from '../services/types';

export function EmptyLibrary({ onAdd }: { onAdd: () => void }) {
  return (
    <section className="state-card favorites" aria-labelledby="empty-title">
      <h2 id="empty-title">Your library is empty</h2>
      <p>
        Paste any prompt, workflow, or instruction set you want to keep. Sparkwell stores it on this device and finds it
        when you describe your goal.
      </p>
      <div className="actions">
        <button type="button" className="button is-fire" onClick={onAdd}>
          <Icon name="plus" size={16} /> Add your first Spark
        </button>
      </div>
    </section>
  );
}

interface LibraryUnavailableProps {
  library: LibraryInfo;
  onChanged: (library: LibraryInfo) => void;
  onOpenSettings: () => void;
}

/** Shown when the library file can't be opened (moved drive, permissions,
 *  corruption). Nothing is created or deleted without an explicit choice. */
export function LibraryUnavailable({ library, onChanged, onOpenSettings }: LibraryUnavailableProps) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const run = async (action: () => Promise<LibraryInfo>) => {
    setBusy(true);
    setError(null);
    try {
      onChanged(await action());
    } catch (err) {
      setError(toApiError(err).message);
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="state-card is-error" role="alert" aria-labelledby="unavailable-title">
      <h2 id="unavailable-title">
        <Icon name="alert" size={16} style={{ display: 'inline', verticalAlign: '-3px', marginRight: 7, color: 'var(--danger)' }} />
        Your library isn't available
      </h2>
      <p>{library.error ?? 'Sparkwell could not open your Spark library.'}</p>
      <p className="state-path selectable">{library.file}</p>
      {error && error !== library.error && <p style={{ marginTop: 8, color: 'var(--danger)' }}>{error}</p>}
      <div className="actions">
        <button type="button" className="button is-ice" disabled={busy} onClick={() => void run(api.retryLibrary)}>
          <Icon name="refresh" size={15} /> Try again
        </button>
        <button type="button" className="button" disabled={busy} onClick={onOpenSettings}>
          Choose location…
        </button>
        {!library.isDefault && (
          <button type="button" className="button is-quiet" disabled={busy} onClick={() => void run(api.useDefaultLibrary)}>
            Use default location
          </button>
        )}
      </div>
    </section>
  );
}
