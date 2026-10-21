import { afterAll, beforeEach, describe, expect, mock, test } from 'bun:test';
import { act, create, type ReactTestRenderer, type ReactTestInstance } from 'react-test-renderer';

const originalService = { ...await import('../../src/services/assistantService') };
const g = globalThis as Record<string, unknown>;
const hadWindow = g.window;
const hadDocument = g.document;
// Module mocks and browser stubs are process-wide in bun: put them back so other test files are unaffected.
afterAll(() => {
  mock.module('../../src/services/assistantService', () => originalService);
  if (hadWindow === undefined) delete g.window; else g.window = hadWindow;
  if (hadDocument === undefined) delete g.document; else g.document = hadDocument;
});

// The real AssistantProvider runs; only the Tauri-facing service is replaced.
let asksListener: ((asks: unknown) => void) | undefined;
let statusListener: ((status: unknown) => void) | undefined;
let voiceListener: ((voice: unknown) => void) | undefined;
const deleteItems = mock((_ids: string[]) => Promise.resolve(0));
const addTodo = mock((_text: string): Promise<void> => Promise.resolve());
const askQuestion = mock((_text: string) => Promise.resolve());
mock.module('../../src/services/assistantService', () => ({
  assistantService: {
    getState: () => new Promise(() => {}),
    onCard: async () => () => {},
    onStatus: async (cb: (status: unknown) => void) => { statusListener = cb; return () => {}; },
    onVoice: async (cb: (voice: unknown) => void) => { voiceListener = cb; return () => {}; },
    onNote: async () => () => {},
    onAsks: async (cb: (asks: unknown) => void) => { asksListener = cb; return () => {}; },
    onItemsDeleted: async () => () => {},
    deleteItems,
    addTodo,
    ask: askQuestion,
  },
}));

// A minimal browser: window keydown listeners are kept so keys can be sent by hand.
type KeyHandler = (e: Record<string, unknown>) => void;
const keyHandlers = new Set<KeyHandler>();
g.window = {
  addEventListener: (type: string, h: KeyHandler) => { if (type === 'keydown') keyHandlers.add(h); },
  removeEventListener: (type: string, h: KeyHandler) => { if (type === 'keydown') keyHandlers.delete(h); },
};
// A minimal document: what has focus, and whether a modal dialog is open.
const doc = { activeElement: null as unknown, dialogOpen: false };
g.document = {
  get activeElement() { return doc.activeElement; },
  querySelector: (selector: string) => (doc.dialogOpen && selector.includes('role="dialog"') ? {} : null),
};
const pressKey = (e: Record<string, unknown>) => {
  const event = { defaultPrevented: false, preventDefault: mock(() => {}), ...e };
  for (const h of [...keyHandlers]) h(event);
  return event;
};

const { AssistantProvider } = await import('../../src/contexts/AssistantContext');
const { TodoDock } = await import('../../src/components/AssistantPanel/TodoDock');
const { AskBox } = await import('../../src/components/AssistantPanel/AskBox');

const done = { how: 'said it was checked', callTime: 2530, at: 6 };
const todo = (o: Record<string, unknown>) => ({ id: '', who: '', due: '', done: null, ...o });
const todos = [
  todo({ id: 't5', text: 'Send Dana the runbook template', owner: 'you', origin: 'typed', callTime: 2480, at: 5 }),
  todo({ id: 't4', text: 'Send the host inventory export', owner: 'them', who: 'Marcus', due: 'Monday', origin: 'call', callTime: 2430, at: 4 }),
  todo({ id: 't3', text: 'Confirm whether the DMZ cluster moved', owner: 'them', origin: 'call', callTime: 2381, at: 3 }),
  todo({ id: 't2', text: 'Send the design doc with the domain layout', owner: 'you', due: 'By Friday', origin: 'call', callTime: 2405, at: 2 }),
  todo({ id: 't1', text: 'Check the licence count', owner: 'you', origin: 'call', callTime: 2170, at: 1, done }),
];
const payload = (list: unknown[]) => ({
  state: 'idle', asks: [], forYou: [], answered: [], updatedAt: 1000, brief: '', checked: true, checking: false, changed: [],
  todos: list, dismissedTodos: [],
});

let renderer: ReactTestRenderer;
// Buttons get a stand-in element that records which one was focused.
let focused: string[] = [];
const inputNode = { focus: () => {} };
const createNodeMock = (element: { type: unknown; props: Record<string, unknown> }) =>
  element.type === 'input' ? inputNode : element.type === 'button' ? { focus: () => { focused.push(String(element.props['aria-label'])); } } : null;
