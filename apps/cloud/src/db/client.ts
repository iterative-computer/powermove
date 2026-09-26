import { neon, Pool } from '@neondatabase/serverless';
import { drizzle as neonHttp } from 'drizzle-orm/neon-http';
import { drizzle as neonWs } from 'drizzle-orm/neon-serverless';
import { drizzle as nodePostgres } from 'drizzle-orm/node-postgres';
import { Pool as PgPool } from 'pg';
import type { PgDatabase } from 'drizzle-orm/pg-core';
import type { PgQueryResultHKT } from 'drizzle-orm/pg-core/session';
import * as schema from './schema';
import * as authSchema from './auth-schema';
export const tables = { ...schema, ...authSchema };
export type Db = PgDatabase<PgQueryResultHKT, typeof tables>;
export interface Data { db: Db; tx<T>(fn: (tx: Db) => Promise<T>): Promise<T>; authDb(): Db; end(): Promise<void> }
export function neonData(databaseUrl: string): Data {
  const db = neonHttp(neon(databaseUrl), { schema: tables }) as unknown as Db;
  let pool: Pool | undefined;
  let ws: ReturnType<typeof neonWs<typeof tables>> | undefined;
  const authDb = (): Db => { pool ??= new Pool({ connectionString: databaseUrl }); ws ??= neonWs(pool, { schema: tables }); return ws as unknown as Db; };
  return { db, authDb, async tx(fn) { const writable = authDb(); return writable.transaction(tx => fn(tx as Db)); }, async end() { if (pool) await pool.end(); } };
}
export function localData(databaseUrl: string): Data {
  const pool = new PgPool({ connectionString: databaseUrl, max: 2 });
  const concrete = nodePostgres(pool, { schema: tables });
  const db = concrete as unknown as Db;
  return {
    db,
    authDb: () => db,
    tx: (fn) => concrete.transaction(tx => fn(tx as Db)),
    end: () => pool.end(),
  };
}
