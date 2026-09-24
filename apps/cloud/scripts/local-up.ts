import { constants } from 'node:fs';
import { copyFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { Pool } from 'pg';
import { drizzle } from 'drizzle-orm/node-postgres';
import { migrate } from 'drizzle-orm/node-postgres/migrator';

const cloud = resolve(import.meta.dir, '..');
const databaseUrl = 'postgres://powermove:powermove@localhost:54329/powermove';

const docker = Bun.spawn(['docker', 'compose', 'up', '-d', '--wait'], { cwd: cloud, stdout: 'inherit', stderr: 'inherit' });
if (await docker.exited !== 0) throw new Error('docker compose up failed');

const pool = new Pool({ connectionString: databaseUrl, max: 2 });
try {
  await migrate(drizzle(pool), { migrationsFolder: resolve(cloud, 'drizzle') });
  console.log('Postgres migrations applied.');
} finally {
  await pool.end();
}

try {
  await copyFile(resolve(cloud, '.dev.vars.example'), resolve(cloud, '.dev.vars'), constants.COPYFILE_EXCL);
  console.log('Created .dev.vars from .dev.vars.example.');
} catch (error) {
  if ((error as NodeJS.ErrnoException).code !== 'EEXIST') throw error;
  console.log('Kept existing .dev.vars.');
}
console.log('Next: bun run dev:local (this terminal), then bun run local:seed --publish (another terminal).');
