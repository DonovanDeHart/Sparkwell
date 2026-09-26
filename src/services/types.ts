// Types mirroring the Rust command surface (src-tauri/src/commands.rs).

export interface SparkSummary {
  id: number;
  title: string;
  summary: string;
  tags: string[];
  favorite: boolean;
  usageCount: number;
}

export interface SparkDetail extends SparkSummary {
  body: string;
  sourceNote: string | null;
  createdAt: number;
  updatedAt: number;
  lastCopiedAt: number | null;
}

export interface SparkInput {
  title: string;
  summary: string;
  body: string;
  tags: string[];
  favorite: boolean;
  sourceNote?: string | null;
  allowDuplicate?: boolean;
}

export type SearchMode = 'semantic' | 'standard';
/** Why a search used standard retrieval instead of local intelligence. */
export type Fallback = 'offline' | 'noEmbeddingModel' | 'indexing' | 'timedOut' | 'failed';
export type Confidence = 'strong' | 'weak' | 'none';

export interface SearchOutcome {
  query: string;
  mode: SearchMode;
  /** Set for standard results, so the mode never changes silently. */
  fallback: Fallback | null;
  confidence: Confidence;
  best: SparkSummary | null;
  alternatives: SparkSummary[];
  partiallyIndexed: boolean;
}

export type AiState = 'checking' | 'online' | 'offline';

export interface AiStatus {
  state: AiState;
  embedModel: string | null;
  /** Small local chat model used by Auto-fill; null when unavailable. */
  chatModel: string | null;
  /** Local chat models exist but all are too large for quick drafting. */
  chatModelsTooLarge: boolean;
  indexed: number;
  total: number;
  indexing: boolean;
}

export interface HotkeyStatus {
  accelerator: string;
  registered: boolean;
  error: string | null;
}

export interface LibraryInfo {
  dir: string;
  file: string;
  isDefault: boolean;
  available: boolean;
  error: string | null;
  sparkCount: number;
}

export interface TargetInfo {
  path: string;
  hasExistingLibrary: boolean;
  existingSparkCount: number | null;
  isCurrent: boolean;
}

export interface AppSnapshot {
  version: string;
  platform: string;
  pinned: boolean;
  /** First-run welcome finished (shortcut chosen or skipped). */
  onboarded: boolean;
  /** Frosted glass is on; otherwise the panel is painted opaque. */
  glass: boolean;
  hotkey: HotkeyStatus;
  launchAtStartup: boolean;
  library: LibraryInfo;
  ai: AiStatus;
}

export interface CopyResult {
  id: number;
  title: string;
  characters: number;
}

export interface MetadataSuggestion {
  title: string;
  summary: string;
  tags: string[];
}

export type SwitchMode = 'copy' | 'open' | 'create';

export type ErrorKind =
  | 'validation'
  | 'notFound'
  | 'duplicate'
  | 'libraryUnavailable'
  | 'database'
  | 'hotkeyInvalid'
  | 'hotkeyConflict'
  | 'clipboard'
  | 'ai'
  | 'io'
  | 'internal';

export const semanticReady = (ai: AiStatus) => ai.state === 'online' && ai.embedModel !== null;
export const smartAddReady = (ai: AiStatus) => ai.state === 'online' && ai.chatModel !== null;
