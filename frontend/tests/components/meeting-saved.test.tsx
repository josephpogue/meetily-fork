import { afterAll, beforeEach, describe, expect, mock, test } from 'bun:test';
import { act, create, type ReactTestInstance, type ReactTestRenderer } from 'react-test-renderer';

const originalCore = { ...await import('@tauri-apps/api/core') };
afterAll(() => {
  mock.module('@tauri-apps/api/core', () => originalCore);
});

// Only the Tauri call is replaced; the backend writes the wording (Rust tests cover it),
// so these tests check that the components place what the command returns.
let reply: unknown;
const invoke = mock(async (_command: string, _args?: Record<string, unknown>): Promise<unknown> => {
  if (reply instanceof Error) throw reply;
  return reply;
});
mock.module('@tauri-apps/api/core', () => ({ invoke }));
const { BriefLine, MeetingSavedHeader, SavedTotals } = await import('../../src/components/MeetingSaved');

const text = (node: ReactTestInstance | string): string =>
  typeof node === 'string' ? node : node.children.map(text).join('');
const pageText = (r: ReactTestRenderer) => r.root.findAll((n) => typeof n.type === 'string' && n.type === 'div')[0].children.map(text).filter(Boolean).join('|');

let renderer: ReactTestRenderer;
async function mountHeader(meetingId = 'm1', title = 'Sale planning') {
  await act(async () => { renderer = create(<MeetingSavedHeader meetingId={meetingId} title={title} />); });
}

beforeEach(() => {
  invoke.mockClear();
  reply = undefined;
});

describe('MeetingSavedHeader', () => {
  test('shows the What was saved line, the title and the Brief', async () => {
    reply = {
      brief: 'Plan the sale of the Elm St duplex',
      line: '10:04 to 10:47, 43 min, 181 pictures, 165 MB',
      durationSeconds: 2572.24, pictures: 181, bytes: 172782838,
    };
    await mountHeader();
    expect(invoke).toHaveBeenCalledWith('api_get_meeting_saved', { meetingId: 'm1' });
    expect(pageText(renderer)).toBe('Sale planning|Plan the sale of the Elm St duplex|What was saved: 10:04 to 10:47, 43 min, 181 pictures, 165 MB');
    const brief = renderer.root.findAllByType('p').find((p) => p.props.title);
    expect(brief?.props.title).toBe('Plan the sale of the Elm St duplex');
    expect(brief?.props.className).toContain('truncate');
  });

  test('no Brief shows no Brief line and no placeholder', async () => {
    reply = { brief: null, line: '10:04, 0 pictures, under 1 MB', durationSeconds: 0, pictures: 0, bytes: 0 };
    await mountHeader();
    expect(pageText(renderer)).toBe('Sale planning|What was saved: 10:04, 0 pictures, under 1 MB');
  });

  test('a missing folder shows the backend wording', async () => {
    reply = { brief: null, line: 'Recording folder not found', durationSeconds: 0, pictures: 0, bytes: 0 };
    await mountHeader();
    expect(pageText(renderer)).toContain('What was saved: Recording folder not found');
  });

  test('a failed or empty reply leaves just the title', async () => {
    reply = null;
    await mountHeader();
    expect(pageText(renderer)).toBe('Sale planning');
  });

  test('loads again for another meeting', async () => {
    reply = { brief: null, line: 'A', durationSeconds: 0, pictures: 0, bytes: 0 };
    await mountHeader('m1');
    reply = { brief: null, line: 'B', durationSeconds: 0, pictures: 0, bytes: 0 };
    await act(async () => renderer.update(<MeetingSavedHeader meetingId="m2" title="Other" />));
    expect(invoke).toHaveBeenLastCalledWith('api_get_meeting_saved', { meetingId: 'm2' });
    expect(pageText(renderer)).toBe('Other|What was saved: B');
  });
});

describe('SavedTotals', () => {
  test('shows the totals line above the list', () => {
    const r = create(<SavedTotals totals="10 meetings, 7.4 hours, 1,204 pictures, 2.1 GB" meetingCount={10} />);
    expect(JSON.stringify(r.toJSON())).toContain('10 meetings, 7.4 hours, 1,204 pictures, 2.1 GB');
  });

  test('shows nothing before the first load or with no meetings', () => {
    expect(create(<SavedTotals meetingCount={0} />).toJSON()).toBeNull();
    expect(create(<SavedTotals totals="0 meetings, 0.0 hours, 0 pictures, under 1 MB" meetingCount={0} />).toJSON()).toBeNull();
  });
});

describe('BriefLine', () => {
  test('is muted, truncated and carries the full text in title', () => {
    const p = create(<BriefLine brief="Plan the sale" />).root.findByType('p');
    expect(p.props.title).toBe('Plan the sale');
    expect(p.props.className).toContain('truncate');
    expect(p.props.className).toContain('text-gray-500');
  });

  test('renders nothing for no Brief', () => {
    expect(create(<BriefLine brief={null} />).toJSON()).toBeNull();
    expect(create(<BriefLine brief="" />).toJSON()).toBeNull();
    expect(create(<BriefLine />).toJSON()).toBeNull();
  });
});
