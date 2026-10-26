import { randomUUID } from 'node:crypto';
import { lstat, mkdir, readdir, rename, rm } from 'node:fs/promises';
import path from 'node:path';
import { renameSync } from 'node:fs';

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
  const workspacesRoot = path.dirname(agentWorkspaceRoot(userData, 'project'));
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

/** Generated dependency and API caches can be recreated. Never remove user
 * artifacts, source, attachments, session state or unfinished extension edits. */
export async function cleanWorkspaceCaches(userData: string, anyBusy: () => boolean, manual = false, now = Date.now(), dryRun = false): Promise<number> {
  const root = path.dirname(agentWorkspaceRoot(userData, 'project'));
  if (anyBusy()) return 0;
  try { if (!(await lstat(root)).isDirectory()) return 0; } catch (error) { if (missing(error)) return 0; throw error; }
  let bytes = 0;
  const size = async (file: string): Promise<number> => {
    const entry = await lstat(file);
    if (entry.isSymbolicLink()) return 0;
    if (entry.isFile()) return entry.size;
    let total = 0;
    if (entry.isDirectory()) for (const name of await list(file)) total += await size(path.join(file, name));
    return total;
  };
  for (const name of await list(root)) {
    if (name.startsWith('.') || name === APP_AGENT_WORKSPACE || anyBusy()) continue;
    const workspace = path.join(root, name);
    if (!(await lstat(workspace)).isDirectory()) continue;
    if (now - await lastTouched(workspace) < (manual ? 60 * 60 * 1000 : ORPHAN_IDLE_MS)) continue;
    for (const cache of ['powermove-api', 'node_modules', '__pycache__', '.pytest_cache']) {
      const file = path.join(workspace, cache);
      try {
        const entry = await lstat(file);
        if (!entry.isDirectory()) continue;
        const count = await size(file);
        // Busy is checked immediately before a synchronous detach. New runs
        // see an empty cache, never a directory being removed beneath them.
        if (anyBusy()) return bytes;
        if (dryRun) { bytes += count; continue; }
        const holding = path.join(root, `${REMOVING_PREFIX}${randomUUID()}`);
        renameSync(file, holding);
        await rm(holding, { recursive: true, force: true });
        bytes += count;
      } catch (error) { if (!missing(error)) throw error; }
    }
  }
  return bytes;
}
