import * as fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { afterEach, describe, expect, it, vi } from 'vitest';

const compileExtension = vi.hoisted(() => vi.fn(async () => ({ ok: true as const, bundlePath: '/tmp/bundle.js', hash: 'h' })));
vi.mock('../extensions/compiler', () => ({ compileExtension }));

import { AgentResultValidationError } from './result-repair';
import { validateStagedExtensions } from './validate-staged-extensions';

const roots: string[] = [];
afterEach(async () => {
  compileExtension.mockClear();
  await Promise.all(roots.splice(0).map((dir) => fs.rm(dir, { recursive: true, force: true })));
});

async function stage(files: Record<string, string | Uint8Array>) {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'powermove-stage-'));
  roots.push(root);
  const stagingDirectory = path.join(root, 'staging');
  const dir = path.join(stagingDirectory, 'weather');
  const all = {
    'manifest.json': JSON.stringify({ id: 'weather', name: 'Weather', version: '1.0.0', apiVersion: 2 }),
    'index.ts': 'export default () => undefined;\n',
    ...files
  };
  for (const [name, contents] of Object.entries(all)) {
    await fs.mkdir(path.dirname(path.join(dir, name)), { recursive: true });
    await fs.writeFile(path.join(dir, name), contents);
  }
  return { stagingDirectory, compiledDirectory: path.join(root, 'compiled') };
}

// Assembled so this test file does not itself look like it holds a key.
const FAKE_OPENAI_KEY = ['sk', 'proj', 'A1b2C3d4E5f6G7h8I9j0K1l2'].join('-');
const RANDOM = 'q8ZxT3vN7kLm2Wp9Rb4Hc6Yd1Gf5Js0A';

describe('promotion scan', () => {
  it('blocks a staged key with one line per finding and never compiles', async () => {
    const layout = await stage({
      'index.ts': `const key = '${FAKE_OPENAI_KEY}';\nexport default () => key;\n`,
      'lib/client.ts': `\n\nexport const token = "${RANDOM}";\n`,
      '.env': `OPENAI_API_KEY=${FAKE_OPENAI_KEY}\n`,
      'icon.png': new Uint8Array([0, 1, 2, 3])
    });
    const error = await validateStagedExtensions(layout, [{ id: 'weather', action: 'created' }]).catch((cause) => cause);
    expect(error).toBeInstanceOf(AgentResultValidationError);
    expect((error as Error).message.split('\n')).toEqual([
      'weather/.env:1 looks like an OpenAI API key. Declare it in manifest vars and read api.vars. Nothing was published.',
      'weather/index.ts:1 looks like an OpenAI API key. Declare it in manifest vars and read api.vars. Nothing was published.',
      'weather/lib/client.ts:3 looks like a secret. Declare it in manifest vars and read api.vars. Nothing was published.'
    ]);
    expect(compileExtension).not.toHaveBeenCalled();
  });

  it('warns about undeclared permissions but still promotes', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => undefined);
    try {
      const layout = await stage({ 'index.ts': 'fetch("https://example.com");\napi.render.draw();\n' });
      await expect(validateStagedExtensions(layout, [{ id: 'weather', action: 'created' }])).resolves.toEqual([]);
      expect(warn).toHaveBeenCalledWith(expect.stringContaining('Uses fetch() at index.ts:1'));
      expect(warn).toHaveBeenCalledWith(expect.stringContaining('Uses api.render (full access) at index.ts:2'));
      expect(compileExtension).toHaveBeenCalledTimes(1);
    } finally { warn.mockRestore(); }
  });

  it('does not warn about code that reads the project', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => undefined);
    try {
      const layout = await stage({ 'index.ts': 'api.events.on("project:changed", async () => draw(await api.project.get()));\napi.on("selection", draw);\n' });
      await expect(validateStagedExtensions(layout, [{ id: 'weather', action: 'created' }])).resolves.toEqual([]);
      expect(warn).not.toHaveBeenCalled();
      expect(compileExtension).toHaveBeenCalledTimes(1);
    } finally { warn.mockRestore(); }
  });

  it('lets a waived high-entropy string through to compilation', async () => {
    const layout = await stage({
      'index.ts': `// powermove-secret-ok: fixture hash for the test palette\nconst id = '${RANDOM}';\nexport default () => id;\n`
    });
    await expect(validateStagedExtensions(layout, [{ id: 'weather', action: 'updated' }])).resolves.toEqual([]);
    expect(compileExtension).toHaveBeenCalledTimes(1);
  });
});

describe('deprecated compatibility APIs', () => {
  it('returns each use with its file, line and replacement, and still publishes', async () => {
    const layout = await stage({
      'index.ts': "export default function activate(api) {\n  const shot = api.render.snapshot(0);\n}\n",
      'Panel.svelte': "<script>\n  let { api } = $props();\n  const live = api.media.assets.get('a');\n</script>\n<video src={live.url}></video>\n"
    });
    const warnings = await validateStagedExtensions(layout, [{ id: 'weather', action: 'updated', summary: '' }]);
    expect(warnings.map((w) => `${w.extension}/${w.file}:${w.line} ${w.member}`)).toEqual([
      'weather/index.ts:2 render.snapshot',
      'weather/Panel.svelte:5 media.assets.get().url'
    ]);
    expect(warnings[0]).toMatchObject({ replacement: 'await api.render.snapshotAsync(time, maxWidth)', access: 'static' });
    expect(compileExtension).toHaveBeenCalledTimes(1);
  });
});
