import { createHash } from 'node:crypto';
import { mkdtemp, mkdir, readFile, readdir, rm, symlink, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { afterEach, describe, expect, it } from 'vitest';
import { MANIFEST_LIMITS } from '../../shared/extensions';
import { compileExtension } from './compiler';

const temporaryDirectories: string[] = [];

async function temporaryDirectory(): Promise<string> {
  const directory = await mkdtemp(path.join(os.tmpdir(), 'powermove-compiler-test-'));
  temporaryDirectories.push(directory);
  return directory;
}

async function fixture(id = 'test-extension'): Promise<{ dir: string; outDir: string }> {
  const root = await temporaryDirectory();
  const dir = path.join(root, id);
  const outDir = path.join(root, 'build');
  await mkdir(dir);
  return { dir, outDir };
}

afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map((directory) => rm(directory, { recursive: true, force: true })));
});

describe('compileExtension', () => {
  it('compiles every built-in and the teaching sample through the same forkable extension pipeline', async () => {
    const outDir = await temporaryDirectory();
    const builtInRoot = path.resolve('src/extensions');
    const builtIns = (await readdir(builtInRoot, { withFileTypes: true }))
      .filter((entry) => entry.isDirectory())
      .map((entry) => ({ id: entry.name, dir: path.join(builtInRoot, entry.name) }))
      .sort((left, right) => left.id.localeCompare(right.id));
    const extensions = [
      ...builtIns,
      { id: 'media-browser', dir: path.resolve('docs/samples/media-browser') },
      { id: 'gradient-tint', dir: path.resolve('docs/samples/gradient-tint') }
    ];

    for (const { id, dir } of extensions) {
      const manifest = JSON.parse(await readFile(path.join(dir, 'manifest.json'), 'utf8')) as { entry?: string };
      const result = await compileExtension({ dir, entry: manifest.entry ?? 'index.ts', outDir });
      if (!result.ok) throw new Error(`${id}: ${result.error}`);
      expect(result.bundlePath).toBe(path.join(outDir, id, 'bundle.js'));
    }
  });

  it('bundles TypeScript to the atomic target and returns its SHA-256 prefix', async () => {
    const { dir, outDir } = await fixture();
    await writeFile(path.join(dir, 'message.ts'), 'export const message: string = "hello";');
    await writeFile(path.join(dir, 'index.ts'), [
      "import { message } from './message';",
      'export default function activate(api: unknown) { return { api, message }; }'
    ].join('\n'));

    const result = await compileExtension({ dir, entry: 'index.ts', outDir });
    expect(result.ok).toBe(true);
    if (!result.ok) return;

    expect(result.bundlePath).toBe(path.join(outDir, 'test-extension', 'bundle.js'));
    const output = await readFile(result.bundlePath);
    expect(result.hash).toBe(createHash('sha256').update(output).digest('hex').slice(0, 16));
    expect(output.toString()).toContain('message = "hello"');
    expect(await readdir(path.dirname(result.bundlePath))).toEqual(['bundle.js']);
  });

  it('injects imported CSS into the single JavaScript bundle', async () => {
    const { dir, outDir } = await fixture('styled-extension');
    await writeFile(path.join(dir, 'panel.css'), '.compiler-css-fixture { color: rebeccapurple; }');
    await writeFile(path.join(dir, 'index.ts'), "import './panel.css'; export default () => undefined;");

    const result = await compileExtension({ dir, entry: 'index.ts', outDir });
    expect(result.ok).toBe(true);
    if (!result.ok) return;

    const output = await readFile(result.bundlePath, 'utf8');
    expect(output).toContain('document.createElement("style")');
    expect(output).toContain('.compiler-css-fixture { color: rebeccapurple; }');
    expect(await readdir(path.dirname(result.bundlePath))).toEqual(['bundle.js']);
  });

  it('compiles Svelte and replaces its runtime imports with global shims', async () => {
    const { dir, outDir } = await fixture('svelte-panel');
    await writeFile(path.join(dir, 'Panel.svelte'), [
      '<script>import { onMount } from "svelte"; import { writable } from "svelte/store";',
      'let count = $state(0); const stored = writable(1); const ready = Promise.resolve("ready"); onMount(() => count++);</script>',
      '<button onclick={() => count++}>{count}: {$stored}</button>',
      '{#await ready}<span>waiting</span>{:then text}<span>{text}</span>{/await}'
    ].join(''));
    await writeFile(path.join(dir, 'index.ts'), "import Panel from './Panel.svelte'; export default () => Panel;");

    const result = await compileExtension({ dir, entry: 'index.ts', outDir });
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    const output = await readFile(result.bundlePath, 'utf8');
    expect(output).toContain('globalThis.__powermove_runtime["svelte/internal/client"]');
    expect(output).toContain('globalThis.__powermove_runtime["svelte"]');
    expect(output).toContain('globalThis.__powermove_runtime["svelte/store"]');
    expect(output).not.toMatch(/\b(?:from\s*|import\s*)["']svelte(?:\/[^"']*)?["']/);
  });

  it('rejects traversal, absolute imports, node_modules, and symlink escapes', async () => {
    const { dir, outDir } = await fixture('safe-extension');
    const root = path.dirname(dir);
    const outside = path.join(root, 'outside.ts');
    await writeFile(outside, 'export const secret = true;');

    for (const specifier of ['../outside.ts', outside]) {
      await writeFile(path.join(dir, 'index.ts'), `import ${JSON.stringify(specifier)};`);
      const result = await compileExtension({ dir, entry: 'index.ts', outDir });
      expect(result.ok).toBe(false);
      if (!result.ok) expect(result.error).toMatch(/outside|absolute|escapes/i);
    }

    await mkdir(path.join(dir, 'node_modules'));
    await writeFile(path.join(dir, 'node_modules', 'hidden.ts'), 'export {};');
    await writeFile(path.join(dir, 'index.ts'), "import './node_modules/hidden.ts';");
    await expect(compileExtension({ dir, entry: 'index.ts', outDir })).resolves.toMatchObject({ ok: false });

    await symlink(outside, path.join(dir, 'linked.ts'));
    await writeFile(path.join(dir, 'index.ts'), "import './linked.ts';");
    const symlinkResult = await compileExtension({ dir, entry: 'index.ts', outDir });
    expect(symlinkResult.ok).toBe(false);
    if (!symlinkResult.ok) expect(symlinkResult.error).toMatch(/escapes/i);
  });

  it('does not allow fork merge-base files into the compiled import graph', async () => {
    const { dir, outDir } = await fixture('forked-extension');
    await mkdir(path.join(dir, '.forked-from'));
    await writeFile(path.join(dir, '.forked-from', 'original.ts'), 'export const original = true;');
    await writeFile(path.join(dir, 'index.ts'), "import './.forked-from/original.ts';");

    const result = await compileExtension({ dir, entry: 'index.ts', outDir });

    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.error).toMatch(/\.forked-from|escapes/i);
  });

  it('enforces the total source byte limit', async () => {
    const { dir, outDir } = await fixture('large-extension');
    await writeFile(path.join(dir, 'index.ts'), `/*${'x'.repeat(MANIFEST_LIMITS.sourceBytes)}*/`);
    const result = await compileExtension({ dir, entry: 'index.ts', outDir });
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.error).toMatch(/source size limit/i);
  });

  it('rejects an invalid extension-directory id and unsafe entry', async () => {
    const invalid = await fixture('Invalid_ID');
    await writeFile(path.join(invalid.dir, 'index.ts'), 'export {};');
    await expect(compileExtension({ ...invalid, entry: 'index.ts' })).resolves.toMatchObject({ ok: false });

    const valid = await fixture('valid-id');
    await expect(compileExtension({ ...valid, entry: '../outside.ts' })).resolves.toMatchObject({ ok: false });
  });
});
