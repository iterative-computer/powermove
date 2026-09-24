import * as fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { afterEach, describe, expect, it, vi } from 'vitest';

const electronMocks = vi.hoisted(() => ({
  getAllWindows: vi.fn(() => []),
  showItemInFolder: vi.fn()
}));

vi.mock('electron', () => ({
  BrowserWindow: { getAllWindows: electronMocks.getAllWindows },
  shell: { showItemInFolder: electronMocks.showItemInFolder }
}));

import type { ExtensionManifest, ExtensionsChangedEvent } from '../../shared/extensions';
import type { Store } from '../storage';
import { createExtensionRegistry, type ExtensionRegistryOptions } from './registry';
import { createProvenanceStore } from '../cloud/provenance';
import { trustLevelFor } from '../cloud/trust';

const temporaryDirectories: string[] = [];

async function temporaryDirectory(): Promise<string> {
  const directory = await fs.mkdtemp(path.join(os.tmpdir(), 'powermove-registry-'));
  temporaryDirectories.push(directory);
  return directory;
}

afterEach(async () => {
  vi.clearAllMocks();
  await Promise.all(
    temporaryDirectories.splice(0).map((directory) => fs.rm(directory, { recursive: true, force: true }))
  );
});

function manifest(id: string, overrides: Partial<ExtensionManifest> = {}): ExtensionManifest {
  return { id, name: id, version: '1.0.0', apiVersion: 1, entry: 'index.ts', ...overrides };
}

async function writeExtension(root: string, id: string, source = 'export default () => undefined'): Promise<string> {
  const directory = path.join(root, id);
  await fs.mkdir(directory, { recursive: true });
  await fs.writeFile(path.join(directory, 'manifest.json'), JSON.stringify(manifest(id)));
  await fs.writeFile(path.join(directory, 'index.ts'), source);
  return directory;
}

function memoryStore(initial: Record<string, unknown> = {}): Store & { values: Record<string, unknown> } {
  const values = structuredClone(initial);
  return {
    values,
    load: vi.fn(async () => undefined),
    snapshot: vi.fn(() => structuredClone(values)),
    set: vi.fn((key: string, value: unknown) => {
      values[key] = structuredClone(value);
    }),
    delete: vi.fn((key: string) => {
      delete values[key];
    }),
    flushAll: vi.fn(async () => undefined),
    onError: vi.fn(() => () => undefined)
  };
}

async function harness(overrides: Partial<ExtensionRegistryOptions> = {}) {
  const root = await temporaryDirectory();
  const userDir = path.join(root, 'user');
  const buildDir = path.join(root, 'build');
  const resourcesDir = path.join(root, 'builtins');
  await Promise.all([
    fs.mkdir(userDir, { recursive: true }),
    fs.mkdir(buildDir, { recursive: true }),
    fs.mkdir(resourcesDir, { recursive: true })
  ]);
  const store = memoryStore();
  let buildNumber = 0;
  const compile = vi.fn(async ({ dir, outDir }: { dir: string; entry: string; outDir: string }) => {
    buildNumber += 1;
    const outputDirectory = path.join(outDir, path.basename(dir));
    const bundlePath = path.join(outputDirectory, 'bundle.js');
    await fs.mkdir(outputDirectory, { recursive: true });
    await fs.writeFile(bundlePath, `bundle-${buildNumber}`);
    return { ok: true as const, bundlePath, hash: `hash${buildNumber}` };
  });
  const events: ExtensionsChangedEvent[] = [];
  const options: ExtensionRegistryOptions = {
    store,
    userDir,
    buildDir,
    builtinIds: [],
    resourcesDir,
    compile,
    broadcast: (event) => events.push(event),
    ...overrides
  };
  return {
    root,
    userDir,
    buildDir,
    resourcesDir,
    store: options.store as Store & { values: Record<string, unknown> },
    compile,
    events,
    registry: createExtensionRegistry(options)
  };
}

