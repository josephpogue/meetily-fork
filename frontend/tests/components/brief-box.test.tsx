import { afterAll, afterEach, beforeEach, describe, expect, mock, test } from 'bun:test';
import { act, create, type ReactTestRenderer } from 'react-test-renderer';

const originalAssistant = { ...await import('../../src/contexts/AssistantContext') };
afterAll(() => {
  mock.module('../../src/contexts/AssistantContext', () => originalAssistant);
});

// The backend's copy of the brief, as BriefBox reads it from useAssistant().
let backendBrief = '';
const setBrief = mock((_text: string) => {});
mock.module('../../src/contexts/AssistantContext', () => ({
  useAssistant: () => ({ asks: { brief: backendBrief }, setBrief }),
}));
const { BriefBox } = await import('../../src/components/AssistantPanel/BriefBox');

let renderer: ReactTestRenderer;
async function show(brief: string) {
  backendBrief = brief;
  await act(async () => {
    if (renderer) renderer.update(<BriefBox />); else renderer = create(<BriefBox />);
  });
}
const textarea = () => renderer.root.findByType('textarea');
const saveButton = () => renderer.root.findAll(n => n.type === 'button' && n.children.includes('Save'))[0];
const hasMarker = () => JSON.stringify(renderer.toJSON()).includes('Unsaved');
async function type(text: string) {
  await act(async () => { textarea().props.onChange({ target: { value: text } }); });
}
async function press(key: string, init: { metaKey?: boolean } = {}) {
  const preventDefault = mock(() => {});
  await act(async () => { textarea().props.onKeyDown?.({ key, metaKey: false, preventDefault, ...init }); });
  return preventDefault;
}

beforeEach(() => { setBrief.mockClear(); });
afterEach(async () => {
  if (renderer) await act(async () => renderer.unmount());
  renderer = undefined as unknown as ReactTestRenderer;
});

describe('BriefBox explicit save', () => {
  test('mounts clean with the backend brief and Save disabled', async () => {
    await show('A');
    expect(textarea().props.value).toBe('A');
    expect(hasMarker()).toBe(false);
    expect(saveButton().props.disabled).toBe(true);
  });

  test('typing shows the Unsaved marker and enables Save; blur does not save', async () => {
    await show('A');
    await type('AB');
    expect(hasMarker()).toBe(true);
    expect(saveButton().props.disabled).toBe(false);
    await act(async () => { textarea().props.onBlur?.({}); });
    expect(setBrief).not.toHaveBeenCalled();
    expect(hasMarker()).toBe(true);
  });

  test('the Save button saves once and clears the marker at once', async () => {
    await show('A');
    await type('AB');
    await act(async () => { saveButton().props.onClick(); });
    expect(setBrief).toHaveBeenCalledTimes(1);
    expect(setBrief).toHaveBeenCalledWith('AB');
    expect(hasMarker()).toBe(false);
    expect(saveButton().props.disabled).toBe(true);
  });

  test('Cmd+Enter saves without a newline; plain Enter is left alone', async () => {
    await show('A');
    await type('ABC');
    const plain = await press('Enter');
    expect(plain).not.toHaveBeenCalled();
    expect(setBrief).not.toHaveBeenCalled();
    const prevented = await press('Enter', { metaKey: true });
    expect(prevented).toHaveBeenCalledTimes(1);
    expect(setBrief).toHaveBeenCalledTimes(1);
    expect(setBrief).toHaveBeenCalledWith('ABC');
    expect(hasMarker()).toBe(false);
  });

  test('Cmd+Enter with nothing unsaved does not call setBrief', async () => {
    await show('A');
    await press('Enter', { metaKey: true });
    expect(setBrief).not.toHaveBeenCalled();
  });

  test('a backend change is adopted when there are no unsaved edits', async () => {
    await show('A');
    await show('A\n- Q? yes');
    expect(textarea().props.value).toBe('A\n- Q? yes');
    expect(hasMarker()).toBe(false);
  });

  test('a backend change does not clobber unsaved typing', async () => {
    await show('A');
    await type('A typed');
    await show('A\n- Q? yes');
    expect(textarea().props.value).toBe('A typed');
    expect(hasMarker()).toBe(true);
  });
});
