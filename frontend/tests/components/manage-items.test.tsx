import { afterAll, afterEach, beforeEach, describe, expect, mock, test } from 'bun:test';
import { act, create, type ReactTestRenderer } from 'react-test-renderer';

const originalService = { ...await import('../../src/services/assistantService') };
const originalSwitch = { ...await import('../../src/components/ui/switch') };
const originalNav = { ...await import('next/navigation') };
const originalRecording = { ...await import('../../src/contexts/RecordingStateContext') };
const g = globalThis as Record<string, unknown>;
const hadGlobals = { window: g.window, document: g.document, ResizeObserver: g.ResizeObserver };
// Module mocks and browser stubs are process-wide in bun: put everything back so other test files are unaffected.
afterAll(() => {
  mock.module('../../src/services/assistantService', () => originalService);
  mock.module('../../src/components/ui/switch', () => originalSwitch);
  mock.module('next/navigation', () => originalNav);
  mock.module('../../src/contexts/RecordingStateContext', () => originalRecording);
  for (const [k, v] of Object.entries(hadGlobals)) {
    if (v === undefined) delete g[k]; else g[k] = v;
  }
});

// The real AssistantProvider and AssistantPanel run; only the Tauri-facing
// service, the router and the recording state are replaced.
let cardListener: ((card: unknown) => void) | undefined;
let asksListener: ((asks: unknown) => void) | undefined;
let statusListener: ((status: unknown) => void) | undefined;
let deletedListener: ((payload: { ids: string[] }) => void) | undefined;
const deleteItems = mock((_ids: string[]): Promise<number> => Promise.resolve(0));
mock.module('../../src/services/assistantService', () => ({
  assistantService: {
    getState: () => new Promise(() => {}),
    onCard: async (cb: (card: unknown) => void) => { cardListener = cb; return () => {}; },
    onStatus: async (cb: (s: unknown) => void) => { statusListener = cb; return () => {}; },
    onVoice: async () => () => {},
    onNote: async () => () => {},
    onAsks: async (cb: (asks: unknown) => void) => { asksListener = cb; return () => {}; },
    onItemsDeleted: async (cb: (p: { ids: string[] }) => void) => { deletedListener = cb; return () => {}; },
    deleteItems,
  },
}));
mock.module('../../src/components/ui/switch', () => ({ Switch: () => null }));
mock.module('next/navigation', () => ({ useRouter: () => ({ push: () => {} }) }));
mock.module('../../src/contexts/RecordingStateContext', () => ({
  useRecordingState: () => ({ isRecording: false }),
}));

// A minimal browser. Window keydown listeners are kept with their capture flag,
// and a key reaches the capture ones first, as it does in a browser.
type KeyEvent = { key: string; defaultPrevented: boolean; preventDefault: () => void };
type KeyHandler = (e: KeyEvent) => void;
const keyHandlers: { handler: KeyHandler; capture: boolean }[] = [];
g.window = {
  localStorage: { getItem: () => null, setItem: () => {} },
  addEventListener: (type: string, handler: KeyHandler, capture?: boolean) => {
    if (type === 'keydown') keyHandlers.push({ handler, capture: capture === true });
  },
  removeEventListener: (type: string, handler: KeyHandler) => {
    const at = keyHandlers.findIndex(h => h.handler === handler);
    if (type === 'keydown' && at !== -1) keyHandlers.splice(at, 1);
  },
};
g.document = {
  documentElement: { style: { setProperty: () => {}, removeProperty: () => {} } },
  body: { style: {} },
  activeElement: null,
  querySelector: () => null,
};
g.ResizeObserver = class { observe() {} disconnect() {} };
const pressEscape = () => {
  const event: KeyEvent = { key: 'Escape', defaultPrevented: false, preventDefault() { this.defaultPrevented = true; } };
  const handlers = [...keyHandlers];
  for (const h of handlers.filter(x => x.capture)) h.handler(event);
  for (const h of handlers.filter(x => !x.capture)) h.handler(event);
  return event;
};

