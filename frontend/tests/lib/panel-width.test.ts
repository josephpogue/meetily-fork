import { describe, expect, test } from 'bun:test';
import { clampPanelWidth } from '../../src/lib/panel-width';

describe('clampPanelWidth', () => {
  const cases: Array<[number | null, number, number]> = [
    [null, 1116, 600],
    [800, 1116, 696],
    [300, 1116, 440],
    [null, 924, 504],
    [null, 800, 440],
    [650, 1500, 650],
    [NaN, 1116, 600],
  ];
  for (const [stored, row, want] of cases) {
    test(`(${stored}, ${row}) -> ${want}`, () => {
      expect(clampPanelWidth(stored, row)).toBe(want);
    });
  }
});
