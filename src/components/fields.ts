/** Sparkwell's text fields are never form data: no browser autofill,
 *  autocomplete suggestions or auto-capitalisation. (WebView2 general
 *  autofill is also switched off for the whole window in tauri.conf.json.) */
export const NO_AUTOFILL = {
  autoComplete: 'off',
  autoCorrect: 'off',
  autoCapitalize: 'off',
} as const;
