import { rm } from 'node:fs/promises';
import { resolve } from 'node:path';
import { composeEnv, stack } from './lib/local-env';

const cloud = resolve(import.meta.dir, '..');
const docker = Bun.spawn(['docker', 'compose', 'down', '-v'], { cwd: cloud, env: composeEnv, stdout: 'inherit', stderr: 'inherit' });
if (await docker.exited !== 0) throw new Error('docker compose down -v failed');
const state = resolve(cloud, '.wrangler/state');
await rm(state, { recursive: true, force: true });
await rm(resolve(cloud, `.wrangler/state-${stack}`), { recursive: true, force: true });
console.log(`Deleted Docker Postgres container and named volume, plus ${state}.`);