describe('extension registry', () => {
  it('uses serve-style provenance with no account to keep a full-access Store install off', async () => {
    const setup = await harness();
    const provenance = createProvenanceStore(setup.root);
    await writeExtension(setup.userDir, 'third-party');
    await fs.writeFile(path.join(setup.userDir, 'third-party', 'manifest.json'), JSON.stringify(manifest('third-party', { apiVersion: 3, permissions: ['full-access'] })));
    await provenance.update('third-party', () => ({ localId: 'third-party', envKey: 'external', origin: {
      repoId: 'external', releaseId: 'release', coordinate: 'other/third-party', version: '1.0.0', treeSha: 'tree', commitSha: 'commit', ownerPublisherId: 'someone-else'
    } }));
    const registry = createExtensionRegistry({ store: setup.store, userDir: setup.userDir, buildDir: setup.buildDir,
      builtinIds: [], resourcesDir: setup.resourcesDir, compile: setup.compile,
      trustFor: async (id, scope) => trustLevelFor({ scope }, scope === 'user' ? await provenance.get(id) : null, null) });
    await registry.refresh();
    expect(registry.list()[0]).toMatchObject({ trust: 'store', bundleUrl: null, health: { state: 'needs-trust' } });
  });
  it('loads enabled intent, persists toggles, and emits one change event', async () => {
    const store = memoryStore({ extensions: { 'sample-ext': false, malformed: 'no' } });
    const setup = await harness({ store });
    await writeExtension(setup.userDir, 'sample-ext');

    await setup.registry.refresh();
    expect(setup.registry.list()[0]).toMatchObject({
      id: 'sample-ext',
      enabled: false,
      health: { state: 'disabled' }
    });

    await setup.registry.setEnabled({ id: 'sample-ext', enabled: true });
    expect(store.values.extensions).toEqual({ 'sample-ext': true });
    expect(setup.registry.list()[0]).toMatchObject({ enabled: true, health: { state: 'ok' } });
    expect(setup.events).toEqual([{ ids: ['sample-ext'], reason: 'enable' }]);
  });

  it('uses the recursive mtime signature to avoid unchanged rebuilds', async () => {
    const setup = await harness();
    const directory = await writeExtension(setup.userDir, 'cached-ext');

    await setup.registry.refresh();
    await setup.registry.refresh();
    expect(setup.compile).toHaveBeenCalledTimes(1);

    await fs.mkdir(path.join(directory, '.forked-from'));
    await fs.writeFile(path.join(directory, '.forked-from', 'index.ts'), 'pristine base changed');
    await setup.registry.refresh(['cached-ext']);
    expect(setup.compile).toHaveBeenCalledTimes(1);

    await fs.writeFile(path.join(directory, 'index.ts'), 'export default () => "changed and longer"');
    await setup.registry.refresh(['cached-ext']);
    expect(setup.compile).toHaveBeenCalledTimes(2);
    expect(setup.registry.list()[0]?.bundleUrl).toContain('?v=hash2');
  });

  it('marks a stale user fork and clears the update after its fork base catches up', async () => {
    const setup = await harness({ builtinIds: ['builtin-tools'] });
    const builtinDirectory = await writeExtension(setup.resourcesDir, 'builtin-tools');
    await fs.writeFile(
      path.join(builtinDirectory, 'manifest.json'),
      JSON.stringify(manifest('builtin-tools', { version: '2.0.0' }))
    );
    const userDirectory = await writeExtension(setup.userDir, 'custom-tools');
    await fs.writeFile(
      path.join(userDirectory, 'manifest.json'),
      JSON.stringify(manifest('custom-tools', { forkedFrom: 'builtin-tools@1.0.0', replaces: ['builtin-tools'] }))
    );

    await setup.registry.refresh();
    expect(setup.registry.list().find((record) => record.id === 'custom-tools')?.update).toEqual({
      forkedFrom: 'builtin-tools',
      base: '1.0.0',
      current: '2.0.0'
    });

    await fs.writeFile(
      path.join(userDirectory, 'manifest.json'),
      JSON.stringify(manifest('custom-tools', { forkedFrom: 'builtin-tools@2.0.0', replaces: ['builtin-tools'] }))
    );
    await setup.registry.refresh();
    expect(setup.registry.list().find((record) => record.id === 'custom-tools')?.update).toBeUndefined();
  });

  it('carries stale-fork updates on user build errors but never marks project forks', async () => {
    const projectDir = await temporaryDirectory();
    const compile = vi.fn(async () => ({ ok: false as const, error: 'broken build' }));
    const setup = await harness({ builtinIds: ['builtin-tools'], projectDirs: [projectDir], compile });
    const builtinDirectory = await writeExtension(setup.resourcesDir, 'builtin-tools');
    await fs.writeFile(
      path.join(builtinDirectory, 'manifest.json'),
      JSON.stringify(manifest('builtin-tools', { version: '2.0.0' }))
    );
    for (const [root, id] of [[setup.userDir, 'user-tools'], [projectDir, 'project-tools']] as const) {
      const directory = await writeExtension(root, id);
      await fs.writeFile(
        path.join(directory, 'manifest.json'),
        JSON.stringify(manifest(id, { forkedFrom: 'builtin-tools@1.0.0', replaces: ['builtin-tools'] }))
      );
    }

    await setup.registry.refresh();
    expect(setup.registry.list().find((record) => record.id === 'user-tools')).toMatchObject({
      health: { state: 'build-error' },
      update: { forkedFrom: 'builtin-tools', base: '1.0.0', current: '2.0.0' }
    });
    expect(setup.registry.list().find((record) => record.id === 'project-tools')?.update).toBeUndefined();
  });

  it('creates a validated extension atomically, refreshes it, and announces creation', async () => {
    const setup = await harness();

    const records = await setup.registry.create({
      manifest: manifest('created-ext', { description: 'Created safely' }),
      files: { 'index.ts': 'export default () => undefined', 'ui/panel.ts': 'export const panel = 1' }
    });

    expect(records[0]).toMatchObject({ id: 'created-ext', scope: 'user', health: { state: 'ok' } });
    expect(JSON.parse(await fs.readFile(path.join(setup.userDir, 'created-ext', 'manifest.json'), 'utf8'))).toMatchObject({
      id: 'created-ext',
      description: 'Created safely'
    });
    expect(await fs.readFile(path.join(setup.userDir, 'created-ext', 'ui/panel.ts'), 'utf8')).toContain('panel');
    expect(setup.events).toEqual([{ ids: ['created-ext'], reason: 'create' }]);
  });

  it('rejects traversal, generated-manifest replacement, missing entries, and binary text', async () => {
    const setup = await harness();
    const base = manifest('unsafe-ext');

    await expect(setup.registry.create({ manifest: base, files: { '../escape.ts': 'no', 'index.ts': 'ok' } })).rejects.toThrow(
      'Invalid extension file path'
    );
    await expect(setup.registry.create({ manifest: base, files: { 'manifest.json': '{}', 'index.ts': 'ok' } })).rejects.toThrow(
      'manifest.json is generated'
    );
    await expect(setup.registry.create({ manifest: base, files: { 'other.ts': 'no entry' } })).rejects.toThrow('entry file is missing');
    await expect(setup.registry.create({ manifest: base, files: { 'index.ts': 'bad\0data' } })).rejects.toThrow('not text');
    const fourHundredFiles = Object.fromEntries(
      Array.from({ length: 400 }, (_, index) => [index === 0 ? 'index.ts' : `file-${index}.ts`, 'text'])
    );
    await expect(setup.registry.create({ manifest: base, files: fourHundredFiles })).rejects.toThrow('too many source files');
    await expect(fs.stat(path.join(setup.root, 'escape.ts'))).rejects.toMatchObject({ code: 'ENOENT' });
  });

  it('removes only a contained user extension and exposes its builtin fallback', async () => {
    const root = await temporaryDirectory();
    const userDir = path.join(root, 'user');
    const buildDir = path.join(root, 'build');
    const resourcesDir = path.join(root, 'builtins');
    await Promise.all([fs.mkdir(userDir), fs.mkdir(buildDir), fs.mkdir(resourcesDir)]);
    await writeExtension(resourcesDir, 'shared-ext');
    await writeExtension(userDir, 'shared-ext');
    const setup = await harness({ userDir, buildDir, resourcesDir, builtinIds: ['shared-ext'] });
    await setup.registry.refresh();
    expect(setup.registry.list()[0]?.scope).toBe('user');

    await setup.registry.remove({ id: 'shared-ext' });

    expect(setup.registry.list()[0]).toMatchObject({ id: 'shared-ext', scope: 'builtin', bundleUrl: null });
    await expect(fs.stat(path.join(userDir, 'shared-ext'))).rejects.toMatchObject({ code: 'ENOENT' });
    await expect(fs.stat(path.join(buildDir, 'shared-ext'))).rejects.toMatchObject({ code: 'ENOENT' });
    expect(setup.events).toEqual([{ ids: ['shared-ext'], reason: 'remove' }]);
    expect((await setup.registry.readSource({ id: 'shared-ext' })).map((file) => file.path)).toContain('index.ts');
    await expect(setup.registry.remove({ id: 'shared-ext' })).rejects.toThrow('Only user extensions');
  });

  it('preflights a hostile build symlink before deleting user source', async () => {
    const setup = await harness();
    const source = await writeExtension(setup.userDir, 'linked-build');
    const outside = path.join(setup.root, 'outside-build');
    await fs.mkdir(outside);
    await setup.registry.refresh();
    await fs.rm(path.join(setup.buildDir, 'linked-build'), { recursive: true });
    await fs.symlink(outside, path.join(setup.buildDir, 'linked-build'));

    await expect(setup.registry.remove({ id: 'linked-build' })).rejects.toThrow('symbolic link');
    expect(await fs.readFile(path.join(source, 'index.ts'), 'utf8')).toContain('export default');
  });

  it('refuses removal when a discovered user record resolves outside the user root', async () => {
    const setup = await harness();
    const outside = path.join(setup.root, 'outside-ext');
    await writeExtension(setup.root, 'outside-ext');
    const unsafe = createExtensionRegistry({
      store: setup.store,
      userDir: setup.userDir,
      buildDir: setup.buildDir,
      resourcesDir: setup.resourcesDir,
      builtinIds: [],
      compile: setup.compile,
      scan: vi.fn(async () => [{
        id: 'outside-ext', scope: 'user' as const, dir: outside, manifest: manifest('outside-ext')
      }]),
      broadcast: vi.fn()
    });
    await unsafe.refresh();

    await expect(unsafe.remove({ id: 'outside-ext' })).rejects.toThrow(/escapes|immediate child/);
    expect(await fs.readFile(path.join(outside, 'index.ts'), 'utf8')).toContain('export default');
  });

  it('reads bounded text source while skipping binaries, oversized files, and symlinks', async () => {
    const setup = await harness();
    const directory = await writeExtension(setup.userDir, 'source-ext');
    await fs.writeFile(path.join(directory, 'notes.txt'), 'hello');
    await fs.writeFile(path.join(directory, 'binary.dat'), Buffer.from([1, 0, 2]));
    await fs.writeFile(path.join(directory, 'large.txt'), 'x'.repeat(64 * 1024 + 1));
    await fs.symlink(path.join(setup.root, 'outside.txt'), path.join(directory, 'link.txt'));
    await fs.writeFile(path.join(setup.root, 'outside.txt'), 'secret');
    await fs.mkdir(path.join(directory, '.forked-from'));
    await fs.writeFile(path.join(directory, '.forked-from', 'manifest.json'), 'pristine');
    await setup.registry.refresh();

    const files = await setup.registry.readSource({ id: 'source-ext' });
    expect(files.map((file) => file.path).sort()).toEqual(['index.ts', 'manifest.json', 'notes.txt']);
    expect(files.find((file) => file.path === 'notes.txt')?.text).toBe('hello');
  });

  it('keeps fork provenance in listed records', async () => {
    const setup = await harness();
    const directory = await writeExtension(setup.userDir, 'timeline-fork');
    await fs.writeFile(path.join(directory, 'manifest.json'), JSON.stringify(manifest('timeline-fork', {
      name: 'Timeline (fork)',
      forkedFrom: 'timeline@1.0.0',
      replaces: ['timeline']
    })));

    await setup.registry.refresh();

    expect(setup.registry.list()[0]?.manifest).toMatchObject({
      forkedFrom: 'timeline@1.0.0',
      replaces: ['timeline']
    });
  });

  it('validates health reports and reveals only a verified record directory', async () => {
    const revealPath = vi.fn();
    const setup = await harness({ revealPath });
    const directory = await writeExtension(setup.userDir, 'healthy-ext');
    await setup.registry.refresh();

    setup.registry.reportHealth({ id: 'healthy-ext', health: { state: 'runtime-error', error: 'boom' } });
    expect(setup.registry.list()[0]).toMatchObject({ enabled: false, health: { state: 'runtime-error', error: 'boom' } });
    setup.registry.reportHealth({ id: 'healthy-ext', health: { state: 'ok' } });
    expect(setup.registry.list()[0]?.health.state).toBe('runtime-error');
    await setup.registry.refresh();
    expect(setup.registry.list()[0]?.enabled).toBe(false);
    await setup.registry.setEnabled({ id: 'healthy-ext', enabled: true });
    expect(setup.registry.list()[0]).toMatchObject({ enabled: true, health: { state: 'ok' } });
    expect(() => setup.registry.reportHealth({ id: '../bad', health: { state: 'ok' } })).toThrow('Invalid extension id');
    await setup.registry.reveal({ id: 'healthy-ext' });
    expect(revealPath).toHaveBeenCalledWith(directory);
  });
});

