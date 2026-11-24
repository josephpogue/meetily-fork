import { test, expect } from 'bun:test';
import { shownPictures, seeAllLabel } from './allPictures';
import type { AllPictures, MeetingPicture } from './allPictures';

const T1_FILE = 'teams-frames/teams-20261118-110300-478.jpg';
const T3_FILE = 'teams-frames/teams-20261118-110332-532.jpg';
const T4_FILE = 'teams-frames/teams-20261118-110433-637.jpg';

function pic(file: string, removed: boolean, time: string, bytes: number): MeetingPicture {
  return {
    file,
    shownFile: removed ? file.replace('-frames/', '-frames/removed/') : file,
    time,
    bytes,
    removed,
    reason: removed ? 'by_you' : null,
    caption: null,
    repeatOf: null,
    deleteAfterMs: removed ? 432_001_000 : 0,
  };
}

function all(pictures: MeetingPicture[]): AllPictures {
  const kept = pictures.filter((p) => !p.removed);
  return {
    title: `${kept.length} pictures kept`,
    kept: kept.length,
    keptBytes: kept.reduce((n, p) => n + p.bytes, 0),
    removed: pictures.length - kept.length,
    pictures,
  };
}

// The brief's worked example: [T1 kept, T3 removed, T4 kept].
const T1 = pic(T1_FILE, false, '11:03:00', 200);
const T3 = pic(T3_FILE, true, '11:03:32', 300);
const T4 = pic(T4_FILE, false, '11:04:33', 400);

test('shownPictures with Show removed off gives only the kept pictures, in order', () => {
  expect(shownPictures(all([T1, T3, T4]), false)).toEqual([T1, T4]);
});

test('shownPictures with Show removed on gives every picture, in the given order', () => {
  expect(shownPictures(all([T1, T3, T4]), true)).toEqual([T1, T3, T4]);
});

test('shownPictures keeps the backend order, it does not sort', () => {
  expect(shownPictures(all([T4, T3, T1]), true)).toEqual([T4, T3, T1]);
  expect(shownPictures(all([T4, T3, T1]), false)).toEqual([T4, T1]);
});

test('shownPictures with nothing removed gives the same list either way', () => {
  expect(shownPictures(all([T1, T4]), false)).toEqual([T1, T4]);
  expect(shownPictures(all([T1, T4]), true)).toEqual([T1, T4]);
});

test('shownPictures with only removed pictures gives none when off and all when on', () => {
  expect(shownPictures(all([T3]), false)).toEqual([]);
  expect(shownPictures(all([T3]), true)).toEqual([T3]);
});

test('shownPictures of an empty meeting is empty', () => {
  expect(shownPictures(all([]), false)).toEqual([]);
  expect(shownPictures(all([]), true)).toEqual([]);
});

test('shownPictures does not change the list it is given', () => {
  const input = all([T1, T3, T4]);
  shownPictures(input, false);
  expect(input.pictures).toEqual([T1, T3, T4]);
});

test('seeAllLabel with 44 kept says See all 44 pictures', () => {
  expect(seeAllLabel(44, 0)).toBe('See all 44 pictures');
  expect(seeAllLabel(44, 5)).toBe('See all 44 pictures');
});

test('seeAllLabel with 1 kept says See 1 picture', () => {
  expect(seeAllLabel(1, 0)).toBe('See 1 picture');
  expect(seeAllLabel(1, 3)).toBe('See 1 picture');
});

test('seeAllLabel with 2 kept says See all 2 pictures', () => {
  expect(seeAllLabel(2, 0)).toBe('See all 2 pictures');
});

test('seeAllLabel with none kept but some removed says See all pictures', () => {
  expect(seeAllLabel(0, 3)).toBe('See all pictures');
});

test('seeAllLabel with no pictures at all is null', () => {
  expect(seeAllLabel(0, 0)).toBeNull();
});

test('seeAllLabel puts commas in counts above 999', () => {
  expect(seeAllLabel(1204, 0)).toBe('See all 1,204 pictures');
  expect(seeAllLabel(1000, 0)).toBe('See all 1,000 pictures');
  expect(seeAllLabel(999, 0)).toBe('See all 999 pictures');
  expect(seeAllLabel(1234567, 0)).toBe('See all 1,234,567 pictures');
});
