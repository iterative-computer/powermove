import { expect, it } from 'vitest';
import { renderInvocation } from './render';
import path from 'node:path';

it('passes user-chosen paths and render arguments without shell interpretation', async () => {
  const env = { ...process.env, POWERMOVE_PM_CLI: process.execPath };
  const args = ['a project.pmv', '../anywhere/a $(literal).mov', '--param', 'label="Hello world"', '--definitions', 'custom.json'];
  expect(renderInvocation(args, '/unused', env)).toEqual({ file: process.execPath, args: ['render', path.resolve(args[0]!), '-o', path.resolve(args[1]!), ...args.slice(2)] });
  expect(() => renderInvocation(['a.pmv'], '/unused', env)).toThrow('Usage:');
  expect(() => renderInvocation(['a.pmv', 'a.mov', '-o', 'b.mov'], '/unused', env)).toThrow('second argument');
  expect(() => renderInvocation(['a.pmv', 'a.mov'], '/missing', {})).toThrow('native-renderer-unavailable');
});
