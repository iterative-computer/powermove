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
import { AgentOrchestrator, type OrchestratedRunContext } from '../agent-orchestrator';
import { orchestrationProviders } from '../orchestration-providers';
import { copyChildArtifacts } from './subagent-artifacts';
import { AGENT_ORCHESTRATION_INSTRUCTIONS } from '../../shared/agent-orchestration';

import {
  IPC,
  LIMITS,
  PROJECT_ID,
  REQUEST_ID,
  type ArtifactRef,
  type ChatGPTAccountStatus,
  type ClaudeModelOption,
  type CodexModelOption,
  type ClaudeAccountStatus,
  type CodexCancelRequest,
  type AgentChangeSetRestoreRequest,
  type CodexFixPromptRequest,
  type CodexRebasePromptRequest,
  type CodexRunRequest,
  type CodexRunResult,
  type CodexAnswerRequest,
  type CodexSteerRequest,
  type CodexTraceEvent,
  type ConsentRequest
} from '../../shared/ipc';
import { EXTENSION_ID } from '../../shared/extensions';
import { IpcValidationError, isRecord, isString } from '../../shared/guards';
import { readArtifact, revealArtifact } from './artifacts';
import { forgetProjectWorkspace, sweepOrphanWorkspaces } from './workspace-lifetime';
import { requestComputerConsent, revokeStandingConsent } from './consent';
import { CodexRunner, isCodexRunRequest } from './runner';
import { ChatGPTAccountClient } from './app-server-account';
import { discoverCodexBinary, forgetCodexDescription } from './env';
import { installLatestRuntime, installRuntimeIfNewer, type RuntimeProvider } from '../runtime-updates';
import { CodexAppServerRunner } from './app-server-runner';
import { ClaudeAccountClient, ClaudeRunner } from '../claude';
import { discoverClaudeBinary } from '../claude/env';
import { buildFixPrompt, buildRebasePrompt } from './instructions';
import { restoreExtensionChangeSet } from './change-history';
import { agentWorkspaceRoot, safeAgentComponent, sessionPathFor, type AgentApiPackFile } from './workspace';
import { PowermoveAgentToolBridge, type OutsideSandboxRunner, type PowermoveAgentToolSession } from '../agent-tools/bridge';
import { AgentApprovals, type RequestApproval } from '../agent-approvals';
import { runWorkspaceCommand } from '../compatible-workspace';
import type { StoreAgentGateway } from '../cloud/store-agent';
import { readForkRebaseInfo, stageForkRebase } from '../extensions/rebase';
import { updateFork } from '../extensions/update-fork';

const FIX_PROMPT_ERROR_CHARS = 4_000;
const FIX_PROMPT_FILES = 40;
const FIX_PROMPT_FILE_BYTES = 64 * 1024;

export interface CodexIpcContext {
  /** The window a request belongs to — the focused editor, or the last one
   *  focused when the request came from somewhere else. */
  getWindow(): BrowserWindow | null;
  /** Account state is app-wide, so it reaches every open window, not just one. */
  broadcast?(send: (webContents: BrowserWindow['webContents']) => void): void;
  userData: string;
  extensionsDir: string;
  apiPackFiles(): Promise<AgentApiPackFile[]>;
  isTrustedSender(event: IpcMainInvokeEvent): boolean;
  codexBinaryPref(): string | null;
  claudeBinaryPref?(): string | null;
  openExternal(url: string): Promise<void>;
  refreshExtensions?(ids: string[]): Promise<void>;
  /** The Store as the agent uses it; null/absent when cloud is unconfigured. */
  storeAgent?(): StoreAgentGateway | null;
  /** Standalone MCP shim copied beside the packaged app resources. */
  agentToolServerPath?: string;
  /** Defaults to process.execPath (Electron with ELECTRON_RUN_AS_NODE=1). */
  agentToolCommand?: string;
  agentToolCommandArgs?: string[];
  /** Test seam; production resolves the same built-in resource root as main boot. */
  builtinExtensionsDir?: string;
  /** The bundled ffmpeg the agent's media tools run (probe, frames, waveform). */
  agentMediaFfmpeg?: string;
}

