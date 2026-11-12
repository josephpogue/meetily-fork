import { afterAll, afterEach, describe, expect, mock, test } from 'bun:test';
import { act, create, type ReactTestRenderer } from 'react-test-renderer';

const originalCore = { ...await import('@tauri-apps/api/core') };
const originalSonner = { ...await import('sonner') };
afterAll(() => {
  mock.module('@tauri-apps/api/core', () => originalCore);
  mock.module('sonner', () => originalSonner);
});

// Only the Tauri boundary is faked: the real assistantService runs, so the test sees the
// exact command and payload the backend would receive.
const base = {
  enabled: true, claudePath: '', fastModel: 'claude-sonnet-5', fastEffort: 'medium',
  deepModel: 'claude-opus-5', deepEffort: 'medium', triggerMode: 'gated', quietGapSecs: 2,
  names: 'joseph,joe', vaultRoot: '', deepReadDirs: '', seeZoomWindow: false,
  standingRules: 'Skip project logistics.',
};
const invoke = mock((cmd: string, _args?: unknown): Promise<unknown> => {
  if (cmd === 'assistant_get_settings') return Promise.resolve(base);
  if (cmd === 'assistant_screen_recording_allowed') return Promise.resolve(true);
  return Promise.resolve(null);
});
mock.module('@tauri-apps/api/core', () => ({ invoke }));
mock.module('sonner', () => ({ toast: { success: () => {}, error: () => {} } }));
const { AssistantSettings } = await import('../../src/components/AssistantSettings');

// The component listens for window focus; bun has no DOM, so give it a bare window (and the timers Radix Select uses) for this file.
const g = globalThis as { window?: unknown };
const hadWindow = 'window' in g;
if (!hadWindow) g.window = { addEventListener: () => {}, removeEventListener: () => {}, setTimeout, clearTimeout };
afterAll(() => { if (!hadWindow) delete g.window; });

let renderer: ReactTestRenderer;
afterEach(async () => {
  if (renderer) await act(async () => renderer.unmount());
  invoke.mockClear();
});

describe('Standing rules setting', () => {
  test('shows the saved rules, and saving sends an edit', async () => {
    await act(async () => { renderer = create(<AssistantSettings />); });
    const box = renderer.root.find(n => n.type === 'textarea' && n.props.id === 'standing-rules');
    expect(box.props.value).toBe('Skip project logistics.');

    await act(async () => { box.props.onChange({ target: { value: 'Keep answers short.' } }); });
    const save = renderer.root.findAll(n => n.type === 'button' && n.children.includes('Save settings'))[0];
    await act(async () => { await save.props.onClick(); });

    const saves = invoke.mock.calls.filter(c => c[0] === 'assistant_save_settings');
    expect(saves).toHaveLength(1);
    expect(saves[0][1]).toEqual({ settings: { ...base, standingRules: 'Keep answers short.' } });
  });
});
