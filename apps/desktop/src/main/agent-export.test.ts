import { mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { expect, it } from 'vitest';
import { agentExportDestination, withAgentExport } from './agent-export';

it('scopes automatic delivery to the owning tool, preserves existing files and cleans failed reservations', async () => {
  const directory = await mkdtemp(path.join(os.tmpdir(), 'pm-agent-export-'));
  const owner = {}, other = {};
  try {
    await writeFile(path.join(directory, 'Video.mp4'), 'existing');
    expect(await agentExportDestination(owner, 'Video.mp4')).toBeUndefined();
    let unused = '';
    await withAgentExport(owner, directory, async () => {
      expect(await agentExportDestination(other, 'Video.mp4')).toBeUndefined();
      await expect(withAgentExport(owner, directory, async () => {})).rejects.toThrow('already running');
      const output = await agentExportDestination(owner, 'Video.mp4');
      expect(output).toBe(path.join(directory, 'Video (2).mp4'));
      await writeFile(output!, 'finished');
      unused = (await agentExportDestination(owner, '../Escape.mp4'))!;
      expect(path.dirname(unused)).toBe(directory);
    });
    expect(await readFile(path.join(directory, 'Video.mp4'), 'utf8')).toBe('existing');
    expect(await readFile(path.join(directory, 'Video (2).mp4'), 'utf8')).toBe('finished');
    await expect(stat(unused)).rejects.toMatchObject({ code: 'ENOENT' });
    expect(await agentExportDestination(owner, 'Video.mp4')).toBeUndefined();
    await expect(withAgentExport(owner, directory, async () => { throw new Error('failed'); })).rejects.toThrow('failed');
    expect(await agentExportDestination(owner, 'Video.mp4')).toBeUndefined();
  } finally { await rm(directory, { recursive: true, force: true }); }
});
