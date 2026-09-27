import { useState } from 'react';
import { Icon } from '../../components/Icon';
import { api, toApiError } from '../../services/api';
import type { LibraryInfo, SwitchMode, TargetInfo } from '../../services/types';

interface LibraryLocationProps {
  library: LibraryInfo;
  onChanged: (library: LibraryInfo) => void;
}

type Message = { tone: 'success' | 'error'; text: string } | null;

const plural = (n: number) => `${n} Spark${n === 1 ? '' : 's'}`;

/** Shows where the library lives and changes it with an explicit, verified migration. */
export function LibraryLocation({ library, onChanged }: LibraryLocationProps) {
  const [target, setTarget] = useState<TargetInfo | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<Message>(null);

  const choose = async () => {
    setMessage(null);
    try {
      const picked = await api.chooseLibraryFolder();
      setTarget(picked);
    } catch (err) {
      setMessage({ tone: 'error', text: toApiError(err).message });
    }
  };

  const apply = async (mode: SwitchMode) => {
    if (!target) return;
    const previous = library.dir;
    setBusy(true);
    try {
      const info = await api.changeLibraryLocation(target.path, mode);
      onChanged(info);
      setTarget(null);
      setMessage({
        tone: 'success',
        text:
          mode === 'copy'
            ? `Library moved. The previous copy is still at ${previous} as a backup.`
            : mode === 'open'
              ? `Now using the library in ${info.dir}.`
              : `New library created in ${info.dir}.`,
      });
    } catch (err) {
      setMessage({ tone: 'error', text: `${toApiError(err).message} Your library was not changed.` });
    } finally {
      setBusy(false);
    }
  };

  const useDefault = async () => {
    setBusy(true);
    setMessage(null);
    try {
      onChanged(await api.useDefaultLibrary());
      setMessage({ tone: 'success', text: 'Using the default library location.' });
    } catch (err) {
      setMessage({ tone: 'error', text: toApiError(err).message });
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="library-location">
      <div className="setting-control-row">
        <div className="path selectable" title={library.file}>
          <Icon name="folder" size={15} />
          {/* LRM marks keep the start-truncated (rtl) path in natural order. */}
          <span className="path-text">{`\u200e${library.dir}\u200e`}</span>
        </div>
        <button type="button" className="button" onClick={() => void choose()} disabled={busy}>
          Change…
        </button>
      </div>
      <p className="setting-note">
        {library.available
          ? `${plural(library.sparkCount)} · ${library.isDefault ? 'default location' : 'custom location'}`
          : 'Library unavailable'}
      </p>

      {target && (
        <div className={`notice ${target.isCurrent ? '' : 'is-warning'}`} role="alertdialog" aria-label="Confirm library location">
          <Icon name={target.isCurrent ? 'folder' : 'alert'} size={17} />
          <div className="notice-body">
            <span className="path-inline selectable">{target.path}</span>
            {target.isCurrent ? (
              <>
                <span>That folder is already your library location.</span>
                <div className="notice-actions">
                  <button type="button" className="button" onClick={() => setTarget(null)}>
                    OK
                  </button>
                </div>
              </>
            ) : target.hasExistingLibrary ? (
              <>
                <span>
                  This folder already has a Sparkwell library
                  {target.existingSparkCount !== null ? ` with ${plural(target.existingSparkCount)}` : ''}. Switch to it?
                  Your current library stays where it is.
                </span>
                <div className="notice-actions">
                  <button type="button" className="button is-ice" disabled={busy} onClick={() => void apply('open')}>
                    Use that library
                  </button>
                  <button type="button" className="button is-quiet" onClick={() => setTarget(null)}>
                    Cancel
                  </button>
                </div>
              </>
            ) : library.available ? (
              <>
                <span>
                  Copy your {plural(library.sparkCount)} here and switch to it? Sparkwell verifies the copy before
                  switching. Your current library file is left untouched as a backup.
                </span>
                <div className="notice-actions">
                  <button type="button" className="button is-ice" disabled={busy} onClick={() => void apply('copy')}>
                    {busy ? <span className="spinner" aria-hidden="true" /> : null} Copy & switch
                  </button>
                  <button type="button" className="button is-quiet" onClick={() => setTarget(null)}>
                    Cancel
                  </button>
                </div>
              </>
            ) : (
              <>
                <span>Start a new library in this folder?</span>
                <div className="notice-actions">
                  <button type="button" className="button is-ice" disabled={busy} onClick={() => void apply('create')}>
                    Create library here
                  </button>
                  <button type="button" className="button is-quiet" onClick={() => setTarget(null)}>
                    Cancel
                  </button>
                </div>
              </>
            )}
          </div>
        </div>
      )}

      {!library.available && !library.isDefault && !target && (
        <button type="button" className="button is-quiet" onClick={() => void useDefault()} disabled={busy}>
          Use the default location instead
        </button>
      )}

      {message && (
        <p className={`setting-note ${message.tone === 'error' ? 'is-error' : 'is-success'}`} role="status">
          <Icon name={message.tone === 'error' ? 'alert' : 'check'} size={14} /> {message.text}
        </p>
      )}
    </div>
  );
}
