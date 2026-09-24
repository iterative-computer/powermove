import { rm } from 'node:fs/promises';
import { resolve } from 'node:path';

const cloud = resolve(import.meta.dir, '..');
const docker = Bun.spawn(['docker', 'compose', 'down', '-v'], { cwd: cloud, stdout: 'inherit', stderr: 'inherit' });
if (await docker.exited !== 0) throw new Error('docker compose down -v failed');
const state = resolve(cloud, '.wrangler/state');
await rm(state, { recursive: true, force: true });
console.log(`Deleted Docker Postgres container and named volume, plus ${state}.`);
