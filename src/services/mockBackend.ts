// In-memory stand-in for the Rust core, used only by `npm run dev` in a plain
// browser and by the automated UI tests. It mirrors the command contract
// (including error kinds) closely enough to exercise every UI state; the real
// behaviour lives in src-tauri and is tested there.

import type {
  AiStatus,
  AppSnapshot,
  Fallback,
  HotkeyStatus,
  LibraryInfo,
  SearchOutcome,
  SparkDetail,
  SparkInput,
  SparkSummary,
  TargetInfo,
} from './types';

type Handler = (payload: unknown) => void;

interface MockSpark extends SparkDetail {
  favoritedAt: number | null;
}

const STARTERS: Array<Pick<SparkDetail, 'title' | 'summary' | 'tags' | 'favorite' | 'body'>> = [
  {
    title: 'Codex Architecture Expert',
    summary:
      'Turns an AI coding agent into a disciplined software architect that inspects the codebase, plans in phases, and ships minimal, verified changes.',
    tags: ['Coding', 'Architecture', 'Agents'],
    favorite: true,
    body: 'You are a senior software architect working inside my repository as an autonomous coding agent.\n\nThe task:\n[Describe the feature]',
  },
  {
    title: 'AI Agent System Designer',
    summary:
      'Designs a production-grade AI agent system: roles, tools, memory, orchestration, guardrails, evaluation, and failure handling.',
    tags: ['Agents', 'Architecture', 'Systems'],
    favorite: true,
    body: 'Act as a principal engineer who designs reliable AI agent systems.\n\nGoal:\n[Describe what the agent should accomplish]',
  },
  {
    title: 'Deep Research Framework',
    summary:
      'Runs a rigorous research protocol: scoping questions, source triangulation, evidence grading, and a decision-ready synthesis.',
    tags: ['Research', 'Analysis'],
    favorite: true,
    body: 'You are a meticulous research analyst. Investigate the topic below using this protocol.\n\nTopic:\n[Enter the research question]',
  },
  {
    title: 'Prompt Engineering Master',
    summary:
      'Diagnoses and rewrites a prompt into a clear, testable instruction system with explicit goals, constraints, and output format.',
    tags: ['Prompting', 'Writing'],
    favorite: true,
    body: 'You are an expert prompt engineer. Improve the prompt I provide.\n\nPrompt to improve:\n[Paste the prompt here]',
  },
  {
    title: 'MCP Server Architect',
    summary:
      'Design and build production-ready MCP servers with scalable architecture, security, tools, resources, prompts, and best practices.',
    tags: ['MCP', 'Architecture', 'Python', 'Best Practices'],
    favorite: false,
    body: 'You are an expert in the Model Context Protocol (MCP) and production backend engineering. Help me design and build an MCP server.\n\nWhat the server should do:\n[Describe the server]',
  },
  {
    title: 'YouTube Script Architect',
    summary:
      'Plans and writes a high-retention YouTube video script with a strong hook, clear structure, pacing notes, and a title/thumbnail concept.',
    tags: ['YouTube', 'Content', 'Writing'],
    favorite: true,
    body: 'You are a YouTube strategist and scriptwriter known for high-retention videos.\n\nTopic:\n[Describe the video]',
  },
  {
    title: 'Root Cause Detective',
    summary:
      'Troubleshoots a bug or system failure methodically: gathers clues, ranks hypotheses, designs tests, and confirms the true root cause.',
    tags: ['Debugging', 'Troubleshooting'],
    favorite: false,
    body: 'Act as a senior troubleshooting engineer. Help me find the root cause of the problem below.\n\nThe problem:\n[Describe the symptoms]',
  },
  {
    title: 'Project Launch Planner',
    summary:
      'Converts a vague project idea into an executable plan with scope, phases, milestones, risks, and the first concrete next actions.',
    tags: ['Planning', 'Strategy'],
    favorite: false,
    body: 'You are a pragmatic technical program lead. Turn my project idea into an execution plan.\n\nMy project idea:\n[Describe the project]',
  },
];

const STOPWORDS = new Set(
  (
    'a able about accomplish ai also am an and any anything are as assist at be been being but by can chatgpt claude could d did do does doing done for from gemini get got help helping helps how i if im in into is it its just like ll llm m make making may me might mine must my need needed needs of on or our please re really s shall should so some something spark sparks t than that the then these thing things this those to try trying us use using ve very want wanted wants was way we were what when where which who why will with would you your'
  ).split(' '),
);

