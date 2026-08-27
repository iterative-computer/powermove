import { randomUUID } from 'node:crypto';
import { mkdir, readFile, readdir, rename, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';

import { LIMITS, type CodexRunRequest } from '../../shared/ipc';

export type CodexAuthority = 'project' | 'computer';

export interface AgentWorkspace {
  root: string;
  inputsDirectory: string;
  apiPackDirectory: string;
  attachmentsDirectory: string;
  referencesDirectory: string;
  internalDirectory: string;
  artifactRoot: string;
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

export function sessionPathFor(root: string, authority: CodexAuthority): string {
  return path.join(root, '.powermove', `session-v${SESSION_CONTRACT_VERSION}-${authority}.txt`);
}

export async function readSession(sessionPath: string): Promise<string | null> {
  try {
    const value = (await readFile(sessionPath, 'utf8')).trim();
    return value.length > 0 ? value : null;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') return null;
    throw error;
  }
}

export async function clearSession(sessionPath: string): Promise<void> {
  await rm(sessionPath, { force: true });
}

export async function writeSession(sessionPath: string, threadId: string): Promise<void> {
  if (!threadId.trim()) return;
  await mkdir(path.dirname(sessionPath), { recursive: true });
  const temporaryPath = `${sessionPath}.${process.pid}.${randomUUID()}.tmp`;
  await writeFile(temporaryPath, threadId.trim(), { encoding: 'utf8', mode: 0o600 });
  await rename(temporaryPath, sessionPath);
}

function imageExtension(bytes: Uint8Array): 'png' | 'jpg' {
  const png = bytes.length >= 8
    && bytes[0] === 0x89 && bytes[1] === 0x50 && bytes[2] === 0x4e && bytes[3] === 0x47
    && bytes[4] === 0x0d && bytes[5] === 0x0a && bytes[6] === 0x1a && bytes[7] === 0x0a;
  return png ? 'png' : 'jpg';
}

export async function prepareAgentWorkspace(
  req: CodexRunRequest,
  userData: string,
  authority: CodexAuthority,
  schema: Record<string, unknown>,
  options: PrepareAgentWorkspaceOptions,
  runId = safeAgentComponent(`${Math.floor(Date.now() / 1000)}-${randomUUID()}`, 'run')
): Promise<AgentWorkspace> {
  if (req.projectJSON === null) throw new Error('Autonomous runs require a project snapshot.');
  if (byteLength(req.projectJSON) > LIMITS.codexProjectJsonBytes) {
    throw new Error('The project snapshot is larger than 24 MB.');
  }
  if (!path.isAbsolute(options.extensionsDir)) {
    throw new Error('The user extensions directory must be an absolute path.');
  }

  const apiPackNames = new Set<string>();
  for (const file of options.apiPackFiles) {
    if (
      file.name.length === 0 ||
      file.name === '.' ||
      file.name === '..' ||
      path.basename(file.name) !== file.name ||
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

  const root = agentWorkspaceRoot(userData, req.projectId);
  const inputsDirectory = path.join(root, 'inputs');
  const apiPackDirectory = path.join(root, 'powermove-api');
  const attachmentsDirectory = path.join(inputsDirectory, 'attachments');
  const referencesDirectory = path.join(inputsDirectory, 'references');
  const internalDirectory = path.join(root, '.powermove');
  const artifactRoot = path.join(root, 'artifacts');
  const runDirectory = path.join(artifactRoot, runId);
  const schemaPath = path.join(internalDirectory, 'result-schema.json');
  const outputPath = path.join(internalDirectory, `result-${runId}.json`);
  const sessionPath = sessionPathFor(root, authority);

  await Promise.all([
    mkdir(apiPackDirectory, { recursive: true }),
    mkdir(attachmentsDirectory, { recursive: true }),
    mkdir(referencesDirectory, { recursive: true }),
    mkdir(internalDirectory, { recursive: true }),
    mkdir(runDirectory, { recursive: true })
  ]);
  await writeFile(path.join(inputsDirectory, 'powermove-project.json'), req.projectJSON, 'utf8');
  await writeFile(schemaPath, `${JSON.stringify(schema, null, 2)}\n`, 'utf8');

  for (const file of apiPackFiles) {
    await writeFile(path.join(apiPackDirectory, file.name), file.text, 'utf8');
  }
  // Remove stale pack files individually (never rm -rf: a concurrent run on the
  // same project may be reading the directory).
  for (const entry of await readdir(apiPackDirectory)) {
    if (!apiPackNames.has(entry)) await rm(path.join(apiPackDirectory, entry), { force: true });
  }

  for (const [index, attachment] of req.attachments.entries()) {
    if (index >= LIMITS.codexAttachments) break;
    if (attachment.data.byteLength > LIMITS.codexAttachmentBytes) continue;
    const name = safeAgentComponent(attachment.name, `attachment-${index}`);
    await writeFile(path.join(attachmentsDirectory, name), attachment.data);
  }

  const imagePaths: string[] = [];
  for (const [index, image] of req.images.entries()) {
    if (index >= LIMITS.codexImages) break;
    if (image.byteLength > LIMITS.codexImageBytes) continue;
    const imagePath = path.join(referencesDirectory, `reference-${index}.${imageExtension(image)}`);
    await writeFile(imagePath, image);
    imagePaths.push(imagePath);
  }

  return {
    root,
    inputsDirectory,
    apiPackDirectory,
    attachmentsDirectory,
    referencesDirectory,
    internalDirectory,
    artifactRoot,
    extensionsDir: options.extensionsDir,
    runId,
    runDirectory,
    schemaPath,
    outputPath,
    sessionPath,
    imagePaths
  };
}

export async function discardPartialRun(workspace: Pick<AgentWorkspace, 'runDirectory'>): Promise<void> {
  await rm(workspace.runDirectory, { recursive: true, force: true });
}