const { AssistantProvider } = await import('../../src/contexts/AssistantContext');
const { AssistantPanel } = await import('../../src/components/AssistantPanel');

const status = (o: Record<string, unknown> = {}) => ({
  enabled: true, claudeOk: true, sessionOpen: true, lanesReady: true, listening: true, mode: 'gated',
  lastError: null, lastErrorSeq: 0, ...o,
});
const card = (o: Record<string, unknown>) => ({
  question: '', lead: '', bullets: [], source: '', phase: 'checked', changedLines: [], ...o,
});
const cards = [
  card({ id: 'c1', kind: 'answer', question: 'Does the cluster stay on 8.0?', lead: 'Yes, until the last wave.', ts: 2000 }),
  card({ id: 'c2', kind: 'ask', question: 'how many hosts?', lead: '12 hosts across two clusters.', ts: 1000 }),
];
const todo = (o: Record<string, unknown>) => ({ who: '', due: '', done: null, origin: 'call', callTime: 100, ...o });
const asksPayload = (o: Record<string, unknown> = {}) => ({
  state: 'idle', updatedAt: 1000, brief: '', checked: true, checking: false, changed: [], dismissedTodos: [],
  asks: [
    { id: 'a1', question: 'Which wave carries core banking?', why: '' },
    { id: 'a2', question: 'Is there a change freeze?', why: '' },
    { id: 'a3', question: 'Who signs off on the DR test?', why: '' },
  ],
  forYou: [{ id: 'f1', question: 'What date did you give Jim?', why: '' }],
  answered: [{ id: 'r1', question: 'How many vCenters?', answer: 'Three.', how: 'call', at: 5 }],
  todos: [
    todo({ id: 't1', text: 'Send Jim the wave plan', owner: 'you', at: 2 }),
    todo({ id: 't2', text: 'Send the firewall export', owner: 'them', at: 1 }),
  ],
  ...o,
});
const ALL_IDS = ['c1', 'c2', 'a1', 'a2', 'a3', 'f1', 'r1', 't1', 't2'];

let renderer: ReactTestRenderer;
const nodeMock = () => ({ parentElement: { clientWidth: 1116 }, focus: () => {}, indeterminate: false });
async function mount(statusOver: Record<string, unknown> = {}, asks: unknown = asksPayload()) {
  await act(async () => { renderer = create(<AssistantProvider><AssistantPanel /></AssistantProvider>, { createNodeMock: nodeMock }); });
  await act(async () => {
    statusListener!(status(statusOver));
    asksListener!(asks);
    for (const c of cards) cardListener!(c);
  });
}
const control = (label: string) =>
  renderer.root.findAll(n => (n.type === 'button' || n.type === 'input') && n.props['aria-label'] === label)[0];
const press = (label: string) => act(async () => { control(label).props.onClick({ stopPropagation: () => {} }); });
const check = (label: string) => act(async () => { control(label).props.onChange(); });
const text = () => JSON.stringify(renderer.toJSON());
const openDrawer = () => act(async () => { control('Open to-do list').props.onClick({ stopPropagation: () => {} }); });

beforeEach(() => {
  deleteItems.mockClear();
  deleteItems.mockImplementation(() => Promise.resolve(0));
  cardListener = undefined; asksListener = undefined; statusListener = undefined; deletedListener = undefined; keyHandlers.length = 0;
});
afterEach(async () => {
  if (renderer) await act(async () => renderer.unmount());
  renderer = undefined as unknown as ReactTestRenderer;
});

