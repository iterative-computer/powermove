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
  type CodexFixPromptRequest,
  type CodexRunRequest,
  type ConsentRequest
} from '../../shared/ipc';
import { EXTENSION_ID } from '../../shared/extensions';
import { IpcValidationError, isRecord, isString } from '../../shared/guards';
import { readArtifact, revealArtifact } from './artifacts';
import { requestComputerConsent } from './consent';
import { CodexRunner, isCodexRunRequest } from './runner';
import { ChatGPTAccountClient } from './app-server-account';
import { ClaudeAccountClient, ClaudeRunner } from '../claude';
import { buildFixPrompt } from './instructions';
import { agentWorkspaceRoot, type AgentApiPackFile } from './workspace';

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
  })
): void {
  const runner = new CodexRunner();
  const claudeRunner = new ClaudeRunner();
  const owners = new Map<string, WebContents>();

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
    const rendererDestroyed = (): void => {
      void Promise.all([runner.cancel(req.id), claudeRunner.cancel(req.id)]);
    };
    owner.once('destroyed', rendererDestroyed);
    try {
      const selectedRunner = req.provider === 'claude' ? claudeRunner : runner;
      return await selectedRunner.run(req, {
        userData: ctx.userData,
        extensionsDir: ctx.extensionsDir,
        apiPackFiles: ctx.apiPackFiles,
        codexBinaryPref: ctx.codexBinaryPref(),
        claudeBinaryPref: ctx.claudeBinaryPref?.() ?? null,
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
    } finally {
      owner.removeListener('destroyed', rendererDestroyed);
      if (owners.get(req.id) === owner) owners.delete(req.id);
    }
  });

  ipcMain.handle(IPC.codexCancel, async (event, rawRequest: unknown) => {
    requireTrusted(event, ctx);
    const req = requireCancelRequest(rawRequest);
    if (owners.get(req.id) === event.sender) {
      await Promise.all([runner.cancel(req.id), claudeRunner.cancel(req.id)]);
    }
  });

  ipcMain.handle(IPC.codexFixPrompt, async (event, rawRequest: unknown) => {
    requireTrusted(event, ctx);
    return buildFixPrompt(requireFixPromptRequest(rawRequest));
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
    void claudeRunner.cancelAll();
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
