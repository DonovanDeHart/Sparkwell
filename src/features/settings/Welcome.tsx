import { useState } from 'react';
import { LogoMark } from '../../components/LogoMark';
import { api, toApiError } from '../../services/api';
import type { HotkeyStatus } from '../../services/types';
import { HotkeyRecorder } from './HotkeyRecorder';
import './settings.css';

interface WelcomeProps {
  hotkey: HotkeyStatus;
  onHotkeyChange: (hotkey: HotkeyStatus) => void;
  onDone: () => void;
}

/** First run: the user picks the activation shortcut (no single combination
 *  is free on every machine) or skips; the tray icon always opens Sparkwell. */
export function Welcome({ hotkey, onHotkeyChange, onDone }: WelcomeProps) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const finish = async () => {
    setBusy(true);
    setError(null);
    try {
      await api.finishOnboarding();
      onDone();
    } catch (err) {
      setError(toApiError(err).message);
      setBusy(false);
    }
  };

  return (
    <div className="overlay welcome" role="dialog" aria-modal="true" aria-labelledby="welcome-title" tabIndex={-1}>
      <div className="overlay-content scroll welcome-content">
        <LogoMark size={44} />
        <h2 id="welcome-title" className="welcome-title">
          Welcome to Sparkwell
        </h2>
        <p className="welcome-lead">Choose a keyboard shortcut that opens Sparkwell from any app.</p>
        <HotkeyRecorder hotkey={hotkey} onChange={onHotkeyChange} setLabel="Choose a shortcut" autoFocus />
        <p className="setting-note">
          Sparkwell also lives in the system tray: click its icon any time. You can change the shortcut later in
          Settings.
        </p>
        {error && (
          <p className="setting-note is-error" role="alert">
            {error}
          </p>
        )}
      </div>
      <div className="overlay-foot welcome-foot">
        <button type="button" className="button is-quiet" disabled={busy} onClick={() => void finish()}>
          Skip for now
        </button>
        <button
          type="button"
          className="button is-ice"
          disabled={busy || !hotkey.registered}
          onClick={() => void finish()}
        >
          Continue
        </button>
      </div>
    </div>
  );
}
