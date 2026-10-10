import { describe, expect, test } from 'bun:test';
import { appendBriefAnswer, formatUpdatedAgo } from '../../src/lib/ask-next';

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
