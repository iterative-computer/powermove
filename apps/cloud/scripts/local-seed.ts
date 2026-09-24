import { randomBytes, randomUUID } from 'node:crypto';
import { resolve } from 'node:path';
import { Pool } from 'pg';

const cloud = resolve(import.meta.dir, '..');
const databaseUrl = 'postgres://powermove:powermove@localhost:54329/powermove';
const pool = new Pool({ connectionString: databaseUrl, max: 2 });

async function seedUser(email: string, name: string, handle: string): Promise<string> {
  const id = randomUUID();
  const user = await pool.query<{ id: string }>(
    `insert into "user" (id, name, email, email_verified, username, display_username)
     values ($1, $2, $3, true, $4, $4)
     on conflict (email) do update set email_verified = true
     returning id`,
    [id, name, email, handle],
  );
  const userId = user.rows[0]!.id;
  await pool.query(
    `insert into publishers (handle, user_id) values ($1, $2)
     on conflict (handle) do nothing`,
    [handle, userId],
  );
  const token = Buffer.from(randomBytes(32)).toString('hex');
  await pool.query(
    `insert into "session" (id, token, user_id, expires_at, created_at, updated_at)
     values ($1, $2, $3, now() + interval '90 days', now(), now())`,
    [randomUUID(), token, userId],
  );
  return token;
}

let judeToken: string;
let maraToken: string;
try {
  judeToken = await seedUser('jude@localhost', 'Jude', 'powermove');
  maraToken = await seedUser('mara@localhost', 'Mara', 'mara');
} finally {
  await pool.end();
}
console.log(`POWERMOVE_REGISTRY_TOKEN=${judeToken}`);
console.log(`MARA_REGISTRY_TOKEN=${maraToken}`);

if (process.argv.includes('--publish')) {
  const health = await fetch('http://localhost:8787/health').catch(() => null);
  if (!health?.ok) throw new Error('Worker is not reachable at http://localhost:8787. Run `bun run dev:local` before `bun run local:seed --publish`.');
  const desktop = resolve(cloud, '../desktop');
  const publisher = Bun.spawn(['bun', 'scripts/publish-builtins.mjs'], {
    cwd: desktop,
    env: { ...process.env, POWERMOVE_REGISTRY_URL: 'http://localhost:8787', POWERMOVE_REGISTRY_TOKEN: judeToken },
    stdout: 'inherit', stderr: 'inherit',
  });
  if (await publisher.exited !== 0) throw new Error('Built-ins publisher failed.');
}
