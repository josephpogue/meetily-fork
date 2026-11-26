import { test, expect } from 'bun:test';
import { titleBoxText } from './noteTitle';
import type { TitleSuggestions } from './noteTitle';

const AUTO = 'Meeting 25_11_26_11_03_31';
const TYPED = 'CSL - Maikol Intro / Catch Up Call 11/9/26';

function fresh(): TitleSuggestions {
  return {
    byProject: {
      'broadcom-pnc-bank': 'PNC - VCF Design Workshop 10/26/26',
      'job-search': 'VCF Design Workshop 10/26/26',
    },
    noProject: 'VCF Design Workshop 10/26/26',
    current: AUTO,
    keepCurrent: false,
  };
}

function kept(): TitleSuggestions {
  return {
    byProject: { 'broadcom-csl-behring': TYPED, 'job-search': TYPED },
    noProject: TYPED,
    current: TYPED,
    keepCurrent: true,
  };
}

test('edited text is returned exactly for any choice', () => {
  const s = fresh();
  expect(titleBoxText(s, 'broadcom-pnc-bank', 'My own title')).toBe('My own title');
  expect(titleBoxText(s, '', 'My own title')).toBe('My own title');
  expect(titleBoxText(s, 'collaborator', 'My own title')).toBe('My own title');
});

test('edited text is returned exactly when keepCurrent is true', () => {
  expect(titleBoxText(kept(), 'job-search', 'PNC - Wave Plan 12/31/26')).toBe('PNC - Wave Plan 12/31/26');
});

test('edited text is not trimmed or changed', () => {
  expect(titleBoxText(fresh(), '', '  spaced  ')).toBe('  spaced  ');
});

test('an edited empty string is still the edited text', () => {
  expect(titleBoxText(fresh(), 'broadcom-pnc-bank', '')).toBe('');
});

test('keepCurrent with no edit returns current for every choice', () => {
  const s = kept();
  expect(titleBoxText(s, '', null)).toBe(TYPED);
  expect(titleBoxText(s, 'broadcom-csl-behring', null)).toBe(TYPED);
  expect(titleBoxText(s, 'job-search', null)).toBe(TYPED);
  expect(titleBoxText(s, 'collaborator', null)).toBe(TYPED);
});

test('listed project gives its framework title', () => {
  const s = fresh();
  expect(titleBoxText(s, 'broadcom-pnc-bank', null)).toBe('PNC - VCF Design Workshop 10/26/26');
  expect(titleBoxText(s, 'job-search', null)).toBe('VCF Design Workshop 10/26/26');
});

test('No project gives noProject', () => {
  expect(titleBoxText(fresh(), '', null)).toBe('VCF Design Workshop 10/26/26');
});

test('a slug not in byProject falls back to noProject', () => {
  expect(titleBoxText(fresh(), 'collaborator-design', null)).toBe('VCF Design Workshop 10/26/26');
});

test('a new draft with edited text still returns the edited text', () => {
  const next: TitleSuggestions = {
    byProject: { 'broadcom-csl-behring': 'CSL - Sprint Planning Meeting 11/5/26' },
    noProject: 'Sprint Planning Meeting 11/5/26',
    current: 'Meeting 2026-11-05_09-00-00',
    keepCurrent: false,
  };
  expect(titleBoxText(next, 'broadcom-csl-behring', 'Joseph typed this')).toBe('Joseph typed this');
  expect(titleBoxText(next, 'broadcom-csl-behring', null)).toBe('CSL - Sprint Planning Meeting 11/5/26');
});
