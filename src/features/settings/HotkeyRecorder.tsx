import { useEffect, useRef, useState, type KeyboardEvent } from 'react';
import { Icon } from '../../components/Icon';
import { api, toApiError } from '../../services/api';
import { captureFromEvent, describeAccelerator, keycaps, toAccelerator, type Modifier } from '../../services/hotkeys';
import type { HotkeyStatus } from '../../services/types';

interface HotkeyRecorderProps {
  hotkey: HotkeyStatus;
  onChange: (hotkey: HotkeyStatus) => void;
}

type Message = { tone: 'success' | 'error'; text: string } | null;

function Keycaps({ accelerator }: { accelerator: string }) {
  return (
    <span className="keycaps" aria-label={describeAccelerator(accelerator)}>
      {keycaps(accelerator).map((k, i) => (
        <kbd key={`${k}-${i}`} className="keycap">
          {k}
        </kbd>
      ))}
    </span>
  );
}

/** Click Change -> press a combination -> validated and registered by the core. */
export function HotkeyRecorder({ hotkey, onChange }: HotkeyRecorderProps) {
  const [capturing, setCapturing] = useState(false);
  const [saving, setSaving] = useState(false);
  const [held, setHeld] = useState<Modifier[]>([]);
  const [message, setMessage] = useState<Message>(null);
  const boxRef = useRef<HTMLDivElement>(null);
  const capturingRef = useRef(false);

  const start = async () => {
    setMessage(null);
    setHeld([]);
    try {
      // Release the current shortcut so pressing it can be recorded.
      await api.beginHotkeyCapture();
    } catch {
      /* recording still works; worst case the old shortcut fires */
    }
    capturingRef.current = true;
    setCapturing(true);
    requestAnimationFrame(() => boxRef.current?.focus());
  };

  const stop = async () => {
    capturingRef.current = false;
    setCapturing(false);
    setHeld([]);
    try {
      onChange(await api.endHotkeyCapture());
    } catch {
      /* ignore */
    }
  };

  // Never leave the shortcut suspended if Settings closes mid-recording.
  useEffect(
    () => () => {
      if (capturingRef.current) void api.endHotkeyCapture().catch(() => undefined);
    },
    [],
  );

  const onKeyDown = async (e: KeyboardEvent<HTMLDivElement>) => {
    if (!capturing || saving) return;
    e.preventDefault();
    e.stopPropagation();
    const capture = captureFromEvent(e.nativeEvent);
    if (e.key === 'Escape' && capture.modifiers.length === 0) {
      void stop();
      return;
    }
    const accelerator = toAccelerator(capture);
    if (!accelerator) {
      setHeld(capture.modifiers);
      return;
    }
    setSaving(true);
    try {
      const status = await api.setHotkey(accelerator);
      onChange(status);
      capturingRef.current = false;
      setCapturing(false);
      setMessage({
        tone: 'success',
        text: `Saved. Press ${describeAccelerator(status.accelerator)} from any app to open Sparkwell.`,
      });
    } catch (err) {
      const apiErr = toApiError(err);
      setMessage({ tone: 'error', text: apiErr.message });
      // The core restored the previous shortcut; release it again so the user
      // can keep trying without triggering it.
      await api.beginHotkeyCapture().catch(() => undefined);
      boxRef.current?.focus();
    } finally {
      setSaving(false);
      setHeld([]);
    }
  };

  return (
    <div className="hotkey">
      <div className="setting-control-row">
        {capturing ? (
          <div
            ref={boxRef}
            className="hotkey-capture"
            tabIndex={0}
            role="textbox"
            aria-label="Press the new activation shortcut. Escape cancels."
            onKeyDown={(e) => void onKeyDown(e)}
            onKeyUp={(e) => setHeld(captureFromEvent(e.nativeEvent).modifiers)}
            onBlur={() => {
              if (!saving) void stop();
            }}
          >
            {held.length > 0 ? (
              <Keycaps accelerator={`${held.join('+')}+…`} />
            ) : (
              <span className="hotkey-prompt">
                <Icon name="keyboard" size={16} /> Press your shortcut…
              </span>
            )}
          </div>
        ) : (
          <div className="hotkey-current">
            <Keycaps accelerator={hotkey.accelerator} />
          </div>
        )}
        {capturing ? (
          <button type="button" className="button is-quiet" onMouseDown={(e) => e.preventDefault()} onClick={() => void stop()}>
            Cancel
          </button>
        ) : (
          <button type="button" className="button" onClick={() => void start()}>
            Change
          </button>
        )}
      </div>
      {capturing && !message && (
        <p className="setting-note">Use two modifiers with a letter, e.g. Ctrl+Alt+Space. Esc cancels.</p>
      )}
      {!capturing && !message && !hotkey.registered && hotkey.error && (
        <p className="setting-note is-error" role="alert">
          <Icon name="alert" size={14} /> {hotkey.error}
        </p>
      )}
      {message && (
        <p className={`setting-note ${message.tone === 'error' ? 'is-error' : 'is-success'}`} role="status">
          <Icon name={message.tone === 'error' ? 'alert' : 'check'} size={14} /> {message.text}
        </p>
      )}
    </div>
  );
}
