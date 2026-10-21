import { afterAll, describe, expect, mock, test } from 'bun:test';
import { act, create } from 'react-test-renderer';

const originalCore = { ...await import('@tauri-apps/api/core') };
const originalNav = { ...await import('next/navigation') };
afterAll(() => {
  mock.module('@tauri-apps/api/core', () => originalCore);
  mock.module('next/navigation', () => originalNav);
});

// Two api_get_meetings_saved calls whose replies we release by hand, in reverse order.
const pending: Array<(v: unknown) => void> = [];
const invoke = mock((command: string): Promise<unknown> => {
  if (command === 'api_get_meetings') return Promise.resolve([]);
  if (command === 'api_get_meetings_saved') return new Promise((resolve) => pending.push(resolve));
  return Promise.resolve(null);
});
mock.module('@tauri-apps/api/core', () => ({ invoke }));
mock.module('next/navigation', () => ({ usePathname: () => '/x', useRouter: () => ({ push() {} }) }));
mock.module('../../src/contexts/RecordingStateContext', () => ({ useRecordingState: () => ({ isRecording: false }) }));
mock.module('../../src/lib/analytics', () => ({
  default: new Proxy({}, { get: () => () => {} }),
}));
const { SidebarProvider, useSidebar } = await import('../../src/components/Sidebar/SidebarProvider');

const overview = (totals: string) => ({ meetingCount: 1, totals, meetings: {} });

describe('SidebarProvider saved numbers', () => {
  test('a slow older reply cannot overwrite a newer one', async () => {
    let ctx: ReturnType<typeof useSidebar> | undefined;
    const Probe = () => { ctx = useSidebar(); return null; };
    await act(async () => { create(<SidebarProvider><Probe /></SidebarProvider>); });
    // Wait for the first load (serverAddress is set in an effect), then start a second.
    await act(async () => { await Promise.resolve(); });
    await act(async () => { await ctx!.refetchMeetings(); });
    expect(pending.length).toBeGreaterThanOrEqual(2);
    const [older, ...rest] = pending;
    const newest = rest[rest.length - 1];
    await act(async () => { newest(overview('newest')); });
    await act(async () => { older(overview('older')); });
    expect(ctx!.saved?.totals).toBe('newest');
  });
});
