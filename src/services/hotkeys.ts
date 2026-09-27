// Keyboard helpers for the activation-hotkey recorder. The Rust core is the
// authority on validity; this module only turns key events into accelerator
// strings and renders them as keycaps.

export type Modifier = 'Ctrl' | 'Alt' | 'Shift' | 'Super';

const MODIFIER_ORDER: Modifier[] = ['Ctrl', 'Alt', 'Shift', 'Super'];

const MODIFIER_CODES = new Set([
  'ControlLeft',
  'ControlRight',
  'AltLeft',
  'AltRight',
  'ShiftLeft',
  'ShiftRight',
  'MetaLeft',
  'MetaRight',
  'OSLeft',
  'OSRight',
]);

export interface KeyCapture {
  modifiers: Modifier[];
  /** Main key in accelerator syntax, or null while only modifiers are held. */
  key: string | null;
}

type KeyLike = Pick<KeyboardEvent, 'code' | 'key' | 'ctrlKey' | 'altKey' | 'shiftKey' | 'metaKey'>;

/** Maps KeyboardEvent.code (layout independent) to accelerator key names. */
export function keyFromCode(code: string): string | null {
  if (!code || MODIFIER_CODES.has(code)) return null;
  const letter = /^Key([A-Z])$/.exec(code);
  if (letter) return letter[1]!;
  const digit = /^Digit([0-9])$/.exec(code);
  if (digit) return digit[1]!;
  return code; // Space, F5, ArrowUp, Backquote, Numpad1, ...
}

export function captureFromEvent(e: KeyLike): KeyCapture {
  const modifiers = MODIFIER_ORDER.filter(
    (m) => (m === 'Ctrl' && e.ctrlKey) || (m === 'Alt' && e.altKey) || (m === 'Shift' && e.shiftKey) || (m === 'Super' && e.metaKey),
  );
  return { modifiers, key: keyFromCode(e.code) };
}

export function toAccelerator(capture: KeyCapture): string | null {
  if (!capture.key) return null;
  return [...capture.modifiers, capture.key].join('+');
}

const KEY_LABELS: Record<string, string> = {
  Super: 'Win',
  ArrowUp: '↑',
  ArrowDown: '↓',
  ArrowLeft: '←',
  ArrowRight: '→',
  Backquote: '`',
  Minus: '-',
  Equal: '=',
  BracketLeft: '[',
  BracketRight: ']',
  Backslash: '\\',
  Semicolon: ';',
  Quote: "'",
  Comma: ',',
  Period: '.',
  Slash: '/',
  PageUp: 'Page Up',
  PageDown: 'Page Down',
  NumpadAdd: 'Num +',
  NumpadSubtract: 'Num -',
  NumpadMultiply: 'Num *',
  NumpadDivide: 'Num /',
  NumpadDecimal: 'Num .',
  NumpadEnter: 'Num Enter',
};

/** Splits an accelerator into human-readable keycap labels. */
export function keycaps(accelerator: string): string[] {
  return accelerator
    .split('+')
    .filter(Boolean)
    .map((part) => KEY_LABELS[part] ?? part.replace(/^Numpad(\d)$/, 'Num $1'));
}

export function describeAccelerator(accelerator: string): string {
  return keycaps(accelerator).join('+');
}
