import { IconButton } from '../components/IconButton';
import { LogoMark } from '../components/LogoMark';

interface HeaderProps {
  pinned: boolean;
  settingsOpen: boolean;
  onTogglePin: () => void;
  onToggleSettings: () => void;
  onHide: () => void;
}

/** Quiet header: wordmark left, Pin / Settings / Hide right. Doubles as the
 *  window drag region. */
export function Header({ pinned, settingsOpen, onTogglePin, onToggleSettings, onHide }: HeaderProps) {
  return (
    <header className="header" data-tauri-drag-region>
      <div className="brand" data-tauri-drag-region>
        <LogoMark size={26} />
        <span className="wordmark" data-tauri-drag-region>
          <span className="wordmark-spark">Spark</span>
          <span className="wordmark-well">Well</span>
        </span>
      </div>
      <div className="header-actions">
        <IconButton
          icon={pinned ? 'pinFilled' : 'pin'}
          label={pinned ? 'Unpin (stop keeping Sparkwell on top)' : 'Pin (keep Sparkwell open and on top)'}
          active={pinned}
          aria-pressed={pinned}
          onClick={onTogglePin}
        />
        <IconButton
          icon="settings"
          label="Settings"
          active={settingsOpen}
          aria-pressed={settingsOpen}
          onClick={onToggleSettings}
        />
        <IconButton icon="collapse" label="Hide Sparkwell (Esc)" onClick={onHide} />
      </div>
    </header>
  );
}