export interface ChatGPTAccountController {
  status(): Promise<ChatGPTAccountStatus>;
  models?(): Promise<CodexModelOption[]>;
  connect(): Promise<ChatGPTAccountStatus>;
  disconnect(): Promise<ChatGPTAccountStatus>;
  shutdown(): Promise<void>;
  onChanged(listener: (status: ChatGPTAccountStatus) => void): () => void;
}

export interface ClaudeAccountController {
  status(): Promise<ClaudeAccountStatus>;
  models?(): Promise<ClaudeModelOption[]>;
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
  if (!isRecord(value) || !isString(value.id) || !REQUEST_ID.test(value.id) ||
      (value.preserveChanges !== undefined && typeof value.preserveChanges !== 'boolean')) {
    throw new IpcValidationError(IPC.codexCancel, 'invalid request id');
  }
  return { id: value.id, preserveChanges: value.preserveChanges === true };
}

function requireSteerRequest(value: unknown): CodexSteerRequest {
  if (
    !isRecord(value) ||
    !isString(value.id) || !REQUEST_ID.test(value.id) ||
    !isString(value.prompt, 200_000) || value.prompt.length === 0 ||
    !Array.isArray(value.images) || value.images.length > LIMITS.codexImages ||
    value.images.some((image) => !(image instanceof Uint8Array) || image.byteLength > 4 * 1024 * 1024)
  ) throw new IpcValidationError(IPC.codexSteer, 'invalid steering request');
  return { id: value.id, prompt: value.prompt, images: value.images as Uint8Array[] };
}

