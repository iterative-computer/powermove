import { randomUUID } from 'node:crypto';
import { lstat, mkdir, readFile, readdir, rename, rm, unlink, writeFile } from 'node:fs/promises';
import path from 'node:path';

import { EXTENSION_ID } from '../../shared/extensions';
import { LIMITS, type AgentProviderId, type CodexRunRequest } from '../../shared/ipc';
import { imageExtension } from '../image-extension';
import { prepareExtensionStage, type ExtensionStage } from './change-history';

export type CodexAuthority = 'project' | 'computer';

export interface AgentWorkspace extends ExtensionStage {
  root: string;
  inputsDirectory: string;
  apiPackDirectory: string;
  attachmentsDirectory: string;
  referencesDirectory: string;
  internalDirectory: string;
  artifactRoot: string;
  /** Run-private extension copy. The agent never receives the live directory. */
  extensionsDir: string;
  runId: string;
  runDirectory: string;
  schemaPath: string;
  outputPath: string;
  sessionPath: string;
  imagePaths: string[];
}

export interface AgentApiPackFile {
  name: string;
  text: string;
}

export interface PrepareAgentWorkspaceOptions {
  extensionsDir: string;
  apiPackFiles: readonly AgentApiPackFile[];
  /** Main-owned child identity; keeps sibling inputs and result files apart. */
  workspaceId?: string;
}

const byteLength = (value: string): number => Buffer.byteLength(value, 'utf8');
const SESSION_CONTRACT_VERSION = 2;

export function safeAgentComponent(value: string, fallback = 'project'): string {
  const cleaned = value.replace(/[^A-Za-z0-9_-]+/g, '-').replace(/-+/g, '-').replace(/^-+|-+$/g, '');
  return (cleaned || fallback).slice(0, 120);
}

export function agentWorkspaceRoot(userData: string, projectId: string): string {
  return path.join(userData, 'Agent Workspaces', safeAgentComponent(projectId));
}

/** The app data directory that holds a workspace from agentWorkspaceRoot. */
export function agentWorkspaceUserData(root: string): string {
  return path.dirname(path.dirname(path.resolve(root)));
}

/**
 * Main-owned run files live beside the workspace, never in it: Project
 * commands can write anywhere in the workspace, so main neither trusts what
 * it reads there nor writes through a link planted there. Same volume as the
 * workspace, so scratch folders here rename in and out of it.
 */
export function agentStateRoot(root: string): string {
  return path.join(agentWorkspaceUserData(root), 'Agent State', path.basename(path.resolve(root)));
}

const SCRATCH_PREFIX = '.scratch-';
const missing = (error: unknown): boolean => (error as NodeJS.ErrnoException).code === 'ENOENT';

async function scratchFolder(root: string): Promise<string> {
  const folder = path.join(agentStateRoot(root), `${SCRATCH_PREFIX}${randomUUID()}`);
  await mkdir(folder, { recursive: true });
  return folder;
}

async function writeAtomic(file: string, data: string): Promise<void> {
  await mkdir(path.dirname(file), { recursive: true });
  const temporary = `${file}.${process.pid}.${randomUUID()}.tmp`;
  try {
    await writeFile(temporary, data, { encoding: 'utf8', mode: 0o600, flag: 'wx' });
    await rename(temporary, file);
  } finally { await rm(temporary, { force: true }); }
}

/** A real folder at `directory` inside a verified parent: a link or file there is unlinked, never followed. */
async function realDirectory(directory: string): Promise<void> {
  let metadata;
  try { metadata = await lstat(directory); } catch (error) { if (!missing(error)) throw error; }
  if (metadata?.isDirectory()) return;
  if (metadata) await unlink(directory);
  await mkdir(directory);
}

/** Whether every folder from the workspace root down to `directory` is still a real folder, not a link. */
async function realDirectories(root: string, directory: string): Promise<boolean> {
  const relative = path.relative(root, directory);
  if (relative === '..' || relative.startsWith(`..${path.sep}`) || path.isAbsolute(relative)) return false;
  let current = root;
  for (const segment of relative ? relative.split(path.sep) : []) {
    current = path.join(current, segment);
    try { if (!(await lstat(current)).isDirectory()) return false; }
    catch (error) { if (missing(error)) return false; throw error; }
  }
  return true;
}

