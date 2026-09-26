import { readdirSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { PGlite } from '@electric-sql/pglite';
import { drizzle } from 'drizzle-orm/pglite';
import { sql } from 'drizzle-orm';
import { localData, tables, type Data, type Db } from '../src/db/client';
export async function pgliteData(): Promise<Data> {
  const client = new PGlite();
  for (const name of readdirSync(resolve(import.meta.dir,'../drizzle')).filter(x=>x.endsWith('.sql')).sort()) {
    const migration = readFileSync(resolve(import.meta.dir,'../drizzle',name),'utf8');
    for (const statement of migration.split('--> statement-breakpoint').map(s=>s.trim()).filter(Boolean)) await client.exec(statement);
  }
  const concrete = drizzle(client, { schema: tables }); const db = concrete as unknown as Db;
  const data = { db, authDb:()=>db, tx: <T>(fn:(tx:Db)=>Promise<T>) => concrete.transaction(tx => fn(tx as unknown as Db)), end:async()=>{}, close:()=>client.close() };
  return data;
}
export async function withData<T>(fn: (data: Data) => Promise<T>): Promise<T> {
  const data = process.env.TEST_DATABASE_URL ? localData(process.env.TEST_DATABASE_URL) : await pgliteData();
  try { if (process.env.TEST_DATABASE_URL) await data.authDb().execute(sql`truncate table "user", "session", "account", verification, publishers, repos, refs, extensions, releases, release_objects, objects, object_leases, installs, user_settings, desktop_auth, featured, abuse_counters, moderation_log, reports cascade`); return await fn(data); }
  finally { if (process.env.TEST_DATABASE_URL) await data.end(); else await (data as Data & {close():Promise<void>}).close(); }
}
