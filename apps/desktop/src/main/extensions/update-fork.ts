import { randomUUID } from 'node:crypto';
import { mkdir, mkdtemp, rename, rm } from 'node:fs/promises';
import path from 'node:path';
import type { ForkUpdateResult } from '../../shared/ipc';
import { prepareExtensionStage, publishExtensionChanges, withStageSnapshot } from '../codex/change-history';
import { AgentResultValidationError } from '../codex/result-repair';
import { validateStagedExtensions } from '../codex/validate-staged-extensions';
import { stageForkRebase } from './rebase';

export const FORK_UPDATE_PROJECT_ID = 'fork-updates';

/** No agent or network request: merge and compile locally, then promote the
 * checked snapshot through the existing guarded transaction and undo history. */
export async function updateFork(options: {
  forkId: string; userData: string; userExtensionsDir: string; builtinExtensionsDir: string;
}): Promise<ForkUpdateResult> {
  const root = path.join(options.userData, 'Fork Updates');
  await mkdir(root, { recursive: true });
  const temporary = await mkdtemp(path.join(root, 'update-'));
  try {
    const stage = await prepareExtensionStage({
      liveDirectory: options.userExtensionsDir,
      stagingDirectory: path.join(temporary, 'stage'),
      historyRoot: path.join(options.userData, 'Agent Change History', FORK_UPDATE_PROJECT_ID),
      projectId: FORK_UPDATE_PROJECT_ID,
      runId: `fork-update-${randomUUID()}`
    });
    // Read from the isolated baseline; publication refuses intervening edits.
    const rebase = await stageForkRebase({
      ...options, userExtensionsDir: stage.stagingDirectory,
      stagingDirectory: path.join(temporary, 'merge')
    });
    if (rebase.conflicts.length) return { kind: 'needs-agent', reason: 'conflict', conflicts: rebase.conflicts };
    // Keep all other extension folders in the baseline; replace only this fork.
    await rm(path.join(stage.stagingDirectory, options.forkId), { recursive: true });
    await rename(rebase.workingDir, path.join(stage.stagingDirectory, options.forkId));
    const changes = [{ id: options.forkId, action: 'updated' as const,
      summary: `Updated to ${rebase.forkedFrom} ${rebase.current}; customizations kept.` }];
    const record = await withStageSnapshot(stage, async snapshot => {
      await validateStagedExtensions(snapshot, changes);
      return publishExtensionChanges(snapshot, changes);
    }, [options.forkId]);
    if (!record) throw new Error('The extension update made no changes.');
    return { kind: 'updated', version: rebase.current,
      projectId: FORK_UPDATE_PROJECT_ID, changeSetId: record.id };
  } catch (error) {
    if (error instanceof AgentResultValidationError) return { kind: 'needs-agent', reason: 'validation', conflicts: [] };
    throw error;
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
}
