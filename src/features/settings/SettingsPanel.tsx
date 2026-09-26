import { useState, type KeyboardEvent } from 'react';
import { Icon } from '../../components/Icon';
import { IconButton } from '../../components/IconButton';
import { Toggle } from '../../components/Toggle';
import { api, toApiError } from '../../services/api';
import type { AiStatus, AppSnapshot, HotkeyStatus, LibraryInfo } from '../../services/types';
import { HotkeyRecorder } from './HotkeyRecorder';
import { LibraryLocation } from './LibraryLocation';
import './settings.css';

interface SettingsPanelProps {
  snapshot: AppSnapshot;
  onClose: () => void;
  onHotkeyChange: (hotkey: HotkeyStatus) => void;
  onLibraryChange: (library: LibraryInfo) => void;
  onLaunchAtStartupChange: (enabled: boolean) => void;
}

function intelligenceText(ai: AiStatus): { title: string; detail: string | null; ready: boolean } {
  if (ai.state === 'checking') return { title: 'Checking for local intelligence…', detail: null, ready: false };
  if (ai.state === 'offline')
    return {
      title: 'Local intelligence offline · standard search active',
      detail: 'Optional: run Ollama on this computer for semantic search and Smart Add.',
      ready: false,
    };
  if (!ai.embedModel)
    return {
      title: 'Ollama running · standard search active',
      detail: 'Install an embedding model (for example: ollama pull nomic-embed-text) to enable semantic search.',
      ready: false,
    };
  return {
    title: ai.indexing
      ? `Local intelligence ready · indexing ${ai.indexed}/${ai.total}`
      : 'Local intelligence ready · semantic search active',
    detail: ai.chatModel
      ? `Auto-fill in Add New Spark drafts titles, summaries and tags with ${ai.chatModel}.`
      : `Auto-fill in Add New Spark needs a ${ai.chatModelsTooLarge ? 'smaller' : 'small'} local model (for example: ollama pull qwen2.5:3b).`,
    ready: true,
  };
}

/** Compact settings. Deliberately limited to the MVP set. */
export function SettingsPanel({
  snapshot,
  onClose,
  onHotkeyChange,
  onLibraryChange,
  onLaunchAtStartupChange,
}: SettingsPanelProps) {
  const [startupBusy, setStartupBusy] = useState(false);
  const [startupError, setStartupError] = useState<string | null>(null);
  const ai = intelligenceText(snapshot.ai);

  const setStartup = async (enabled: boolean) => {
    setStartupBusy(true);
    setStartupError(null);
    try {
      onLaunchAtStartupChange(await api.setLaunchAtStartup(enabled));
    } catch (err) {
      setStartupError(toApiError(err).message);
    } finally {
      setStartupBusy(false);
    }
  };

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key === 'Escape' && !e.defaultPrevented) {
      e.preventDefault();
      e.stopPropagation();
      onClose();
    }
  };

  return (
    <div className="overlay settings" role="dialog" aria-modal="true" aria-label="Settings" onKeyDown={onKeyDown}>
      <div className="overlay-head">
        <IconButton icon="back" label="Back" onClick={onClose} />
        <h2 className="overlay-title">Settings</h2>
        <IconButton icon="close" label="Close settings" onClick={onClose} />
      </div>

      <div className="overlay-content scroll">
        <section className="setting">
          <div className="setting-row">
            <div className="setting-text">
              <h3 className="setting-title" id="startup-label">
                Launch at Startup
              </h3>
              <p className="setting-desc">Start quietly in the tray when you sign in to Windows.</p>
            </div>
            <Toggle
              checked={snapshot.launchAtStartup}
              label="Launch at Startup"
              disabled={startupBusy}
              onChange={(v) => void setStartup(v)}
            />
          </div>
          {startupError && (
            <p className="setting-note is-error" role="alert">
              <Icon name="alert" size={14} /> {startupError}
            </p>
          )}
        </section>

        <section className="setting">
          <div className="setting-text">
            <h3 className="setting-title">Activation Hotkey</h3>
            <p className="setting-desc">Shows or hides Sparkwell from any app.</p>
          </div>
          <HotkeyRecorder hotkey={snapshot.hotkey} onChange={onHotkeyChange} />
        </section>

        <section className="setting">
          <div className="setting-text">
            <h3 className="setting-title">Library Location</h3>
            <p className="setting-desc">Your Sparks live in a single file on this device.</p>
          </div>
          <LibraryLocation library={snapshot.library} onChanged={onLibraryChange} />
        </section>

        <section className="setting">
          <div className="setting-row">
            <div className="setting-text">
              <h3 className="setting-title">Local Only</h3>
              <p className="setting-desc">
                Sparks stay on this device. No account, no cloud sync, no telemetry.
              </p>
            </div>
            <span className="status-pill" title="Local Only is always on">
              <Icon name="shield" size={14} /> On
            </span>
          </div>
          <div className={`intelligence${ai.ready ? ' is-ready' : ''}`} title={
            snapshot.ai.embedModel ? `Ollama · ${snapshot.ai.embedModel}${snapshot.ai.chatModel ? ` · ${snapshot.ai.chatModel}` : ''}` : undefined
          }>
            <span className="intelligence-dot" aria-hidden="true" />
            <div>
              <p className="intelligence-title">{ai.title}</p>
              {ai.detail && <p className="intelligence-detail">{ai.detail}</p>}
            </div>
          </div>
        </section>

        <section className="setting about">
          <div className="setting-text">
            <h3 className="setting-title">About</h3>
            <p className="setting-desc">Sparkwell {snapshot.version} · Your Sparks, always within reach.</p>
          </div>
          <div className="about-card">
            <h4>What is a Spark?</h4>
            <p>
              A Spark is a reusable expression of intent — a prompt, workflow, role definition, research protocol, or
              project launcher that gives AI trajectory, context, and shape. Describe what you're trying to accomplish;
              Sparkwell finds the right Spark so you never rebuild it from scratch.
            </p>
          </div>
          <button type="button" className="button is-quiet quit" onClick={() => void api.quitApp()}>
            Quit Sparkwell
          </button>
        </section>
      </div>
    </div>
  );
}
