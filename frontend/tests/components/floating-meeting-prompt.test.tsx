import { afterAll, beforeEach, describe, expect, mock, test } from 'bun:test';
import { act, create, type ReactTestInstance, type ReactTestRenderer } from 'react-test-renderer';

const originalCore = { ...await import('@tauri-apps/api/core') };
const originalEvent = { ...await import('@tauri-apps/api/event') };
// Module mocks are process-wide in bun: put them back.
afterAll(() => {
  mock.module('@tauri-apps/api/core', () => originalCore);
  mock.module('@tauri-apps/api/event', () => originalEvent);
});

// Tauri: invoke records every call; listen keeps the handlers so a test can fire events.
let currentCards: unknown[] = [];
let failCommand: string | null = null;
const invoke = mock(async (command: string, _args?: Record<string, unknown>): Promise<unknown> => {
  if (command === 'meeting_prompt_cards') return currentCards;
  if (command === failCommand) throw new Error(`${command} failed`);
  return undefined;
});
const handlers = new Map<string, (e: { payload: unknown }) => void>();
const listen = mock(async (name: string, cb: (e: { payload: unknown }) => void) => {
  handlers.set(name, cb);
  return () => { handlers.delete(name); };
});
mock.module('@tauri-apps/api/core', () => ({ invoke }));
mock.module('@tauri-apps/api/event', () => ({ listen }));

const { FloatingMeetingPrompt } = await import('../../src/components/MeetingPrompt/FloatingMeetingPrompt');
const { meetingDetectService } = await import('../../src/services/meetingDetectService');

const text = (node: ReactTestInstance | string): string =>
  typeof node === 'string' ? node : node.children.map(text).join('');
let renderer: ReactTestRenderer | null = null;
const all = () => (renderer ? renderer.root.children.map(text).join('|') : '');
const buttons = (name: string) =>
  renderer!.root.findAll((n) => n.type === 'button' && text(n).trim() === name);
const press = async (name: string, index = 0) => {
  await act(async () => { await buttons(name)[index].props.onClick(); });
};
const fire = async (name: string, payload: unknown) => {
  await act(async () => { handlers.get(name)!({ payload }); });
};
const settle = () => act(async () => { await new Promise((r) => setTimeout(r, 10)); });
const commandCalls = (command: string) => invoke.mock.calls.filter((c) => c[0] === command);

async function mount() {
  await act(async () => { renderer = create(<FloatingMeetingPrompt />); });
  await settle();
}

const card = (app: 'zoom' | 'teams', kind: 'started' | 'ended', title: string | null = null) => ({
  app, label: app === 'zoom' ? 'Zoom' : 'Teams', kind, title,
});

beforeEach(async () => {
  if (renderer) await act(async () => { renderer!.unmount(); });
  renderer = null;
  invoke.mockClear();
  listen.mockClear();
  handlers.clear();
  currentCards = [];
  failCommand = null;
});

describe('meetingDetectService floating prompt methods', () => {
  test('cards() invokes meeting_prompt_cards with no args and returns the list', async () => {
    currentCards = [card('zoom', 'started', 'Weekly sync')];
    const got = await meetingDetectService.cards();
    expect(got).toEqual([card('zoom', 'started', 'Weekly sync')]);
    const calls = commandCalls('meeting_prompt_cards');
    expect(calls).toHaveLength(1);
    expect(calls[0][1] ?? {}).toEqual({});
  });

  test('onCards() listens to meeting-prompt-cards and hands over the full payload', async () => {
    const seen: unknown[] = [];
    await meetingDetectService.onCards((c) => seen.push(c));
    expect(handlers.has('meeting-prompt-cards')).toBe(true);
    const payload = [card('zoom', 'started'), card('teams', 'ended')];
    handlers.get('meeting-prompt-cards')!({ payload });
    handlers.get('meeting-prompt-cards')!({ payload: [] });
    expect(seen).toEqual([payload, []]);
  });

  test('record(app) invokes meeting_detect_record with { app }', async () => {
    await meetingDetectService.record('teams');
    expect(invoke).toHaveBeenCalledWith('meeting_detect_record', { app: 'teams' });
  });
});

