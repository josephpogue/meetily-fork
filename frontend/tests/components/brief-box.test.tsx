import { afterAll, afterEach, beforeEach, describe, expect, mock, test } from 'bun:test';
import { act, create, type ReactTestRenderer } from 'react-test-renderer';

const originalService = { ...await import('../../src/services/assistantService') };
afterAll(() => {
  mock.module('../../src/services/assistantService', () => originalService);
});

// The real AssistantProvider runs; only the Tauri-facing service is replaced.
// `emitAsks` plays the backend sending an assistant-asks event with its copy of the brief.
let asksListener: ((asks: unknown) => void) | undefined;
const setBrief = mock((_text: string) => Promise.resolve());
mock.module('../../src/services/assistantService', () => ({
  assistantService: {
    getState: () => new Promise(() => {}),
    onCard: async () => () => {},
    onStatus: async () => () => {},
    onVoice: async () => () => {},
    onNote: async () => () => {},
    onAsks: async (cb: (asks: unknown) => void) => { asksListener = cb; return () => {}; },
    setBrief,
  },
}));
const { AssistantProvider } = await import('../../src/contexts/AssistantContext');
const { BriefBox } = await import('../../src/components/AssistantPanel/BriefBox');

let renderer: ReactTestRenderer;
const tree = (boxShown: boolean) => <AssistantProvider>{boxShown ? <BriefBox /> : null}</AssistantProvider>;
async function mount() {
  await act(async () => { renderer = create(tree(true)); });
}
// Collapsing the assistant panel unmounts BriefBox; the provider stays above it.
async function collapse() {
  await act(async () => renderer.update(tree(false)));
}
async function expand() {
  await act(async () => renderer.update(tree(true)));
}
async function emitAsks(brief: string) {
  await act(async () => {
    asksListener!({ state: 'idle', asks: [], forYou: [], updatedAt: null, brief });
  });
}
async function show(brief: string) {
  if (!renderer) await mount();
  await emitAsks(brief);
}
const textarea = () => renderer.root.findByType('textarea');
const saveButton = () => renderer.root.findAll(n => n.type === 'button' && n.children.includes('Save'))[0];
const hasMarker = () => JSON.stringify(renderer.toJSON()).includes('Unsaved');
async function type(text: string) {
  await act(async () => { textarea().props.onChange({ target: { value: text } }); });
}
async function press(key: string, init: { metaKey?: boolean } = {}) {
  const preventDefault = mock(() => {});
  await act(async () => { textarea().props.onKeyDown?.({ key, metaKey: false, preventDefault, ...init }); });
  return preventDefault;
}

beforeEach(() => { setBrief.mockClear(); asksListener = undefined; });
afterEach(async () => {
  if (renderer) await act(async () => renderer.unmount());
  renderer = undefined as unknown as ReactTestRenderer;
});

describe('BriefBox explicit save', () => {
  test('mounts clean with the backend brief and Save disabled', async () => {
    await show('A');
    expect(textarea().props.value).toBe('A');
    expect(hasMarker()).toBe(false);
    expect(saveButton().props.disabled).toBe(true);
  });

  test('typing shows the Unsaved marker and enables Save; blur does not save', async () => {
    await show('A');
    await type('AB');
    expect(hasMarker()).toBe(true);
    expect(saveButton().props.disabled).toBe(false);
    await act(async () => { textarea().props.onBlur?.({}); });
    expect(setBrief).not.toHaveBeenCalled();
    expect(hasMarker()).toBe(true);
  });

  test('the Save button saves once and clears the marker at once', async () => {
    await show('A');
    await type('AB');
    await act(async () => { saveButton().props.onClick(); });
    expect(setBrief).toHaveBeenCalledTimes(1);
    expect(setBrief).toHaveBeenCalledWith('AB');
    expect(hasMarker()).toBe(false);
    expect(saveButton().props.disabled).toBe(true);
  });

  test('Cmd+Enter saves without a newline; plain Enter is left alone', async () => {
    await show('A');
    await type('ABC');
    const plain = await press('Enter');
    expect(plain).not.toHaveBeenCalled();
    expect(setBrief).not.toHaveBeenCalled();
    const prevented = await press('Enter', { metaKey: true });
    expect(prevented).toHaveBeenCalledTimes(1);
    expect(setBrief).toHaveBeenCalledTimes(1);
    expect(setBrief).toHaveBeenCalledWith('ABC');
    expect(hasMarker()).toBe(false);
  });

  test('Cmd+Enter with nothing unsaved does not call setBrief', async () => {
    await show('A');
    await press('Enter', { metaKey: true });
    expect(setBrief).not.toHaveBeenCalled();
  });

  test('a backend change is adopted when there are no unsaved edits', async () => {
    await show('A');
    await show('A\n- Q? yes');
    expect(textarea().props.value).toBe('A\n- Q? yes');
    expect(hasMarker()).toBe(false);
  });

  test('a backend change that is not an extension does not clobber unsaved typing', async () => {
    await show('A');
    await type('A typed');
    await show('B');
    expect(textarea().props.value).toBe('A typed');
    expect(hasMarker()).toBe(true);
  });
});

describe('BriefBox keeps the draft above the box', () => {
  test('an unsaved draft survives collapsing and expanding the panel', async () => {
    await show('A');
    await type('A plus');
    await collapse();
    await expand();
    expect(textarea().props.value).toBe('A plus');
    expect(hasMarker()).toBe(true);
    expect(setBrief).not.toHaveBeenCalled();
  });

  test('the draft is still there after a backend event arrives while the box is unmounted', async () => {
    await show('A');
    await type('A plus');
    await collapse();
    await emitAsks('A\n- Q? yes');
    await expand();
    expect(textarea().props.value).toBe('A plus\n- Q? yes');
    expect(hasMarker()).toBe(true);
  });
});

describe('BriefBox merges a For-you answer into unsaved edits', () => {
  test('appends the added line to the draft, keeps the marker, and Save sends both', async () => {
    await show('A');
    await type('A plus');
    await show('A\n- Is pricing in scope? yes');
    expect(textarea().props.value).toBe('A plus\n- Is pricing in scope? yes');
    expect(hasMarker()).toBe(true);
    await act(async () => { saveButton().props.onClick(); });
    expect(setBrief).toHaveBeenCalledTimes(1);
    expect(setBrief).toHaveBeenCalledWith('A plus\n- Is pricing in scope? yes');
    expect(hasMarker()).toBe(false);
  });

  test('a saved baseline with trailing space still matches the backend copy', async () => {
    await show('A ');
    await type('A plus');
    await show('A\n- Q? yes');
    expect(textarea().props.value).toBe('A plus\n- Q? yes');
    expect(hasMarker()).toBe(true);
  });

  test('an empty saved brief joins the added line with a newline', async () => {
    await show('');
    await type('notes');
    await show('- Q? yes');
    expect(textarea().props.value).toBe('notes\n- Q? yes');
    expect(hasMarker()).toBe(true);
  });
});
