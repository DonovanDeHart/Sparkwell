import { useLayoutEffect, type RefObject } from 'react';
import { Icon } from '../../components/Icon';

interface GoalInputProps {
  value: string;
  onChange: (value: string) => void;
  onSubmit: () => void;
  inputRef: RefObject<HTMLTextAreaElement | null>;
  disabled?: boolean;
  /** The current text has already been searched and a Spark is showing. */
  hasResult: boolean;
}

const MAX_HEIGHT = 180;

/** The primary question. Natural language in, Enter to retrieve. */
export function GoalInput({ value, onChange, onSubmit, inputRef, disabled, hasResult }: GoalInputProps) {
  // Grow with the text up to a comfortable maximum. Measurements taken while
  // the window is hidden are unreliable, so re-measure whenever it resizes.
  useLayoutEffect(() => {
    const el = inputRef.current;
    if (!el) return;
    const fit = () => {
      if (!el.value) {
        el.style.height = '';
        return;
      }
      el.style.height = 'auto';
      el.style.height = `${Math.min(el.scrollHeight, MAX_HEIGHT)}px`;
    };
    fit();
    window.addEventListener('resize', fit);
    window.addEventListener('focus', fit);
    return () => {
      window.removeEventListener('resize', fit);
      window.removeEventListener('focus', fit);
    };
  }, [value, inputRef]);

  const canSubmit = value.trim().length > 0 && !disabled;

  return (
    <div className="goal-block">
      <label className="goal-label" htmlFor="goal-input">
        What are you trying to accomplish?
      </label>
      <div className="goal">
        <textarea
          id="goal-input"
          ref={inputRef}
          className="goal-input"
          value={value}
          rows={3}
          disabled={disabled}
          spellCheck={false}
          autoComplete="off"
          placeholder="e.g. I need AI to help me build an MCP server"
          aria-describedby="goal-hint"
          onChange={(e) => onChange(e.target.value)}
          onKeyDown={(e) => {
            if (e.key !== 'Enter' || e.nativeEvent.isComposing) return;
            // Ctrl+Enter bubbles to the app shell, which copies the Best Match.
            if (e.ctrlKey || e.metaKey) return;
            if (!e.shiftKey) {
              e.preventDefault();
              if (canSubmit) onSubmit();
            }
          }}
        />
        <button
          type="button"
          className="goal-submit"
          onClick={onSubmit}
          disabled={!canSubmit}
          aria-label="Find the best Spark"
          title="Find the best Spark (Enter)"
        >
          <Icon name="sparkle" size={19} strokeWidth={1.6} />
        </button>
      </div>
      <div className="goal-hint" id="goal-hint">
        {hasResult ? (
          <span>
            <kbd>Ctrl</kbd>+<kbd>Enter</kbd> copies the Best Match
          </span>
        ) : value.trim() ? (
          <span>
            <kbd>Enter</kbd> to find · <kbd>Shift</kbd>+<kbd>Enter</kbd> for a new line
          </span>
        ) : null}
      </div>
    </div>
  );
}
