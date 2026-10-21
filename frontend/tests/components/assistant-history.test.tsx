import { afterAll, afterEach, beforeEach, describe, expect, mock, spyOn, test } from 'bun:test';
import type { ReactNode } from 'react';
import { act, create, type ReactTestRenderer, type ReactTestInstance } from 'react-test-renderer';

const originalCore = { ...await import('@tauri-apps/api/core') };
const originalDialog = { ...await import('../../src/components/ui/dialog') };
afterAll(() => {
  mock.module('@tauri-apps/api/core', () => originalCore);
  mock.module('../../src/components/ui/dialog', () => originalDialog);
});

// react-test-renderer has no DOM, so the Radix portal is replaced by an inline stand-in
// that renders its children only while open. Esc and focus are covered in the browser pass.
let closeAutoFocus: ((event: { preventDefault: () => void }) => void) | undefined;
mock.module('../../src/components/ui/dialog', () => ({
  Dialog: ({ open, children }: { open: boolean; children: ReactNode }) => (open ? <div data-testid="dialog">{children}</div> : null),
  DialogContent: ({ children, onCloseAutoFocus }: { children: ReactNode; onCloseAutoFocus?: (event: { preventDefault: () => void }) => void }) => {
    closeAutoFocus = onCloseAutoFocus;
    return <div>{children}</div>;
  },
  DialogHeader: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  DialogFooter: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  DialogTitle: ({ children }: { children: ReactNode }) => <h2>{children}</h2>,
  DialogDescription: ({ children }: { children: ReactNode }) => <p>{children}</p>,
}));

let stored: unknown[] | Error;
const invoke = mock(async (command: string, _args?: Record<string, unknown>): Promise<unknown> => {
  if (command === 'api_get_meeting_assistant') {
    if (stored instanceof Error) throw stored;
    return stored;
  }
  if (command === 'api_delete_meeting_assistant') return 3;
  throw new Error(`unexpected ${command}`);
});
mock.module('@tauri-apps/api/core', () => ({ invoke }));

const { AssistantHistoryPanel } = await import('../../src/components/MeetingDetails/AssistantHistoryPanel');
const { formatRecordingTime } = await import('../../src/components/VirtualizedTranscriptView');

function item(id: string, kind: string, callTime: number, extra: Record<string, unknown> = {}) {
  return {
    id, kind, callTime, shownAt: '2026-10-20T17:34:21.030Z', question: '', lead: `lead ${id}`, bullets: [],
    why: '', source: '', phase: 'checked', changedLines: [], answer: '', answeredHow: '', answeredCallTime: null,
    owner: '', ownerName: '', due: '',
    ...extra,
  };
}

let renderer: ReactTestRenderer;
const focused: string[] = [];
async function mount() {
  await act(async () => { renderer = create(<AssistantHistoryPanel meetingId="m1" />, { createNodeMock: (element) => ({ focus: () => { focused.push(String(element.type)); } }) }); });
}
const textOf = (node: ReactTestInstance | string): string =>
  typeof node === 'string' ? node : node.children.map(textOf).join('');
const allText = () => textOf(renderer.root);
const button = (label: string) => renderer.root.findAll((n) => n.type === 'button' && textOf(n) === label);
async function press(label: string) {
  await act(async () => { button(label)[0].props.onClick(); });
}
const deleteCalls = () => invoke.mock.calls.filter((c) => c[0] === 'api_delete_meeting_assistant');

afterEach(() => { mock.restore(); });
beforeEach(() => {
  invoke.mockClear();
  stored = [];
  spyOn(console, 'error').mockImplementation(() => {});
});

