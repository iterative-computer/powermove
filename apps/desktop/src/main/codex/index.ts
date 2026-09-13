import { CompatibleProvider } from '../compatible-provider';
/**
 * Bootstrap contract: call `registerCodexIpc(ipcMain, { extensionsDir,
 * apiPackFiles, ... })`. `extensionsDir` is the writable user-extension root.
 * `apiPackFiles` should return `EXTENSIONS.md`, `api.ts`, `extensions.ts`,
 * `project.ts`, and `commands.ts`, read from `app.getAppPath()` sources in
 * development or `process.resourcesPath/api-pack/` when packaged.
 */
import {
  app,
  type BrowserWindow,
  type IpcMain,
  type IpcMainInvokeEvent,
  type WebContents
} from 'electron';
import path from 'node:path';
import { readFile, readdir, stat } from 'node:fs/promises';

import {
  IPC,
  PROJECT_ID,
  REQUEST_ID,
  type ArtifactRef,
  type ChatGPTAccountStatus,
  type ClaudeAccountStatus,
  type CodexCancelRequest,
  type AgentChangeSetRestoreRequest,
  type CodexFixPromptRequest,
  type CodexRebasePromptRequest,
  type CodexRunRequest,
  type CodexSteerRequest,
  type ConsentRequest
} from '../../shared/ipc';
import { EXTENSION_ID } from '../../shared/extensions';
import { IpcValidationError, isRecord, isString } from '../../shared/guards';
import { readArtifact, revealArtifact } from './artifacts';
import { requestComputerConsent } from './consent';
import { CodexRunner, isCodexRunRequest } from './runner';
import { ChatGPTAccountClient } from './app-server-account';
import { CodexAppServerRunner } from './app-server-runner';
import { ClaudeAccountClient, ClaudeRunner } from '../claude';
import { buildFixPrompt, buildRebasePrompt } from './instructions';
import { restoreExtensionChangeSet } from './change-history';
import { agentWorkspaceRoot, safeAgentComponent, sessionPathFor, type AgentApiPackFile } from './workspace';
import { PowermoveAgentToolBridge, type PowermoveAgentToolSession } from '../agent-tools/bridge';
import { readForkRebaseInfo, stageForkRebase } from '../extensions/rebase';

const FIX_PROMPT_ERROR_CHARS = 4_000;
const FIX_PROMPT_FILES = 40;
const FIX_PROMPT_FILE_BYTES = 64 * 1024;

export interface CodexIpcContext {
  getWindow(): BrowserWindow | null;
  userData: string;
  extensionsDir: string;
  apiPackFiles(): Promise<AgentApiPackFile[]>;
  isTrustedSender(event: IpcMainInvokeEvent): boolean;
  codexBinaryPref(): string | null;
  claudeBinaryPref?(): string | null;
  openExternal(url: string): Promise<void>;
  refreshExtensions?(ids: string[]): Promise<void>;
  /** Standalone MCP shim copied beside the packaged app resources. */
  agentToolServerPath?: string;
  /** Defaults to process.execPath (Electron with ELECTRON_RUN_AS_NODE=1). */
  agentToolCommand?: string;
  /** Test seam; production resolves the same built-in resource root as main boot. */
  builtinExtensionsDir?: string;
}

export interface ChatGPTAccountController {
  status(): Promise<ChatGPTAccountStatus>;
  connect(): Promise<ChatGPTAccountStatus>;
  disconnect(): Promise<ChatGPTAccountStatus>;
  shutdown(): Promise<void>;
  onChanged(listener: (status: ChatGPTAccountStatus) => void): () => void;
}

export interface ClaudeAccountController {
  status(): Promise<ClaudeAccountStatus>;
  connect(): Promise<ClaudeAccountStatus>;
  disconnect(): Promise<ClaudeAccountStatus>;
  shutdown(): Promise<void>;
  onChanged(listener: (status: ClaudeAccountStatus) => void): () => void;
}

