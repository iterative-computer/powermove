import { readFile } from 'node:fs/promises';
import path from 'node:path';
import type { AgentExtensionChange } from '../../shared/ipc';
import { EXTENSION_ID, parseManifest } from '../../shared/extensions';
import { compileExtension } from '../extensions/compiler';
import type { AgentWorkspace } from './workspace';
import { AgentResultValidationError } from './result-repair';

/** Check generated source with the same compiler used by the live mod loader. */
export async function validateStagedExtensions(
  layout: Pick<AgentWorkspace, 'stagingDirectory' | 'runDirectory'>,
  changes: readonly AgentExtensionChange[]
): Promise<void> {
  for (const change of changes) {
    if (!EXTENSION_ID.test(change.id)) throw new Error('Invalid staged extension id.');
    if (change.action === 'removed') continue;
    const dir = path.join(layout.stagingDirectory, change.id);
    let raw: unknown;
    try { raw = JSON.parse(await readFile(path.join(dir, 'manifest.json'), 'utf8')); }
    catch (error) {
      if (!(error instanceof SyntaxError) && (error as NodeJS.ErrnoException).code !== 'ENOENT') throw error;
      throw new AgentResultValidationError(`${change.id}: provide a valid manifest.json in the current staging directory. Nothing was published.`);
    }
    const manifest = parseManifest(raw);
    if (!manifest.ok) throw new AgentResultValidationError(`${change.id}: ${manifest.error}. Nothing was published.`);
    if (manifest.manifest.id !== change.id) throw new AgentResultValidationError(`${change.id}: the manifest id must match its folder. Nothing was published.`);
    const compiled = await compileExtension({ dir, entry: manifest.manifest.entry || 'index.ts', outDir: path.join(layout.runDirectory, '.compiled') });
    if (!compiled.ok) throw new AgentResultValidationError(`${change.id} failed compilation: ${compiled.error}. Repair its staged source. Nothing was published.`);
  }
}
