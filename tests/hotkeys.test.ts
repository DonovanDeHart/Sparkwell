import { describe, expect, it } from 'vitest';
import { captureFromEvent, describeAccelerator, keyFromCode, keycaps, toAccelerator } from '../src/services/hotkeys';

const ev = (code: string, mods: Partial<Record<'ctrlKey' | 'altKey' | 'shiftKey' | 'metaKey', boolean>> = {}) => ({
  code,
  key: '',
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  metaKey: false,
  ...mods,
});

describe('hotkey capture', () => {
  it('maps layout-independent codes to accelerator keys', () => {
    expect(keyFromCode('KeyK')).toBe('K');
    expect(keyFromCode('Digit7')).toBe('7');
    expect(keyFromCode('Space')).toBe('Space');
    expect(keyFromCode('F13')).toBe('F13');
    expect(keyFromCode('ControlLeft')).toBeNull();
    expect(keyFromCode('MetaRight')).toBeNull();
  });

  it('orders modifiers canonically', () => {
    const capture = captureFromEvent(ev('Space', { altKey: true, ctrlKey: true }));
    expect(toAccelerator(capture)).toBe('Ctrl+Alt+Space');
    expect(toAccelerator(captureFromEvent(ev('KeyJ', { metaKey: true, shiftKey: true })))).toBe('Shift+Super+J');
  });

  it('reports modifier-only presses as incomplete', () => {
    const capture = captureFromEvent(ev('ControlLeft', { ctrlKey: true }));
    expect(capture.modifiers).toEqual(['Ctrl']);
    expect(toAccelerator(capture)).toBeNull();
  });

  it('renders friendly keycaps', () => {
    expect(keycaps('Ctrl+Super+ArrowUp')).toEqual(['Ctrl', 'Win', '↑']);
    expect(keycaps('Alt+Shift+Numpad5')).toEqual(['Alt', 'Shift', 'Num 5']);
    expect(describeAccelerator('Ctrl+Alt+Period')).toBe('Ctrl+Alt+.');
  });
});
