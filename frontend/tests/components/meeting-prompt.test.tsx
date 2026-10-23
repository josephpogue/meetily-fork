import { afterAll, beforeEach, describe, expect, mock, test } from 'bun:test';
import { useState } from 'react';
import { act, create, type ReactTestInstance, type ReactTestRenderer } from 'react-test-renderer';

const originalCore = { ...await import('@tauri-apps/api/core') };
const originalEvent = { ...await import('@tauri-apps/api/event') };
const g = globalThis as Record<string, unknown>;
const hadGlobals = { window: g.window, sessionStorage: g.sessionStorage, CustomEvent: g.CustomEvent };
// Module mocks and browser stubs are process-wide in bun: put everything back.
afterAll(() => {
  mock.module('@tauri-apps/api/core', () => originalCore);
  mock.module('@tauri-apps/api/event', () => originalEvent);
  mock.module('next/navigation', () => originalNav);
  mock.module('../../src/contexts/RecordingStateContext', () => originalRecording);
  mock.module('../../src/contexts/TranscriptContext', () => originalTranscripts);
  mock.module('../../src/components/Sidebar/SidebarProvider', () => originalSidebar);
  mock.module('../../src/contexts/ConfigContext', () => originalConfig);
  mock.module('../../src/lib/analytics', () => originalAnalytics);
  mock.module('../../src/lib/recordingNotification', () => originalNotification);
  for (const [k, v] of Object.entries(hadGlobals)) {
    if (v === undefined) delete g[k]; else g[k] = v;
  }
});

// Tauri: invoke records every call; listen keeps the handlers so a test can fire events.
let currentCalls: unknown[] = [];
let startError: Error | null = null;
let teamsRunning = false;
let suppressedNow = false;
// When set, the model check waits on it so a test can act while a start is in flight.
let modelGate: Promise<void> | null = null;
const invoke = mock(async (command: string, _args?: Record<string, unknown>): Promise<unknown> => {
  if (command === 'meeting_detect_current') return currentCalls;
  if (command === 'list_call_apps') return [
    { id: 'zoom', label: 'Zoom', running: true },
    { id: 'teams', label: 'Teams', running: teamsRunning },
  ];
  if (command === 'api_get_transcript_config') return { provider: 'parakeet' };
  if (command === 'parakeet_has_available_models') { if (modelGate) await modelGate; return true; }
  if (command === 'start_recording_with_devices_and_meeting' && startError) throw startError;
  return undefined;
});
const handlers = new Map<string, (e: { payload: unknown }) => void>();
const listen = mock(async (name: string, cb: (e: { payload: unknown }) => void) => {
  handlers.set(name, cb);
  return () => { handlers.delete(name); };
});
mock.module('@tauri-apps/api/core', () => ({ invoke }));
mock.module('@tauri-apps/api/event', () => ({ listen }));
// Snapshots of the other modules are taken after the Tauri mocks, so none of their import chains binds the real listen.
const originalNav = { ...await import('next/navigation') };
const originalRecording = { ...await import('../../src/contexts/RecordingStateContext') };
const originalTranscripts = { ...await import('../../src/contexts/TranscriptContext') };
const originalSidebar = { ...await import('../../src/components/Sidebar/SidebarProvider') };
const originalConfig = { ...await import('../../src/contexts/ConfigContext') };
const originalAnalytics = { ...await import('../../src/lib/analytics') };
const originalNotification = { ...await import('../../src/lib/recordingNotification') };

let pathname = '/';
const push = mock((_to: string) => {});
mock.module('next/navigation', () => ({ usePathname: () => pathname, useRouter: () => ({ push }) }));

let recNow = false;
mock.module('../../src/contexts/RecordingStateContext', () => ({
  RecordingStatus: { IDLE: 'idle', STARTING: 'starting', RECORDING: 'recording', ERROR: 'error' },
  useRecordingState: () => ({ isRecording: recNow, setStatus: () => {} }),
}));
mock.module('../../src/contexts/TranscriptContext', () => ({
  useTranscripts: () => ({ clearTranscripts: () => {}, setMeetingTitle: () => {} }),
}));
mock.module('../../src/components/Sidebar/SidebarProvider', () => ({
  useSidebar: () => ({ setIsMeetingActive: () => {} }),
}));
mock.module('../../src/contexts/ConfigContext', () => ({
  useConfig: () => ({ selectedDevices: { micDevice: 'Mic', systemDevice: 'Speakers' } }),
}));
mock.module('../../src/lib/analytics', () => ({ default: { trackButtonClick: () => {} } }));
mock.module('../../src/lib/recordingNotification', () => ({ showRecordingNotification: async () => {} }));