describe('FloatingMeetingPrompt', () => {
  test('an empty list renders nothing', async () => {
    await mount();
    expect(renderer!.toJSON()).toBeNull();
  });

  test('on mount it shows the card from cards(): Zoom started with Record and Dismiss', async () => {
    currentCards = [card('zoom', 'started', 'Weekly sync')];
    await mount();
    expect(all()).toContain('Zoom meeting started');
    expect(all()).toContain('Weekly sync');
    expect(buttons('Record')).toHaveLength(1);
    expect(buttons('Dismiss')).toHaveLength(1);
  });

  test('pressing Record invokes meeting_detect_record for zoom and the card goes', async () => {
    currentCards = [card('zoom', 'started', 'Weekly sync')];
    await mount();
    await press('Record');
    expect(invoke).toHaveBeenCalledWith('meeting_detect_record', { app: 'zoom' });
    expect(renderer!.toJSON()).toBeNull();
  });

  test('pressing Dismiss invokes meeting_detect_dismiss and the card goes', async () => {
    currentCards = [card('teams', 'started')];
    await mount();
    await press('Dismiss');
    expect(invoke).toHaveBeenCalledWith('meeting_detect_dismiss', { app: 'teams' });
    expect(renderer!.toJSON()).toBeNull();
  });

  test('a meeting-prompt-cards payload replaces the list; an empty payload renders nothing', async () => {
    currentCards = [card('zoom', 'started', 'Weekly sync')];
    await mount();
    await fire('meeting-prompt-cards', [card('teams', 'started', 'Retro')]);
    expect(all()).toContain('Teams meeting started');
    expect(all()).not.toContain('Zoom meeting started');
    await fire('meeting-prompt-cards', []);
    expect(renderer!.toJSON()).toBeNull();
  });

  test('two started cards show Zoom first, each button acts on its own app', async () => {
    currentCards = [card('zoom', 'started', 'Weekly sync'), card('teams', 'started')];
    await mount();
    const out = all();
    expect(out.indexOf('Zoom meeting started')).toBeGreaterThanOrEqual(0);
    expect(out.indexOf('Teams meeting started')).toBeGreaterThan(out.indexOf('Zoom meeting started'));
    expect(buttons('Record')).toHaveLength(2);
    await press('Dismiss', 1);
    expect(invoke).toHaveBeenCalledWith('meeting_detect_dismiss', { app: 'teams' });
    expect(all()).toContain('Zoom meeting started');
    expect(all()).not.toContain('Teams meeting started');
  });

  test('an ended Teams card shows Stop recording and Keep recording; Stop calls meeting_detect_stop', async () => {
    currentCards = [card('teams', 'ended')];
    await mount();
    expect(all()).toContain('Teams meeting ended');
    expect(buttons('Stop recording')).toHaveLength(1);
    expect(buttons('Keep recording')).toHaveLength(1);
    await press('Stop recording');
    expect(invoke).toHaveBeenCalledWith('meeting_detect_stop', { app: 'teams' });
    expect(renderer!.toJSON()).toBeNull();
  });

  test('Keep recording calls meeting_detect_keep and the card goes', async () => {
    currentCards = [card('teams', 'ended')];
    await mount();
    await press('Keep recording');
    expect(invoke).toHaveBeenCalledWith('meeting_detect_keep', { app: 'teams' });
    expect(renderer!.toJSON()).toBeNull();
  });

  test('an ended Zoom card reads "Zoom meeting ended" and has no Record button', async () => {
    currentCards = [card('zoom', 'ended')];
    await mount();
    expect(all()).toContain('Zoom meeting ended');
    expect(buttons('Record')).toHaveLength(0);
  });

  for (const [button, command] of [
    ['Record', 'meeting_detect_record'],
    ['Dismiss', 'meeting_detect_dismiss'],
  ] as const) {
    test(`a failing ${command} leaves the card until the next payload, and nothing throws`, async () => {
      const quiet = console.error;
      console.error = () => {};
      try {
        failCommand = command;
        currentCards = [card('zoom', 'started', 'Weekly sync')];
        await mount();
        await press(button);
        await settle();
        expect(all()).toContain('Zoom meeting started');
        await fire('meeting-prompt-cards', []);
        expect(renderer!.toJSON()).toBeNull();
      } finally {
        console.error = quiet;
      }
    });
  }

  for (const [button, command] of [
    ['Stop recording', 'meeting_detect_stop'],
    ['Keep recording', 'meeting_detect_keep'],
  ] as const) {
    test(`a failing ${command} leaves the ended card in place`, async () => {
      const quiet = console.error;
      console.error = () => {};
      try {
        failCommand = command;
        currentCards = [card('zoom', 'ended')];
        await mount();
        await press(button);
        await settle();
        expect(all()).toContain('Zoom meeting ended');
      } finally {
        console.error = quiet;
      }
    });
  }
});