describe('select all and delete', () => {
  test('Select, Select all, then Delete and the confirm deletes all 9 shown items', async () => {
    await mount();
    await press('Select items');
    await check('Select all items');
    expect(text()).toContain('9 selected');
    await press('Delete selected items');
    expect(text()).toContain('Delete 9 items for good?');
    expect(deleteItems).not.toHaveBeenCalled();
    await press('Confirm delete selected items');
    expect(deleteItems).toHaveBeenCalledTimes(1);
    expect([...deleteItems.mock.calls[0][0]].sort()).toEqual([...ALL_IDS].sort());
    const out = text();
    for (const gone of ['Yes, until the last wave.', '12 hosts', 'Which wave carries', 'Is there a change freeze', 'What date did you give Jim', 'How many vCenters']) {
      expect(out).not.toContain(gone);
    }
    expect(out).toContain('No cards yet.');
    expect(out).toContain('Nothing to ask right now');
    expect(out).toContain('Deleted 9 items.');
    // Select mode ended with the delete.
    expect(control('Select all items')).toBeUndefined();
    await openDrawer();
    expect(text()).not.toContain('Send Jim the wave plan');
    expect(text()).not.toContain('Send the firewall export');
  });

  test('to-dos are picked by Select all while the drawer is closed, and the strip says so', async () => {
    await mount();
    await press('Select items');
    await check('Select all items');
    expect(text()).toContain('2 of 2 selected');
    await check('Select all in Say');
    expect(text()).toContain('7 selected');
  });

  test('a lane All box picks just that lane, and a single item box toggles', async () => {
    await mount();
    await press('Select items');
    expect(text()).toContain('Pick items to delete');
    await check('Select all in Ask');
    expect(text()).toContain('5 selected');
    await check('Select question: Is there a change freeze?');
    expect(text()).toContain('4 selected');
    await press('Delete selected items');
    await press('Confirm delete selected items');
    expect([...deleteItems.mock.calls[0][0]].sort()).toEqual(['a1', 'a3', 'f1', 'r1']);
  });

  test('Keep returns to selecting with the picks intact', async () => {
    await mount();
    await press('Select items');
    await check('Select card: Yes, until the last wave.');
    await press('Delete selected items');
    await press('Keep selected items');
    expect(text()).toContain('1 selected');
    expect(deleteItems).not.toHaveBeenCalled();
  });

  test('a failed delete brings the items back, keeps them picked and says so', async () => {
    deleteItems.mockImplementation(() => Promise.reject('boom'));
    const logged = console.error;
    console.error = () => {};
    try {
      await mount();
      await press('Select items');
      await check('Select question: Who signs off on the DR test?');
      await press('Delete selected items');
      await press('Confirm delete selected items');
    } finally {
      console.error = logged;
    }
    expect(text()).toContain('Who signs off on the DR test?');
    expect(text()).toContain('Could not delete. Try again.');
    expect(text()).toContain('1 selected');
    expect(control('Leave select mode')).toBeDefined();
  });
});

describe('single x', () => {
  test('after Stop the x on a question is enabled and deletes just that id', async () => {
    await mount({ sessionOpen: false });
    const x = control('Dismiss Is there a change freeze?');
    expect(x.props.disabled).toBeUndefined();
    await act(async () => { x.props.onClick(); });
    expect(deleteItems).toHaveBeenCalledWith(['a2']);
    expect(deleteItems).toHaveBeenCalledTimes(1);
    expect(text()).not.toContain('Is there a change freeze');
  });

  test('cards, For-you and answered questions each have an x that deletes at once', async () => {
    await mount({ sessionOpen: false });
    await act(async () => { control('Dismiss card: Yes, until the last wave.').props.onClick(); });
    await act(async () => { control('Dismiss answered: How many vCenters?').props.onClick(); });
    await act(async () => { control('Dismiss What date did you give Jim?').props.onClick(); });
    expect(deleteItems.mock.calls.map(c => c[0])).toEqual([['c1'], ['r1'], ['f1']]);
  });
});

