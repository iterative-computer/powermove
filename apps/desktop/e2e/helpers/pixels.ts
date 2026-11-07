import { expect } from '@playwright/test';

/** Exact pixel equality with bounded diagnostics. Diffing two full image arrays
 * can monopolize the test worker for minutes when a single frame differs. */
export function expectPixelsEqual(actual: ArrayLike<number>, expected: ArrayLike<number>): void {
  expect(actual.length, 'pixel buffer length').toBe(expected.length);
  let differences = 0;
  const first: Array<{ index: number; actual: number; expected: number }> = [];
  for (let index = 0; index < actual.length; index++) {
    if (Object.is(actual[index], expected[index])) continue;
    differences++;
    if (first.length < 8) first.push({ index, actual: actual[index]!, expected: expected[index]! });
  }
  expect(differences, `pixel differences (first eight): ${JSON.stringify(first)}`).toBe(0);
}
