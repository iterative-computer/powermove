import { expect, it } from 'vitest';
import { figmaImportSummary } from './summary';

const base = { update: false, layers: 3, issues: [], missing: [], clipboard: false };
it('keeps the plain import line when nothing needs attention', () => {
  expect(figmaImportSummary({ ...base, layers: 1, warnings: [] }).message).toBe('Imported 1 layer');
});
it('names missing fonts once each, in plain words', () => {
  const warnings = [
    { id: 'a', name: 'Title', reason: 'Missing font: Inter Display' },
    { id: 'b', name: 'Body', reason: 'Missing font: SF Pro Rounded' },
    { id: 'c', name: 'Caption', reason: 'Missing font: Inter Display' },
  ];
  const summary = figmaImportSummary({ ...base, warnings });
  expect(summary.message).toBe('Imported 3 layers. 2 fonts missing: Inter Display, SF Pro Rounded. Text stays editable with a substitute font.');
  expect(summary.fonts).toEqual(['Inter Display', 'SF Pro Rounded']);
  expect(summary.warned).toEqual(['a', 'b', 'c']);
});
it('shortens long font lists and keeps other notes', () => {
  const warnings = ['A', 'B', 'C', 'D', 'E'].map((font, i) => ({ id: String(i), name: font, reason: `Missing font: ${font}` }));
  warnings.push({ id: 'x', name: 'Frame', reason: 'Approximated progressive blur' });
  expect(figmaImportSummary({ ...base, update: true, warnings }).message).toBe(
    'Updated 3 layers. 5 fonts missing: A, B, C and 2 more. Text stays editable with a substitute font. Note: Approximated progressive blur.',
  );
});