function requireTrusted(event: IpcMainInvokeEvent, ctx: CodexIpcContext): void {
  if (!ctx.isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
}

function requireRunRequest(value: unknown): CodexRunRequest {
  if (!isCodexRunRequest(value)) throw new IpcValidationError(IPC.codexRun, 'invalid request');
  return value;
}

function requireCancelRequest(value: unknown): CodexCancelRequest {
  if (!isRecord(value) || !isString(value.id) || !REQUEST_ID.test(value.id)) {
    throw new IpcValidationError(IPC.codexCancel, 'invalid request id');
  }
  return { id: value.id };
}

function requireSteerRequest(value: unknown): CodexSteerRequest {
  if (
    !isRecord(value) ||
    !isString(value.id) || !REQUEST_ID.test(value.id) ||
    !isString(value.prompt, 200_000) || value.prompt.length === 0 ||
    !Array.isArray(value.images) || value.images.length > 6 ||
    value.images.some((image) => !(image instanceof Uint8Array) || image.byteLength > 4 * 1024 * 1024)
  ) throw new IpcValidationError(IPC.codexSteer, 'invalid steering request');
  return { id: value.id, prompt: value.prompt, images: value.images as Uint8Array[] };
}

function requireRestoreRequest(value: unknown): AgentChangeSetRestoreRequest {
  if (
    !isRecord(value) ||
    !isString(value.projectId) || !PROJECT_ID.test(value.projectId) ||
    !isString(value.changeSetId) || !/^[A-Za-z0-9_-]{1,160}$/.test(value.changeSetId)
  ) throw new IpcValidationError(IPC.codexRestoreChangeSet, 'invalid change-set request');
  return { projectId: value.projectId, changeSetId: value.changeSetId };
}

function requireFixPromptRequest(value: unknown): CodexFixPromptRequest {
  if (
    !isRecord(value) ||
    !isString(value.id) ||
    !EXTENSION_ID.test(value.id) ||
    !isString(value.error, FIX_PROMPT_ERROR_CHARS) ||
    !Array.isArray(value.files)
  ) {
    throw new IpcValidationError(IPC.codexFixPrompt, 'invalid request');
  }

  // Degrade instead of rejecting: read-source may return up to 400 files.
  const files = value.files.slice(0, FIX_PROMPT_FILES).map((file) => {
    if (!isRecord(file) || !isString(file.path, 1_000) || file.path.length === 0 || !isString(file.text)) {
      throw new IpcValidationError(IPC.codexFixPrompt, 'invalid extension file');
    }
    const text =
      Buffer.byteLength(file.text, 'utf8') > FIX_PROMPT_FILE_BYTES
        ? `${file.text.slice(0, FIX_PROMPT_FILE_BYTES)}\n/* …truncated… */`
        : file.text;
    return { path: file.path, text };
  });

  return { id: value.id, error: value.error, files };
}

function requireRebasePromptRequest(value: unknown): CodexRebasePromptRequest {
  if (!isRecord(value) || !isString(value.id) || !EXTENSION_ID.test(value.id)) {
    throw new IpcValidationError(IPC.codexRebasePrompt, 'invalid request');
  }
  return { id: value.id };
}

function builtinExtensionsDirectory(ctx: CodexIpcContext): string {
  if (ctx.builtinExtensionsDir) return ctx.builtinExtensionsDir;
  return app.isPackaged
    ? path.join(process.resourcesPath, 'builtin-extensions')
    : path.resolve(app.getAppPath(), 'src/extensions');
}

async function createStagingDirectoryResolver(
  req: CodexRunRequest,
  userData: string
): Promise<(forkId: string) => Promise<string>> {
  const root = agentWorkspaceRoot(userData, req.projectId);
  const stagingRoot = path.join(root, '.powermove', 'extension-runs');
  const authority = req.access === 'computer' ? 'computer' : 'project';
  const checkpointPath = `${sessionPathFor(root, authority, req.threadId, req.provider ?? 'chatgpt')}.checkpoint.json`;
  let checkpointStage: string | null = null;
  try {
    const saved: unknown = JSON.parse(await readFile(checkpointPath, 'utf8'));
    if (isRecord(saved) && typeof saved.stagingDirectory === 'string') {
      const relative = path.relative(stagingRoot, saved.stagingDirectory);
      if (relative && relative !== '..' && !relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative)) {
        checkpointStage = saved.stagingDirectory;
      }
    }
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error;
  }
  const before = new Set<string>();
  try {
    for (const entry of await readdir(stagingRoot, { withFileTypes: true })) {
      if (entry.isDirectory()) before.add(entry.name);
    }
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error;
  }

  return async (forkId: string): Promise<string> => {
    if (checkpointStage) {
      const metadata = await stat(path.join(checkpointStage, forkId));
      if (!metadata.isDirectory()) throw new Error('The resumed run does not contain the requested fork.');
      return checkpointStage;
    }
    const candidates: string[] = [];
    for (const entry of await readdir(stagingRoot, { withFileTypes: true })) {
      if (!entry.isDirectory() || before.has(entry.name)) continue;
      const directory = path.join(stagingRoot, entry.name);
      try {
        if ((await stat(path.join(directory, forkId))).isDirectory()) candidates.push(directory);
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error;
      }
    }
    if (candidates.length !== 1) {
      throw new Error(candidates.length === 0
        ? 'The current run extension stage is not ready.'
        : 'More than one extension stage was created concurrently; retry the rebase after the other run finishes.');
    }
    return candidates[0]!;
  };
}