const tokenize = (s: string) =>
  s
    .toLowerCase()
    .normalize('NFD')
    .replace(/[\u0300-\u036f]/g, '')
    .split(/[^\p{L}\p{N}]+/u)
    .filter(Boolean);

function stem(t: string): string {
  if (t.length <= 3) return t;
  for (const [suf, rep] of [
    ['ies', 'y'],
    ['ied', 'y'],
    ['ing', ''],
    ['ers', ''],
    ['ed', ''],
    ['es', ''],
    ['er', ''],
    ['s', ''],
  ] as const) {
    if (t.endsWith(suf)) {
      const base = t.slice(0, -suf.length);
      if (base.length >= 3 && !(suf === 's' && base.endsWith('s'))) return base + rep;
    }
  }
  return t;
}

const matches = (a: string, b: string) => {
  if (a === b) return true;
  const [s, l] = a.length <= b.length ? [a, b] : [b, a];
  return s.length >= 4 && l.startsWith(s);
};

function queryTerms(q: string): string[] {
  const raw = tokenize(q);
  let picked = raw.filter((t) => !STOPWORDS.has(t)).map(stem);
  if (picked.length === 0) picked = raw.filter((t) => t.length > 1).map(stem);
  return [...new Set(picked)];
}

function lexical(terms: string[], s: MockSpark): number {
  if (terms.length === 0) return 0;
  const f = (text: string) => tokenize(text).map(stem);
  const title = f(s.title);
  const tags = s.tags.flatMap(f);
  const summary = f(s.summary);
  const body = f(s.body);
  const any = (t: string, list: string[]) => list.some((x) => matches(t, x));
  const covered = terms.reduce((acc, t) => {
    if (any(t, title)) return acc + 1;
    if (any(t, tags)) return acc + 0.85;
    if (any(t, summary)) return acc + 0.55;
    if (any(t, body)) return acc + 0.25;
    return acc;
  }, 0);
  const titleContent = tokenize(s.title)
    .filter((t) => !STOPWORDS.has(t))
    .map(stem);
  const recall = titleContent.length
    ? titleContent.filter((t) => any(t, terms)).length / titleContent.length
    : 0;
  return 0.8 * (covered / terms.length) + 0.2 * recall;
}

function validateHotkey(accel: string): string | { error: string } {
  const parts = accel.split('+').map((p) => p.trim());
  if (!accel.trim() || parts.some((p) => !p)) return { error: 'Press a key combination.' };
  const mods = new Set<string>();
  let key: string | null = null;
  for (const p of parts) {
    const u = p.toUpperCase();
    if (u === 'CTRL' || u === 'CONTROL') mods.add('Ctrl');
    else if (u === 'ALT') mods.add('Alt');
    else if (u === 'SHIFT') mods.add('Shift');
    else if (u === 'SUPER' || u === 'WIN' || u === 'META') mods.add('Super');
    else if (key) return { error: 'Use only one key with your modifiers.' };
    else key = p.replace(/^Key(?=[A-Z]$)/, '').replace(/^Digit(?=\d$)/, '');
  }
  if (!key) return { error: 'Add a letter, number, Space or function key to go with the modifier keys.' };
  if (['Escape', 'Tab', 'CapsLock', 'Delete', 'Backspace'].includes(key))
    return { error: `${key} can't be used in the activation shortcut.` };
  const typing = /^[A-Z0-9]$/.test(key) || ['Space', 'Enter', 'Period', 'Comma', 'Slash'].includes(key);
  if (typing && mods.size < 2)
    return {
      error: `Hold two modifier keys with ${key} (for example Ctrl and Shift) so the shortcut doesn't take over typing in other apps.`,
    };
  if (!typing && !/^F(1[3-9]|2[0-4])$/.test(key) && mods.size < 1)
    return { error: `Add Ctrl, Alt, Shift, or Win to ${key}.` };
  const order = ['Ctrl', 'Alt', 'Shift', 'Super'].filter((m) => mods.has(m));
  return [...order, key].join('+');
}