const status = (enabled: boolean) => ({
  enabled, sessionOpen: true, lanesReady: true, mode: 'manual', listening: false, claudeOk: true, lastError: null, lastErrorSeq: 0,
});
async function mount(list: unknown[] = todos) {
  await act(async () => {
    renderer = create(<AssistantProvider><TodoDock /><AskBox disabled={false} /></AssistantProvider>, { createNodeMock });
  });
  // The session opens first: a session's start clears the last call's lists.
  await act(async () => { statusListener!(status(true)); asksListener!(payload(list)); });
}
// Rendered text of one node, with a bar between block-ish elements so adjacent pieces stay apart.
function collect(node: unknown): string {
  if (node === null || node === undefined) return '';
  if (typeof node === 'string') return node;
  if (Array.isArray(node)) return node.map(collect).join('');
  const n = node as { type: string; children: unknown[] | null };
  return (['p', 'span', 'h3', 'h4', 'b'].includes(n.type) ? '|' : '') + collect(n.children);
}
const textOf = (n: ReactTestInstance) => collect(n.children.map((c) => (typeof c === 'string' ? c : toJSON(c))));
function toJSON(i: ReactTestInstance): unknown {
  return { type: String(i.type), children: i.children.map((c) => (typeof c === 'string' ? c : toJSON(c))) };
}
const column = (title: string) => renderer.root.findAll((n) => n.type === 'section' && n.props['aria-label'] === title)[0];
const byLabel = (label: string) => renderer.root.findAll((n) => n.type === 'button' && n.props['aria-label'] === label)[0];
const inputEl = () => renderer.root.findAll((n) => n.type === 'input' && n.props['aria-label'] === 'Ask a question')[0];
const stripText = () => textOf(byLabel('Open to-do list').parent!);
const open = () => act(async () => { byLabel('Open to-do list').props.onClick({ stopPropagation: () => {} }); });
const type = (value: string) => act(async () => { inputEl().props.onChange({ target: { value } }); });

beforeEach(() => {
  deleteItems.mockClear(); addTodo.mockClear(); askQuestion.mockClear();
  asksListener = undefined; statusListener = undefined; voiceListener = undefined; keyHandlers.clear();
  focused = []; doc.activeElement = null; doc.dialogOpen = false;
});

describe('To do strip', () => {
  test('shows the open counts and the newest open to-do in full, with the chevron as a labelled button', async () => {
    await mount();
    const out = stripText();
    expect(out).toContain('|You|2');
    expect(out).toContain('|Them|2');
    expect(out).toContain('Send Dana the runbook template');
    const chevron = byLabel('Open to-do list');
    expect(chevron.props['aria-expanded']).toBe(false);
    expect(chevron.type).toBe('button');
  });

  test('with no to-dos the counts read 0 and the text says Nothing yet', async () => {
    await mount([]);
    const out = stripText();
    expect(out).toContain('|You|0');
    expect(out).toContain('|Them|0');
    expect(out).toContain('Nothing yet');
  });

  test('an older payload with no todos field still renders', async () => {
    await mount();
    await act(async () => { const { todos: _t, ...old } = payload([]); asksListener!(old); });
    expect(stripText()).toContain('Nothing yet');
  });
});

