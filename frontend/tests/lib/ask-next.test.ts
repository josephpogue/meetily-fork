import { describe, expect, test } from 'bun:test';
import { answeredHowLine, appendBriefAnswer, formatOpenStamp, formatUpdatedAgo, newAskIds, reconcileBrief } from '../../src/lib/ask-next';

describe('appendBriefAnswer', () => {
  test('adds the answer as a new line under the brief', () => {
    expect(appendBriefAnswer('Intro call with a property manager.', 'Is pricing in scope?', 'yes, fees only'))
      .toBe('Intro call with a property manager.\n- Is pricing in scope? yes, fees only');
  });

  test('works on an empty brief and trims stray whitespace', () => {
    expect(appendBriefAnswer('  \n', ' Is pricing in scope? ', '  no ')).toBe('- Is pricing in scope? no');
  });
});

describe('formatUpdatedAgo', () => {
  test('is empty before the first update', () => {
    expect(formatUpdatedAgo(null, 1_000)).toBe('');
  });

  test('reads just now, seconds, then minutes', () => {
    expect(formatUpdatedAgo(10_000, 12_000)).toBe('updated just now');
    expect(formatUpdatedAgo(10_000, 22_000)).toBe('updated 12s ago');
    expect(formatUpdatedAgo(10_000, 190_000)).toBe('updated 3m ago');
  });
});

describe('reconcileBrief', () => {
  test('adopts the backend copy when there are no unsaved edits', () => {
    expect(reconcileBrief({ draft: 'A', saved: 'A' }, 'A\n- Q? yes')).toEqual({ draft: 'A\n- Q? yes', saved: 'A\n- Q? yes' });
  });

  test('appends the added suffix to an unsaved draft', () => {
    expect(reconcileBrief({ draft: 'A plus', saved: 'A' }, 'A\n- Is pricing in scope? yes'))
      .toEqual({ draft: 'A plus\n- Is pricing in scope? yes', saved: 'A\n- Is pricing in scope? yes' });
  });

  test('matches a saved baseline with trailing whitespace', () => {
    expect(reconcileBrief({ draft: 'A plus', saved: 'A ' }, 'A\n- Q? yes'))
      .toEqual({ draft: 'A plus\n- Q? yes', saved: 'A\n- Q? yes' });
  });

  test('joins with a newline when the saved baseline is empty', () => {
    expect(reconcileBrief({ draft: 'notes', saved: '' }, '- Q? yes'))
      .toEqual({ draft: 'notes\n- Q? yes', saved: '- Q? yes' });
  });

  test('keeps the draft when the backend change is not an extension of the baseline', () => {
    expect(reconcileBrief({ draft: 'A plus', saved: 'A' }, 'B')).toEqual({ draft: 'A plus', saved: 'B' });
  });

  test('keeps the draft when the backend copy extends the baseline mid-line', () => {
    expect(reconcileBrief({ draft: 'A plus', saved: 'A' }, 'AB')).toEqual({ draft: 'A plus', saved: 'AB' });
  });

  test('changes nothing when the backend copy equals the baseline', () => {
    expect(reconcileBrief({ draft: 'A plus', saved: 'A' }, 'A')).toEqual({ draft: 'A plus', saved: 'A' });
  });
});

describe('formatOpenStamp', () => {
  test('says Checked after a deep check and Updated otherwise', () => {
    expect(formatOpenStamp(10_000, 22_000, true)).toBe('Checked 12s ago');
    expect(formatOpenStamp(10_000, 22_000, false)).toBe('Updated 12s ago');
    expect(formatOpenStamp(null, 22_000, true)).toBe('');
  });
});

describe('newAskIds', () => {
  test('nothing is new on the first list', () => {
    expect(newAskIds(null, ['a', 'b'])).toEqual([]);
  });

  test('ids absent from the previous list are new', () => {
    expect(newAskIds(['a', 'b'], ['b', 'c'])).toEqual(['c']);
  });
});

describe('answeredHowLine', () => {
  const at = new Date(2026, 9, 20, 15, 41).getTime();
  const time = new Date(at).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  test('names how each question got answered', () => {
    expect(answeredHowLine('call', at)).toBe(`On the call at ${time}`);
    expect(answeredHowLine('chat', at)).toBe(`Worked out with you in Ask at ${time}`);
    expect(answeredHowLine('you', at)).toBe(`You answered at ${time}`);
  });
});
