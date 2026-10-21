import { afterAll, afterEach, describe, expect, mock, test } from 'bun:test';
import { act, create, type ReactTestRenderer } from 'react-test-renderer';

const originalService = { ...await import('../../src/services/assistantService') };
afterAll(() => {
  mock.module('../../src/services/assistantService', () => originalService);
});

// The real AssistantProvider runs; only the Tauri-facing service is replaced.
let cardListener: ((card: unknown) => void) | undefined;
let statusListener: ((status: unknown) => void) | undefined;
mock.module('../../src/services/assistantService', () => ({
  assistantService: {
    getState: () => new Promise(() => {}),
    onCard: async (cb: (card: unknown) => void) => { cardListener = cb; return () => {}; },
    onStatus: async (cb: (status: unknown) => void) => { statusListener = cb; return () => {}; },
    onVoice: async () => () => {},
    onNote: async () => () => {},
    onAsks: async () => () => {},
    onItemsDeleted: async () => () => {},
  },
}));
const { AssistantProvider } = await import('../../src/contexts/AssistantContext');
const { SayLane } = await import('../../src/components/AssistantPanel/SayLane');

let renderer: ReactTestRenderer;

function card(id: string, kind: string, ts: number, extra: Record<string, unknown> = {}) {
  return {
    id, kind, question: '', lead: `lead ${id}`, bullets: [], source: '', phase: 'checked', changedLines: [], ts, ...extra,
  };
}
const bullets = (p: string, n: number) => Array.from({ length: n }, (_, i) => `${p}${i + 1}`);

async function emit(c: unknown) {
  await act(async () => { cardListener!(c); });
}
async function mount(seed: unknown[] = []) {
  await act(async () => { renderer = create(<AssistantProvider><SayLane /></AssistantProvider>); });
  for (const c of seed) await emit(c);
}
// Cards prepend as they arrive, so this is the arrival order from the brief.
async function mountSeeded() {
  await mount([
    card('a1', 'answer', 1000, { lead: 'Old answer', bullets: bullets('b', 4), phase: 'checked' }),
    card('c1', 'catchup', 2000),
    card('k1', 'ask', 3000, { question: 'does 9.1 still have badges?' }),
    card('a2', 'answer', 4000, { lead: 'New answer', bullets: bullets('x', 5), phase: 'drafting' }),
    card('e1', 'explain', 5000),
  ]);
}
const text = () => JSON.stringify(renderer.toJSON());
const order = () => [...text().matchAll(/"(?:lead|Old answer|New answer)[^"]*"/g)].map(m => m[0].replace(/"/g, ''));
async function filter(label: string) {
  const btn = renderer.root.findAll(n => n.type === 'button' && n.children.includes(label))[0];
  await act(async () => { btn.props.onClick(); });
}
const countOf = (s: string) => text().split(s).length - 1;

afterEach(async () => {
  if (renderer) await act(async () => renderer.unmount());
  renderer = undefined as unknown as ReactTestRenderer;
  cardListener = undefined;
  statusListener = undefined;
});

describe('SayLane', () => {
  test('All: pinned newest answer with every bullet and Drafting, then Earlier newest first', async () => {
    await mountSeeded();
    expect(order()).toEqual(['New answer', 'lead e1', 'lead k1', 'lead c1', 'Old answer']);
    for (const b of bullets('x', 5)) expect(text()).toContain(`"${b}"`);
    for (const b of bullets('b', 4)) expect(text()).toContain(`"${b}"`);
    expect(text()).toContain('Drafting');
    expect(text()).toContain('Checked');
    expect(text()).toContain('Earlier');
    expect(text().indexOf('Earlier')).toBeGreaterThan(text().indexOf('New answer'));
    expect(text().indexOf('Earlier')).toBeLessThan(text().indexOf('lead e1'));
    const pressed = renderer.root.findAll(n => n.type === 'button' && n.props['aria-pressed'] === true);
    expect(pressed.map(b => b.children[0])).toEqual(['All']);
  });

  test('Answers: pinned a2 then a1 only', async () => {
    await mountSeeded();
    await filter('Answers');
    expect(order()).toEqual(['New answer', 'Old answer']);
  });

  test('Mine: pinned a2, then e1, k1, c1 with the question and You asked', async () => {
    await mountSeeded();
    await filter('Mine');
    expect(order()).toEqual(['New answer', 'lead e1', 'lead k1', 'lead c1']);
    expect(text()).toContain('does 9.1 still have badges?');
    expect(text()).toContain('You asked');
    expect(text()).toContain('Explain');
    expect(text()).toContain('Catch up');
    expect(text()).toContain('Option-E at');
    expect(text()).toContain('Option-C at');
  });

  test('a corrected update shows Corrected and no Drafting', async () => {
    await mountSeeded();
    await emit(card('a2', 'answer', 4000, { lead: 'New answer', bullets: bullets('x', 5), phase: 'corrected', changedLines: ['x2'] }));
    expect(text()).toContain('Corrected');
    expect(text()).not.toContain('Drafting');
  });

  test('no answer card: no pinned card and no divider', async () => {
    await mount([card('e1', 'explain', 5000)]);
    expect(text()).not.toContain('Earlier');
    expect(order()).toEqual(['lead e1']);
  });

  test('empty states: no cards hint, warming skeleton, empty filter line', async () => {
    await mount();
    expect(text()).toContain('No cards yet.');
    expect(text()).toContain('Option-A voice ask, Option-E explain, Option-C catch up, Option-M cycle mode, Option-Q refresh questions');
    await act(async () => { statusListener!({ enabled: true, claudeOk: true, sessionOpen: true, lanesReady: false, listening: true, mode: 'gated' }); });
    expect(text()).toContain('Lanes warming up...');
    await emit(card('a1', 'answer', 1000));
    expect(text()).not.toContain('Earlier');
    expect(text()).not.toContain('Nothing here yet.');
    await filter('Mine');
    expect(text()).toContain('Nothing here yet.');
    expect(countOf('Earlier')).toBe(1);
  });
});
