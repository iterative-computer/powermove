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
import { buildFixPrompt } from './instructions';
import { restoreExtensionChangeSet } from './change-history';
import { agentWorkspaceRoot, safeAgentComponent, type AgentApiPackFile } from './workspace';
import { PowermoveAgentToolBridge, type PowermoveAgentToolSession } from '../agent-tools/bridge';

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
  const runner = new CodexRunner();
  const claudeRunner = new ClaudeRunner();
  const owners = new Map<string, WebContents>();
  const toolBridge = ctx.agentToolServerPath
    ? new PowermoveAgentToolBridge(ipcMain, {
        mcpServerPath: ctx.agentToolServerPath,
        ...(ctx.agentToolCommand ? { command: ctx.agentToolCommand } : {})
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

  ipcMain.handle(IPC.codexRun, async (event, rawRequest: unknown) => {
    requireTrusted(event, ctx);
    const req = requireRunRequest(rawRequest);
    if (owners.has(req.id)) throw new IpcValidationError(IPC.codexRun, 'request id is already active');

    const owner = event.sender;
    owners.set(req.id, owner);
    let toolSession: PowermoveAgentToolSession | null = null;
    const rendererDestroyed = (): void => {
      void Promise.all([runner.cancel(req.id), appServerRunner.cancel(req.id), claudeRunner.cancel(req.id)]);
      void toolSession?.finish(false).catch(() => undefined);
    };
    owner.once('destroyed', rendererDestroyed);
    try {
      const selectedRunner = req.provider === 'claude'
        ? claudeRunner
        : (req.mode === 'editor' ? appServerRunner : runner);
      if (req.mode === 'autonomous' && toolBridge) {
        toolSession = await toolBridge.openSession({
          runId: req.id,
          owner,
          baseRevision: projectRevision(req.projectJSON)
        });
      }
      let result = await selectedRunner.run(req, {
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
        if (result.ok) {
          result = {
            ...result,
            text: amendLiveResult(result.text, changedBeforeFinish || finish.changed, finish.warning)
          };
        }
        if (result.ok && finish.changed) {
          result = {
            ...result,
            liveEditsApplied: true,
            ...(finish.historyId ? { liveEditHistoryId: finish.historyId } : {})
          };
        }
        if (finish.warning) console.warn(`[agent-tools] ${finish.warning}`);
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
      await Promise.all([runner.cancel(req.id), appServerRunner.cancel(req.id), claudeRunner.cancel(req.id)]);
    }
  });

  ipcMain.handle(IPC.codexFixPrompt, async (event, rawRequest: unknown) => {
    requireTrusted(event, ctx);
    return buildFixPrompt(requireFixPromptRequest(rawRequest));
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