describe('To do drawer', () => {
  test('left column lists what you promised with its meta lines and a Done group', async () => {
    await mount();
    await open();
    expect(byLabel('Close to-do list').props['aria-expanded']).toBe(true);
    const out = textOf(column('You promised'));
    expect(out).toContain('You promised');
    const a = out.indexOf('Send Dana the runbook template');
    const b = out.indexOf('Send the design doc with the domain layout');
    const d = out.indexOf('Check the licence count');
    expect(a).toBeGreaterThan(-1);
    expect(b).toBeGreaterThan(a);
    expect(d).toBeGreaterThan(b);
    expect(out).toContain('You added at 41:20');
    expect(out).toMatch(/By Friday\|?[^|]*\|?·?\|?Said at 40:05|By Friday.*Said at 40:05/);
    expect(out).toContain('Done|1');
    expect(out).toContain('Done at 42:10 · said it was checked');
  });

  test('right column shows the named pill, then the due and the call time', async () => {
    await mount();
    await open();
    const col = column('They owe you');
    const out = textOf(col);
    expect(out.indexOf('Send the host inventory export')).toBeLessThan(out.indexOf('Confirm whether the DMZ cluster moved'));
    expect(out).toContain('|Marcus');
    expect(out).toMatch(/Marcus.*Monday.*Said at 40:30/);
    expect(out).toContain('Said at 39:41');
    // The unnamed item's pill reads Them (column header pill plus that pill).
    expect(out.match(/\|Them/g)!.length).toBe(2);
    expect(out).not.toContain('Nothing owed yet');
  });

  test('empty columns say so, and the left one gives the todo: hint', async () => {
    await mount([]);
    await open();
    expect(textOf(column('You promised'))).toContain('Nothing promised yet');
    expect(textOf(column('You promised'))).toContain('Type todo: in the Ask box to add one.');
    expect(textOf(column('They owe you'))).toContain('Nothing owed yet');
  });

  test('dismiss X deletes the item by its id', async () => {
    await mount();
    await open();
    const x = byLabel('Dismiss to-do: Send the host inventory export');
    await act(async () => { x.props.onClick(); });
    expect(deleteItems).toHaveBeenCalledWith(['t4']);
    expect(byLabel('Dismiss to-do: Check the licence count')).toBeDefined();
  });

  test('Esc closes the drawer', async () => {
    await mount();
    await open();
    await act(async () => { pressKey({ key: 'Escape' }); });
    expect(renderer.root.findAll((n) => n.props['aria-label'] === 'To do list').length).toBe(0);
    expect(byLabel('Close to-do list')).toBeUndefined();
  });

  test('Esc while voice capture is listening cancels the capture only: the drawer stays open', async () => {
    await mount();
    await open();
    await act(async () => { voiceListener!({ state: 'listening', heard: '' }); });
    const event = pressKey({ key: 'Escape' });
    expect(event.preventDefault).not.toHaveBeenCalled();
    expect(byLabel('Close to-do list')).toBeDefined();
    await act(async () => { voiceListener!({ state: 'off', heard: '' }); });
    await act(async () => { pressKey({ key: 'Escape' }); });
    expect(byLabel('Close to-do list')).toBeUndefined();
  });

  test('dismissing moves focus to the next item in the column, else the previous, else Close', async () => {
    await mount();
    await open();
    // "You promised" holds t5, t2 (open) then t1 (done). Dismiss the middle one.
    const dismiss = (label: string) => act(async () => { byLabel(label).props.onClick(); });
    const without = async (gone: string[]) => {
      await act(async () => { asksListener!(payload(todos.filter((t) => !gone.includes(t.id)))); });
    };
    await dismiss('Dismiss to-do: Send the design doc with the domain layout');
    await without(['t2']);
    expect(focused.at(-1)).toBe('Dismiss to-do: Check the licence count');
    // The last item of the column: focus goes to the previous one.
    await dismiss('Dismiss to-do: Check the licence count');
    await without(['t2', 't1']);
    expect(focused.at(-1)).toBe('Dismiss to-do: Send Dana the runbook template');
    // The only item left in the column: focus goes to Close.
    await dismiss('Dismiss to-do: Send Dana the runbook template');
    await without(['t2', 't1', 't5']);
    expect(focused.at(-1)).toBe('Close to-do list');
  });

  test('a delete moves focus once, and a backend that keeps sending the item never steals it later', async () => {
    await mount([todos[0]]);
    await open();
    focused = [];
    await act(async () => { byLabel('Dismiss to-do: Send Dana the runbook template').props.onClick(); });
    // The item is gone at once and focus goes to Close; the same list coming back, then a reset, moves nothing more.
    expect(focused).toEqual(['Close to-do list']);
    await act(async () => { asksListener!(payload([todos[0]])); });
    await act(async () => { asksListener!(payload([])); });
    expect(focused).toEqual(['Close to-do list']);
  });

  test('the drawer x stays enabled and deletes after Stop (session closed)', async () => {
    await act(async () => {
      renderer = create(<AssistantProvider><TodoDock /></AssistantProvider>, { createNodeMock });
    });
    await act(async () => { asksListener!(payload(todos)); statusListener!({ ...status(true), sessionOpen: false }); });
    await open();
    const x = byLabel('Dismiss to-do: Send Dana the runbook template');
    expect(x.props.disabled).toBeUndefined();
    await act(async () => { x.props.onClick(); });
    expect(deleteItems).toHaveBeenCalledWith(['t5']);
  });

  test('New marks items that arrived since the drawer was last closed', async () => {
    await mount();
    await open();
    expect(textOf(column('You promised'))).toContain('|New');
    await act(async () => { pressKey({ key: 'Escape' }); });
    await open();
    expect(textOf(column('You promised'))).not.toContain('|New');
    await act(async () => { asksListener!(payload([todo({ id: 'n', text: 'Call Ana back', owner: 'you', origin: 'typed', callTime: 2600, at: Date.now() + 1000 }), ...todos])); });
    expect(textOf(column('You promised'))).toContain('|New');
  });
});

