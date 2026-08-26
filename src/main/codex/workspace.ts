import { randomUUID } from 'node:crypto';
import { mkdir, readFile, rename, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';

import { LIMITS, type CodexRunRequest } from '../../shared/ipc';

export type CodexAuthority = 'project' | 'computer';

export interface AgentWorkspace {
  root: string;
  inputsDirectory: string;
  attachmentsDirectory: string;
  referencesDirectory: string;
  internalDirectory: string;
  artifactRoot: string;
  runId: string;
  runDirectory: string;
  schemaPath: string;
  outputPath: string;
  sessionPath: string;
  imagePaths: string[];
}

const byteLength = (value: string): number => Buffer.byteLength(value, 'utf8');

export function safeAgentComponent(value: string, fallback = 'project'): string {
  const cleaned = value.replace(/[^A-Za-z0-9_-]+/g, '-').replace(/-+/g, '-').replace(/^-+|-+$/g, '');
  return (cleaned || fallback).slice(0, 120);
}

export function agentWorkspaceRoot(userData: string, projectId: string): string {
  return path.join(userData, 'Agent Workspaces', safeAgentComponent(projectId));
}

export function sessionPathFor(root: string, authority: CodexAuthority): string {
  return path.join(root, '.powermove', `session-${authority}.txt`);
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
  runId = safeAgentComponent(`${Math.floor(Date.now() / 1000)}-${randomUUID()}`, 'run')
): Promise<AgentWorkspace> {
  if (req.projectJSON === null) throw new Error('Autonomous runs require a project snapshot.');
  if (byteLength(req.projectJSON) > LIMITS.codexProjectJsonBytes) {
    throw new Error('The project snapshot is larger than 24 MB.');
  }

  const root = agentWorkspaceRoot(userData, req.projectId);
  const inputsDirectory = path.join(root, 'inputs');
  const attachmentsDirectory = path.join(inputsDirectory, 'attachments');
  const referencesDirectory = path.join(inputsDirectory, 'references');
  const internalDirectory = path.join(root, '.powermove');
  const artifactRoot = path.join(root, 'artifacts');
  const runDirectory = path.join(artifactRoot, runId);
  const schemaPath = path.join(internalDirectory, 'result-schema.json');
  const outputPath = path.join(internalDirectory, `result-${runId}.json`);
  const sessionPath = sessionPathFor(root, authority);

  await Promise.all([
    mkdir(attachmentsDirectory, { recursive: true }),
    mkdir(referencesDirectory, { recursive: true }),
    mkdir(internalDirectory, { recursive: true }),
    mkdir(runDirectory, { recursive: true })
  ]);
  await writeFile(path.join(inputsDirectory, 'powermove-project.json'), req.projectJSON, 'utf8');
  await writeFile(schemaPath, `${JSON.stringify(schema, null, 2)}\n`, 'utf8');

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
    attachmentsDirectory,
    referencesDirectory,
    internalDirectory,
    artifactRoot,
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
