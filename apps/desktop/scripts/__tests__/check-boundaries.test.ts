import { execFile } from 'node:child_process';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { promisify } from 'node:util';

import { afterEach, describe, expect, it } from 'vitest';

const execute = promisify(execFile);
const temporaryDirectories: string[] = [];

async function fixture(source: string): Promise<{ root: string; script: string }> {
  const root = await mkdtemp(path.join(os.tmpdir(), 'powermove-boundaries-'));
  temporaryDirectories.push(root);
  await mkdir(path.join(root, 'scripts'), { recursive: true });
  await mkdir(path.join(root, 'src', 'extensions', 'fixture'), { recursive: true });
  const script = path.join(root, 'scripts', 'check-boundaries.mjs');
  await writeFile(script, await readFile(new URL('../check-boundaries.mjs', import.meta.url), 'utf8'));
  await writeFile(path.join(root, 'src', 'extensions', 'fixture', 'index.ts'), source);
  return { root, script };
}

async function run(script: string): Promise<{ code: number; stdout: string; stderr: string }> {
  try {
    const result = await execute(process.execPath, [script]);
    return { code: 0, stdout: result.stdout, stderr: result.stderr };
  } catch (error) {
    const failure = error as { code: number; stdout: string; stderr: string };
    return { code: failure.code, stdout: failure.stdout, stderr: failure.stderr };
  }
}

afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map((directory) => rm(directory, { recursive: true, force: true })));
});

describe('check-boundaries', () => {
  it('reports every legacy API occurrence with its file and line', async () => {
    const { root, script } = await fixture([
      "import type { PowermoveAPI } from 'powermove';",
      'export const oldRegistry = api.host.pm;',
      'export const oldState = api.host.state;',
      'export const direct = PM;',
      'export const somePMValue = true;'
    ].join('\n'));
    await writeFile(path.join(root, 'src', 'extensions', 'fixture', 'ignored.test.ts'), 'const testRegistry = PM;');

    const result = await run(script);

    expect(result.code).toBe(1);
    expect(result.stderr).toContain('src/extensions/fixture/index.ts:2: forbidden legacy API reference "host.pm"');
    expect(result.stderr).toContain('src/extensions/fixture/index.ts:3: forbidden legacy API reference "host.state"');
    expect(result.stderr).toContain('src/extensions/fixture/index.ts:4: forbidden legacy API reference "PM"');
    expect(result.stderr).toContain('check-boundaries: 3 violation(s)');
    expect(result.stderr).not.toContain('ignored.test.ts');
    expect(result.stderr).not.toContain('somePMValue');
  });

  it('keeps allowed imports and identifier substrings green', async () => {
    const { script } = await fixture([
      "import type { PowermoveAPI } from 'powermove';",
      "import { mount } from 'svelte';",
      "import helper from './helper';",
      'export const somePMValue = { api: true, helper, mount } satisfies Record<string, unknown>;'
    ].join('\n'));
    await writeFile(path.join(path.dirname(script), '..', 'src', 'extensions', 'fixture', 'helper.ts'), 'export default true;');

    const result = await run(script);

    expect(result).toMatchObject({ code: 0, stderr: '' });
    expect(result.stdout).toContain('check-boundaries: ok (1 extension(s))');
  });
});
