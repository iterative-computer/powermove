import { mkdir, open, stat, unlink } from 'node:fs/promises';
import path from 'node:path';

type ExportScope = { directory: string; files: string[] };
const scopes = new WeakMap<object, ExportScope>();

/** Only an authenticated project tool call grants automatic delivery. Normal
 * exports keep their save dialog. No renderer-supplied filesystem path is used. */
export async function withAgentExport<T>(owner: object, directory: string | undefined, run: () => Promise<T>): Promise<T> {
  if (!directory) return run();
  if (scopes.has(owner)) throw new Error('An agent export is already running.');
  const scope: ExportScope = { directory, files: [] };
  scopes.set(owner, scope);
  try { return await run(); }
  finally {
    scopes.delete(owner);
    for (const file of scope.files) {
      if ((await stat(file).catch(() => null))?.size === 0) await unlink(file).catch(() => {});
    }
  }
}

export async function agentExportDestination(owner: object, name: string): Promise<string | undefined> {
  const scope = scopes.get(owner);
  if (!scope) return undefined;
  let safe = name.replace(/[\\/\x00-\x1f:]/g, '_').trim().replace(/^\.+$/, '_') || 'Powermove';
  while (Buffer.byteLength(safe) > 180) safe = safe.slice(0, -1);
  const extension = path.extname(safe), base = safe.slice(0, safe.length - extension.length);
  await mkdir(scope.directory, { recursive: true });
  for (let i = 0; i < 10000; i++) {
    const destination = path.join(scope.directory, `${base}${i ? ` (${i + 1})` : ''}${extension}`);
    try {
      const file = await open(destination, 'wx', 0o600);
      await file.close();
      scope.files.push(destination);
      return destination;
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== 'EEXIST') throw error;
    }
  }
  throw new Error('Could not choose a free export filename.');
}