describe('deleted items stay deleted', () => {
  test('a card emitted again (checked) after its delete does not reappear', async () => {
    await mount();
    await act(async () => { control('Dismiss card: 12 hosts across two clusters.').props.onClick(); });
    expect(text()).not.toContain('12 hosts');
    await act(async () => { cardListener!({ ...cards[1], phase: 'checked', lead: '12 hosts across two clusters, checked.' }); });
    expect(text()).not.toContain('12 hosts');
  });

  test('an answered question deleted, then a snapshot that still lists it, stays gone', async () => {
    await mount();
    await act(async () => { control('Dismiss answered: How many vCenters?').props.onClick(); });
    expect(text()).not.toContain('How many vCenters');
    await act(async () => { asksListener!(asksPayload({ updatedAt: 2000 })); });
    expect(text()).not.toContain('How many vCenters');
    expect(text()).toContain('Which wave carries');
  });

  test('a new session starts with an empty deleted set', async () => {
    await mount();
    await act(async () => { control('Dismiss answered: How many vCenters?').props.onClick(); });
    await act(async () => { statusListener!(status({ sessionOpen: false })); });
    await act(async () => { statusListener!(status({ sessionOpen: true })); });
    await act(async () => { asksListener!(asksPayload({ updatedAt: 3000 })); });
    expect(text()).toContain('How many vCenters');
  });
});

describe('Esc', () => {
  test('Esc while confirming is Keep, Esc again leaves select mode, and the open drawer stays open', async () => {
    await mount();
    await openDrawer();
    await press('Select items');
    await check('Select all items');
    await press('Delete selected items');
    expect(control('Confirm delete selected items')).toBeDefined();

    let event!: KeyEvent;
    await act(async () => { event = pressEscape(); });
    expect(event.defaultPrevented).toBe(true);
    expect(control('Confirm delete selected items')).toBeUndefined();
    expect(control('Delete selected items')).toBeDefined();
    expect(text()).toContain('9 selected');

    await act(async () => { event = pressEscape(); });
    expect(event.defaultPrevented).toBe(true);
    expect(control('Select all items')).toBeUndefined();
    expect(control('Close to-do list')).toBeDefined();
    expect(deleteItems).not.toHaveBeenCalled();

    // Not selecting any more: Esc now closes the drawer, as before.
    await act(async () => { pressEscape(); });
    expect(control('Close to-do list')).toBeUndefined();
  });
});

describe('deletes made elsewhere', () => {
  test('an assistant-items-deleted event removes those live items from the panel', async () => {
    await mount();
    expect(text()).toContain('Who signs off on the DR test?');
    await act(async () => { deletedListener!({ ids: ['a3', 'c2', 'r1'] }); });
    const out = text();
    expect(out).not.toContain('Who signs off on the DR test?');
    expect(out).not.toContain('12 hosts');
    expect(out).not.toContain('How many vCenters');
    expect(out).toContain('Which wave carries');
    expect(out).toContain('Yes, until the last wave.');
  });
});

describe('select all matches what the lanes show', () => {
  test('questions hidden by "Questions unavailable" are not in Select all or the counts', async () => {
    await mount({}, asksPayload({ state: 'unavailable' }));
    expect(text()).toContain('Questions unavailable for this call.');
    expect(text()).not.toContain('Which wave carries');
    await press('Select items');
    await check('Select all items');
    // 9 items less the 3 hidden open questions.
    expect(text()).toContain('6 selected');
    await press('Delete selected items');
    await press('Confirm delete selected items');
    expect([...deleteItems.mock.calls[0][0]].sort()).toEqual(['c1', 'c2', 'f1', 'r1', 't1', 't2']);
  });

  test('the Ask lane Select all leaves the hidden questions out too', async () => {
    await mount({}, asksPayload({ state: 'unavailable' }));
    await press('Select items');
    await check('Select all in Ask');
    await press('Delete selected items');
    await press('Confirm delete selected items');
    expect([...deleteItems.mock.calls[0][0]].sort()).toEqual(['f1', 'r1']);
  });
});