// A minimal browser: a window that dispatches events to its listeners, and sessionStorage.
const winListeners = new Map<string, Set<(e: unknown) => void>>();
g.CustomEvent = class { type: string; detail: unknown; constructor(type: string, init?: { detail?: unknown }) { this.type = type; this.detail = init?.detail; } };
const store = new Map<string, string>();
g.sessionStorage = {
  getItem: (k: string) => store.get(k) ?? null,
  setItem: (k: string, v: string) => { store.set(k, v); },
  removeItem: (k: string) => { store.delete(k); },
};
g.window = {
  addEventListener: (t: string, f: (e: unknown) => void) => { (winListeners.get(t) ?? winListeners.set(t, new Set()).get(t)!).add(f); },
  removeEventListener: (t: string, f: (e: unknown) => void) => { winListeners.get(t)?.delete(f); },
  dispatchEvent: (e: { type: string }) => { winListeners.get(e.type)?.forEach((f) => f(e)); return true; },
};

const { MeetingPrompt, CallAudioPicker } = await import('../../src/components/MeetingPrompt');
const { useRecordingStart } = await import('../../src/hooks/useRecordingStart');
const { callAudioStore } = await import('../../src/services/meetingDetectService');

let setRec: (v: boolean) => void;
let startManual: () => Promise<void>;
function Harness() {
  const [rec, setRecState] = useState(false);
  setRec = setRecState;
  recNow = rec;
  const { handleRecordingStart } = useRecordingStart(rec, setRecState, () => {});
  startManual = handleRecordingStart;
  return (
    <>
      <MeetingPrompt suppressed={suppressedNow} />
      <CallAudioPicker isRecording={rec} />
    </>
  );
}

const text = (node: ReactTestInstance | string): string =>
  typeof node === 'string' ? node : node.children.map(text).join('');
let renderer: ReactTestRenderer;
const sections = () => renderer.root.findAll((n) => n.type === 'section');
const cardText = () => sections().map((s) => s.findAll((n) => n.type === 'p').map(text));
const button = (name: string) =>
  renderer.root.findAll((n) => n.type === 'button' && (n.props['aria-label'] ?? text(n)) === name)[0];
const fire = async (name: string, payload: unknown) => {
  await act(async () => { handlers.get(name)!({ payload }); });
};
const startCalls = () => invoke.mock.calls.filter((c) => c[0] === 'start_recording_with_devices_and_meeting');

async function mount() {
  await act(async () => { renderer = create(<Harness />); });
}

beforeEach(async () => {
  if (renderer) await act(async () => { renderer.unmount(); });
  invoke.mockClear();
  push.mockClear();
  handlers.clear();
  store.clear();
  currentCalls = [];
  startError = null;
  teamsRunning = false;
  suppressedNow = false;
  modelGate = null;
  pathname = '/';
  recNow = false;
  callAudioStore.set(null);
});

const zoom = (title: string | null) => ({ app: 'zoom', label: 'Zoom', title });
const teams = (title: string | null) => ({ app: 'teams', label: 'Teams', title });

describe('meeting prompt cards', () => {
  test('walks the card list through detect, update, clear and dismiss', async () => {
    await mount();
    await fire('meeting-detected', zoom('Q3 storage review'));
    expect(cardText()).toEqual([['Zoom meeting started', 'Q3 storage review']]);

    await fire('meeting-detected', teams(null));
    expect(cardText()).toEqual([['Zoom meeting started', 'Q3 storage review'], ['Teams meeting started']]);

    await fire('meeting-detected', zoom('Q4 plan'));
    expect(cardText()).toEqual([['Zoom meeting started', 'Q4 plan'], ['Teams meeting started']]);

    await fire('meeting-detected-cleared', { app: 'zoom' });
    expect(cardText()).toEqual([['Teams meeting started']]);

    await act(async () => { button('Dismiss Teams meeting').props.onClick(); });
    expect(invoke).toHaveBeenCalledWith('meeting_detect_dismiss', { app: 'teams' });
    expect(cardText()).toEqual([]);
  });

  test('a window that loads late shows the calls the backend already holds', async () => {
    currentCalls = [zoom('Standup'), teams(null)];
    await mount();
    expect(cardText()).toEqual([['Zoom meeting started', 'Standup'], ['Teams meeting started']]);
  });

  test('Record on Zoom starts a Zoom-only recording and clears the cards', async () => {
    await mount();
    await fire('meeting-detected', zoom('Q3 storage review'));
    await fire('meeting-detected', teams(null));
    await act(async () => { button('Record Zoom meeting').props.onClick(); });
    await act(async () => { await new Promise((r) => setTimeout(r, 10)); });
    expect(startCalls()).toHaveLength(1);
    expect(startCalls()[0][1]).toMatchObject({ micDeviceName: 'Mic', systemDeviceName: 'Speakers', callApp: 'zoom' });
    expect(cardText()).toEqual([]);
    expect(callAudioStore.get()).toBe('zoom');
  });

  test('a Record that fails leaves the card in place', async () => {
    startError = new Error('no audio');
    g.alert = () => {};
    await mount();
    await fire('meeting-detected', zoom('Q3 storage review'));
    await act(async () => { button('Record Zoom meeting').props.onClick(); });
    await act(async () => { await new Promise((r) => setTimeout(r, 10)); });
    expect(startCalls()).toHaveLength(1);
    expect(cardText()).toEqual([['Zoom meeting started', 'Q3 storage review']]);
    expect(callAudioStore.get()).toBeNull();
  });

  test('Record from another page goes home with the app named for the auto-start', async () => {
    pathname = '/meeting-details';
    await mount();
    await fire('meeting-detected', teams('Retro'));
    await act(async () => { button('Record Teams meeting').props.onClick(); });
    expect(push).toHaveBeenCalledWith('/');
    expect(store.get('autoStartCallApp')).toBe('teams');
    expect(startCalls()).toHaveLength(0);
  });

  test('a start from anywhere removes the cards, and none appear while recording', async () => {
    await mount();
    await fire('meeting-detected', zoom('A'));
    await act(async () => { setRec(true); });
    expect(cardText()).toEqual([]);
    await fire('meeting-detected', teams(null));
    expect(cardText()).toEqual([]);
  });
});