describe('AssistantHistoryPanel', () => {
  test('shows items in call-time order with the transcript time format and kind labels', async () => {
    stored = [
      item('c', 'catchup', 400),
      item('a', 'answer', 108.473),
      item('b', 'ask', 200),
      item('d', 'explain', 500),
      item('e', 'question', 600, { why: 'it is open' }),
      item('f', 'for_you', 700),
    ];
    await mount();
    expect(invoke).toHaveBeenCalledWith('api_get_meeting_assistant', { meetingId: 'm1' });
    const text = allText();
    expect(formatRecordingTime(108.473)).toBe('[01:48]');
    const order = ['lead a', 'lead b', 'lead c', 'lead d', 'lead e', 'lead f'].map((s) => text.indexOf(s));
    expect(order).toEqual([...order].sort((x, y) => x - y));
    expect(order.every((i) => i >= 0)).toBe(true);
    expect(text.indexOf('[01:48]')).toBeLessThan(text.indexOf('lead a'));
    for (const label of ['Answer', 'Your question', 'Explain', 'Catch-up', 'Question to ask', 'For you']) {
      expect(text).toContain(label);
    }
    expect(text).toContain('it is open');
    expect(text).toContain('Assistant6');
  });

  test('shows a two-column To do ledger above the call-order list, and todos are not in the list', async () => {
    stored = [
      item('t1', 'todo', 2405, { lead: 'Send the design doc', owner: 'you', due: 'By Friday', source: 'call' }),
      item('t2', 'todo', 2430, { lead: 'Send the export', owner: 'them', ownerName: 'Marcus', due: 'Monday', source: 'call' }),
      item('t3', 'todo', 2170, {
        lead: 'Check the licence count', owner: 'you', source: 'call',
        answer: 'said it was checked', answeredHow: 'call', answeredCallTime: 2530,
      }),
      item('q', 'question', 600),
    ];
    await mount();
    const ledger = renderer.root.findByProps({ 'aria-label': 'To do' });
    const text = textOf(ledger);
    expect(text).toContain('3 from this call. Kept here only, never sent.');
    const [left, right] = ledger.findAll((n) => n.type === 'div' && String(n.props.className ?? '').startsWith('min-w-0 px-3'));
    expect(textOf(left)).toContain('You promised');
    expect(textOf(left)).toContain('Send the design doc');
    expect(textOf(left)).toContain('By Friday');
    expect(textOf(left)).toContain('[40:05]');
    expect(textOf(left)).toContain('Done1');
    expect(textOf(left)).toContain('Done [42:10] \u00b7 said it was checked');
    expect(textOf(left)).not.toContain('Send the export');
    expect(textOf(right)).toContain('They owe you');
    expect(textOf(right)).toContain('Marcus \u00b7 Monday \u00b7 [40:30]');
    // The call-order list holds only the question, and the ledger sits above it.
    const list = renderer.root.findAll((n) => n.type === 'ol')[0];
    expect(textOf(list)).toContain('lead q');
    expect(textOf(list)).not.toContain('Send the design doc');
    expect(allText().indexOf('3 from this call')).toBeLessThan(allText().indexOf('lead q'));
  });

  test('a meeting with no todos shows no ledger', async () => {
    stored = [item('q', 'question', 600)];
    await mount();
    expect(renderer.root.findAllByProps({ 'aria-label': 'To do' })).toHaveLength(0);
    expect(allText()).not.toContain('from this call');
  });

  test('shows the answer line of an answered question', async () => {
    stored = [item('q', 'question', 60, { answer: 'Tuesday', answeredHow: 'chat', answeredCallTime: 125 })];
    await mount();
    expect(allText()).toContain(`Answered (in Ask) at ${formatRecordingTime(125)}: Tuesday`);
  });

  test('shows the empty state and no delete button when nothing was saved', async () => {
    await mount();
    expect(allText()).toContain('No assistant items were saved for this meeting.');
    expect(button('Delete assistant items')).toHaveLength(0);
  });

  test('shows a plain message when loading fails', async () => {
    stored = new Error('secret path /Users/x');
    await mount();
    expect(allText()).toContain('Could not load the assistant items.');
    expect(allText()).not.toContain('secret path');
  });

  test('deletes only after confirm; Cancel does not delete', async () => {
    stored = [item('a', 'answer', 10)];
    await mount();
    await press('Delete assistant items');
    expect(allText()).toContain('Delete the saved assistant items for this meeting? This also removes assistant.json from the recording folder.');
    expect(deleteCalls()).toHaveLength(0);

    await press('Cancel');
    expect(deleteCalls()).toHaveLength(0);
    expect(allText()).toContain('lead a');

    await press('Delete assistant items');
    await press('Delete');
    expect(deleteCalls()).toEqual([['api_delete_meeting_assistant', { meetingId: 'm1' }]]);
    expect(allText()).toContain('No assistant items were saved for this meeting.');
  });

  test('after a delete the button is gone, so focus goes to the panel heading', async () => {
    stored = [item('a', 'answer', 10)];
    focused.length = 0;
    await mount();
    await press('Delete assistant items');
    await press('Delete');
    expect(button('Delete assistant items')).toHaveLength(0);
    const heading = renderer.root.findByType('h2');
    expect(heading.props.tabIndex).toBe(-1);
    const event = { preventDefault: mock(() => {}) };
    closeAutoFocus?.(event);
    expect(event.preventDefault).toHaveBeenCalled();
    expect(focused).toEqual(['h2']);
  });
});
