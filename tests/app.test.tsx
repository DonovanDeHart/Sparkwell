// End-to-end UI behaviour against the in-memory backend (src/services/mockBackend.ts),
// which mirrors the Rust command contract.

import { act, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it } from 'vitest';
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
    // Standard search is labelled only because local intelligence is offline.
    expect(screen.getByText(/Local intelligence offline · standard search active/)).toBeInTheDocument();

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
    expect(screen.queryByText(/standard search active/)).not.toBeInTheDocument();
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

  it('removing a favorite can be undone', async () => {
    const user = await renderApp();
    await user.click(screen.getByRole('button', { name: 'Remove Deep Research Framework from Favorites' }));
    await screen.findByText(/Removed “Deep Research Framework” from Favorites/);
    expect(screen.queryByRole('button', { name: 'Copy Deep Research Framework' })).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Undo' }));
    expect(await screen.findByRole('button', { name: 'Copy Deep Research Framework' })).toBeInTheDocument();
  });
});

describe('Add New Spark', () => {
  it('saves manually without local intelligence and makes it retrievable', async () => {
    const user = await renderApp();
    await user.click(screen.getByRole('button', { name: /Add New Spark/ }));
    const dialog = await screen.findByRole('dialog', { name: 'Add New Spark' });
    expect(within(dialog).queryByText(/Auto-fill details/)).not.toBeInTheDocument();
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

  it('drafts editable metadata when local intelligence is ready', async () => {
    const user = await renderApp();
    act(() => mock.setAi({ state: 'online', embedModel: 'nomic-embed-text', chatModel: 'llama3.2' }));
    await user.click(screen.getByRole('button', { name: /Add New Spark/ }));
    const dialog = await screen.findByRole('dialog');
    await user.type(within(dialog).getByLabelText(/^Spark/), 'Analyze quarterly revenue spreadsheets carefully');
    await user.click(within(dialog).getByRole('button', { name: /Auto-fill details/ }));
    await waitFor(() => expect(within(dialog).getByLabelText(/^Title/)).not.toHaveValue(''));
    expect(within(dialog).getByText(/Details drafted locally/)).toBeInTheDocument();
    // Nothing is saved until the user presses Save.
    expect(calls('create_spark')).toHaveLength(0);
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
    expect(within(dialog).getByLabelText('Ctrl+Alt+Space')).toBeInTheDocument();

    await user.click(within(dialog).getByRole('button', { name: 'Change' }));
    expect(calls('begin_hotkey_capture')).toHaveLength(1);
    const box = within(dialog).getByRole('textbox', { name: /Press the new activation shortcut/ });
    await waitFor(() => expect(box).toHaveFocus());

    // Single-modifier letter: rejected, still recording.
    await user.keyboard('{Control>}k{/Control}');
    expect(await within(dialog).findByText(/at least two modifiers/)).toBeInTheDocument();

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
    expect(within(dialog).getByLabelText('Ctrl+Alt+Space')).toBeInTheDocument();
    // Escape inside the recorder must not close Settings or hide the panel.
    expect(screen.getByRole('dialog', { name: 'Settings' })).toBeInTheDocument();
    expect(calls('hide_panel')).toHaveLength(0);
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
