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
let asksListener: ((asks: unknown) => void) | undefined;
let statusListener: ((status: unknown) => void) | undefined;
const setBrief = mock((_text: string) => Promise.resolve());
mock.module('../../src/services/assistantService', () => ({
  assistantService: {
    getState: () => new Promise(() => {}),
    onCard: async () => () => {},
    onStatus: async (cb: (s: unknown) => void) => { statusListener = cb; return () => {}; },
    onVoice: async () => () => {},
    onNote: async () => () => {},
    onAsks: async (cb: (asks: unknown) => void) => { asksListener = cb; return () => {}; },
    setBrief,
    setEnabled: async () => {},
  },
}));
// Radix Switch needs a real DOM (HTMLInputElement); a plain button stands in.
mock.module('../../src/components/ui/switch', () => ({ Switch: () => null }));
mock.module('next/navigation', () => ({ useRouter: () => ({ push: () => {} }) }));
mock.module('../../src/contexts/RecordingStateContext', () => ({
  useRecordingState: () => ({ isRecording: false }),
}));

// A minimal browser: storage, the root style, and a ResizeObserver.
const store = new Map<string, string>();
g.window = {
  localStorage: {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => { store.set(k, v); },
  },
  addEventListener: () => {},
  removeEventListener: () => {},
};
g.document = {
  documentElement: { style: { setProperty: () => {}, removeProperty: () => {} } },
  body: { style: {} },
};
g.ResizeObserver = class { observe() {} disconnect() {} };

const { AssistantProvider } = await import('../../src/contexts/AssistantContext');
const { AssistantPanel } = await import('../../src/components/AssistantPanel');

const KEY = 'meetily-fork.assistant-panel-width';
const baseStatus = { enabled: false, claudeOk: true, sessionOpen: false, listening: true, mode: 'gated' };
const asks = (brief: string) => ({ state: 'idle', asks: [], forYou: [], answered: [], updatedAt: null, brief, checked: true, checking: false, changed: [] });

let renderer: ReactTestRenderer;
let rowWidth = 1116;
async function mount(status: Record<string, unknown>, brief = '') {
  await act(async () => {
    renderer = create(<AssistantProvider><AssistantPanel /></AssistantProvider>, {
      createNodeMock: () => ({ parentElement: { clientWidth: rowWidth } }),
    });
  });
  await act(async () => {
    statusListener!({ ...baseStatus, ...status });
    asksListener!(asks(brief));
  });
}
const button = (label: string) =>
  renderer.root.findAll(n => n.type === 'button' && n.children.includes(label))[0];
const click = (label: string) => act(async () => { button(label).props.onClick(); });
const text = () => JSON.stringify(renderer.toJSON());
const handle = () => renderer.root.findByProps({ role: 'separator' });
const panelStyleWidth = () => (renderer.toJSON() as unknown as { props: { style: { width: number } } }).props.style.width;
const ptr = (clientX: number) => ({ button: 0, pointerId: 1, clientX, currentTarget: { setPointerCapture: () => {} } });

beforeEach(() => { setBrief.mockClear(); store.clear(); rowWidth = 1116; asksListener = undefined; statusListener = undefined; });
afterEach(async () => {
  if (renderer) await act(async () => renderer.unmount());
  renderer = undefined as unknown as ReactTestRenderer;
});

describe('brief while the assistant is off or Claude is missing', () => {
  for (const [name, status, message] of [
    ['assistant off', { enabled: false, claudeOk: true }, 'Assistant is off.'],
    ['Claude missing', { enabled: true, claudeOk: false }, "Claude CLI isn't available."],
  ] as const) {
    test(`${name}: Add brief opens the editor and Save calls setBrief`, async () => {
      await mount(status);
      expect(text()).toContain(message);
      await click('Add brief');
      expect(text()).not.toContain(message);
      const textarea = renderer.root.findByType('textarea');
      await act(async () => { textarea.props.onChange({ target: { value: 'Call with Jim' } }); });
      await act(async () => { button('Save').props.onClick(); });
      expect(setBrief).toHaveBeenCalledWith('Call with Jim');
    });

    test(`${name}: Read all shows the whole brief and Back to questions returns`, async () => {
      await mount(status, 'Line one\nLine two of the saved brief');
      await click('Read all');
      expect(text()).toContain('Line two of the saved brief');
      expect(text()).not.toContain(message);
      await click('Back to questions');
      expect(text()).toContain(message);
    });
  }
});

describe('resize handle storage', () => {
  test('a click with no movement leaves storage alone', async () => {
    store.set(KEY, '800');
    rowWidth = 924; // shows 504
    await mount({ enabled: true });
    expect(panelStyleWidth()).toBe(504);
    await act(async () => { handle().props.onPointerDown(ptr(500)); });
    await act(async () => { handle().props.onPointerUp({}); });
    expect(store.get(KEY)).toBe('800');
  });

  test('double-click resets to 600 and writes it', async () => {
    store.set(KEY, '800');
    await mount({ enabled: true });
    await act(async () => { handle().props.onDoubleClick(); });
    expect(store.get(KEY)).toBe('600');
  });

  test('a drag stores the width after the last pointermove', async () => {
    rowWidth = 1500;
    await mount({ enabled: true });
    await act(async () => { handle().props.onPointerDown(ptr(500)); });
    await act(async () => { handle().props.onPointerMove(ptr(450)); });
    await act(async () => { handle().props.onPointerMove(ptr(400)); });
    await act(async () => { handle().props.onPointerUp({}); });
    expect(store.get(KEY)).toBe('700');
  });
});
