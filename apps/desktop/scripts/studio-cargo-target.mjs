/** Cargo fingerprints are unsafe to share between concurrently edited worktrees.
 * Keep each checkout under the configured cache root, outside the repository. */
import { createHash } from 'node:crypto';
import { realpathSync } from 'node:fs';
import { homedir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export function studioCargoTarget() {
  const engine = realpathSync(path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../../engine'));
  const key = createHash('sha256').update(engine).digest('hex').slice(0, 12);
  return path.join(process.env.CARGO_TARGET_DIR ?? path.join(homedir(), 'Developer/pm-target'), 'studio', key);
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) console.log(studioCargoTarget());