/** Rename `from` to `target`, moving aside into `scratch` whatever an agent process put there meanwhile. */
async function checkIn(from: string, target: string, scratch: string): Promise<void> {
  for (let attempt = 0; ; attempt += 1) {
    try { await rename(from, target); return; }
    catch (error) {
      const code = (error as NodeJS.ErrnoException).code ?? '';
      if (attempt === 2 || !['EEXIST', 'ENOTEMPTY', 'ENOTDIR'].includes(code)) throw error;
      try { await rename(target, path.join(scratch, `displaced-${attempt}`)); } catch (moved) { if (!missing(moved)) throw moved; }
    }
  }
}

/**
 * Delete `target` from the workspace by first moving it out, where no agent
 * process can refill it or swap a folder for a link mid-delete. Skipped when
 * a folder above it is no longer the real one: then the path leads elsewhere.
 */
async function discardWithin(root: string, target: string): Promise<void> {
  if (!await realDirectories(root, path.dirname(target))) return;
  const scratch = await scratchFolder(root);
  try {
    try { await rename(target, path.join(scratch, 'discarded')); } catch (error) { if (!missing(error)) throw error; }
  } finally { await rm(scratch, { recursive: true, force: true }); }
}

/* Two preparations of one workspace would check the same folders out at once. */
const preparing = new Map<string, Promise<unknown>>();
function oneAtATime<T>(key: string, task: () => Promise<T>): Promise<T> {
  const next = (preparing.get(key) ?? Promise.resolve()).catch(() => undefined).then(task);
  preparing.set(key, next);
  void next.catch(() => undefined).then(() => { if (preparing.get(key) === next) preparing.delete(key); });
  return next;
}

/** Sessions and checkpoints are main's own state (agentStateRoot), not workspace files. */
export function sessionPathFor(
  root: string,
  authority: CodexAuthority,
  threadId?: string,
  provider: AgentProviderId = 'chatgpt'
): string {
  if (threadId !== undefined && !/^[A-Za-z0-9_-]{1,120}$/.test(threadId)) throw new Error('Invalid agent thread id');
  const providerPart = provider === 'chatgpt' ? '' : `-${provider}`;
  const state = agentStateRoot(root);
  return threadId
    ? path.join(state, 'threads', threadId, `session-v${SESSION_CONTRACT_VERSION}${providerPart}-${authority}.txt`)
    : path.join(state, `session-v${SESSION_CONTRACT_VERSION}${providerPart}-${authority}.txt`);
}

/* A native session id becomes a CLI argument; anything else is dropped so it
   can never read as a flag. */
const SESSION_ID = /^[A-Za-z0-9][A-Za-z0-9_-]{0,199}$/;

export async function readSession(sessionPath: string): Promise<string | null> {
  try {
    const value = (await readFile(sessionPath, 'utf8')).trim();
    return SESSION_ID.test(value) ? value : null;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') return null;
    throw error;
  }
}

export async function clearSession(sessionPath: string): Promise<void> {
  await rm(sessionPath, { force: true });
}

export async function writeSession(sessionPath: string, threadId: string): Promise<void> {
  if (!SESSION_ID.test(threadId.trim())) return;
  await writeAtomic(sessionPath, threadId.trim());
}

const SHA256 = /^[0-9a-f]{64}$/;
const isBaseline = (value: unknown): value is Record<string, string> => !!value && typeof value === 'object'
  && !Array.isArray(value) && Object.entries(value).every(([id, hash]) => EXTENSION_ID.test(id) && typeof hash === 'string' && SHA256.test(hash));

