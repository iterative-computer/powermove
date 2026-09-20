// Read-only source input. Production FileStore runs exclusively on a disposable
// copy because its normal recovery logic can quarantine damaged files.
// node --expose-gc scripts/profile-store-memory.mjs /path/to/store
import { build } from 'esbuild';
import { fork } from 'node:child_process';
import { readdir, readFile, mkdtemp, copyFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const tick = () => new Promise(resolve => setImmediate(resolve));
async function heap() {
  await tick();
  for (let i = 0; i < 3; i++) { global.gc(); await tick(); }
  return process.memoryUsage().heapUsed;
}

if (process.argv[2] === '--worker') {
  const [mode, directory, module] = process.argv.slice(3);
  const { createStore } = await import(pathToFileURL(module).href);
  const baseline = await heap();
  let owner, serialized;
  const start = performance.now();
  if (mode === 'legacy') {
    owner = new Map();
    for (const name of await readdir(directory)) if (name.endsWith('.json')) {
      owner.set(name.slice(0, -5), JSON.parse(await readFile(join(directory, name), 'utf8')));
    }
  } else { owner = createStore(directory); await owner.load(); await owner.flushAll(); }
  const loadMs = performance.now() - start;
  const retained = await heap();
  serialized = mode === 'legacy'
    ? Object.fromEntries([...owner].map(([key, value]) => [key, JSON.stringify(value)]))
    : owner.bootstrapSerialized();
  const bootstrapBytes = Object.values(serialized).reduce((n, value) => n + Buffer.byteLength(value), 0);
  process.send({ mode, heapIncrement: retained - baseline, bootstrapBytes, loadMs });
  // Keep the graph live through the observation, without printing document data.
  if (!owner) throw new Error('Missing store');
} else {
  if (!global.gc || process.argv.length !== 3) throw new Error('Use node --expose-gc profile-store-memory.mjs /path/to/store');
  const temporary = await mkdtemp(join(tmpdir(), 'powermove-store-profile-'));
  try {
    const directory = join(temporary, 'store');
    await import('node:fs/promises').then(fs => fs.mkdir(directory));
    for (const name of await readdir(process.argv[2])) if (name.endsWith('.json')) await copyFile(join(process.argv[2], name), join(directory, name));
    const module = join(temporary, 'storage.mjs');
    await build({ entryPoints: [fileURLToPath(new URL('../src/main/storage.ts', import.meta.url))], outfile: module, platform: 'node', format: 'esm', bundle: true });
    const results = [];
    for (const mode of ['legacy', 'production', 'production-reopen']) results.push(await new Promise((resolve, reject) => {
      const child = fork(fileURLToPath(import.meta.url), ['--worker', mode, directory, module], { execArgv: ['--expose-gc'], stdio: ['ignore', 'ignore', 'inherit', 'ipc'] });
      let result;
      child.on('message', value => { result = value; });
      child.on('error', reject);
      child.on('exit', code => code === 0 && result ? resolve(result) : reject(new Error(`Profile worker exited ${code}`)));
    }));
    console.log(JSON.stringify({ node: process.version, results, reductionPercent: 100 * (1 - results[1].heapIncrement / results[0].heapIncrement) }, null, 2));
  } finally { await rm(temporary, { recursive: true, force: true }); }
}
