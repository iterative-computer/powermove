import { randomUUID } from 'node:crypto';
import { lstat, mkdir, readdir, rename, rm } from 'node:fs/promises';
import path from 'node:path';

import { agentStateRoot, agentWorkspaceRoot, safeAgentComponent } from './workspace';

/**
 * A project's agent workspace, artifacts included, lives exactly as long as
 * the project. Anything a composition uses is copied in on import, so removing
 * the workspace never changes a project; it only frees what the agent left.
 */

/** App-level runs have no project and their workspace is never removed. */
export const APP_AGENT_WORKSPACE = 'powermove-global';
/** A workspace absent from the library is left alone until it has been idle this long. */
export const ORPHAN_IDLE_MS = 7 * 24 * 60 * 60 * 1000;

const REMOVING_PREFIX = '.removing-';
const missing = (error: unknown): boolean => (error as NodeJS.ErrnoException).code === 'ENOENT';

async function list(directory: string): Promise<string[]> {
  try { return await readdir(directory); } catch (error) { if (missing(error)) return []; throw error; }
}

/** Rename out first, so a crash never leaves half a workspace that looks live. */
async function removeDirectory(directory: string, holding: string): Promise<void> {
  try {
    if (!(await lstat(directory)).isDirectory()) return;
  } catch (error) { if (missing(error)) return; throw error; }
  await mkdir(holding, { recursive: true });
  const removing = path.join(holding, `${REMOVING_PREFIX}${randomUUID()}`);
  try { await rename(directory, removing); } catch (error) { if (missing(error)) return; throw error; }
  await rm(removing, { recursive: true, force: true });
}

/** Delete a project's workspace and its saved agent sessions. */
export async function forgetProjectWorkspace(userData: string, projectId: string): Promise<void> {
  if (safeAgentComponent(projectId) === APP_AGENT_WORKSPACE) return;
  const workspace = agentWorkspaceRoot(userData, projectId);
  const holding = path.dirname(workspace);
  await removeDirectory(workspace, holding);
  await removeDirectory(agentStateRoot(workspace), holding);
}

/** Newest change anywhere below, without following links. */
async function lastTouched(fullPath: string): Promise<number> {
  const metadata = await lstat(fullPath);
  let touched = metadata.mtimeMs;
  if (metadata.isDirectory()) {
    for (const name of await list(fullPath)) {
      try { touched = Math.max(touched, await lastTouched(path.join(fullPath, name))); } catch (error) { if (!missing(error)) throw error; }
    }
  }
  return touched;
}

/**
 * Remove workspaces whose project no longer exists: deleted before this
 * cleanup existed, or by a crash between discard and removal. `liveProjectIds`
 * is every project in the library and Trash; `busy` names projects with a run.
 */
export async function sweepOrphanWorkspaces(
  userData: string,
  liveProjectIds: Iterable<string>,
  busy: ReadonlySet<string> = new Set(),
  now = Date.now()
): Promise<string[]> {
  const workspacesRoot = path.join(userData, 'Agent Workspaces');
  const live = new Set([...liveProjectIds].map((id) => safeAgentComponent(id)));
  const keep = new Set([APP_AGENT_WORKSPACE, ...[...busy].map((id) => safeAgentComponent(id))]);
  const removed: string[] = [];
  for (const name of await list(workspacesRoot)) {
    const fullPath = path.join(workspacesRoot, name);
    if (name.startsWith(REMOVING_PREFIX)) {
      await rm(fullPath, { recursive: true, force: true });
      continue;
    }
    if (name.startsWith('.') || live.has(name) || keep.has(name)) continue;
    try {
      if (!(await lstat(fullPath)).isDirectory()) continue;
      if (now - await lastTouched(fullPath) < ORPHAN_IDLE_MS) continue;
      await forgetProjectWorkspace(userData, name);
      removed.push(name);
    } catch (error) {
      console.warn(`[agent-workspaces] Could not remove ${name}: ${String(error)}`);
    }
  }
  return removed;
}