export async function prepareAgentWorkspace(
  req: CodexRunRequest,
  userData: string,
  authority: CodexAuthority,
  schema: Record<string, unknown>,
  options: PrepareAgentWorkspaceOptions,
  runId = safeAgentComponent(`${Math.floor(Date.now() / 1000)}-${randomUUID()}`, 'run')
): Promise<AgentWorkspace> {
  const projectJSON = req.projectJSON;
  if (projectJSON === null) throw new Error('Autonomous runs require a project snapshot.');
  if (byteLength(projectJSON) > LIMITS.codexProjectJsonBytes) {
    throw new Error(`The project snapshot exceeds the ${LIMITS.codexProjectJsonBytes / (1024 * 1024)} MiB agent limit.`);
  }
  if (!path.isAbsolute(options.extensionsDir)) {
    throw new Error('The user extensions directory must be an absolute path.');
  }

  /* Pack names are relative POSIX paths (`api.ts`, `samples/x/index.ts`).
     Anything absolute, escaping, or with empty segments is skipped. */
  const isSafePackPath = (name: string): boolean => {
    if (path.isAbsolute(name) || name.includes('\\')) return false;
    const segments = name.split('/');
    return segments.every((segment) => segment.length > 0 && segment !== '.' && segment !== '..');
  };
  const apiPackNames = new Set<string>();
  for (const file of options.apiPackFiles) {
    if (
      file.name.length === 0 ||
      file.name === '.' ||
      file.name === '..' ||
      !isSafePackPath(file.name) ||
      file.name.includes('\0') ||
      apiPackNames.has(file.name)
    ) {
      continue; // skip malformed pack entries; the run proceeds without them
    }
    apiPackNames.add(file.name);
  }
  const seen = new Set<string>();
  const apiPackFiles = options.apiPackFiles.filter((file) => {
    if (!apiPackNames.has(file.name) || seen.has(file.name)) return false;
    seen.add(file.name);
    return true;
  });

  const root = agentWorkspaceRoot(userData, options.workspaceId ?? req.projectId);
  return oneAtATime(root, async () => {
    const checkpointPath = `${sessionPathFor(root, authority, req.threadId, req.provider ?? 'chatgpt')}.checkpoint.json`;
    let checkpoint: ExtensionStage | null = null;
    try {
      const saved = JSON.parse(await readFile(checkpointPath, 'utf8')) as ExtensionStage;
      if (!saved || typeof saved.runId !== 'string' || !/^[A-Za-z0-9_-]+$/.test(saved.runId)
        || saved.projectId !== req.projectId || saved.liveDirectory !== options.extensionsDir
        || saved.stagingDirectory !== path.join(root, '.powermove', 'extension-runs', saved.runId)
        || saved.historyRoot !== path.join(userData, 'Agent Change History', safeAgentComponent(req.projectId))
        || typeof saved.baselineRootHash !== 'string' || !SHA256.test(saved.baselineRootHash) || !isBaseline(saved.baselineHashes)) {
        throw new Error('The saved agent checkpoint is invalid; its files have been preserved.');
      }
      // A stage that is gone, or now a link, is not resumed.
      if (await realDirectories(root, saved.stagingDirectory)) {
        checkpoint = saved;
        runId = saved.runId;
      }
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error;
    }
    const state = agentStateRoot(root);
    const inputsDirectory = path.join(root, 'inputs');
    const apiPackDirectory = path.join(root, 'powermove-api');
    const attachmentsDirectory = path.join(inputsDirectory, 'attachments');
    const referencesDirectory = path.join(inputsDirectory, 'references');
    const internalDirectory = path.join(root, '.powermove');
    const artifactRoot = path.join(root, 'artifacts');
    const runDirectory = path.join(artifactRoot, runId);
    const extensionRunsDirectory = path.join(internalDirectory, 'extension-runs');
    const extensionsDir = path.join(extensionRunsDirectory, runId);
    const historyRoot = path.join(userData, 'Agent Change History', safeAgentComponent(req.projectId));
    // Codex reads the schema and writes the result itself, outside its sandbox.
    const schemaPath = path.join(state, 'result-schema.json');
    const outputPath = path.join(state, `result-${runId}.json`);
    const sessionPath = sessionPathFor(root, authority, req.threadId, req.provider ?? 'chatgpt');

    /* Main writes the workspace only by renaming finished folders and files
       into it. Its root is app-owned, so a rename there acts on the entry
       itself; below it, folders are checked to be real first. */
    await mkdir(root, { recursive: true });
    await mkdir(state, { recursive: true });
    // Scratch a crash left behind; a discard running now keeps its fresh one.
    for (const entry of await readdir(state)) {
      if (!entry.startsWith(SCRATCH_PREFIX)) continue;
      const scratch = path.join(state, entry);
      if ((await lstat(scratch)).mtimeMs < Date.now() - 60_000) await rm(scratch, { recursive: true, force: true });
    }
    await writeAtomic(schemaPath, `${JSON.stringify(schema, null, 2)}\n`);
    const scratch = await scratchFolder(root);
    try {
      const incoming = path.join(scratch, 'incoming');
      const staged: string[] = ['powermove-project.json'];
      await mkdir(path.join(incoming, 'attachments'), { recursive: true });
      await mkdir(path.join(incoming, 'references'), { recursive: true });
      await writeFile(path.join(incoming, 'powermove-project.json'), projectJSON, 'utf8');
      for (const [index, attachment] of req.attachments.entries()) {
        if (index >= LIMITS.codexAttachments) break;
        const name = path.join('attachments', safeAgentComponent(attachment.name, `attachment-${index}`));
        await writeFile(path.join(incoming, name), attachment.data);
        staged.push(name);
      }
      const imagePaths: string[] = [];
      for (const [index, image] of req.images.entries()) {
        if (index >= LIMITS.codexImages) break;
        const name = path.join('references', `reference-${index}.${imageExtension(image)}`);
        await writeFile(path.join(incoming, name), image);
        staged.push(name);
        imagePaths.push(path.join(inputsDirectory, name));
      }
      // Other threads' inputs stay; the folder is edited outside and put back.
      const held = path.join(scratch, 'inputs');
      try { await rename(inputsDirectory, held); } catch (error) { if (!missing(error)) throw error; }
      await realDirectory(held);
      await realDirectory(path.join(held, 'attachments'));
      await realDirectory(path.join(held, 'references'));
      for (const name of staged) {
        await rm(path.join(held, name), { recursive: true, force: true });
        await rename(path.join(incoming, name), path.join(held, name));
      }
      await checkIn(held, inputsDirectory, scratch);

      // The pack is rebuilt whole: stale files go, and nothing left inside survives.
      const pack = path.join(scratch, 'powermove-api');
      await mkdir(pack);
      for (const file of apiPackFiles) {
        const target = path.join(pack, file.name);
        await mkdir(path.dirname(target), { recursive: true });
        await writeFile(target, file.text, 'utf8');
      }
      try { await rename(apiPackDirectory, path.join(scratch, 'previous-api')); } catch (error) { if (!missing(error)) throw error; }
      await checkIn(pack, apiPackDirectory, scratch);

      await realDirectory(artifactRoot);
      await realDirectory(internalDirectory);
      await realDirectory(extensionRunsDirectory);
      // Run folders carry fresh random names, so even a folder above swapped
      // for a link after the check cannot make these land on existing files.
      await realDirectory(runDirectory);
      let extensionStage = checkpoint;
      if (!extensionStage) {
        const prepared = await prepareExtensionStage({
          liveDirectory: options.extensionsDir,
          stagingDirectory: path.join(scratch, 'stage'),
          historyRoot,
          projectId: req.projectId,
          runId
        });
        await checkIn(prepared.stagingDirectory, extensionsDir, scratch);
        extensionStage = { ...prepared, stagingDirectory: extensionsDir };
      }

      return {
        root,
        inputsDirectory,
        apiPackDirectory,
        attachmentsDirectory,
        referencesDirectory,
        internalDirectory,
        artifactRoot,
        ...extensionStage,
        extensionsDir,
        runId,
        runDirectory,
        schemaPath,
        outputPath,
        sessionPath,
        imagePaths
      };
    } finally {
      await rm(scratch, { recursive: true, force: true });
    }
  });
}

export async function discardPartialRun(workspace: Pick<AgentWorkspace, 'root' | 'runDirectory' | 'stagingDirectory'>): Promise<void> {
  await discardWithin(workspace.root, workspace.runDirectory);
  await discardWithin(workspace.root, workspace.stagingDirectory);
}

export async function preserveCancelledRun(workspace: AgentWorkspace): Promise<void> {
  const { liveDirectory, stagingDirectory, historyRoot, projectId, runId, baselineHashes, baselineRootHash } = workspace;
  await writeAtomic(`${workspace.sessionPath}.checkpoint.json`,
    JSON.stringify({ liveDirectory, stagingDirectory, historyRoot, projectId, runId, baselineHashes, baselineRootHash }));
}

export async function discardExtensionStage(workspace: Pick<AgentWorkspace, 'root' | 'stagingDirectory'> & { sessionPath?: string }): Promise<void> {
  await discardWithin(workspace.root, workspace.stagingDirectory);
  if (workspace.sessionPath) await rm(`${workspace.sessionPath}.checkpoint.json`, { force: true });
}
