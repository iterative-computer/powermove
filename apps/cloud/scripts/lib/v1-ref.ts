import { resolve } from 'node:path';

/**
 * The Electron app's tree. It ships from `main` and is not on this branch, so
 * tests and the local-dev seeder that read its built-ins or its extension
 * compiler take it from a v1 checkout. Point `PM_V1_REF` at that checkout's
 * root (default `/tmp/pm-ref-v1.2.0`, the same default the `tools/` harnesses
 * use); `apps/desktop` lives under it.
 */
export const v1Desktop = resolve(process.env.PM_V1_REF ?? '/tmp/pm-ref-v1.2.0', 'apps/desktop');
