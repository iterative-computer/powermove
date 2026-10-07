import { constants } from 'node:fs';
import { copyFile, lstat, mkdir } from 'node:fs/promises';
import path from 'node:path';
import { validatedArtifactPath } from './artifacts';
import { agentWorkspaceRoot } from './workspace';

/** Child artifacts keep their random run paths, but become readable from the
 * parent project's normal artifact API. Never copy shell-created links. */
export async function copyChildArtifacts(userData: string, workspaceId: string, projectId: string, resultText: string): Promise<void> {
  let result: unknown;
  try { result = JSON.parse(resultText); } catch { return; }
  const artifacts = (result as { artifacts?: unknown[] } | null)?.artifacts;
  if (!Array.isArray(artifacts)) return;
  const source = path.join(agentWorkspaceRoot(userData, workspaceId), 'artifacts');
  const destination = path.join(agentWorkspaceRoot(userData, projectId), 'artifacts');
  for (const item of artifacts) {
    if (!item || typeof item !== 'object' || typeof (item as { path?: unknown }).path !== 'string') continue;
    const relative = (item as { path: string }).path;
    const file = await validatedArtifactPath(source, relative);
    const parts = relative.split('/');
    if (parts.some(part => !part || part === '.' || part === '..')) throw new Error('Invalid subagent artifact path.');
    const parent = path.dirname(destination);
    await mkdir(parent, { recursive: true });
    if (!(await lstat(parent)).isDirectory()) throw new Error('Project artifact workspace is not a directory.');
    let directory = destination;
    for (const part of ['', ...parts.slice(0, -1)]) {
      if (part) directory = path.join(directory, part);
      await mkdir(directory).catch(error => { if (error.code !== 'EEXIST') throw error; });
      if (!(await lstat(directory)).isDirectory()) throw new Error('Subagent artifacts cannot be copied through a link.');
    }
    await copyFile(file, path.join(destination, relative), constants.COPYFILE_EXCL);
  }
}