export interface MockControl {
  setAi(partial: Partial<AiStatus>): void;
  failNext(command: string, error: { kind: string; message: string }): void;
  delay(command: string, ms: number): void;
  setLibraryAvailable(available: boolean, error?: string): void;
  /** Startup state of the activation shortcut (e.g. taken by another app). */
  setHotkeyStatus(status: HotkeyStatus): void;
  /** Accelerators "owned by another app". */
  takenHotkeys: Set<string>;
  /** Forces standard search with this reason (null: decide from AI status). */
  searchFallback: Fallback | null;
  lastClipboard: string | null;
  calls: Array<{ cmd: string; args?: Record<string, unknown> }>;
  /** Raises a core event (e.g. the panel being shown by the hotkey). */
  emit(event: string, payload: unknown): void;
  /** `firstRun`: no config yet, so no shortcut and the welcome is pending. */
  reset(options?: { empty?: boolean; firstRun?: boolean }): void;
}

export function createMockBackend() {
  const listeners = new Map<string, Set<Handler>>();
  let sparks: MockSpark[] = [];
  let nextId = 1;
  const failures = new Map<string, { kind: string; message: string }>();
  const delays = new Map<string, number>();
  let pinned = false;
  let launchAtStartup = false;
  let onboarded = true;
  let hotkey: HotkeyStatus = { accelerator: 'Ctrl+Shift+Space', registered: true, error: null };
  let library: Omit<LibraryInfo, 'sparkCount'> = {
    dir: 'C:\\Users\\you\\AppData\\Local\\Sparkwell\\Library',
    file: 'C:\\Users\\you\\AppData\\Local\\Sparkwell\\Library\\sparkwell.db',
    isDefault: true,
    available: true,
    error: null,
  };
  let ai: AiStatus = { state: 'offline', embedModel: null, chatModel: null, chatModelsTooLarge: false, indexed: 0, total: 0, indexing: false };

  const emit = (event: string, payload: unknown) => listeners.get(event)?.forEach((h) => h(payload));
  const summary = (s: MockSpark): SparkSummary => ({
    id: s.id,
    title: s.title,
    summary: s.summary,
    tags: [...s.tags],
    favorite: s.favorite,
    usageCount: s.usageCount,
  });
  const libInfo = (): LibraryInfo => ({ ...library, sparkCount: library.available ? sparks.length : 0 });
  const find = (id: number) => {
    const s = sparks.find((x) => x.id === id);
    if (!s) throw { kind: 'notFound', message: 'That Spark no longer exists.' };
    return s;
  };
  const requireLibrary = () => {
    if (!library.available)
      throw { kind: 'libraryUnavailable', message: library.error ?? 'The library is not available.' };
  };
  const normBody = (b: string) => b.trim().split(/\s+/).join(' ');

  function seed() {
    sparks = [];
    nextId = 1;
    const now = Date.now();
    STARTERS.forEach((s, i) => {
      sparks.push({
        ...s,
        tags: [...s.tags],
        id: nextId++,
        usageCount: 0,
        sourceNote: 'Sparkwell starter Spark',
        createdAt: now,
        updatedAt: now,
        lastCopiedAt: null,
        favoritedAt: s.favorite ? now + i : null,
      });
    });
  }

  function clean(input: SparkInput) {
    const body = input.body.trim();
    if (!body) throw { kind: 'validation', message: 'Paste or type the Spark itself before saving.' };
    const title = input.title.trim().replace(/\s+/g, ' ');
    if (title.length > 120) throw { kind: 'validation', message: 'Keep the title under 120 characters.' };
    const firstLine = body.split('\n').find((l) => l.trim())?.replace(/^[#*>\-\s]+/, '').trim() ?? 'Untitled Spark';
    const tags: string[] = [];
    for (const t of input.tags.flatMap((x) => x.split(','))) {
      const tag = t.trim().replace(/^#/, '');
      if (tag && !tags.some((x) => x.toLowerCase() === tag.toLowerCase()) && tags.length < 8) tags.push(tag);
    }
    return {
      body,
      title: title || firstLine.slice(0, 60),
      summary: input.summary.trim() || body.replace(/\s+/g, ' ').slice(0, 180),
      tags,
    };
  }

  function dupCheck(body: string, excludeId?: number) {
    const hit = sparks.find((s) => s.id !== excludeId && normBody(s.body) === normBody(body));
    if (hit)
      throw {
        kind: 'duplicate',
        message: `A Spark with this exact content already exists: "${hit.title}".`,
        existingId: hit.id,
        existingTitle: hit.title,
      };
  }

  const handlers: Record<string, (args: Record<string, unknown>) => unknown> = {
    get_app_snapshot: (): AppSnapshot => ({
      version: '0.1.0',
      platform: 'windows',
      pinned,
      onboarded,
      hotkey,
      launchAtStartup,
      library: libInfo(),
      ai: { ...ai, total: sparks.length },
    }),
    list_favorites: () => {
      requireLibrary();
      return sparks
        .filter((s) => s.favorite)
        .sort((a, b) => (a.favoritedAt ?? 0) - (b.favoritedAt ?? 0) || a.id - b.id)
        .map(summary);
    },
    get_spark: ({ id }) => {
      requireLibrary();
      const s = find(id as number);
      return { ...summary(s), body: s.body, sourceNote: s.sourceNote, createdAt: s.createdAt, updatedAt: s.updatedAt, lastCopiedAt: s.lastCopiedAt };
    },
    create_spark: ({ input }) => {
      requireLibrary();
      const i = input as SparkInput;
      const c = clean(i);
      if (!i.allowDuplicate) dupCheck(c.body);
      const now = Date.now();
      const s: MockSpark = {
        id: nextId++,
        ...c,
        favorite: i.favorite,
        usageCount: 0,
        sourceNote: i.sourceNote ?? null,
        createdAt: now,
        updatedAt: now,
        lastCopiedAt: null,
        favoritedAt: i.favorite ? now : null,
      };
      sparks.push(s);
      return summary(s);
    },
    update_spark: ({ id, input }) => {
      requireLibrary();
      const s = find(id as number);
      const i = input as SparkInput;
      const c = clean(i);
      if (!i.allowDuplicate) dupCheck(c.body, s.id);
      Object.assign(s, c, {
        favorite: i.favorite,
        favoritedAt: i.favorite ? (s.favoritedAt ?? Date.now()) : null,
        updatedAt: Date.now(),
      });
      return summary(s);
    },
    delete_spark: ({ id }) => {
      requireLibrary();
      find(id as number);
      sparks = sparks.filter((s) => s.id !== id);
      return null;
    },
    set_favorite: ({ id, favorite }) => {
      requireLibrary();
      const s = find(id as number);
      s.favorite = favorite as boolean;
      s.favoritedAt = s.favorite ? (s.favoritedAt ?? Date.now()) : null;
      return summary(s);
    },
    copy_spark: ({ id }) => {
      requireLibrary();
      const s = find(id as number);
      control.lastClipboard = s.body;
      s.usageCount += 1;
      s.lastCopiedAt = Date.now();
      return { id: s.id, title: s.title, characters: s.body.length };
    },
    search_sparks: ({ query }): SearchOutcome => {
      requireLibrary();
      const q = String(query).trim();
      const terms = queryTerms(q);
      if (terms.length === 0) throw { kind: 'validation', message: "Describe what you're trying to accomplish." };
      const ranked = sparks
        .map((s) => ({ s, score: lexical(terms, s) + (s.favorite ? 0.015 : 0) }))
        .filter((r) => r.score > 0)
        .sort((a, b) => b.score - a.score || a.s.id - b.s.id);
      const top = ranked[0];
      const semantic = control.searchFallback === null && ai.state === 'online' && ai.embedModel !== null;
      const fallback: Fallback | null = semantic
        ? null
        : (control.searchFallback ?? (ai.state !== 'online' ? 'offline' : ai.embedModel ? 'indexing' : 'noEmbeddingModel'));
      if (top && top.score >= 0.42) {
        return {
          query: q,
          mode: semantic ? 'semantic' : 'standard',
          fallback,
          confidence: 'strong',
          best: summary(top.s),
          alternatives: [],
          partiallyIndexed: false,
        };
      }
      const alts = ranked.filter((r) => r.score >= 0.1).slice(0, 3).map((r) => summary(r.s));
      return {
        query: q,
        mode: semantic ? 'semantic' : 'standard',
        fallback,
        confidence: alts.length ? 'weak' : 'none',
        best: null,
        alternatives: alts,
        partiallyIndexed: false,
      };
    },
    suggest_metadata: ({ body }) => {
      if (!(ai.state === 'online' && ai.chatModel))
        throw { kind: 'ai', message: 'Local intelligence is offline. Add the details yourself.' };
      const text = String(body);
      const words = tokenize(text).filter((w) => !STOPWORDS.has(w) && w.length > 3);
      const title = words
        .slice(0, 3)
        .map((w) => w[0]!.toUpperCase() + w.slice(1))
        .join(' ');
      return { title: title || 'New Spark', summary: text.replace(/\s+/g, ' ').slice(0, 120), tags: words.slice(0, 3) };
    },
    get_ai_status: () => ({ ...ai, total: sparks.length }),
    set_pinned: ({ pinned: p }) => {
      pinned = p as boolean;
      return pinned;
    },
    hide_panel: () => {
      emit('sparkwell://hidden', null);
      return null;
    },
    quit_app: () => null,
    set_hotkey: ({ accelerator }) => {
      const v = validateHotkey(String(accelerator));
      if (typeof v !== 'string') throw { kind: 'hotkeyInvalid', message: v.error };
      if (control.takenHotkeys.has(v))
        throw {
          kind: 'hotkeyConflict',
          message: `${v.replace('Super', 'Win')} is already in use by another app or by Windows. Try a different combination.`,
        };
      hotkey = { accelerator: v, registered: true, error: null };
      return hotkey;
    },
    finish_onboarding: () => {
      onboarded = true;
      return true;
    },
    begin_hotkey_capture: () => null,
    end_hotkey_capture: () => hotkey,
    set_launch_at_startup: ({ enabled }) => {
      launchAtStartup = enabled as boolean;
      return launchAtStartup;
    },
    choose_library_folder: (): TargetInfo => ({
      path: 'D:\\Sync\\Sparks',
      hasExistingLibrary: false,
      existingSparkCount: null,
      isCurrent: false,
    }),
    change_library_location: ({ path }) => {
      library = {
        dir: String(path),
        file: `${String(path)}\\sparkwell.db`,
        isDefault: false,
        available: true,
        error: null,
      };
      emit('sparkwell://library-changed', libInfo());
      return libInfo();
    },
    use_default_library: () => {
      library = {
        dir: 'C:\\Users\\you\\AppData\\Local\\Sparkwell\\Library',
        file: 'C:\\Users\\you\\AppData\\Local\\Sparkwell\\Library\\sparkwell.db',
        isDefault: true,
        available: true,
        error: null,
      };
      emit('sparkwell://library-changed', libInfo());
      return libInfo();
    },
    retry_library: () => {
      if (!library.available) throw { kind: 'libraryUnavailable', message: library.error ?? 'Still unavailable.' };
      return libInfo();
    },
    get_library_info: () => libInfo(),
  };

  const control: MockControl = {
    setAi(partial) {
      ai = { ...ai, ...partial };
      emit('sparkwell://ai-status', { ...ai, total: sparks.length });
    },
    failNext(command, error) {
      failures.set(command, error);
    },
    delay(command, ms) {
      delays.set(command, ms);
    },
    setLibraryAvailable(available, error) {
      library = { ...library, available, error: available ? null : (error ?? 'The library could not be opened.') };
    },
    setHotkeyStatus(status) {
      hotkey = { ...status };
    },
    takenHotkeys: new Set(['Ctrl+Alt+K']),
    searchFallback: null,
    lastClipboard: null,
    calls: [],
    emit: (event, payload) => emit(event, payload),
    reset(options) {
      seed();
      if (options?.empty) sparks = [];
      pinned = false;
      launchAtStartup = false;
      onboarded = !options?.firstRun;
      hotkey = options?.firstRun
        ? { accelerator: '', registered: false, error: null }
        : { accelerator: 'Ctrl+Shift+Space', registered: true, error: null };
      ai = { state: 'offline', embedModel: null, chatModel: null, chatModelsTooLarge: false, indexed: 0, total: 0, indexing: false };
      failures.clear();
      delays.clear();
      control.searchFallback = null;
      control.lastClipboard = null;
      control.calls = [];
      this.setLibraryAvailable(true);
    },
  };

  seed();
  (globalThis as { __sparkwellMock?: MockControl }).__sparkwellMock = control;

  return {
    control,
    async invoke<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
      control.calls.push({ cmd, args });
      const wait = delays.get(cmd);
      if (wait) await new Promise((r) => setTimeout(r, wait));
      const failure = failures.get(cmd);
      if (failure) {
        failures.delete(cmd);
        throw failure;
      }
      const handler = handlers[cmd];
      if (!handler) throw { kind: 'internal', message: `Unknown command ${cmd}` };
      return structuredClone(handler(args)) as T;
    },
    async listen<T>(event: string, handler: (payload: T) => void) {
      const set = listeners.get(event) ?? new Set<Handler>();
      set.add(handler as Handler);
      listeners.set(event, set);
      return () => set.delete(handler as Handler);
    },
    emit,
  };
}