it('does not overwrite an auto-disable received while compilation is pending', async () => {
  let release!: () => void;
  let entered!: () => void;
  const pending = new Promise<void>(resolve => { release = resolve; });
  const started = new Promise<void>(resolve => { entered = resolve; });
  let calls = 0;
  const setup = await harness({ compile: async () => {
    if (++calls === 2) { entered(); await pending; }
    return { ok: true, bundlePath: '/bundle.js', hash: String(calls) };
  } });
  const directory = await writeExtension(setup.userDir, 'flaky');
  await setup.registry.refresh();
  await fs.writeFile(path.join(directory, 'index.ts'), 'export default () => "changed"');
  const refresh = setup.registry.refresh();
  await started;
  setup.registry.reportHealth({ id: 'flaky', health: { state: 'runtime-error', error: 'boom' } });
  release();
  await refresh;
  expect(setup.registry.list()[0]?.enabled).toBe(false);
});

describe('extension values', () => {
  async function writeVarsExtension(root: string, id: string): Promise<void> {
    const directory = path.join(root, id);
    await fs.mkdir(directory, { recursive: true });
    await fs.writeFile(path.join(directory, 'manifest.json'), JSON.stringify(manifest(id, {
      apiVersion: 2,
      vars: [
        { key: 'API_KEY', label: 'API key', secret: true, required: true },
        { key: 'REGION', label: 'Region' }
      ]
    })));
    await fs.writeFile(path.join(directory, 'index.ts'), 'export default () => undefined');
  }

  it('keeps a user extension with a missing required value off until it is set', async () => {
    const stored = new Map<string, { value: string | null; secret: boolean }>();
    const { resolveVars } = await import('../env/resolve');
    const resolver = vi.fn(async (_id: string, decls: NonNullable<ExtensionManifest['vars']>) => resolveVars(decls, stored));
    const setup = await harness({ resolveVars: resolver });
    await writeVarsExtension(setup.userDir, 'colour-match');

    await setup.registry.refresh();
    expect(setup.registry.list()[0]).toMatchObject({
      id: 'colour-match',
      enabled: true,
      bundleUrl: null,
      bundleHash: null,
      health: { state: 'needs-setup', missing: ['API_KEY'] }
    });
    expect(resolver).toHaveBeenCalledWith('colour-match', expect.any(Array));

    // A stale renderer report cannot flip it to ok.
    setup.registry.reportHealth({ id: 'colour-match', health: { state: 'ok' } });
    expect(setup.registry.list()[0]?.health.state).toBe('needs-setup');

    stored.set('API_KEY', { value: 'sk-test', secret: true });
    await setup.registry.refresh(['colour-match']);
    expect(setup.registry.list()[0]).toMatchObject({ health: { state: 'ok' } });
    expect(setup.registry.list()[0]?.bundleUrl).toMatch(/^app:\/\/powermove\/ext\/colour-match\/bundle\.js/);
    expect(setup.compile).toHaveBeenCalledTimes(1);

    // Undecryptable values need re-entry too.
    stored.set('REGION', { value: null, secret: true });
    await setup.registry.refresh(['colour-match']);
    expect(setup.registry.list()[0]?.health).toEqual({ state: 'needs-setup', missing: ['REGION'] });
  });

  it('without a resolver treats every value as unset, and a disabled extension stays disabled', async () => {
    const store = memoryStore({ extensions: { 'colour-match': false } });
    const setup = await harness({ store });
    await writeVarsExtension(setup.userDir, 'colour-match');
    await setup.registry.refresh();
    expect(setup.registry.list()[0]?.health).toEqual({ state: 'disabled' });
    await setup.registry.setEnabled({ id: 'colour-match', enabled: true });
    expect(setup.registry.list()[0]).toMatchObject({ bundleUrl: null, health: { state: 'needs-setup', missing: ['API_KEY'] } });
  });

  async function writeFullAccessExtension(root: string, id: string): Promise<void> {
    const directory = path.join(root, id);
    await fs.mkdir(directory, { recursive: true });
    await fs.writeFile(path.join(directory, 'manifest.json'), JSON.stringify(manifest(id, { apiVersion: 3, permissions: ['full-access'] })));
    await fs.writeFile(path.join(directory, 'index.ts'), 'export default () => undefined');
  }

  it('stamps every record with its trust level and holds back untrusted full access', async () => {
    const levels = new Map<string, 'local' | 'store' | 'store-trusted'>([['theirs', 'store'], ['mine', 'local']]);
    const trustFor = vi.fn(async (id: string) => levels.get(id) ?? 'local');
    const setup = await harness({ trustFor });
    await writeFullAccessExtension(setup.userDir, 'theirs');
    await writeFullAccessExtension(setup.userDir, 'mine');
    await writeExtension(setup.userDir, 'plain');
    levels.set('plain', 'store');

    await setup.registry.refresh();
    const byId = Object.fromEntries(setup.registry.list().map((record) => [record.id, record]));
    expect(byId['theirs']).toMatchObject({ trust: 'store', enabled: true, bundleUrl: null, bundleHash: null, health: { state: 'needs-trust' } });
    // Made here: permissions are ignored.
    expect(byId['mine']).toMatchObject({ trust: 'local', health: { state: 'ok' } });
    expect(byId['mine']?.bundleUrl).toMatch(/^app:\/\/powermove\/ext\/mine\//);
    // Someone else's, but it doesn't ask for full access: runs as today.
    expect(byId['plain']).toMatchObject({ trust: 'store', health: { state: 'ok' } });
    expect(trustFor).toHaveBeenCalledWith('theirs', 'user');

    // A stale renderer report can't turn it on.
    setup.registry.reportHealth({ id: 'theirs', health: { state: 'ok' } });
    expect(setup.registry.list().find((record) => record.id === 'theirs')?.health.state).toBe('needs-trust');

    levels.set('theirs', 'store-trusted');
    await setup.registry.refresh(['theirs']);
    expect(setup.registry.list().find((record) => record.id === 'theirs')).toMatchObject({ trust: 'store-trusted', health: { state: 'ok' } });
  });

  it('reads as someone else\'s code when trust cannot be decided', async () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => undefined);
    const setup = await harness({ trustFor: async () => { throw new Error('provenance damaged'); } });
    await writeFullAccessExtension(setup.userDir, 'unknown');
    await setup.registry.refresh();
    expect(setup.registry.list()[0]).toMatchObject({ trust: 'store', bundleUrl: null, health: { state: 'needs-trust' } });
    error.mockRestore();
  });

  it('without a resolver (powermove serve) every folder is local', async () => {
    const setup = await harness();
    await writeFullAccessExtension(setup.userDir, 'agent-made');
    await setup.registry.refresh();
    expect(setup.registry.list()[0]).toMatchObject({ trust: 'local', health: { state: 'ok' } });
  });
});
