import { afterAll, afterEach, describe, expect, mock, test } from 'bun:test';
import { act, create, type ReactTestRenderer } from 'react-test-renderer';

const originalService = { ...await import('../../src/services/assistantService') };
const originalSonner = { ...await import('sonner') };
afterAll(() => {
  mock.module('../../src/services/assistantService', () => originalService);
  mock.module('sonner', () => originalSonner);
});

const base = {
  enabled: true, claudePath: '', fastModel: 'claude-sonnet-5', fastEffort: 'medium',
  deepModel: 'claude-opus-5', deepEffort: 'medium', triggerMode: 'gated', quietGapSecs: 2,
  names: 'joseph,joe', vaultRoot: '', deepReadDirs: '', seeZoomWindow: false,
  standingRules: 'Skip project logistics.',
};
const saveSettings = mock((_s: unknown) => Promise.resolve());
mock.module('sonner', () => ({ toast: { success: () => {}, error: () => {} } }));
mock.module('../../src/services/assistantService', () => ({
  assistantService: {
    getSettings: () => Promise.resolve(base),
    saveSettings,
    screenRecordingAllowed: () => Promise.resolve(true),
    openScreenRecordingSettings: () => Promise.resolve(),
    testClaude: () => Promise.resolve({ ok: true, version: '1', error: null }),
  },
}));
const { AssistantSettings } = await import('../../src/components/AssistantSettings');

// The component listens for window focus; bun has no DOM, so give it a bare window (and the timers Radix Select uses) for this file.
const g = globalThis as { window?: unknown };
const hadWindow = 'window' in g;
if (!hadWindow) g.window = { addEventListener: () => {}, removeEventListener: () => {}, setTimeout, clearTimeout };
afterAll(() => { if (!hadWindow) delete g.window; });

let renderer: ReactTestRenderer;
afterEach(async () => {
  if (renderer) await act(async () => renderer.unmount());
  saveSettings.mockClear();
});

describe('Standing rules setting', () => {
  test('shows the saved rules, and saving sends an edit', async () => {
    await act(async () => { renderer = create(<AssistantSettings />); });
    const box = renderer.root.find(n => n.type === 'textarea' && n.props.id === 'standing-rules');
    expect(box.props.value).toBe('Skip project logistics.');

    await act(async () => { box.props.onChange({ target: { value: 'Keep answers short.' } }); });
    const save = renderer.root.findAll(n => n.type === 'button' && n.children.includes('Save settings'))[0];
    await act(async () => { await save.props.onClick(); });

    expect(saveSettings).toHaveBeenCalledTimes(1);
    expect((saveSettings.mock.calls[0][0] as { standingRules: string }).standingRules).toBe('Keep answers short.');
  });
});