describe('Call audio picker', () => {
  test('Teams only on the Start button records Teams, and Stop resets to All apps', async () => {
    await mount();
    const select = () => renderer.root.findByType('select');
    expect(select().props.value).toBe('');
    const options = select().findAllByType('option').map((o) => [text(o), !!o.props.disabled]);
    expect(options).toEqual([['All apps', false], ['Zoom only', false], ['Teams only (not open)', true]]);

    teamsRunning = true;
    await act(async () => { select().props.onFocus(); });
    expect(select().findAllByType('option').map((o) => !!o.props.disabled)).toEqual([false, false, false]);
    await act(async () => { select().props.onChange({ target: { value: 'teams' } }); });
    expect(select().props.value).toBe('teams');
    await act(async () => { await startManual(); });
    expect(startCalls()).toHaveLength(1);
    expect(startCalls()[0][1]).toMatchObject({ callApp: 'teams' });

    await act(async () => { setRec(true); });
    expect(renderer.root.findAll((n) => n.type === 'p').map(text)).toContain('Call audio: Teams only');
    await act(async () => { setRec(false); });
    expect(select().props.value).toBe('');
  });

  test('a start with All apps sends a null callApp', async () => {
    await mount();
    await act(async () => { await startManual(); });
    expect(startCalls()[0][1]).toMatchObject({ callApp: null });
  });

  test('the sidebar start event uses the picker value', async () => {
    await mount();
    callAudioStore.set('zoom');
    await act(async () => { (g.window as { dispatchEvent: (e: unknown) => void }).dispatchEvent(new (g.CustomEvent as typeof CustomEvent)('start-recording-from-sidebar')); });
    await act(async () => { await new Promise((r) => setTimeout(r, 10)); });
    expect(startCalls()[0][1]).toMatchObject({ callApp: 'zoom' });
  });
});

describe('one start at a time', () => {
  const sidebarStart = (detail?: unknown) =>
    (g.window as { dispatchEvent: (e: unknown) => void }).dispatchEvent(new (g.CustomEvent as typeof CustomEvent)('start-recording-from-sidebar', detail ? { detail } : undefined));
  const settle = () => act(async () => { await new Promise((r) => setTimeout(r, 10)); });

  test('a card Record pressed while the button start is in flight is ignored', async () => {
    let release!: () => void;
    modelGate = new Promise<void>((r) => { release = r; });
    await mount();
    await fire('meeting-detected', zoom('Q3 storage review'));
    let first!: Promise<void>;
    await act(async () => { first = startManual(); });
    await act(async () => { button('Record Zoom meeting').props.onClick(); });
    await settle();
    expect(cardText()).toEqual([['Zoom meeting started', 'Q3 storage review']]);
    await act(async () => { release(); await first; });
    await settle();
    expect(startCalls()).toHaveLength(1);
    expect(startCalls()[0][1]).toMatchObject({ callApp: null });
    expect(cardText()).toEqual([]);
  });

  test('the button start is ignored while a sidebar start is in flight', async () => {
    let release!: () => void;
    modelGate = new Promise<void>((r) => { release = r; });
    await mount();
    await act(async () => { sidebarStart({ callApp: 'zoom' }); });
    await act(async () => { await startManual(); });
    await act(async () => { release(); });
    await settle();
    expect(startCalls()).toHaveLength(1);
    expect(startCalls()[0][1]).toMatchObject({ callApp: 'zoom' });
  });

  test('changing the picker during a start leaves the label on what was sent', async () => {
    let release!: () => void;
    modelGate = new Promise<void>((r) => { release = r; });
    await mount();
    let first!: Promise<void>;
    await act(async () => { first = startManual(); });
    await act(async () => { callAudioStore.set('zoom'); });
    await act(async () => { release(); await first; });
    expect(startCalls()[0][1]).toMatchObject({ callApp: null });
    expect(callAudioStore.get()).toBeNull();
  });
});

describe('onboarding', () => {
  test('no card shows and a native Record is ignored while onboarding is up', async () => {
    suppressedNow = true;
    await mount();
    await fire('meeting-detected', zoom('Q3 storage review'));
    expect(cardText()).toEqual([]);
    await fire('meeting-detect-record', zoom('Q3 storage review'));
    await act(async () => { await new Promise((r) => setTimeout(r, 10)); });
    expect(startCalls()).toHaveLength(0);
    expect(push).not.toHaveBeenCalled();
    expect(store.get('autoStartRecording')).toBeUndefined();
  });
});
