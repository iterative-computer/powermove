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
  type CodexCancelRequest,
  type CodexRunRequest,
  type ConsentRequest
} from '../../shared/ipc';
import { IpcValidationError, isRecord, isString } from '../../shared/guards';
import { readArtifact, revealArtifact } from './artifacts';
import { requestComputerConsent } from './consent';
import { CodexRunner, isCodexRunRequest } from './runner';
import { agentWorkspaceRoot } from './workspace';

export interface CodexIpcContext {
  getWindow(): BrowserWindow | null;
  userData: string;
  isTrustedSender(event: IpcMainInvokeEvent): boolean;
  codexBinaryPref(): string | null;
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
export function registerCodexIpc(ipcMain: IpcMain, ctx: CodexIpcContext): void {
  const runner = new CodexRunner();
  const owners = new Map<string, WebContents>();

  ipcMain.handle(IPC.codexRun, async (event, rawRequest: unknown) => {
    requireTrusted(event, ctx);
    const req = requireRunRequest(rawRequest);
    if (owners.has(req.id)) throw new IpcValidationError(IPC.codexRun, 'request id is already active');

    const owner = event.sender;
    owners.set(req.id, owner);
    const rendererDestroyed = (): void => { void runner.cancel(req.id); };
    owner.once('destroyed', rendererDestroyed);
    try {
      return await runner.run(req, {
        userData: ctx.userData,
        codexBinaryPref: ctx.codexBinaryPref(),
        onProgress: (text) => {
          if (!owner.isDestroyed()) {
            owner.send(IPC.codexEvent, { id: req.id, kind: 'progress', text });
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
    if (owners.get(req.id) === event.sender) await runner.cancel(req.id);
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

  app.once('before-quit', () => { void runner.cancelAll(); });
}

export * from './adapter';
export * from './artifacts';
export * from './consent';
export * from './env';
export * from './events';
export * from './instructions';
export * from './runner';
export * from './workspace';
