import { Icon } from '../components/Icon';
import type { AiStatus } from '../services/types';

/** Subtle status strip: background indexing (when it happens) and the
 *  reassuring Local Only posture. */
export function Footer({ ai }: { ai: AiStatus | null }) {
  const indexing = ai?.state === 'online' && ai.indexing;
  return (
    <footer className="footer">
      <span className="footer-status" aria-live="polite">
        {indexing && (
          <>
            <span className="pulse-dot" aria-hidden="true" />
            Indexing Sparks… {ai.indexed}/{ai.total}
          </>
        )}
      </span>
      <span className="footer-local" title="Your Sparks never leave this device">
        <Icon name="localOnly" size={14} strokeWidth={1.6} />
        Local Only Mode
      </span>
    </footer>
  );
}
