import { expect, it } from 'vitest';
import { renderInvocation, renderProject } from './render';
import path from 'node:path';

it('passes user-chosen paths and render arguments without shell interpretation', async () => {
  const env = { ...process.env, POWERMOVE_PM_CLI: process.execPath };
  const args = ['a project.pmv', '../anywhere/a $(literal).mov', '--param', 'label="Hello world"', '--definitions', 'custom.json'];
  expect(renderInvocation(args, '/unused', env)).toEqual({ file: process.execPath, args: ['render', path.resolve(args[0]!), '-o', path.resolve(args[1]!), ...args.slice(2)] });
  expect(() => renderInvocation(['a.pmv'], '/unused', env)).toThrow('Usage:');
  expect(() => renderInvocation(['a.pmv', 'a.mov', '-o', 'b.mov'], '/unused', env)).toThrow('second argument');
  expect(() => renderInvocation(['a.pmv', 'a.mov'], '/missing', {})).toThrow('native-renderer-unavailable');
});

it('draws the profile\'s published definitions unless the caller names a set', async () => {
  const { mkdtemp, mkdir, writeFile, rm } = await import('node:fs/promises');
  const os = await import('node:os');
  const profile = await mkdtemp(path.join(os.tmpdir(), 'pm-render-defs-'));
  try {
    const env = { ...process.env, POWERMOVE_PM_CLI: process.execPath, POWERMOVE_USER_DATA: profile };
    // No snapshot yet: the built-in set, as pm-cli's default.
    expect(renderInvocation(['p.pmv', 'o.mov'], '/unused', env).args).not.toContain('--definitions');
    await mkdir(path.join(profile, 'Engine Cache'), { recursive: true });
    const file = path.join(profile, 'Engine Cache', 'definitions.json');
    await writeFile(file, '{"generation":1,"definitions":[],"missing":[]}');
    expect(renderInvocation(['p.pmv', 'o.mov', '--format', 'png'], '/unused', env).args.slice(-4)).toEqual(['--format', 'png', '--definitions', file]);
    // The caller's own set wins.
    for (const own of [['--definitions', 'mine.json'], ['--definitions=mine.json']]) {
      expect(renderInvocation(['p.pmv', 'o.mov', ...own], '/unused', env).args.filter(a => a.includes(file))).toEqual([]);
    }
  } finally { await rm(profile, { recursive: true, force: true }); }
});

