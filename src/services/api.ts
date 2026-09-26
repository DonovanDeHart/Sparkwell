// The only way the UI talks to the Sparkwell core. Every call is a typed
// Tauri command; errors arrive as `{ kind, message }` and become ApiError.
//
// Outside the desktop shell (plain `npm run dev` in a browser, and tests) an
// in-memory mock backend stands in so the UI can be developed and verified.
// The mock is only reachable in development builds.

import { invoke as tauriInvoke } from '@tauri-apps/api/core';
import { listen as tauriListen, type UnlistenFn } from '@tauri-apps/api/event';
import type {
  AiStatus,
  AppSnapshot,
  CopyResult,
  ErrorKind,
  HotkeyStatus,
  LibraryInfo,
  MetadataSuggestion,
  SearchOutcome,
  SparkDetail,
  SparkInput,
  SparkSummary,
  SwitchMode,
  TargetInfo,
} from './types';

export class ApiError extends Error {
  readonly kind: ErrorKind;
  readonly existingId?: number;
  readonly existingTitle?: string;

  constructor(kind: ErrorKind, message: string, extra?: { existingId?: number; existingTitle?: string }) {
    super(message);
    this.name = 'ApiError';
    this.kind = kind;
    this.existingId = extra?.existingId;
    this.existingTitle = extra?.existingTitle;
  }
}

export function toApiError(err: unknown): ApiError {
  if (err instanceof ApiError) return err;
  if (err && typeof err === 'object' && 'kind' in err && 'message' in err) {
    const e = err as { kind: ErrorKind; message: string; existingId?: number; existingTitle?: string };
    return new ApiError(e.kind, e.message, e);
  }
  const message = err instanceof Error ? err.message : typeof err === 'string' ? err : 'Something went wrong.';
  return new ApiError('internal', message);
}

export const isDesktop = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

interface Backend {
  invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T>;
  listen<T>(event: string, handler: (payload: T) => void): Promise<UnlistenFn>;
}

let mockBackend: Promise<Backend> | null = null;

function backend(): Promise<Backend> {
  if (isDesktop) {
    return Promise.resolve({
      invoke: (cmd, args) => tauriInvoke(cmd, args),
      listen: (event, handler) => tauriListen(event, (e) => handler(e.payload as never)),
    });
  }
  if (import.meta.env.DEV) {
    mockBackend ??= import('./mockBackend').then((m) => m.createMockBackend());
    return mockBackend;
  }
  return Promise.reject(new ApiError('internal', 'Sparkwell must run inside the desktop app.'));
}

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    const b = await backend();
    return await b.invoke<T>(cmd, args);
  } catch (err) {
    throw toApiError(err);
  }
}

export async function listen<T>(event: string, handler: (payload: T) => void): Promise<UnlistenFn> {
  const b = await backend();
  return b.listen<T>(event, handler);
}

export const EVENTS = {
  aiStatus: 'sparkwell://ai-status',
  libraryChanged: 'sparkwell://library-changed',
  shown: 'sparkwell://shown',
  hidden: 'sparkwell://hidden',
} as const;

export const api = {
  getAppSnapshot: () => call<AppSnapshot>('get_app_snapshot'),
  listFavorites: () => call<SparkSummary[]>('list_favorites'),
  getSpark: (id: number) => call<SparkDetail>('get_spark', { id }),
  createSpark: (input: SparkInput) => call<SparkSummary>('create_spark', { input }),
  updateSpark: (id: number, input: SparkInput) => call<SparkSummary>('update_spark', { id, input }),
  deleteSpark: (id: number) => call<void>('delete_spark', { id }),
  setFavorite: (id: number, favorite: boolean) => call<SparkSummary>('set_favorite', { id, favorite }),
  copySpark: (id: number) => call<CopyResult>('copy_spark', { id }),
  searchSparks: (query: string) => call<SearchOutcome>('search_sparks', { query }),
  suggestMetadata: (body: string) => call<MetadataSuggestion>('suggest_metadata', { body }),
  getAiStatus: () => call<AiStatus>('get_ai_status'),
  setPinned: (pinned: boolean) => call<boolean>('set_pinned', { pinned }),
  hidePanel: () => call<void>('hide_panel'),
  quitApp: () => call<void>('quit_app'),
  setHotkey: (accelerator: string) => call<HotkeyStatus>('set_hotkey', { accelerator }),
  finishOnboarding: () => call<boolean>('finish_onboarding'),
  beginHotkeyCapture: () => call<void>('begin_hotkey_capture'),
  endHotkeyCapture: () => call<HotkeyStatus>('end_hotkey_capture'),
  setLaunchAtStartup: (enabled: boolean) => call<boolean>('set_launch_at_startup', { enabled }),
  chooseLibraryFolder: () => call<TargetInfo | null>('choose_library_folder'),
  changeLibraryLocation: (path: string, mode: SwitchMode) =>
    call<LibraryInfo>('change_library_location', { path, mode }),
  useDefaultLibrary: () => call<LibraryInfo>('use_default_library'),
  retryLibrary: () => call<LibraryInfo>('retry_library'),
  getLibraryInfo: () => call<LibraryInfo>('get_library_info'),
};

export type Api = typeof api;