function requireConsentRequest(value: unknown): ConsentRequest {
  if (!isRecord(value) || !isString(value.projectName) || !isString(value.summary)) {
    throw new IpcValidationError(IPC.consentComputer, 'invalid request');
  }
  return { projectName: value.projectName, summary: value.summary };
}

function requireArtifactRef(channel: string, value: unknown): ArtifactRef {
  if (
    !isRecord(value) ||
    !isString(value.projectId) ||
    !PROJECT_ID.test(value.projectId) ||
    !isString(value.path)
  ) {
    throw new IpcValidationError(channel, 'invalid artifact reference');
  }
  return { projectId: value.projectId, path: value.path };
}

function projectRevision(projectJSON: string | null): number {
  if (!projectJSON) return 0;
  try {
    const value: unknown = JSON.parse(projectJSON);
    return isRecord(value) && typeof value.revision === 'number' && Number.isFinite(value.revision)
      ? Math.max(0, Math.trunc(value.revision))
      : 0;
  } catch {
    return 0;
  }
}

function amendLiveResult(text: string, dropCommands: boolean, warning?: string): string {
  try {
    const value: unknown = JSON.parse(text);
    if (!isRecord(value)) return text;
    const notes = Array.isArray(value.notes) ? value.notes.filter((item): item is string => typeof item === 'string') : [];
    const duplicated = dropCommands && Array.isArray(value.commands) && value.commands.length > 0;
    if (!duplicated && !warning) return text;
    return JSON.stringify({
      ...value,
      ...(dropCommands ? { commands: [] } : {}),
      notes: [
        ...notes,
        ...(duplicated ? ['Powermove ignored final commands that duplicated edits already applied through the live tool transaction.'] : []),
        ...(warning ? [warning] : [])
      ]
    });
  } catch {
    return text;
  }
}

