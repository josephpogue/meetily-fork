import { afterAll, beforeEach, describe, expect, mock, test } from 'bun:test';
import { act, create, type ReactTestRenderer } from 'react-test-renderer';

const originalService = { ...await import('../../src/services/assistantService') };
afterAll(() => {
  mock.module('../../src/services/assistantService', () => originalService);
});

// The real AssistantProvider runs; only the Tauri-facing service is replaced.
let noteListener: ((note: unknown) => void) | undefined;
let projects: string[] = [];
let suggestion: string | null = null;
const saveNote = mock((project: string | null) => Promise.resolve({ notePath: '/v/n.md', project }));
mock.module('../../src/services/assistantService', () => ({
  assistantService: {
    getState: () => new Promise(() => {}),
    onCard: async () => () => {},
    onStatus: async () => () => {},
    onVoice: async () => () => {},
    onAsks: async () => () => {},
    onNote: async (cb: (note: unknown) => void) => { noteListener = cb; return () => {}; },
    listVaultProjects: () => Promise.resolve(projects),
    suggestVaultProject: () => Promise.resolve(suggestion),
    saveNote,
  },
}));
const { AssistantProvider } = await import('../../src/contexts/AssistantContext');
const { NoteBar } = await import('../../src/components/AssistantPanel/NoteBar');

let renderer: ReactTestRenderer;
async function showNote(state: string) {
  if (!renderer) await act(async () => { renderer = create(<AssistantProvider><NoteBar /></AssistantProvider>); });
  await act(async () => { noteListener!({ state, markdown: '## Summary\n\nWave order.', error: null }); });
}
function collect(node: unknown): string {
  if (node === null || node === undefined) return '';
  if (typeof node === 'string') return node;
  if (Array.isArray(node)) return node.map(collect).join('');
  return collect((node as { children: unknown[] | null }).children);
}
const text = () => collect(renderer.toJSON());
const select = () => renderer.root.findByProps({ 'aria-label': 'Vault project' });
const optionLabels = () => select().findAllByType('option').map((o) => collect(o.children));
const saveButton = () => renderer.root.findAllByType('button').find((b) => collect(b.children) === 'Save')!;

beforeEach(() => {
  saveNote.mockClear();
  noteListener = undefined;
  renderer = undefined as unknown as ReactTestRenderer;
  projects = ['a', 'b', 'broadcom-pnc-bank'];
  suggestion = 'broadcom-pnc-bank';
});

describe('NoteBar project picker', () => {
  test('preselects the suggestion, listed first with the other projects after No project', async () => {
    await showNote('ready');
    expect(select().props.value).toBe('broadcom-pnc-bank');
    expect(optionLabels()).toEqual(['broadcom-pnc-bank (suggested)', 'No project', 'a', 'b']);
    expect(text()).toContain('Suggested from the brief and the call. Save files the note under this project.');
  });

  test('No project shows its own hint and Save passes null', async () => {
    await showNote('ready');
    await act(async () => { select().props.onChange({ target: { value: '' } }); });
    expect(text()).toContain('Save keeps the note without a project.');
    await act(async () => { saveButton().props.onClick(); });
    expect(saveNote).toHaveBeenCalledWith(null);
  });

  test('Save passes the suggested slug, then the saved line names it', async () => {
    await showNote('ready');
    await act(async () => { saveButton().props.onClick(); });
    expect(saveNote).toHaveBeenCalledWith('broadcom-pnc-bank');
    await showNote('saved');
    expect(text()).toContain('Note saved to broadcom-pnc-bank.');
  });

  test('another project gets the plain hint, and a save with none says no project', async () => {
    await showNote('ready');
    await act(async () => { select().props.onChange({ target: { value: 'b' } }); });
    expect(text()).toContain('Save files the note under this project.');
    expect(text()).not.toContain('Suggested from the brief');
    await act(async () => { select().props.onChange({ target: { value: '' } }); });
    await act(async () => { saveButton().props.onClick(); });
    await showNote('saved');
    expect(text()).toContain('Note saved, no project.');
  });

  test('with no suggestion No project is preselected and there is no Suggested group', async () => {
    suggestion = null;
    await showNote('ready');
    expect(select().props.value).toBe('');
    expect(optionLabels()).toEqual(['No project', 'a', 'b', 'broadcom-pnc-bank']);
    expect(renderer.root.findAllByType('optgroup').map((g) => g.props.label)).toEqual(['All projects']);
  });
});
