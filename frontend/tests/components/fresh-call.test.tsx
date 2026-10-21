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
const setBrief = mock((_text: string) => Promise.resolve());
mock.module('../../src/services/assistantService', () => ({
  assistantService: {
    getState: () => new Promise(() => {}),
    onCard: async (cb: (card: unknown) => void) => { cardListener = cb; return () => {}; },
    onStatus: async (cb: (s: unknown) => void) => { statusListener = cb; return () => {}; },
    onVoice: async () => () => {},
    onNote: async () => () => {},
    onAsks: async (cb: (asks: unknown) => void) => { asksListener = cb; return () => {}; },
    onItemsDeleted: async () => () => {},
    setBrief,
  },
}));
mock.module('../../src/components/ui/switch', () => ({ Switch: () => null }));
mock.module('next/navigation', () => ({ useRouter: () => ({ push: () => {} }) }));
mock.module('../../src/contexts/RecordingStateContext', () => ({
  useRecordingState: () => ({ isRecording: false }),
}));

// A minimal browser, as in manage-items.test.tsx.
g.window = {
  localStorage: { getItem: () => null, setItem: () => {} },
  addEventListener: () => {},
  removeEventListener: () => {},
};
g.document = {
  documentElement: { style: { setProperty: () => {}, removeProperty: () => {} } },
  body: { style: {} },
  activeElement: null,
  querySelector: () => null,
};
g.ResizeObserver = class { observe() {} disconnect() {} };

const { AssistantProvider, useAssistant } = await import('../../src/contexts/AssistantContext');
const { AssistantPanel } = await import('../../src/components/AssistantPanel');

type Ctx = ReturnType<typeof useAssistant>;
let ctx: Ctx;
function Probe() {
  ctx = useAssistant();
  return null;
}

const status = (o: Record<string, unknown> = {}) => ({
  enabled: true, claudeOk: true, sessionOpen: true, lanesReady: true, listening: true, mode: 'gated',
  lastError: null, lastErrorSeq: 0, ...o,
});
const card = (o: Record<string, unknown>) => ({
  question: '', lead: '', bullets: [], source: '', phase: 'checked', changedLines: [], ...o,
});
const todo = (o: Record<string, unknown>) => ({ who: '', due: '', done: null, origin: 'call', callTime: 100, ...o });
const callOne = (brief: string) => ({
  state: 'idle', updatedAt: 1000, brief, checked: true, checking: false, changed: [], dismissedTodos: [],
  asks: [{ id: 'a1', question: 'Which wave carries core banking?', why: '' }],
  forYou: [{ id: 'f1', question: 'What date did you give Jim?', why: '' }],
  answered: [{ id: 'r1', question: 'How many vCenters?', answer: 'Three.', how: 'call', at: 5 }],
  todos: [todo({ id: 't1', text: 'Send Jim the wave plan', owner: 'you', at: 2 })],
});
const callTwo = (brief: string) => ({
  state: 'idle', updatedAt: null, brief, checked: false, checking: false, changed: [], dismissedTodos: [],
  asks: [], forYou: [], answered: [], todos: [],
});
const GONE = ['12 hosts across two clusters', 'Which wave carries', 'What date did you give Jim', 'How many vCenters', 'Send Jim the wave plan'];

let renderer: ReactTestRenderer;
const nodeMock = () => ({ parentElement: { clientWidth: 1116 }, focus: () => {}, indeterminate: false });
const text = () => JSON.stringify(renderer.toJSON());

// Call one is live: a Say card, an Ask question, a For-you question, an answered question, a to-do and brief A.
async function callOneLive() {
  await act(async () => {
    renderer = create(<AssistantProvider><Probe /><AssistantPanel /></AssistantProvider>, { createNodeMock: nodeMock });
  });
  await act(async () => {
    statusListener!(status());
    asksListener!(callOne('PNC storage sync with Jim'));
    cardListener!(card({ id: 'c1', kind: 'ask', question: 'how many hosts?', lead: '12 hosts across two clusters.', ts: 1000 }));
  });
}
const stop = () => act(async () => { statusListener!(status({ sessionOpen: false })); });

beforeEach(() => {
  setBrief.mockClear();
  cardListener = undefined; asksListener = undefined; statusListener = undefined;
});
afterEach(async () => {
  if (renderer) await act(async () => renderer.unmount());
  renderer = undefined as unknown as ReactTestRenderer;
});

describe('a new recording starts with nothing from the last call', () => {
  test('call one is showing everything before the test starts', async () => {
    await callOneLive();
    for (const gone of GONE) expect(text()).toContain(gone);
    expect(text()).toContain('PNC storage sync with Jim');
    expect(ctx.briefDraft).toBe('PNC storage sync with Jim');
  });

  test('after Stop the cards, questions, to-dos and brief are still shown for deleting and drafting', async () => {
    await callOneLive();
    await stop();
    for (const gone of GONE) expect(text()).toContain(gone);
    expect(ctx.briefDraft).toBe('PNC storage sync with Jim');
  });

  test('the backend opening call two (asks first, then status) shows no card, question, to-do or brief', async () => {
    await callOneLive();
    await stop();
    await act(async () => { asksListener!(callTwo('')); });
    await act(async () => { statusListener!(status({ sessionOpen: true })); });
    for (const gone of GONE) expect(text()).not.toContain(gone);
    expect(text()).not.toContain('PNC storage sync with Jim');
    expect(text()).toContain('No brief yet.');
    expect(ctx.briefDraft).toBe('');
    expect(ctx.briefSaved).toBe('');
    expect(ctx.cards).toEqual([]);
    expect(ctx.asks.asks).toEqual([]);
    expect(ctx.asks.forYou).toEqual([]);
    expect(ctx.asks.answered).toEqual([]);
    expect(ctx.asks.todos).toEqual([]);
  });

  test('the same when status arrives before the new asks payload', async () => {
    await callOneLive();
    await stop();
    await act(async () => { statusListener!(status({ sessionOpen: true })); });
    await act(async () => { asksListener!(callTwo('')); });
    for (const gone of GONE) expect(text()).not.toContain(gone);
    expect(text()).toContain('No brief yet.');
    expect(ctx.briefDraft).toBe('');
  });

  test('a brief saved between the calls is call two\'s brief', async () => {
    await callOneLive();
    await stop();
    await act(async () => { ctx.setBriefDraft('Pricing follow-up'); });
    await act(async () => { ctx.saveBrief(); });
    expect(setBrief).toHaveBeenCalledWith('Pricing follow-up');
    // The backend echoes the brief it holds for the next call.
    await act(async () => { asksListener!(callTwo('Pricing follow-up')); });
    await act(async () => { statusListener!(status({ sessionOpen: true })); });
    await act(async () => { asksListener!(callTwo('Pricing follow-up')); });
    expect(ctx.briefDraft).toBe('Pricing follow-up');
    expect(ctx.briefSaved).toBe('Pricing follow-up');
    expect(text()).toContain('Pricing follow-up');
    for (const gone of GONE) expect(text()).not.toContain(gone);
  });
});
