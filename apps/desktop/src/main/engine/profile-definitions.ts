/** The app's winning definitions, as a `pm-cli render --definitions` snapshot
 * (contract 10), kept in the profile for `powermove render` (plan section 22,
 * "The CLI"; amendment 24). A project file does not carry the definitions it
 * was made with, so a render outside the app would otherwise draw only the
 * built-in set: a user's extensions, overrides by id and forks of built-ins
 * would fail by name instead of drawing. The desktop writes the set its
 * windows pushed last, the hidden host the set its document worker published.
 * `powermove render` passes it when the caller gives no `--definitions`. */
import { randomUUID } from 'node:crypto';
import { mkdir, open, rename, unlink } from 'node:fs/promises';
import path from 'node:path';
import type { DefinitionSnapshot } from '../../shared/engine-protocol';

export const profileDefinitionsPath = (userData: string): string => path.join(userData, 'Engine Cache', 'definitions.json');

/** pm-protocol `DefinitionSetSnapshot`. */
export type ProfileDefinitions = { generation: number; definitions: DefinitionSnapshot[]; missing: string[] };

/** Writes the latest set atomically (owner-only), coalescing bursts of
 *  updates; an unchanged set is not written again. */
export class ProfileDefinitionsWriter {
  private pending: ProfileDefinitions | null = null;
  private written = '';
  private timer: ReturnType<typeof setTimeout> | null = null;
  private writing: Promise<void> = Promise.resolve();

  constructor(private readonly file: string, private readonly log: (message: string) => void = () => {}, private readonly delayMs = 250) {}

  write(set: ProfileDefinitions): void {
    this.pending = set;
    if (this.timer) return;
    this.timer = setTimeout(() => { this.timer = null; void this.flush(); }, this.delayMs);
    this.timer.unref?.();
  }

  /** Writes what is pending now; resolves once it is on disk. */
  flush(): Promise<void> {
    if (this.timer) { clearTimeout(this.timer); this.timer = null; }
    const set = this.pending;
    this.pending = null;
    if (set) this.writing = this.writing.then(() => this.commit(set)).catch((error) => this.log(`the profile definitions snapshot was not written: ${error instanceof Error ? error.message : String(error)}`));
    return this.writing;
  }

  private async commit(set: ProfileDefinitions): Promise<void> {
    const bytes = JSON.stringify(set);
    if (bytes === this.written) return;
    await mkdir(path.dirname(this.file), { recursive: true });
    const temporary = path.join(path.dirname(this.file), `.definitions.${randomUUID()}.tmp`);
    try {
      const handle = await open(temporary, 'wx', 0o600);
      try { await handle.writeFile(bytes); await handle.sync(); } finally { await handle.close(); }
      await rename(temporary, this.file);
      this.written = bytes;
    } catch (error) {
      await unlink(temporary).catch(() => undefined);
      throw error;
    }
  }
}
