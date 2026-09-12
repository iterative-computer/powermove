import { createHash, randomUUID } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { copyFile, mkdir, open, readFile, rename, stat, unlink } from 'node:fs/promises';
import path from 'node:path';
import { LIMITS, PROJECT_ID } from '../shared/ipc';
import { decodeProjectContainer } from '../shared/project-container';

/** Replace only after every byte has reached disk; a failed write preserves the original. */
export async function atomicWrite(filePath: string, data: Uint8Array): Promise<void> {
  const temporary = path.join(path.dirname(filePath), `.${path.basename(filePath)}.${randomUUID()}.tmp`);
  try {
    const handle = await open(temporary, 'wx', 0o600);
    try { await handle.writeFile(data); await handle.sync(); } finally { await handle.close(); }
    await rename(temporary, filePath);
  } catch (error) {
    await unlink(temporary).catch(() => undefined);
    throw error;
  }
}

type Association = { path: string; hash: string };
const hash = (data: Uint8Array) => createHash('sha256').update(data).digest('hex');
const hashFile = (filePath: string): Promise<string> => new Promise((resolve, reject) => {
  const digest = createHash('sha256');
  const stream = createReadStream(filePath);
  stream.on('data', chunk => digest.update(chunk));
  stream.on('error', reject);
  stream.on('end', () => resolve(digest.digest('hex')));
});

/** Main-process-only grants: renderer data can never invent an overwrite path. */
export class ProjectFiles {
  private associations = new Map<string, Association>();
  private ready: Promise<void>;
  private queue: Promise<unknown> = Promise.resolve();
  constructor(private readonly registryPath: string) {
    this.ready = this.load();
  }
  private async load(): Promise<void> {
    try {
      const data = JSON.parse(await readFile(this.registryPath, 'utf8'));
      for (const [id, value] of Object.entries<any>(data)) {
        if (PROJECT_ID.test(id) && typeof value?.path === 'string' && path.isAbsolute(value.path)
          && typeof value.hash === 'string') this.associations.set(id, value);
      }
    } catch (error: any) {
      if (error.code !== 'ENOENT') console.warn('Project file associations unavailable; Save As will be used.', error);
    }
  }
  private async persist(): Promise<void> {
    await mkdir(path.dirname(this.registryPath), { recursive: true });
    await atomicWrite(this.registryPath, Buffer.from(JSON.stringify(Object.fromEntries(this.associations))));
  }
  async destination(id: string): Promise<string | undefined> {
    await this.ready;
    return this.associations.get(id)?.path;
  }
  private serial<T>(operation: () => Promise<T>): Promise<T> {
    const next = this.queue.then(async () => { await this.ready; return operation(); });
    this.queue = next.catch(() => undefined);
    return next;
  }
  save(id: string, data: Uint8Array, selectedPath?: string): Promise<string> {
    return this.serial(async () => {
      const known = this.associations.get(id);
      const destination = selectedPath || known?.path;
      if (!destination) throw new Error('Choose a destination with Save As.');
      let previous = false;
      let previousHash: string | undefined;
      try {
        const info = await stat(destination);
        if (!info.isFile() || info.size > LIMITS.fileSaveBytes) throw new Error('The destination cannot be safely backed up. Choose another file with Save As.');
        previous = true;
        previousHash = await hashFile(destination);
      } catch (error: any) { if (error.code !== 'ENOENT') throw error; }
      if (!selectedPath && known && (!previous || previousHash !== known.hash)) {
        throw new Error('The project file was moved, deleted, or changed outside Powermove. Use Save As to avoid overwriting other work.');
      }
      if (previous) await copyFile(destination, destination + '1');
      await atomicWrite(destination, data);
      // Other open copies of this file keep their old hash, so they cannot silently overwrite it.
      this.associations.set(id, { path: destination, hash: hash(data) });
      await this.persist();
      return destination;
    });
  }
  open(filePath: string): Promise<{ path: string; projectId: string; data: Uint8Array }> {
    return this.serial(async () => {
      const info = await stat(filePath);
      if (!info.isFile() || info.size > LIMITS.fileSaveBytes) throw new Error('Project files must be smaller than 256 MB.');
      const data = await readFile(filePath);
      const document = decodeProjectContainer(data).document;
      const project = document?.proj || document;
      if (!project || !Array.isArray(project.layers) || !Number.isFinite(project.w) || !Number.isFinite(project.h)) {
        throw new Error('This is not a Powermove project.');
      }
      // Opening a copy never replaces another open document with the same embedded id.
      const projectId = randomUUID();
      this.associations.set(projectId, { path: filePath, hash: hash(data) });
      await this.persist();
      return { path: filePath, projectId, data };
    });
  }
}