describe('Ask box to-do entry', () => {
  test('todo: shows the chip, and Send adds the text after the prefix instead of asking', async () => {
    await mount();
    expect(byLabel('Send question')).toBeDefined();
    await type('todo: call Ana back');
    expect(inputEl().props.value).toBe('call Ana back');
    await act(async () => { byLabel('Add to-do').props.onClick(); });
    expect(addTodo).toHaveBeenCalledWith('call Ana back');
    expect(askQuestion).not.toHaveBeenCalled();
    expect(inputEl().props.value).toBe('');
  });

  test('a plain question still asks, and the prefix is case-insensitive with leading spaces', async () => {
    await mount();
    await type('who owns this?');
    await act(async () => { byLabel('Send question').props.onClick(); });
    expect(askQuestion).toHaveBeenCalledWith('who owns this?');
    await type('  TODO: Book the room');
    await act(async () => { inputEl().props.onKeyDown({ key: 'Enter', preventDefault: () => {} }); });
    expect(addTodo).toHaveBeenCalledWith('Book the room');
  });

  test('a command error goes to onError and keeps the text', async () => {
    addTodo.mockImplementationOnce(() => Promise.reject('Start the assistant first.'));
    const onError = mock((_m: string) => {});
    await act(async () => {
      renderer = create(<AssistantProvider><AskBox disabled={false} onError={onError} /></AssistantProvider>);
    });
    await type('todo: x');
    await act(async () => { byLabel('Add to-do').props.onClick(); });
    expect(onError).toHaveBeenCalledWith('Start the assistant first.');
    expect(inputEl().props.value).toBe('x');
  });

  test('Backspace on an empty entry goes back to asking', async () => {
    await mount();
    await type('todo:');
    expect(byLabel('Add to-do')).toBeDefined();
    await act(async () => { inputEl().props.onKeyDown({ key: 'Backspace', preventDefault: () => {} }); });
    expect(byLabel('Send question')).toBeDefined();
  });

  test('Opt-T puts todo: in front of what is typed and swallows the key', async () => {
    await mount();
    await type('book the room');
    let event!: { preventDefault: ReturnType<typeof mock> };
    await act(async () => { event = pressKey({ altKey: true, code: 'KeyT', key: '†' }); });
    expect(event.preventDefault).toHaveBeenCalled();
    expect(byLabel('Add to-do')).toBeDefined();
    expect(inputEl().props.value).toBe('book the room');
    const plain = pressKey({ altKey: false, code: 'KeyT', key: 't' });
    expect(plain.preventDefault).not.toHaveBeenCalled();
  });

  test('Opt-T is left alone unless the assistant is on, Option is alone, no dialog is open and no other field has focus', async () => {
    await mount();
    const left = (e: Record<string, unknown>) => {
      const event = pressKey({ altKey: true, code: 'KeyT', key: '†', ...e });
      expect(event.preventDefault).not.toHaveBeenCalled();
    };
    left({ metaKey: true });
    left({ ctrlKey: true });
    left({ shiftKey: true });
    doc.dialogOpen = true;
    left({});
    doc.dialogOpen = false;
    doc.activeElement = { tagName: 'TEXTAREA', isContentEditable: false };
    left({});
    doc.activeElement = { tagName: 'DIV', isContentEditable: true };
    left({});
    doc.activeElement = null;
    await act(async () => { statusListener!(status(false)); });
    left({});
    expect(byLabel('Send question')).toBeDefined();
    await act(async () => { statusListener!(status(true)); });
    // With focus in the Ask box itself the key still works.
    doc.activeElement = inputNode;
    const ok = pressKey({ altKey: true, code: 'KeyT', key: '†' });
    expect(ok.preventDefault).toHaveBeenCalled();
    expect(byLabel('Add to-do')).toBeDefined();
  });
});
