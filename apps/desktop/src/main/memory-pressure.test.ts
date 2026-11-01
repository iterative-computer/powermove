import { expect, it } from 'vitest';
import { levelFromSysctl } from './memory-pressure';

it('maps the kernel pressure level to the editor event', () => {
  expect(levelFromSysctl('1\n')).toBe('normal');
  expect(levelFromSysctl('2\n')).toBe('moderate');
  expect(levelFromSysctl('4\n')).toBe('critical');
  expect(levelFromSysctl('garbage')).toBe('normal');
});
