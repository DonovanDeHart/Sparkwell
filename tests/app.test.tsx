// End-to-end UI behaviour against the in-memory backend (src/services/mockBackend.ts),
// which mirrors the Rust command contract.

import { act, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { App } from '../src/app/App';
import { api } from '../src/services/api';
import type { MockControl } from '../src/services/mockBackend';

let mock: MockControl;

beforeEach(async () => {
  await api.getAiStatus(); // instantiates the mock backend
  mock = (globalThis as { __sparkwellMock?: MockControl }).__sparkwellMock!;
  mock.reset();
});

async function renderApp() {
  const user = userEvent.setup();
  render(<App />);
  await screen.findByText('Codex Architecture Expert');
  return user;
}

async function searchFor(user: ReturnType<typeof userEvent.setup>, text: string) {
  const input = screen.getByLabelText('What are you trying to accomplish?');
  await user.clear(input);
  await user.type(input, `${text}{Enter}`);
}

const calls = (cmd: string) => mock.calls.filter((c) => c.cmd === cmd);

describe('Sparkwell sidebar', () => {
  it('opens on the goal question with Favorites ready and focus in the input', async () => {
    await renderApp();
    const input = screen.getByLabelText('What are you trying to accomplish?');
    await waitFor(() => expect(input).toHaveFocus());
    const favorites = screen.getByRole('region', { name: /^Favorites/ });
    expect(within(favorites).getAllByRole('listitem')).toHaveLength(5);
    expect(screen.getByRole('button', { name: /Add New Spark/ })).toBeEnabled();
    expect(screen.getByText('Local Only Mode')).toBeInTheDocument();
    // MVP exclusions: no View/Fork/model selectors/collections.
    expect(screen.queryByText(/Collections|Fork|View Details|Send to AI/i)).not.toBeInTheDocument();
  });

  it('finds the Best Match by intent and copies the complete Spark body', async () => {
    const user = await renderApp();
    await searchFor(user, 'I need AI to help me build an MCP server.');
    const card = await screen.findByRole('article');
    expect(within(card).getByText('Best Match')).toBeInTheDocument();
    expect(within(card).getByText('MCP Server Architect')).toBeInTheDocument();
    // Standard search always says so, and why.
    expect(screen.getByText('Standard search · local intelligence offline')).toBeInTheDocument();

    await user.click(within(card).getByRole('button', { name: /Copy Spark/ }));
    await within(card).findByText('Copied');
    expect(mock.lastClipboard).toContain('Model Context Protocol');
    expect(mock.lastClipboard).toContain('[Describe the server]');
    // Unpinned: the panel collapses back into its bay after copying.
    await waitFor(() => expect(calls('hide_panel')).toHaveLength(1), { timeout: 2000 });
  });

  it('stays open after copying when pinned', async () => {
    const user = await renderApp();
    await user.click(screen.getByRole('button', { name: /^Pin/ }));
    await waitFor(() => expect(screen.getByRole('button', { name: /^Unpin/ })).toHaveAttribute('aria-pressed', 'true'));
    await user.click(screen.getByRole('button', { name: 'Copy Deep Research Framework' }));
    await screen.findByText(/Copied “Deep Research Framework”/);
    await new Promise((r) => setTimeout(r, 900));
    expect(calls('hide_panel')).toHaveLength(0);
  });

  it('copies a Favorite with one click', async () => {
    const user = await renderApp();
    await user.click(screen.getByRole('button', { name: 'Copy Codex Architecture Expert' }));
    await screen.findByText(/Copied “Codex Architecture Expert”/);
    expect(mock.lastClipboard).toContain('senior software architect');
  });

  it('Ctrl+Enter copies the Best Match from the keyboard', async () => {
    const user = await renderApp();
    await searchFor(user, 'youtube video script');
    await screen.findByRole('article');
    await user.keyboard('{Control>}{Enter}{/Control}');
    await screen.findByText(/Copied “YouTube Script Architect”/);
    expect(mock.lastClipboard).toContain('YouTube strategist');
  });

  it('Enter again copies the Best Match, and a changed goal searches again', async () => {
    const user = await renderApp();
    await searchFor(user, 'I need AI to help me build an MCP server.');
    await screen.findByRole('article');
    expect(screen.getByText(/again copies the Best Match/)).toBeInTheDocument();
    expect(screen.queryByText(/Ctrl/, { selector: '#goal-hint *' })).not.toBeInTheDocument();

    const input = screen.getByLabelText('What are you trying to accomplish?');
    await user.type(input, ' Quickly');
    await user.keyboard('{Enter}');
    await waitFor(() => expect(calls('search_sparks')).toHaveLength(2));
    expect(calls('copy_spark')).toHaveLength(0);

    await screen.findByText(/again copies the Best Match/);
    await user.keyboard('{Enter}');
    await waitFor(() => expect(calls('copy_spark')).toHaveLength(1));
    expect(mock.lastClipboard).toContain('Model Context Protocol');
    expect(calls('search_sparks')).toHaveLength(2);
  });

  it('is honest when nothing matches and offers to add a Spark', async () => {
    const user = await renderApp();
    await searchFor(user, 'bake sourdough bread');
    expect(await screen.findByText('No strong match')).toBeInTheDocument();
    expect(screen.queryByRole('article')).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: /Save a new Spark for this goal/ }));
    expect(await screen.findByRole('dialog', { name: 'Add New Spark' })).toBeInTheDocument();
  });

  it('shows closest candidates for a weak match', async () => {
    const user = await renderApp();
    await searchFor(user, 'write polished prose');
    expect(await screen.findByText('Closest Sparks')).toBeInTheDocument();
    const miss = screen.getByText('No strong match').closest('.no-match') as HTMLElement;
    expect(within(miss).getByRole('button', { name: 'Copy Prompt Engineering Master' })).toBeInTheDocument();
    expect(screen.queryByRole('article')).not.toBeInTheDocument();
  });

  it('labels semantic results when local intelligence is ready', async () => {
    const user = await renderApp();
    act(() => mock.setAi({ state: 'online', embedModel: 'nomic-embed-text', chatModel: null }));
    await searchFor(user, 'build an mcp server');
    expect(await screen.findByText(/Matched by local intelligence/)).toBeInTheDocument();
    expect(screen.queryByText(/Standard search/)).not.toBeInTheDocument();
  });

  it('never switches to standard search silently', async () => {
    const user = await renderApp();
    act(() => mock.setAi({ state: 'online', embedModel: 'qwen3-embedding:0.6b', chatModel: null }));
    mock.searchFallback = 'timedOut';
    await searchFor(user, 'build an mcp server');
    expect(await screen.findByText("Standard search · local intelligence didn't answer in time")).toBeInTheDocument();
    expect(screen.queryByText(/Matched by local intelligence/)).not.toBeInTheDocument();

    mock.searchFallback = 'indexing';
    await searchFor(user, 'youtube script');
    expect(await screen.findByText('Standard search · local intelligence is still indexing')).toBeInTheDocument();
  });

  it('shows a calm waking state while local intelligence loads its model', async () => {
    const user = await renderApp();
    act(() => mock.setAi({ state: 'online', embedModel: 'qwen3-embedding:0.6b', chatModel: null }));
    mock.delay('search_sparks', 1700);
    await searchFor(user, 'build an mcp server');
    expect(await screen.findByText('Finding your Spark…')).toBeInTheDocument();
    expect(await screen.findByText(/Waking up local intelligence/, {}, { timeout: 1500 })).toBeInTheDocument();
    // The rest of the panel stays usable meanwhile.
    expect(screen.getByRole('button', { name: 'Copy Codex Architecture Expert' })).toBeEnabled();
    expect(await screen.findByRole('article', {}, { timeout: 2000 })).toBeInTheDocument();
  });

  it('clearing the goal returns to the idle state', async () => {
    const user = await renderApp();
    await searchFor(user, 'mcp server');
    await screen.findByRole('article');
    await user.clear(screen.getByLabelText('What are you trying to accomplish?'));
    await waitFor(() => expect(screen.queryByRole('article')).not.toBeInTheDocument());
  });

  it('reports clipboard failures without claiming success', async () => {
    const user = await renderApp();
    mock.failNext('copy_spark', { kind: 'clipboard', message: "Couldn't copy to the clipboard: busy" });
    await user.click(screen.getByRole('button', { name: 'Copy Codex Architecture Expert' }));
    expect(await screen.findByText(/Couldn't copy to the clipboard/)).toBeInTheDocument();
    expect(screen.queryByText(/^Copied/)).not.toBeInTheDocument();
    expect(calls('hide_panel')).toHaveLength(0);
  });

  it('Escape collapses an unpinned panel', async () => {
    const user = await renderApp();
    await user.keyboard('{Escape}');
    expect(calls('hide_panel')).toHaveLength(1);
  });

  it('Esc closes Settings and returns focus to the goal', async () => {
    const user = await renderApp();
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    const dialog = await screen.findByRole('dialog', { name: 'Settings' });
    await waitFor(() => expect(dialog).toHaveFocus());
    await user.keyboard('{Escape}');
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Settings' })).not.toBeInTheDocument());
    expect(calls('hide_panel')).toHaveLength(0);
    await waitFor(() => expect(screen.getByLabelText('What are you trying to accomplish?')).toHaveFocus());
  });

  it('showing the panel again puts focus back inside an open overlay', async () => {
    const user = await renderApp();
    await user.click(screen.getByRole('button', { name: /Add New Spark/ }));
    const dialog = await screen.findByRole('dialog', { name: 'Add New Spark' });
    const body = within(dialog).getByLabelText(/^Spark/);
    await waitFor(() => expect(body).toHaveFocus());
    // Hidden and shown again by the hotkey while the editor was open.
    act(() => {
      body.blur();
      mock.emit('sparkwell://shown', false);
    });
    await waitFor(() => expect(body).toHaveFocus());
    await user.keyboard('{Escape}');
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    await waitFor(() => expect(screen.getByLabelText('What are you trying to accomplish?')).toHaveFocus());
  });

  it('gives overlays the full panel height', async () => {
    vi.stubGlobal(
      'ResizeObserver',
      class {
        observe() {}
        disconnect() {}
      },
    );
    try {
      const user = await renderApp();
      await user.click(screen.getByRole('button', { name: /Add New Spark/ }));
      await waitFor(() => expect(calls('set_panel_height').at(-1)?.args).toEqual({ height: 900 }));
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it('removing a favorite can be undone', async () => {
    const user = await renderApp();
    await user.click(screen.getByRole('button', { name: 'Remove Deep Research Framework from Favorites' }));
    await screen.findByText(/Removed “Deep Research Framework” from Favorites/);
    expect(screen.queryByRole('button', { name: 'Copy Deep Research Framework' })).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Undo' }));
    expect(await screen.findByRole('button', { name: 'Copy Deep Research Framework' })).toBeInTheDocument();
  });

  it('keeps Undo available for ten seconds', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
      render(<App />);
      await screen.findByText('Codex Architecture Expert');
      await user.click(screen.getByRole('button', { name: 'Remove Deep Research Framework from Favorites' }));
      await screen.findByText(/Removed “Deep Research Framework” from Favorites/);
      act(() => vi.advanceTimersByTime(8_000));
      expect(screen.getByRole('button', { name: 'Undo' })).toBeInTheDocument();
      act(() => vi.advanceTimersByTime(2_500));
      expect(screen.queryByRole('button', { name: 'Undo' })).not.toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });
});

describe('Add New Spark', () => {
  it('saves manually without local intelligence and makes it retrievable', async () => {
    const user = await renderApp();
    await user.click(screen.getByRole('button', { name: /Add New Spark/ }));
    const dialog = await screen.findByRole('dialog', { name: 'Add New Spark' });
    // Auto-fill is always offered, and says why it can't run.
    expect(within(dialog).getByRole('button', { name: /Auto-fill details/ })).toBeDisabled();
    expect(within(dialog).getByText(/Auto-fill uses local intelligence \(Ollama\), which is offline/)).toBeInTheDocument();
    await user.type(within(dialog).getByLabelText(/^Spark/), 'You are a Kubernetes expert. Diagnose my cluster.');
    await user.type(within(dialog).getByLabelText(/^Title/), 'Kubernetes Doctor');
    await user.type(within(dialog).getByLabelText(/^Tags/), 'DevOps{Enter}k8s,');
    await user.click(within(dialog).getByRole('switch', { name: 'Add to Favorites' }));
    await user.click(within(dialog).getByRole('button', { name: /Save Spark/ }));

    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(await screen.findByRole('button', { name: 'Copy Kubernetes Doctor' })).toBeInTheDocument();
    const created = calls('create_spark')[0]!.args!.input as { tags: string[]; favorite: boolean };
    expect(created.tags).toEqual(['DevOps', 'k8s']);
    expect(created.favorite).toBe(true);

    await searchFor(user, 'diagnose my kubernetes cluster');
    const card = await screen.findByRole('article');
    expect(within(card).getByText('Kubernetes Doctor')).toBeInTheDocument();
  });

  it('never offers browser autofill in any field', async () => {
    const user = await renderApp();
    await user.click(screen.getByRole('button', { name: /Add New Spark/ }));
    await screen.findByRole('dialog', { name: 'Add New Spark' });
    const fields = document.querySelectorAll('input:not([type="checkbox"]), textarea');
    expect(fields.length).toBeGreaterThanOrEqual(5);
    fields.forEach((field) => {
      expect(field, field.id).toHaveAttribute('autocomplete', 'off');
      expect(field, field.id).toHaveAttribute('autocapitalize', 'off');
    });
  });

  it('turns tags into chips as commas are typed, without moving the layout on blur', async () => {
    const user = await renderApp();
    await user.click(screen.getByRole('button', { name: /Add New Spark/ }));
    const dialog = await screen.findByRole('dialog', { name: 'Add New Spark' });
    await user.type(within(dialog).getByLabelText(/^Spark/), 'Plan a product launch.');
    const tags = within(dialog).getByLabelText(/^Tags/);
    await user.type(tags, 'A, B, C');
    expect(within(dialog).getByRole('button', { name: 'Remove tag A' })).toBeInTheDocument();
    expect(within(dialog).getByRole('button', { name: 'Remove tag B' })).toBeInTheDocument();
    expect(tags).toHaveValue('C');
    // Leaving the field doesn't reflow it: the next click lands where aimed.
    await user.click(within(dialog).getByRole('switch', { name: 'Add to Favorites' }));
    expect(tags).toHaveValue('C');
    expect(within(dialog).queryByRole('button', { name: 'Remove tag C' })).not.toBeInTheDocument();
    expect(within(dialog).getByRole('switch', { name: 'Add to Favorites' })).toHaveAttribute('aria-checked', 'true');

    await user.type(tags, ',');
    await user.paste('Launch, Go-to-market');
    expect(within(dialog).getByRole('button', { name: 'Remove tag Launch' })).toBeInTheDocument();
    expect(tags).toHaveValue('Go-to-market');

    // Pending text is saved as a tag too.
    await user.click(within(dialog).getByRole('button', { name: /Save Spark/ }));
    await waitFor(() => expect(calls('create_spark')).toHaveLength(1));
    expect(calls('create_spark')[0]!.args!.input).toMatchObject({
      tags: ['A', 'B', 'C', 'Launch', 'Go-to-market'],
      favorite: true,
    });
  });

  it('requires the Spark body', async () => {
    const user = await renderApp();
    await user.click(screen.getByRole('button', { name: /Add New Spark/ }));
    const dialog = await screen.findByRole('dialog');
    await user.click(within(dialog).getByRole('button', { name: /Save Spark/ }));
    expect(within(dialog).getByText('Paste or type the Spark itself before saving.')).toBeInTheDocument();
    expect(calls('create_spark')).toHaveLength(0);
  });

  it('warns about exact duplicates and can save anyway', async () => {
    const user = await renderApp();
    const body = 'Duplicate me please, this is a Spark body.';
    await api.createSpark({ title: 'Original', summary: '', body, tags: [], favorite: false });
    await user.click(screen.getByRole('button', { name: /Add New Spark/ }));
    const dialog = await screen.findByRole('dialog');
    await user.type(within(dialog).getByLabelText(/^Spark/), body);
    await user.click(within(dialog).getByRole('button', { name: /Save Spark/ }));
    expect(await within(dialog).findByText(/already in your library/)).toBeInTheDocument();
    await user.click(within(dialog).getByRole('button', { name: 'Save anyway' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(calls('create_spark').at(-1)!.args!.input).toMatchObject({ allowDuplicate: true });
  });

  it('asks before discarding unsaved work', async () => {
    const user = await renderApp();
    await user.click(screen.getByRole('button', { name: /Add New Spark/ }));
    const dialog = await screen.findByRole('dialog');
    await user.type(within(dialog).getByLabelText(/^Spark/), 'half-written');
    await user.keyboard('{Escape}');
    expect(within(dialog).getByText('Discard your unsaved changes?')).toBeInTheDocument();
    await user.click(within(dialog).getByRole('button', { name: 'Discard' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(calls('hide_panel')).toHaveLength(0);
  });

  it('drafts editable metadata only when asked, never on paste', async () => {
    const user = await renderApp();
    act(() => mock.setAi({ state: 'online', embedModel: 'nomic-embed-text', chatModel: 'qwen2.5:3b' }));
    await user.click(screen.getByRole('button', { name: /Add New Spark/ }));
    const dialog = await screen.findByRole('dialog');
    const body = within(dialog).getByLabelText(/^Spark/);
    await waitFor(() => expect(body).toHaveFocus());
    await user.paste('Analyze quarterly revenue spreadsheets carefully');
    expect(body).toHaveValue('Analyze quarterly revenue spreadsheets carefully');
    await new Promise((r) => setTimeout(r, 50));
    expect(calls('suggest_metadata')).toHaveLength(0);
    expect(within(dialog).getByLabelText(/^Title/)).toHaveValue('');

    await user.click(within(dialog).getByRole('button', { name: /Auto-fill details/ }));
    await waitFor(() => expect(within(dialog).getByLabelText(/^Title/)).not.toHaveValue(''));
    expect(within(dialog).getByText(/Details drafted locally/)).toBeInTheDocument();
    // Nothing is saved until the user presses Save.
    expect(calls('create_spark')).toHaveLength(0);
  });

  it('explains that Auto-fill needs a small model when only large ones are installed', async () => {
    const user = await renderApp();
    act(() =>
      mock.setAi({ state: 'online', embedModel: 'qwen3-embedding:0.6b', chatModel: null, chatModelsTooLarge: true }),
    );
    await user.click(screen.getByRole('button', { name: /Add New Spark/ }));
    const dialog = await screen.findByRole('dialog');
    await user.type(within(dialog).getByLabelText(/^Spark/), 'Plan a product launch');
    expect(within(dialog).getByRole('button', { name: /Auto-fill details/ })).toBeDisabled();
    expect(within(dialog).getByText(/too large for quick drafting/)).toBeInTheDocument();
    expect(within(dialog).getByText('ollama pull qwen2.5:3b')).toBeInTheDocument();
  });

  it('edits an existing Spark from the Best Match menu', async () => {
    const user = await renderApp();
    await searchFor(user, 'mcp server');
    const card = await screen.findByRole('article');
    await user.click(within(card).getByRole('button', { name: 'More actions' }));
    await user.click(screen.getByRole('menuitem', { name: /Edit Spark/ }));
    const dialog = await screen.findByRole('dialog', { name: 'Edit Spark' });
    const title = await within(dialog).findByDisplayValue('MCP Server Architect');
    await user.clear(title);
    await user.type(title, 'MCP Server Builder');
    await user.click(within(dialog).getByRole('button', { name: /Save changes/ }));
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(within(screen.getByRole('article')).getByText('MCP Server Builder')).toBeInTheDocument();
  });
});

describe('Library states', () => {
  it('guides the user when the library is empty', async () => {
    mock.reset({ empty: true });
    render(<App />);
    expect(await screen.findByText('Your library is empty')).toBeInTheDocument();
  });

  it('keeps working calmly when the library cannot be opened', async () => {
    mock.setLibraryAvailable(false, 'No Sparkwell library was found at D:\\Sparks.');
    render(<App />);
    expect(await screen.findByText("Your library isn't available")).toBeInTheDocument();
    expect(screen.getByText(/No Sparkwell library was found/)).toBeInTheDocument();
    expect(screen.getByLabelText('What are you trying to accomplish?')).toBeDisabled();
    expect(screen.getByRole('button', { name: /Add New Spark/ })).toBeDisabled();
    expect(screen.getByRole('button', { name: /Try again/ })).toBeInTheDocument();
  });
});

describe('Settings', () => {
  async function openSettings(user: ReturnType<typeof userEvent.setup>) {
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    return screen.findByRole('dialog', { name: 'Settings' });
  }

  it('records, validates and saves a new activation hotkey', async () => {
    const user = await renderApp();
    const dialog = await openSettings(user);
    expect(within(dialog).getByLabelText('Ctrl+Shift+Space')).toBeInTheDocument();

    await user.click(within(dialog).getByRole('button', { name: 'Change' }));
    expect(calls('begin_hotkey_capture')).toHaveLength(1);
    const box = within(dialog).getByRole('textbox', { name: /Press the new activation shortcut/ });
    await waitFor(() => expect(box).toHaveFocus());

    // Single-modifier letter: rejected, still recording.
    await user.keyboard('{Control>}k{/Control}');
    expect(await within(dialog).findByText(/Hold two modifier keys with K/)).toBeInTheDocument();

    // Owned by another app: conflict reported, previous shortcut kept.
    await user.keyboard('{Control>}{Alt>}k{/Alt}{/Control}');
    expect(await within(dialog).findByText(/already in use/)).toBeInTheDocument();

    await user.keyboard('{Control>}{Shift>}j{/Shift}{/Control}');
    expect(await within(dialog).findByText(/Saved. Press Ctrl\+Shift\+J/)).toBeInTheDocument();
    expect(within(dialog).getByLabelText('Ctrl+Shift+J')).toBeInTheDocument();
  });

  it('cancels hotkey recording with Escape and restores the shortcut', async () => {
    const user = await renderApp();
    const dialog = await openSettings(user);
    await user.click(within(dialog).getByRole('button', { name: 'Change' }));
    const box = within(dialog).getByRole('textbox', { name: /Press the new activation shortcut/ });
    await waitFor(() => expect(box).toHaveFocus());
    await user.keyboard('{Escape}');
    await waitFor(() => expect(calls('end_hotkey_capture')).toHaveLength(1));
    expect(within(dialog).getByLabelText('Ctrl+Shift+Space')).toBeInTheDocument();
    // Escape inside the recorder must not close Settings or hide the panel.
    expect(screen.getByRole('dialog', { name: 'Settings' })).toBeInTheDocument();
    expect(calls('hide_panel')).toHaveLength(0);
  });

  it('cancelling a recording clears its message', async () => {
    const user = await renderApp();
    const dialog = await openSettings(user);
    await user.click(within(dialog).getByRole('button', { name: 'Change' }));
    await user.keyboard('{Control>}k{/Control}');
    expect(await within(dialog).findByText(/Hold two modifier keys/)).toBeInTheDocument();
    await user.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(within(dialog).queryByText(/Hold two modifier keys/)).not.toBeInTheDocument());
    expect(within(dialog).getByLabelText('Ctrl+Shift+Space')).toBeInTheDocument();
  });

  it('toggles launch at startup and reflects the registered state', async () => {
    const user = await renderApp();
    const dialog = await openSettings(user);
    const toggle = within(dialog).getByRole('switch', { name: 'Launch at Startup' });
    expect(toggle).toHaveAttribute('aria-checked', 'false');
    await user.click(toggle);
    await waitFor(() => expect(toggle).toHaveAttribute('aria-checked', 'true'));
  });

  it('moves the library only after explicit confirmation', async () => {
    const user = await renderApp();
    const dialog = await openSettings(user);
    await user.click(within(dialog).getByRole('button', { name: 'Change…' }));
    expect(await within(dialog).findByText(/Copy your \d+ Sparks here and switch to it/)).toBeInTheDocument();
    expect(calls('change_library_location')).toHaveLength(0);
    await user.click(within(dialog).getByRole('button', { name: /Copy & switch/ }));
    expect(await within(dialog).findByText(/previous copy is still at/)).toBeInTheDocument();
    expect(calls('change_library_location')[0]!.args).toMatchObject({ mode: 'copy' });
  });

  it('shows local-only posture and intelligence status without model knobs', async () => {
    const user = await renderApp();
    const dialog = await openSettings(user);
    expect(within(dialog).getByText('Local Only')).toBeInTheDocument();
    expect(within(dialog).getByText(/Local intelligence offline/)).toBeInTheDocument();
    // MVP settings only: no model pickers, sliders, accounts, or theme knobs.
    expect(within(dialog).queryAllByRole('combobox')).toHaveLength(0);
    expect(within(dialog).queryAllByRole('slider')).toHaveLength(0);
    expect(within(dialog).getAllByRole('heading', { level: 3 }).map((h) => h.textContent)).toEqual([
      'Launch at Startup',
      'Activation Hotkey',
      'Library Location',
      'Local Only',
      'About',
    ]);
    expect(within(dialog).getByText('What is a Spark?')).toBeInTheDocument();
  });
});

describe('Activation shortcut on first run', () => {
  const welcomeDialog = () => screen.findByRole('dialog', { name: 'Welcome to Sparkwell' });

  it('asks for a shortcut, keeps listening after a conflict, then continues', async () => {
    mock.reset({ firstRun: true });
    const user = userEvent.setup();
    render(<App />);
    const welcome = await welcomeDialog();
    expect(within(welcome).getByText('Not set')).toBeInTheDocument();
    const choose = within(welcome).getByRole('button', { name: 'Choose a shortcut' });
    await waitFor(() => expect(choose).toHaveFocus());
    expect(within(welcome).getByRole('button', { name: 'Continue' })).toBeDisabled();

    await user.click(choose);
    const box = within(welcome).getByRole('textbox', { name: /Press the new activation shortcut/ });
    await waitFor(() => expect(box).toHaveFocus());
    // Guidance explains the rule without recommending a particular combination.
    expect(within(welcome).getByText(/Hold two of Ctrl, Alt, Shift or Win/).textContent).not.toMatch(/\+/);

    // Taken by another app: reported, and still listening for another try.
    await user.keyboard('{Control>}{Alt>}k{/Alt}{/Control}');
    expect(await within(welcome).findByText(/Ctrl\+Alt\+K is already in use/)).toBeInTheDocument();
    expect(box).toHaveFocus();

    await user.keyboard('{Control>}{Shift>}j{/Shift}{/Control}');
    expect(await within(welcome).findByText(/Saved. Press Ctrl\+Shift\+J/)).toBeInTheDocument();
    await user.click(within(welcome).getByRole('button', { name: 'Continue' }));
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Welcome to Sparkwell' })).not.toBeInTheDocument());
    expect(calls('finish_onboarding')).toHaveLength(1);
    await waitFor(() => expect(screen.getByLabelText('What are you trying to accomplish?')).toHaveFocus());
  });

  it('can be skipped, and Sparkwell stays fully usable without a shortcut', async () => {
    mock.reset({ firstRun: true });
    const user = userEvent.setup();
    render(<App />);
    const welcome = await welcomeDialog();
    await user.click(within(welcome).getByRole('button', { name: 'Skip for now' }));
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Welcome to Sparkwell' })).not.toBeInTheDocument());
    expect(calls('finish_onboarding')).toHaveLength(1);
    expect(calls('set_hotkey')).toHaveLength(0);
    // A skipped shortcut is not a problem to nag about.
    expect(screen.queryByRole('button', { name: 'Choose a shortcut' })).not.toBeInTheDocument();

    await searchFor(user, 'I need AI to help me build an MCP server.');
    expect(within(await screen.findByRole('article')).getByText('MCP Server Architect')).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Settings' }));
    const dialog = await screen.findByRole('dialog', { name: 'Settings' });
    expect(within(dialog).getByText('Not set')).toBeInTheDocument();
    expect(within(dialog).getByRole('button', { name: 'Set shortcut' })).toBeInTheDocument();
  });

  it('Escape hides the panel without skipping the welcome', async () => {
    mock.reset({ firstRun: true });
    const user = userEvent.setup();
    render(<App />);
    await welcomeDialog();
    await user.keyboard('{Escape}');
    await waitFor(() => expect(calls('hide_panel')).toHaveLength(1));
    expect(calls('finish_onboarding')).toHaveLength(0);
  });

  it('says so when the saved shortcut could not be registered at startup', async () => {
    mock.setHotkeyStatus({
      accelerator: 'Ctrl+Alt+Space',
      registered: false,
      error: 'Ctrl+Alt+Space is already in use by another app or by Windows. Try a different combination.',
    });
    const user = await renderApp();
    const notice = screen.getByRole('alert');
    expect(notice).toHaveTextContent(/Ctrl\+Alt\+Space is already in use.*tray icon/);

    await user.click(within(notice).getByRole('button', { name: 'Choose a shortcut' }));
    const dialog = await screen.findByRole('dialog', { name: 'Settings' });
    await user.click(within(dialog).getByRole('button', { name: 'Change' }));
    await waitFor(() =>
      expect(within(dialog).getByRole('textbox', { name: /Press the new activation shortcut/ })).toHaveFocus(),
    );
    await user.keyboard('{Control>}{Shift>}j{/Shift}{/Control}');
    await within(dialog).findByText(/Saved. Press Ctrl\+Shift\+J/);
    await user.click(within(dialog).getByRole('button', { name: 'Close settings' }));
    await waitFor(() => expect(screen.queryByText(/is already in use/)).not.toBeInTheDocument());
  });
});