function requireAnswerRequest(value: unknown): CodexAnswerRequest {
  const answers = isRecord(value) && isRecord(value.answers) ? Object.entries(value.answers) : null;
  if (
    !isRecord(value) ||
    !isString(value.id) || !REQUEST_ID.test(value.id) ||
    !isString(value.itemId, 240) || value.itemId.length === 0 ||
    answers === null || answers.length > 12 ||
    answers.some(([id, list]) => id.length > 200 || !Array.isArray(list) || list.length > 12 ||
      list.some((answer) => !isString(answer, 8_000)))
  ) throw new IpcValidationError(IPC.codexAnswer, 'invalid answer');
  return { id: value.id, itemId: value.itemId, answers: Object.fromEntries(answers) as Record<string, string[]> };
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

function requireRebasePromptRequest(value: unknown, channel: string = IPC.codexRebasePrompt): CodexRebasePromptRequest {
  if (!isRecord(value) || !isString(value.id) || !EXTENSION_ID.test(value.id)) {
    throw new IpcValidationError(channel, 'invalid request');
  }
  return { id: value.id };
}

function builtinExtensionsDirectory(ctx: CodexIpcContext): string {
  if (ctx.builtinExtensionsDir) return ctx.builtinExtensionsDir;
  return app.isPackaged
    ? path.join(process.resourcesPath, 'builtin-extensions')
    : path.resolve(app.getAppPath(), 'src/extensions');
}

function createStagingDirectoryResolver(
  req: CodexRunRequest,
  userData: string,
  workspaceId?: string
): (forkId: string) => Promise<string> {
  const prepared = prepareStagingSnapshot(req, userData, workspaceId);
  return async (forkId: string): Promise<string> => {
    const { resolve } = await prepared;
    return resolve(forkId);
  };
}

async function prepareStagingSnapshot(
  req: CodexRunRequest,
  userData: string,
  workspaceId?: string
): Promise<{ resolve: (forkId: string) => Promise<string> }> {
  const root = agentWorkspaceRoot(userData, workspaceId ?? req.projectId);
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

  const resolve = async (forkId: string): Promise<string> => {
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
  return { resolve };
}

function requireConsentRequest(value: unknown): ConsentRequest {
  if (!isRecord(value) || !isString(value.projectName) || !isString(value.summary)
    || (value.standing !== undefined && typeof value.standing !== 'boolean')
    || (value.revoke !== undefined && typeof value.revoke !== 'boolean')) {
    throw new IpcValidationError(IPC.consentComputer, 'invalid request');
  }
  return {
    projectName: value.projectName, summary: value.summary,
    ...(value.standing ? { standing: true } : {}), ...(value.revoke ? { revoke: true } : {})
  };
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

/** Asks the person, then runs one command in the run's workspace without the sandbox. */
function outsideSandboxRunner(root: string, requestApproval: RequestApproval, signal: AbortSignal): OutsideSandboxRunner {
  return async ({ command, reason, timeoutMs }) => {
    const decision = await requestApproval({ title: 'Run a command outside the project sandbox?', detail: command, reason }, signal);
    if (!decision.allowed) throw new Error(decision.message);
    const result = await runWorkspaceCommand(root, 'computer', command, timeoutMs, signal);
    return `Exit code: ${result.exitCode ?? 'none'}${result.truncated ? ' (output truncated)' : ''}\n${result.output}`;
  };
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
  const approvals = new AgentApprovals();
  const runner = new CodexRunner();
  const claudeRunner = new ClaudeRunner();
  const owners = new Map<string, WebContents>();
  let forkUpdateQueue: Promise<unknown> = Promise.resolve();
  const rootToolSessions = new Map<string, PowermoveAgentToolSession>();
  const activeChildWorkspaces = new Set<string>();
  const preserveRunEdits = new Set<string>();
  const cancelProvider = async (id: string): Promise<void> => {
    compatible.cancel(id);
    await Promise.all([runner.cancel(id), appServerRunner.cancel(id), claudeRunner.cancel(id)]);
  };
  const orchestrator: AgentOrchestrator = new AgentOrchestrator({
    directory: path.join(ctx.userData, 'Agent Tasks'),
    providers: () => orchestrationProviders({ chatgpt: account, claude: claudeAccount, compatible }),
    cancel: cancelProvider,
    steer: request => appServerRunner.steer(request),
    answer: request => approvals.answer(request) || appServerRunner.answer(request) || claudeRunner.answer(request),
    threadControl: async (request, tool, args) => {
      const session = rootToolSessions.get(request.id);
      if (!session || !toolBridge) throw new Error('Thread controls are unavailable in this connection.');
      const response = await toolBridge.callRenderer(session, '__agent_thread_control', { tool, ...args });
      if (!response.ok) throw new Error(response.error ?? 'Thread control failed.');
      const value = response.content.find(content => content.type === 'text');
      if (!value || value.type !== 'text') throw new Error('Thread control returned no result.');
      return JSON.parse(value.text);
    },
    execute: (request, context) => executeProvider(request, context)
  });
  const toolBridge: PowermoveAgentToolBridge | null = ctx.agentToolServerPath
    ? new PowermoveAgentToolBridge(ipcMain, {
        mcpServerPath: ctx.agentToolServerPath,
        ...(ctx.agentToolCommand ? { command: ctx.agentToolCommand } : {}),
        ...(ctx.agentToolCommandArgs ? { commandArgs: ctx.agentToolCommandArgs } : {}),
        storeAgent: ctx.storeAgent?.() ?? null,
        ...(ctx.agentMediaFfmpeg ? { ffmpegPath: ctx.agentMediaFfmpeg } : {}),
        orchestrate: (runId, tool, args) => orchestrator.call(runId, tool, args),
        stageForkRebase: ({ forkId, stagingDirectory }) => stageForkRebase({
          forkId,
          stagingDirectory,
          userExtensionsDir: ctx.extensionsDir,
          builtinExtensionsDir: builtinExtensionsDirectory(ctx)
        })
      })
    : null;

  const executeProvider = async (req: CodexRunRequest, execution: OrchestratedRunContext): Promise<CodexRunResult> => {
    const owner = owners.get(execution.rootRequest.id);
    if (!owner || owner.isDestroyed()) return { ok: false, error: 'Powermove window is no longer available.', cancelled: true };
    const workspaceId = execution.isChild ? `subagent-${req.threadId}` : req.threadId?.startsWith('agent-thread-') ? `thread-${req.threadId}` : undefined;
    let session = rootToolSessions.get(execution.rootRequest.id) ?? null;
    let openedChildSession = false;
    if (execution.isChild) {
      owners.set(req.id, owner);
      if (workspaceId) activeChildWorkspaces.add(workspaceId);
    }
    try {
      if (execution.isChild && toolBridge) {
        session = await toolBridge.openSession({
          runId: req.id, owner, baseRevision: projectRevision(req.projectJSON), context: req.context ?? 'project',
          inspectionOnly: req.mode === 'editor', transactionSession: session ?? undefined,
          ...(session?.outsideSandbox ? { outsideSandbox: session.outsideSandbox } : {}),
          ...(req.mode === 'autonomous' ? { resolveStagingDirectory: createStagingDirectoryResolver(req, ctx.userData, workspaceId) } : {})
        });
        openedChildSession = true;
      }
      const selectedRunner = req.provider === 'claude' ? claudeRunner : req.mode === 'editor' ? appServerRunner : runner;
      // The first root turn consumes its grant. Children and continuations are
      // internal runs under that existing authority, never new IPC grants.
      const inheritedGrant = execution.isChild || execution.continuation ? { consumeConsentToken: () => true } : {};
      const additionalInstructions = session ? AGENT_ORCHESTRATION_INSTRUCTIONS + (execution.role ? `\n\nTask role: ${execution.role}` : '') : undefined;
      const result = req.provider === 'compatible'
        ? await compatible.run(req, execution.onTrace,
          session && toolBridge ? (name, args) => toolBridge.callTool(session!, name, args) : undefined,
          { extensionsDir: ctx.extensionsDir, apiPackFiles: ctx.apiPackFiles, workspaceId, additionalInstructions, outsideSandbox: !!session?.outsideSandbox, ...inheritedGrant,
            onWorkspace: directory => { if (session) session.stagingDirectory = directory; } })
        : await selectedRunner.run(req, {
          userData: ctx.userData, extensionsDir: ctx.extensionsDir, apiPackFiles: ctx.apiPackFiles, workspaceId,
          codexBinaryPref: ctx.codexBinaryPref(), claudeBinaryPref: ctx.claudeBinaryPref?.() ?? null,
          ...(session ? { nativeTools: session.mcpConfig } : {}), additionalInstructions, ...inheritedGrant,
          requestApproval: approvals.requester(req.id, execution.onTrace),
          onProgress: execution.onProgress, onTrace: execution.onTrace
        });
      if (result.ok && result.extensions?.length) await ctx.refreshExtensions?.(result.extensions.map(change => change.id));
      if (result.ok && workspaceId && req.mode === 'autonomous') {
        await copyChildArtifacts(ctx.userData, workspaceId, req.projectId, result.text);
      }
      return result;
    } finally {
      if (execution.isChild) {
        if (openedChildSession) await session?.finish(false).catch(() => undefined);
        approvals.closeRun(req.id);
        owners.delete(req.id);
        if (workspaceId) activeChildWorkspaces.delete(workspaceId);
      }
    }
  };

  const announce = (channel: string, status: unknown): void => {
    if (ctx.broadcast) {
      ctx.broadcast((webContents) => webContents.send(channel, status));
      return;
    }
    const window = ctx.getWindow();
    if (window !== null && !window.isDestroyed() && !window.webContents.isDestroyed()) {
      window.webContents.send(channel, status);
    }
  };

  account.onChanged((status) => announce(IPC.chatgptChanged, status));
  claudeAccount.onChanged((status) => announce(IPC.claudeChanged, status));

  /* A project's workspace is removed with the project, but never under a
     running agent: removal waits for that project's last run to finish. */
  const runningProjects = new Map<string, number>();
  const forgetWhenIdle = new Set<string>();
  const forget = async (projectId: string): Promise<void> => {
    if (runningProjects.has(projectId)) { forgetWhenIdle.add(projectId); return; }
    forgetWhenIdle.delete(projectId);
    const childWorkspaces = await orchestrator.forgetProject(projectId);
    await Promise.all(childWorkspaces.map(id => forgetProjectWorkspace(ctx.userData, id)));
    await forgetProjectWorkspace(ctx.userData, projectId);
  };

  ipcMain.handle(IPC.chatgptStatus, async (event) => {
    requireTrusted(event, ctx);
    return account.status();
  });

  ipcMain.handle(IPC.chatgptModels, async (event) => {
    requireTrusted(event, ctx);
    void refreshRuntime('codex');
    return account.models?.() ?? [];
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

  // New models only appear once the runtime knows them, so each launch checks
  // for a newer runtime and re-announces so the pickers refresh.
  const runtimeChecked = new Set<RuntimeProvider>();
  const refreshRuntime = async (provider: RuntimeProvider): Promise<void> => {
    if (runtimeChecked.has(provider)) return;
    runtimeChecked.add(provider);
    // Let the app finish booting before any network or disk work starts.
    await new Promise<void>(resolve => { setTimeout(resolve, 20_000).unref(); });
    try {
      if (provider === 'claude') {
        if (process.env.CLAUDE_BINARY || ctx.claudeBinaryPref?.() || (await claudeAccount.status()).state !== 'connected') return;
        if (await installRuntimeIfNewer('claude', await discoverClaudeBinary(null))) void claudeAccount.status();
        return;
      }
      if (process.env.CODEX_BINARY || ctx.codexBinaryPref() || (await account.status()).state !== 'connected') return;
      if (!await installRuntimeIfNewer('codex', await discoverCodexBinary(null))) return;
      forgetCodexDescription();
      // Restarting would kill a run in flight; those runs keep the old process.
      if (owners.size === 0) {
        await Promise.all([account.shutdown(), appServerRunner.shutdown()]);
        void account.status();
      }
    } catch {
      // The bundled runtime and model list keep working when the check fails.
    }
  };

  ipcMain.handle(IPC.claudeModels, async (event) => {
    requireTrusted(event, ctx);
    void refreshRuntime('claude');
    return claudeAccount.models?.() ?? [];
  });

  ipcMain.handle(IPC.claudeConnect, async (event) => {
    requireTrusted(event, ctx);
    return claudeAccount.connect();
  });

  ipcMain.handle(IPC.claudeDisconnect, async (event) => {
    requireTrusted(event, ctx);
    return claudeAccount.disconnect();
  });

  ipcMain.handle(IPC.agentRuntimeUpdate, async (event, provider: unknown) => {
    requireTrusted(event, ctx);
    if (provider !== 'claude' && provider !== 'codex') throw new Error('Unknown agent runtime.');
    const result = await installLatestRuntime(provider);
    // Long-lived processes still run the old binary; restart them on the new one.
    if (provider === 'codex') {
      forgetCodexDescription();
      await Promise.all([account.shutdown(), appServerRunner.shutdown()]);
      void account.status();
    } else {
      void claudeAccount.status();
    }
    return result;
  });

  ipcMain.handle(IPC.compatibleStatus, async (event) => { requireTrusted(event, ctx); return compatible.status(); });
  ipcMain.handle(IPC.compatibleConfigure, async (event, input) => { requireTrusted(event, ctx); return compatible.configure(input); });

  ipcMain.handle(IPC.codexRun, async (event, rawRequest: unknown) => {
    requireTrusted(event, ctx);
    const req = requireRunRequest(rawRequest);
    if (owners.has(req.id)) throw new IpcValidationError(IPC.codexRun, 'request id is already active');

    const owner = event.sender;
    owners.set(req.id, owner);
    const threadWorkspaceId = req.threadId?.startsWith('agent-thread-') ? `thread-${req.threadId}` : undefined;
    if (threadWorkspaceId) activeChildWorkspaces.add(threadWorkspaceId);
    runningProjects.set(req.projectId, (runningProjects.get(req.projectId) ?? 0) + 1);
    let toolSession: PowermoveAgentToolSession | null = null;
    const emitTrace = (step: CodexTraceEvent): void => {
      if (!owner.isDestroyed()) owner.send(IPC.codexEvent, { id: req.id, kind: 'trace', step });
    };
    const runEnded = new AbortController();
    const requestApproval = approvals.requester(req.id, emitTrace);
    // Codex exec and API models cannot ask the person themselves, so a
    // supervised Project run asks through Powermove's run_outside_sandbox.
    const outsideSandbox = req.mode === 'autonomous' && req.access === 'project'
      && (req.provider === 'compatible' || ((req.provider ?? 'chatgpt') === 'chatgpt' && req.approval === 'supervised'))
      ? outsideSandboxRunner(agentWorkspaceRoot(ctx.userData, threadWorkspaceId ?? req.projectId), requestApproval, runEnded.signal)
      : undefined;
    const rendererDestroyed = (): void => {
      void orchestrator.cancelRun(req.id);
      void cancelProvider(req.id);
    };
    owner.once('destroyed', rendererDestroyed);
    try {
      if (toolBridge) {
        toolSession = await toolBridge.openSession({
          runId: req.id,
          owner,
          baseRevision: projectRevision(req.projectJSON),
          context: req.context ?? 'project',
          inspectionOnly: req.mode === 'editor',
          // Fork rebases stage into the run's extension staging dir, which only
          // autonomous runs own. The resolver snapshots the staging root now and
          // finishes its lookup lazily, so opening the session is not delayed.
          ...(req.mode === 'autonomous'
            ? { resolveStagingDirectory: createStagingDirectoryResolver(req, ctx.userData, threadWorkspaceId) }
            : {}),
          ...(outsideSandbox ? { outsideSandbox } : {})
        });
        rootToolSessions.set(req.id, toolSession);
      }
      let result = await orchestrator.run(req,
        (step) => {
          if (!owner.isDestroyed()) owner.send(IPC.codexEvent, { id: req.id, kind: 'trace', step });
        },
        (text) => {
          if (!owner.isDestroyed()) {
            owner.send(IPC.codexEvent, { id: req.id, kind: 'progress', text });
          }
        });
      if (toolSession) {
        const changedBeforeFinish = toolSession.changed;
        const finish = await toolSession.finish(result.ok || preserveRunEdits.has(req.id));
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
      return result;
    } finally {
      runEnded.abort();
      approvals.closeRun(req.id);
      if (toolSession) await toolSession.finish(false).catch(() => undefined);
      rootToolSessions.delete(req.id);
      if (threadWorkspaceId) activeChildWorkspaces.delete(threadWorkspaceId);
      preserveRunEdits.delete(req.id);
      owner.removeListener('destroyed', rendererDestroyed);
      if (owners.get(req.id) === owner) owners.delete(req.id);
      const remaining = (runningProjects.get(req.projectId) ?? 1) - 1;
      if (remaining > 0) runningProjects.set(req.projectId, remaining);
      else runningProjects.delete(req.projectId);
      if (!remaining && forgetWhenIdle.has(req.projectId)) {
        void forget(req.projectId).catch((error) => console.warn(`[agent-workspaces] ${String(error)}`));
      }
    }
  });

  ipcMain.handle(IPC.codexSteer, async (event, rawRequest: unknown) => {
    requireTrusted(event, ctx);
    const req = requireSteerRequest(rawRequest);
    if (owners.get(req.id) !== event.sender) return { accepted: false };
    return { accepted: await appServerRunner.steer(req) };
  });

  ipcMain.handle(IPC.codexAnswer, async (event, rawRequest: unknown) => {
    requireTrusted(event, ctx);
    const req = requireAnswerRequest(rawRequest);
    if (owners.get(req.id) !== event.sender) return { accepted: false };
    const routed = orchestrator.routeAnswer(req);
    return { accepted: approvals.answer(routed) || appServerRunner.answer(routed) || claudeRunner.answer(routed) };
  });

  ipcMain.handle(IPC.codexCancel, async (event, rawRequest: unknown) => {
    requireTrusted(event, ctx);
    const req = requireCancelRequest(rawRequest);
    if (owners.get(req.id) === event.sender) {
      // A provider without live steering is replaced by a continuation run.
      // Seal its completed edits before interrupting it; an explicit Stop still
      // takes the ordinary rollback path.
      if (req.preserveChanges) preserveRunEdits.add(req.id);
      const stopping = orchestrator.cancelRun(req.id);
      if (req.preserveChanges) await toolBridge?.finishRun(req.id, true).catch(() => undefined);
      await stopping;
      await cancelProvider(req.id);
    }
  });

  ipcMain.handle(IPC.codexCancelTask, async (event, rawRequest: unknown) => {
    requireTrusted(event, ctx);
    if (!isRecord(rawRequest) || !isString(rawRequest.requestId) || !REQUEST_ID.test(rawRequest.requestId)
      || !isString(rawRequest.taskId, 240) || !rawRequest.taskId) throw new IpcValidationError(IPC.codexCancelTask, 'invalid task request');
    if (owners.get(rawRequest.requestId) !== event.sender) throw new Error('This task belongs to another conversation.');
    await orchestrator.cancelTask(rawRequest.requestId, rawRequest.taskId);
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

  ipcMain.handle(IPC.codexUpdateFork, async (event, rawRequest: unknown) => {
    requireTrusted(event, ctx);
    const request = requireRebasePromptRequest(rawRequest, IPC.codexUpdateFork);
    const run = forkUpdateQueue.then(async () => {
      const result = await updateFork({
        forkId: request.id, userData: ctx.userData,
        userExtensionsDir: ctx.extensionsDir,
        builtinExtensionsDir: builtinExtensionsDirectory(ctx)
      });
      if (result.kind === 'updated') {
        // Always return the undo reference after publication, even if registry
        // refresh fails; the renderer can reload or restore the checked update.
        try { await ctx.refreshExtensions?.([request.id]); }
        catch (error) { console.warn('[extensions] Could not refresh the updated fork', error); }
      }
      return result;
    });
    forkUpdateQueue = run.catch(() => undefined);
    return run;
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
    if (req.revoke) { revokeStandingConsent(); return { granted: false }; }
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

  ipcMain.handle(IPC.artifactForget, async (event, projectId: unknown) => {
    requireTrusted(event, ctx);
    if (!isString(projectId) || !PROJECT_ID.test(projectId)) throw new IpcValidationError(IPC.artifactForget, 'invalid project id');
    await forget(projectId);
  });

  ipcMain.handle(IPC.artifactSweep, async (event, liveProjectIds: unknown) => {
    requireTrusted(event, ctx);
    if (!Array.isArray(liveProjectIds) || liveProjectIds.length > 100_000
      || !liveProjectIds.every((id) => isString(id) && PROJECT_ID.test(id))) {
      throw new IpcValidationError(IPC.artifactSweep, 'invalid project ids');
    }
    // An empty library is more likely unreadable storage than no projects.
    if (!liveProjectIds.length) return [];
    return await sweepOrphanWorkspaces(ctx.userData, liveProjectIds,
      new Set([...runningProjects.keys(), ...forgetWhenIdle, ...activeChildWorkspaces]));
  });

  // before-quit can be cancelled by the document save prompt or recovery flush.
  // Keep active runs and their tool bridge alive until closing is confirmed.
  app.once('will-quit', () => {
    void orchestrator.shutdown();
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
