import { describe, expect, test } from 'bun:test';
import { appendBriefAnswer, formatUpdatedAgo, reconcileBrief } from '../../src/lib/ask-next';

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

  test('changes nothing when the backend copy equals the baseline', () => {
    expect(reconcileBrief({ draft: 'A plus', saved: 'A' }, 'A')).toEqual({ draft: 'A plus', saved: 'A' });
  });
});
