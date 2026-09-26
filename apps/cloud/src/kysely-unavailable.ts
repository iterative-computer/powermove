// Better Auth imports its Kysely fallback and migration helpers even when a
// Drizzle adapter is supplied. This Worker always supplies drizzleAdapter.
// The alias keeps optional Kysely dialects out of the Worker bundle.
function unavailable(): never { throw new Error('Kysely adapter is unavailable in this Worker'); }
export const createKyselyAdapter = unavailable;
export const kyselyAdapter = unavailable;
export const getMssqlSchema = unavailable;
export const getPostgresSchema = unavailable;
export const toIntrospectedTables = unavailable;
export const toPhysicalSchema = unavailable;
export function getKyselyDatabaseType(): string { return 'unknown'; }