/** Registers the frozen renderer contract without modifying the main bootstrap. */
export function registerCodexIpc(
  ipcMain: IpcMain,
  ctx: CodexIpcContext,
  account: ChatGPTAccountController = new ChatGPTAccountClient({
    userData: ctx.userData,
    codexBinaryPref: ctx.codexBinaryPref,
    openExternal: ctx.openExternal
  }),
  claudeAccount: ClaudeAccountController = new ClaudeAccountClient({
    userData: ctx.userData,
    claudeBinaryPref: () => ctx.claudeBinaryPref?.() ?? null
  }),
  appServerRunner: CodexAppServerRunner = new CodexAppServerRunner()
): void {
  const compatible = new CompatibleProvider(ctx.userData);
  const runner = new CodexRunner();
  const claudeRunner = new ClaudeRunner();
  const owners = new Map<string, WebContents>();
  const toolBridge = ctx.agentToolServerPath
    ? new PowermoveAgentToolBridge(ipcMain, {
        mcpServerPath: ctx.agentToolServerPath,
        ...(ctx.agentToolCommand ? { command: ctx.agentToolCommand } : {}),
        stageForkRebase: ({ forkId, stagingDirectory }) => stageForkRebase({
          forkId,
          stagingDirectory,
          userExtensionsDir: ctx.extensionsDir,
          builtinExtensionsDir: builtinExtensionsDirectory(ctx)
        })
      })
    : null;

  account.onChanged((status) => {
    const window = ctx.getWindow();
    if (window !== null && !window.isDestroyed() && !window.webContents.isDestroyed()) {
      window.webContents.send(IPC.chatgptChanged, status);
    }
  });

  claudeAccount.onChanged((status) => {
    const window = ctx.getWindow();
    if (window !== null && !window.isDestroyed() && !window.webContents.isDestroyed()) {
      window.webContents.send(IPC.claudeChanged, status);
    }
  });

  ipcMain.handle(IPC.chatgptStatus, async (event) => {
    requireTrusted(event, ctx);
    return account.status();
  });

  ipcMain.handle(IPC.chatgptConnect, async (event) => {
    requireTrusted(event, ctx);
    return account.connect();
  });

  ipcMain.handle(IPC.chatgptDisconnect, async (event) => {
    requireTrusted(event, ctx);
    return account.disconnect();
  });

  ipcMain.handle(IPC.claudeStatus, async (event) => {
    requireTrusted(event, ctx);
    return claudeAccount.status();
  });

  ipcMain.handle(IPC.claudeConnect, async (event) => {
    requireTrusted(event, ctx);
    return claudeAccount.connect();
  });

  ipcMain.handle(IPC.claudeDisconnect, async (event) => {
    requireTrusted(event, ctx);
    return claudeAccount.disconnect();
  });

  ipcMain.handle(IPC.compatibleStatus, async (event) => { requireTrusted(event, ctx); return compatible.status(); });
  ipcMain.handle(IPC.compatibleConfigure, async (event, input) => { requireTrusted(event, ctx); return compatible.configure(input); });

  ipcMain.handle(IPC.codexRun, async (event, rawRequest: unknown) => {
    requireTrusted(event, ctx);
    const req = requireRunRequest(rawRequest);
    if (owners.has(req.id)) throw new IpcValidationError(IPC.codexRun, 'request id is already active');

    const owner = event.sender;
    owners.set(req.id, owner);
    let toolSession: PowermoveAgentToolSession | null = null;
    const rendererDestroyed = (): void => {
      compatible.cancel(req.id);
      void Promise.all([runner.cancel(req.id), appServerRunner.cancel(req.id), claudeRunner.cancel(req.id)]);
      void toolSession?.finish(false).catch(() => undefined);
    };
    owner.once('destroyed', rendererDestroyed);
    try {
      const selectedRunner = req.provider === 'claude'
        ? claudeRunner
        : (req.mode === 'editor' ? appServerRunner : runner);
      if (toolBridge) {
        const resolveStagingDirectory = await createStagingDirectoryResolver(req, ctx.userData);
        toolSession = await toolBridge.openSession({
          runId: req.id,
          owner,
          baseRevision: projectRevision(req.projectJSON),
          resolveStagingDirectory
        });
      }
      let result = req.provider === 'compatible'
        ? await compatible.run(req, step => { if (!owner.isDestroyed()) owner.send(IPC.codexEvent, { id: req.id, kind: 'trace', step }); },
          toolSession && toolBridge ? (name, args) => toolBridge.callRenderer(toolSession!, name, args) : undefined)
        : await selectedRunner.run(req, {
        userData: ctx.userData,
        extensionsDir: ctx.extensionsDir,
        apiPackFiles: ctx.apiPackFiles,
        codexBinaryPref: ctx.codexBinaryPref(),
        claudeBinaryPref: ctx.claudeBinaryPref?.() ?? null,
        ...(toolSession ? { nativeTools: toolSession.mcpConfig } : {}),
        onProgress: (text) => {
          if (!owner.isDestroyed()) {
            owner.send(IPC.codexEvent, { id: req.id, kind: 'progress', text });
          }
        },
        onTrace: (step) => {
          if (!owner.isDestroyed()) {
            owner.send(IPC.codexEvent, { id: req.id, kind: 'trace', step });
          }
        }
      });
      if (toolSession) {
        const changedBeforeFinish = toolSession.changed;
        const finish = await toolSession.finish(result.ok);
        toolSession = null;
        if (result.ok && req.mode === 'autonomous') {
          result = {
            ...result,
            text: amendLiveResult(result.text, changedBeforeFinish || finish.changed, finish.warning)
          };
        }
        if (result.ok && req.mode === 'autonomous' && finish.changed) {
          result = {
            ...result,
            liveEditsApplied: true,
            ...(finish.historyId ? { liveEditHistoryId: finish.historyId } : {})
          };
        }
        if (finish.warning) console.warn(`[agent-tools] ${finish.warning}`);
      }
      if (result.ok && result.extensions?.length) {
        await ctx.refreshExtensions?.(result.extensions.map((change) => change.id));
      }
      return result;
    } finally {
      if (toolSession) await toolSession.finish(false).catch(() => undefined);
      owner.removeListener('destroyed', rendererDestroyed);
      if (owners.get(req.id) === owner) owners.delete(req.id);
    }
  });

  ipcMain.handle(IPC.codexSteer, async (event, rawRequest: unknown) => {
    requireTrusted(event, ctx);
    const req = requireSteerRequest(rawRequest);
    if (owners.get(req.id) !== event.sender) return { accepted: false };
    return { accepted: await appServerRunner.steer(req) };
  });

  ipcMain.handle(IPC.codexCancel, async (event, rawRequest: unknown) => {
    requireTrusted(event, ctx);
    const req = requireCancelRequest(rawRequest);
    if (owners.get(req.id) === event.sender) {
      compatible.cancel(req.id);
      await Promise.all([runner.cancel(req.id), appServerRunner.cancel(req.id), claudeRunner.cancel(req.id)]);
    }
  });

  ipcMain.handle(IPC.codexFixPrompt, async (event, rawRequest: unknown) => {
    requireTrusted(event, ctx);
    return buildFixPrompt(requireFixPromptRequest(rawRequest));
  });

  ipcMain.handle(IPC.codexRebasePrompt, async (event, rawRequest: unknown) => {
    requireTrusted(event, ctx);
    const request = requireRebasePromptRequest(rawRequest);
    return buildRebasePrompt(await readForkRebaseInfo({
      forkId: request.id,
      userExtensionsDir: ctx.extensionsDir,
      builtinExtensionsDir: builtinExtensionsDirectory(ctx)
    }));
  });

  ipcMain.handle(IPC.codexRestoreChangeSet, async (event, rawRequest: unknown) => {
    requireTrusted(event, ctx);
    const req = requireRestoreRequest(rawRequest);
    const record = await restoreExtensionChangeSet({
      liveDirectory: ctx.extensionsDir,
      historyRoot: path.join(ctx.userData, 'Agent Change History', safeAgentComponent(req.projectId)),
      changeSetId: req.changeSetId
    });
    await ctx.refreshExtensions?.(record.changes.map((change) => change.id));
    return { changeSetId: record.id, extensions: record.changes };
  });

  ipcMain.handle(IPC.consentComputer, async (event, rawRequest: unknown) => {
    requireTrusted(event, ctx);
    const req = requireConsentRequest(rawRequest);
    const window = ctx.getWindow();
    if (window === null || window.isDestroyed()) throw new Error('No Powermove window is available.');
    return await requestComputerConsent(window, req);
  });

  ipcMain.handle(IPC.artifactRead, async (event, rawReference: unknown) => {
    requireTrusted(event, ctx);
    const reference = requireArtifactRef(IPC.artifactRead, rawReference);
    const root = path.join(agentWorkspaceRoot(ctx.userData, reference.projectId), 'artifacts');
    return await readArtifact(root, reference.path);
  });

  ipcMain.handle(IPC.artifactReveal, async (event, rawReference: unknown) => {
    requireTrusted(event, ctx);
    const reference = requireArtifactRef(IPC.artifactReveal, rawReference);
    const root = path.join(agentWorkspaceRoot(ctx.userData, reference.projectId), 'artifacts');
    await revealArtifact(root, reference.path);
  });

  app.once('before-quit', () => {
    compatible.cancelAll();
    void runner.cancelAll();
    void appServerRunner.shutdown();
    void claudeRunner.cancelAll();
    void toolBridge?.shutdown();
    void account.shutdown();
    void claudeAccount.shutdown();
  });
}

export * from './adapter';
export * from './app-server-account';
export * from './artifacts';
export * from './consent';
export * from './env';
export * from './events';
export * from './instructions';
export * from './runner';
export * from './workspace';
