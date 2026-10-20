import { afterAll, beforeEach, describe, expect, mock, test } from 'bun:test';
import { act, create, type ReactTestRenderer } from 'react-test-renderer';

const originalService = { ...await import('../../src/services/assistantService') };
afterAll(() => {
  mock.module('../../src/services/assistantService', () => originalService);
});

// The real AssistantProvider runs; only the Tauri-facing service is replaced.
let asksListener: ((asks: unknown) => void) | undefined;
const setBrief = mock((_text: string) => Promise.resolve());
const answerAsk = mock((_id: string, _answer: string) => Promise.resolve());
const dismissAsk = mock((_id: string) => Promise.resolve());
mock.module('../../src/services/assistantService', () => ({
  assistantService: {
    getState: () => new Promise(() => {}),
    onCard: async () => () => {},
    onStatus: async () => () => {},
    onVoice: async () => () => {},
    onNote: async () => () => {},
    onAsks: async (cb: (asks: unknown) => void) => { asksListener = cb; return () => {}; },
    setBrief,
    answerAsk,
    dismissAsk,
  },
}));
const { AssistantProvider } = await import('../../src/contexts/AssistantContext');
const { AskLane } = await import('../../src/components/AssistantPanel/AskLane');

const t1 = new Date(2026, 9, 20, 15, 41).getTime();
const t2 = new Date(2026, 9, 20, 15, 52).getTime();
const time = (t: number) => new Date(t).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });

const g1 = { id: 'g1', question: 'Which PNC environments are on VCF 9.1 today?', why: '9.1 layout assumed' };
const g2 = { id: 'g2', question: 'Who triages alerts today?', why: 'no alert baseline' };
const g3 = { id: 'g3', question: 'How many active alerts are there?', why: 'alert count unknown' };
const y1 = { id: 'y1', question: 'Who is your day-to-day contact?', why: '' };
const d1 = { id: 'd1', question: 'What is the plan for tomorrow?', answer: 'Walk the badges as one action model.', how: 'chat', at: t1 };
const d2 = { id: 'd2', question: 'Should you attend tomorrow?', answer: 'Yes, with Bob and Matt.', how: 'call', at: t2 };
const base = { state: 'idle', updatedAt: 1000, brief: 'Intro call.', checked: true, checking: false, changed: [] };
const payloadA = { ...base, asks: [g1, g2], forYou: [y1], answered: [d2, d1] };
const payloadB = {
  ...base,
  asks: [g2, g3],
  forYou: [],
  answered: [{ id: 'g1', question: g1.question, answer: 'Two of five.', how: 'call', at: t2 + 60000 }, d2, d1],
};

let renderer: ReactTestRenderer;
async function emit(payload: unknown) {
  if (!renderer) await act(async () => { renderer = create(<AssistantProvider><AskLane disabled={false} /></AssistantProvider>); });
  await act(async () => { asksListener!(payload); });
}
// All rendered text in document order, joined; split children (a time inside a template) read as one string.
function collect(node: unknown): string {
  if (node === null || node === undefined) return '';
  if (typeof node === 'string') return node;
  if (Array.isArray(node)) return node.map(collect).join('');
  const n = node as { type: string; children: unknown[] | null };
  return (n.type === 'p' || n.type === 'span' || n.type === 'h2' || n.type === 'h3' ? '|' : '') + collect(n.children);
}
const text = () => collect(renderer.toJSON());
const input = () => renderer.root.findByProps({ 'aria-label': `Answer: ${y1.question}` });

beforeEach(() => {
  setBrief.mockClear(); answerAsk.mockClear(); dismissAsk.mockClear();
  asksListener = undefined;
  renderer = undefined as unknown as ReactTestRenderer;
});

describe('AskLane', () => {
  test('shows open, For-you and answered questions in order with how lines and no New tag', async () => {
    await emit(payloadA);
    const out = text();
    expect(out).toContain('3 open, 2 answered');
    const order = [g1.question, g2.question, y1.question, d2.question, d2.answer, d1.question, d1.answer].map((s) => out.indexOf(s));
    expect(order.every((i) => i >= 0)).toBe(true);
    expect([...order].sort((a, b) => a - b)).toEqual(order);
    expect(out).toContain(`On the call at ${time(t2)}`);
    expect(out).toContain(`Worked out with you in Ask at ${time(t1)}`);
    expect(out).not.toContain('|New|');
  });

  test('a For-you answer goes to the brief and answerAsk, not dismissAsk', async () => {
    await emit(payloadA);
    await act(async () => { input().props.onChange({ target: { value: 'Sam Lee' } }); });
    await act(async () => { input().props.onKeyDown({ key: 'Enter', preventDefault() {} }); });
    expect(setBrief).toHaveBeenCalledWith('Intro call.\n- Who is your day-to-day contact? Sam Lee');
    expect(answerAsk).toHaveBeenCalledWith('y1', 'Sam Lee');
    expect(dismissAsk).not.toHaveBeenCalled();
    expect(input().props.value).toBe('');
  });

  test('the Save answer button sends a For-you answer without Enter', async () => {
    await emit(payloadA);
    const save = () => renderer.root.find((n) => n.type === 'button' && n.props['aria-label'] === `Save answer: ${y1.question}`);
    expect(save().props.disabled).toBe(true);
    await act(async () => { input().props.onChange({ target: { value: 'Sam Lee' } }); });
    expect(save().props.disabled).toBe(false);
    await act(async () => { save().props.onClick(); });
    expect(answerAsk).toHaveBeenCalledWith('y1', 'Sam Lee');
    expect(dismissAsk).not.toHaveBeenCalled();
    expect(input().props.value).toBe('');
  });

  test('the next refresh moves g1 to answered and tags only the new g3', async () => {
    await emit(payloadA);
    await emit(payloadB);
    const out = text();
    expect(out).toContain('2 open, 3 answered');
    expect(out).toContain('Two of five.');
    expect(renderer.root.findAllByProps({ 'aria-label': `Dismiss ${g1.question}` }).length).toBe(0);
    const tags = renderer.root.findAll((n) => n.type === 'span' && n.children.includes('New'));
    expect(tags.length).toBe(1);
    const g3Row = renderer.root.findAll((n) => n.type === 'li' && JSON.stringify(n.children.map((c) => typeof c === 'string' ? c : '')).length > 0 && n.findAll((m) => m.children.includes(g3.question)).length > 0)[0];
    expect(g3Row.findAll((n) => n.type === 'span' && n.children.includes('New')).length).toBe(1);
  });
});
