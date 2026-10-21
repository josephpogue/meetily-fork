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
let storedMeta: unknown | Error;
const EMPTY_META = { brief: '', vaultProject: null, vaultNotePath: null, filedAt: null };
const invoke = mock(async (command: string, _args?: Record<string, unknown>): Promise<unknown> => {
  if (command === 'api_get_meeting_assistant') {
    if (stored instanceof Error) throw stored;
    return stored;
  }
  if (command === 'api_get_meeting_assistant_meta') {
    if (storedMeta instanceof Error) throw storedMeta;
    return storedMeta;
  }
  if (command === 'api_delete_meeting_assistant_items') {
    // Like the backend: the deleted rows are gone from the next read.
    const ids = (_args?.ids ?? []) as string[];
    if (Array.isArray(stored)) stored = (stored as { id: string }[]).filter((row) => !ids.includes(row.id));
    return ids.length;
  }
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
const button = (label: string) =>
  renderer.root.findAll((n) => n.type === 'button' && (n.props['aria-label'] === label || textOf(n) === label));
async function press(label: string) {
  await act(async () => { button(label)[0].props.onClick(); });
}
const box = (label: string) => renderer.root.findAll((n) => n.type === 'input' && n.props['aria-label'] === label);
async function tick(label: string) {
  await act(async () => { box(label)[0].props.onChange(); });
}
const deleteCalls = () => invoke.mock.calls.filter((c) => c[0] === 'api_delete_meeting_assistant_items');
const listCalls = () => invoke.mock.calls.filter((c) => c[0] === 'api_get_meeting_assistant');

// react-test-renderer has no window; the panel's Esc listener needs a stand-in, removed after each test.
const keyListeners = new Set<(event: { key: string; defaultPrevented: boolean }) => void>();
afterEach(() => { mock.restore(); delete (globalThis as { window?: unknown }).window; keyListeners.clear(); });
beforeEach(() => {
  (globalThis as { window?: unknown }).window = {
    addEventListener: (_type: string, listener: never) => { keyListeners.add(listener); },
    removeEventListener: (_type: string, listener: never) => { keyListeners.delete(listener); },
  };
  invoke.mockClear();
  stored = [];
  storedMeta = EMPTY_META;
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

  test('an empty ledger column says Nothing promised or Nothing owed', async () => {
    stored = [item('t2', 'todo', 2430, { lead: 'Send the export', owner: 'them', ownerName: 'Marcus', source: 'call' })];
    await mount();
    const columns = () => renderer.root.findByProps({ 'aria-label': 'To do' })
      .findAll((n) => n.type === 'div' && String(n.props.className ?? '').startsWith('min-w-0 px-3'));
    let [left, right] = columns();
    expect(textOf(left)).toContain('Nothing promised');
    expect(textOf(right)).not.toContain('Nothing owed');
    stored = [item('t1', 'todo', 2405, { lead: 'Send the design doc', owner: 'you', source: 'call' })];
    await mount();
    [left, right] = columns();
    expect(textOf(right)).toContain('Nothing owed');
    expect(textOf(left)).not.toContain('Nothing promised');
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

  test('shows the empty state and no Select button when nothing was saved', async () => {
    await mount();
    expect(allText()).toContain('No assistant items were saved for this meeting.');
    expect(button('Select assistant items')).toHaveLength(0);
  });

  test('shows a plain message when loading fails', async () => {
    stored = new Error('secret path /Users/x');
    await mount();
    expect(allText()).toContain('Could not load the assistant items.');
    expect(allText()).not.toContain('secret path');
  });

  test('the old delete-all button is gone', async () => {
    stored = [item('a', 'answer', 10)];
    await mount();
    expect(button('Delete assistant items')).toHaveLength(0);
    expect(button('Select assistant items')).toHaveLength(1);
    expect(box('Select item: lead a')).toHaveLength(0);
  });

  test('selects one item, deletes it after confirm and reloads; Keep does not delete', async () => {
    stored = [item('a', 'answer', 10), item('b', 'ask', 20), item('c', 'todo', 30, { owner: 'you', source: 'call' })];
    await mount();
    await press('Select assistant items');
    expect(allText()).toContain('Pick items to delete');
    expect(button('Delete selected assistant items')[0].props.disabled).toBe(true);
    await tick('Select item: lead b');
    expect(allText()).toContain('1 of 3 selected');
    await press('Delete selected assistant items');
    expect(allText()).toContain('Delete 1 assistant item?');
    expect(allText()).toContain('They are removed from this meeting and from assistant.json. This can\u2019t be undone.');
    expect(allText()).not.toContain('vault note');

    await press('Keep assistant items');
    expect(deleteCalls()).toHaveLength(0);
    expect(allText()).toContain('lead b');

    await press('Delete selected assistant items');
    const reads = listCalls().length;
    await press('Confirm delete assistant items');
    expect(deleteCalls()).toEqual([['api_delete_meeting_assistant_items', { meetingId: 'm1', ids: ['b'] }]]);
    expect(listCalls().length).toBe(reads + 1);
    expect(allText()).not.toContain('lead b');
    expect(allText()).toContain('lead a');
    expect(allText()).toContain('lead c');
    expect(button('Select assistant items')).toHaveLength(1);
    expect(box('Select item: lead a')).toHaveLength(0);
  });

  test('a to-do row has its own checkbox and is deleted by its id', async () => {
    stored = [
      item('t1', 'todo', 2405, { lead: 'Send the design doc', owner: 'you', source: 'call' }),
      item('t3', 'todo', 2170, { lead: 'Check the licence count', owner: 'you', source: 'call', answer: 'checked', answeredHow: 'call', answeredCallTime: 2530 }),
      item('q', 'question', 600),
    ];
    await mount();
    await press('Select assistant items');
    expect(box('Select to-do: Send the design doc')).toHaveLength(1);
    expect(box('Select to-do: Check the licence count')).toHaveLength(1);
    await tick('Select to-do: Send the design doc');
    await press('Delete selected assistant items');
    await press('Confirm delete assistant items');
    expect(deleteCalls()).toEqual([['api_delete_meeting_assistant_items', { meetingId: 'm1', ids: ['t1'] }]]);
    expect(allText()).not.toContain('Send the design doc');
  });

  test('Select all then Delete confirms with the all wording and sends every id', async () => {
    stored = [item('a', 'answer', 10), item('b', 'ask', 20), item('c', 'todo', 30, { owner: 'them', source: 'call' })];
    focused.length = 0;
    await mount();
    await press('Select assistant items');
    await tick('Select all assistant items');
    expect(allText()).toContain('All 3 selected');
    expect(box('Select all assistant items')[0].props.checked).toBe(true);
    await press('Delete selected assistant items');
    expect(allText()).toContain('Delete all 3 assistant items?');
    expect(allText()).toContain('They are removed from this meeting, and assistant.json is removed from the recording folder.');
    expect(button('Confirm delete assistant items').map(textOf)).toEqual(['Delete all']);
    await press('Confirm delete assistant items');
    expect(deleteCalls()).toEqual([['api_delete_meeting_assistant_items', { meetingId: 'm1', ids: ['a', 'b', 'c'] }]]);
    expect(allText()).toContain('No assistant items were saved for this meeting.');
    // Nothing is left to select, so focus goes to the panel heading.
    expect(button('Select assistant items')).toHaveLength(0);
    expect(focused).toEqual(['input']); // entering select mode focused Select all
    focused.length = 0;
    const event = { preventDefault: mock(() => {}) };
    closeAutoFocus?.(event);
    expect(event.preventDefault).toHaveBeenCalled();
    expect(focused).toEqual(['h2']);
  });

  test('Select all ticks and unticks everything; Cancel leaves select mode', async () => {
    stored = [item('a', 'answer', 10), item('b', 'ask', 20)];
    await mount();
    await press('Select assistant items');
    await tick('Select item: lead a');
    expect(allText()).toContain('1 of 2 selected');
    await tick('Select all assistant items');
    expect(allText()).toContain('All 2 selected');
    await tick('Select all assistant items');
    expect(allText()).toContain('Pick items to delete');
    await press('Leave assistant select mode');
    expect(box('Select item: lead a')).toHaveLength(0);
    expect(button('Select assistant items')).toHaveLength(1);
    expect(deleteCalls()).toHaveLength(0);
  });

  test('Esc leaves select mode, but not while an event is already handled (the dialog)', async () => {
    stored = [item('a', 'answer', 10)];
    await mount();
    await press('Select assistant items');
    await tick('Select item: lead a');
    await act(async () => { keyListeners.forEach((l) => l({ key: 'Escape', defaultPrevented: true })); });
    expect(box('Select item: lead a')).toHaveLength(1);
    await act(async () => { keyListeners.forEach((l) => l({ key: 'Escape', defaultPrevented: false })); });
    expect(box('Select item: lead a')).toHaveLength(0);
    expect(button('Select assistant items')).toHaveLength(1);
    expect(deleteCalls()).toHaveLength(0);
  });

  test('a failed delete keeps the items and shows the alert', async () => {
    stored = [item('a', 'answer', 10)];
    await mount();
    await press('Select assistant items');
    await tick('Select item: lead a');
    await press('Delete selected assistant items');
    invoke.mockImplementationOnce(async () => { throw new Error('disk'); });
    await press('Confirm delete assistant items');
    expect(renderer.root.findByProps({ role: 'alert' }).children.join('')).toBe('Could not delete the selected items.');
    expect(allText()).toContain('lead a');
  });

  describe('Brief and Filed blocks', () => {
    const filed = {
      brief: 'PNC sync with Jim',
      vaultProject: 'broadcom-pnc-bank',
      vaultNotePath: '/v/_sources/meetings/2026-10-21-pnc-sync.md',
      filedAt: '2026-10-21T18:00:00Z',
    };

    test('shows the brief in full and the filed line, above the ledger, and the confirm names the vault note', async () => {
      const longBrief = `PNC sync with Jim. ${'Confirm the wave order. '.repeat(40)}`.trim();
      stored = [item('t1', 'todo', 5, { owner: 'you', source: 'call' }), item('q', 'question', 9)];
      storedMeta = { ...filed, brief: longBrief };
      await mount();
      expect(invoke).toHaveBeenCalledWith('api_get_meeting_assistant_meta', { meetingId: 'm1' });
      const brief = renderer.root.findByProps({ 'aria-label': 'Brief' });
      expect(textOf(brief)).toBe(`Brief: ${longBrief}`);
      expect(brief.findAll((n) => String(n.props.className ?? '').includes('line-clamp'))).toHaveLength(0);
      const text = allText();
      expect(text).toContain('Filed to the vault under');
      expect(text).toContain('broadcom-pnc-bank');
      expect(text).toContain('2026-10-21-pnc-sync.md');
      expect(text).not.toContain('/v/_sources');
      expect(text.indexOf('Filed to the vault')).toBeLessThan(text.indexOf('Brief:'));
      expect(text.indexOf('Brief:')).toBeLessThan(text.indexOf('from this call'));
      expect(text.indexOf('from this call')).toBeLessThan(text.indexOf('lead q'));

      await press('Select assistant items');
      await tick('Select item: lead q');
      await press('Delete selected assistant items');
      expect(allText()).toContain('The vault note 2026-10-21-pnc-sync.md in broadcom-pnc-bank is updated too. This can\u2019t be undone.');
    });

    test('with the meeting already holding the brief, a delete reloads the meta too', async () => {
      stored = [item('a', 'answer', 10), item('b', 'ask', 20)];
      storedMeta = filed;
      await mount();
      const metaReads = () => invoke.mock.calls.filter((c) => c[0] === 'api_get_meeting_assistant_meta').length;
      expect(metaReads()).toBe(1);
      await press('Select assistant items');
      await tick('Select item: lead a');
      await press('Delete selected assistant items');
      await press('Confirm delete assistant items');
      expect(metaReads()).toBe(2);
    });

    test('an empty brief and no filing render neither block', async () => {
      stored = [item('a', 'answer', 10)];
      await mount();
      expect(renderer.root.findAllByProps({ 'aria-label': 'Brief' })).toHaveLength(0);
      expect(allText()).not.toContain('Filed to the vault');
    });

    test('a meta call that rejects (an older build) still shows the items, with no Brief or Filed block', async () => {
      stored = [item('a', 'answer', 10)];
      storedMeta = new Error('command api_get_meeting_assistant_meta not found');
      await mount();
      expect(allText()).toContain('lead a');
      expect(renderer.root.findAllByProps({ 'aria-label': 'Brief' })).toHaveLength(0);
      expect(allText()).not.toContain('Filed to the vault');
    });
  });
});
