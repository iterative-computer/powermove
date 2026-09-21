import { spawn, type ChildProcess, type SpawnOptions } from 'node:child_process';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';

import {
  LIMITS,
  PROJECT_ID,
  REQUEST_ID,
  type AgentExtensionChange,
  type CodexTraceEvent,
  type CodexRunRequest,
  type CodexRunResult
} from '../../shared/ipc';
import { EXTENSION_ID } from '../../shared/extensions';
import { isArrayOf, isBytes, isOneOf, isRecord, isString } from '../../shared/guards';
import { buildAutonomousArgv, buildEditorArgv } from './adapter';
import { publishExtensionChanges } from './change-history';
import { AgentResultValidationError, repairAgentResult } from './result-repair';
import { validateStagedExtensions } from './validate-staged-extensions';
import { collectArtifacts } from './artifacts';
import { consumeToken } from './consent';
import { discoverCodexBinary } from './env';
import { CodexEventParser } from './events';
import { agentInstructions, agentResultSchema } from './instructions';
import type { NativeMcpServerConfig } from '../agent-tools/spec';
import {
  discoverUserSkillFiles,
  isolatedCodexEnvironment,
  prepareIsolatedCodexHome
} from './isolation';
import {
  agentWorkspaceRoot,
  clearSession,
  discardExtensionStage,
  preserveCancelledRun,
  prepareAgentWorkspace,
  readSession,
  sessionPathFor,
  writeSession,
  type AgentWorkspace,
  type AgentApiPackFile,
  type CodexAuthority
} from './workspace';

const MODES = ['editor', 'autonomous'] as const;
const ACCESS = ['editor', 'project', 'computer'] as const;
const EFFORTS = ['low', 'medium', 'high', 'xhigh', 'max'] as const;
const PROVIDERS = ['chatgpt', 'claude', 'compatible'] as const;
const DEFAULT_TIMEOUT_MS = 3_600_000;
const MAX_DIAGNOSTIC_BYTES = 2 * 1024 * 1024;

type SpawnLike = (
  command: string,
  args: readonly string[],
  options: SpawnOptions
) => ChildProcess;

export interface CodexRunOptions {
  userData: string;
  extensionsDir: string;
  apiPackFiles: () => Promise<AgentApiPackFile[]>;
  codexBinaryPref?: string | null;
  binary?: string;
  timeoutMs?: number;
  onProgress?: (text: string) => void;
  onTrace?: (step: CodexTraceEvent) => void;
  onWarning?: (text: string) => void;
  spawnProcess?: SpawnLike;
  consumeConsentToken?: (token: string) => boolean;
  discoverDisabledSkillPaths?: () => Promise<string[]>;
  nativeTools?: NativeMcpServerConfig;
}

interface ActiveRun {
  request: CodexRunRequest;
  child: ChildProcess | null;
  layout: AgentWorkspace | null;
  killTimer: NodeJS.Timeout | null;
  userData: string;
  sessionWrite: Promise<void>;
  done: Promise<void>;
  timedOut: boolean;
}

interface AttemptResult {
  code: number | null;
  signal: NodeJS.Signals | null;
  stderr: string;
  stdout: string;
  spawnError: Error | null;
}

function utf8Bytes(value: string): number {
  return Buffer.byteLength(value, 'utf8');
}

function authorityForAccess(access: CodexRunRequest['access']): CodexAuthority {
  return access === 'computer' ? 'computer' : 'project';
}

export function isCodexRunRequest(value: unknown): value is CodexRunRequest {
  if (!isRecord(value)) return false;
  if (!isString(value.id) || !REQUEST_ID.test(value.id)) return false;
  if (value.provider !== undefined && !isOneOf(value.provider, PROVIDERS)) return false;
  if (value.threadId !== undefined && (!isString(value.threadId) || !/^[A-Za-z0-9_-]{1,120}$/.test(value.threadId))) return false;
  if (!isOneOf(value.mode, MODES)) return false;
  if (!isString(value.prompt, LIMITS.codexPromptChars) || value.prompt.length === 0) return false;
  if (!(value.schema === null || isRecord(value.schema))) return false;
  if (!isArrayOf(value.images, LIMITS.codexImages, (image): image is Uint8Array =>
    isBytes(image))) return false;
  if (!(value.model === null || isString(value.model))) return false;
  if (!(value.reasoningEffort === null || isOneOf(value.reasoningEffort, EFFORTS))) return false;
  if (!isOneOf(value.access, ACCESS)) return false;
  if (!isString(value.projectId) || !PROJECT_ID.test(value.projectId)) return false;
  if (!isString(value.projectName)) return false;
  if (!(value.projectJSON === null || (isString(value.projectJSON) &&
    utf8Bytes(value.projectJSON) <= LIMITS.codexProjectJsonBytes))) return false;
  if (!isArrayOf(value.attachments, LIMITS.codexAttachments, (attachment): attachment is { name: string; data: Uint8Array } =>
    isRecord(attachment) && isString(attachment.name) &&
    isBytes(attachment.data))) return false;
  if (!(value.consentToken === null || isString(value.consentToken))) return false;

  return value.mode === 'editor' || value.projectJSON !== null;
}

function failure(error: string, cancelled = false): CodexRunResult {
  return { ok: false, error, cancelled };
}

function cancelledResult(state: ActiveRun): CodexRunResult {
  if (!state.timedOut) return failure('The Codex run was cancelled.', true);
  return failure(state.layout
    ? 'The coding agent took too long to respond. Your partial work was kept; retry to continue.'
    : 'The coding agent took too long to respond. Try sending your request again.');
}

function errorMessageFromValue(value: unknown, depth = 0): string | null {
  if (depth > 8) return null;
  if (typeof value === 'string') {
    const text = value.trim();
    if (!text) return null;
    if (text.startsWith('{')) {
      try {
        return errorMessageFromValue(JSON.parse(text) as unknown, depth + 1) ?? text;
      } catch {
        // The error message is ordinary prose, not nested JSON.
      }
    }
    return text;
  }
  if (!isRecord(value)) return null;
  return errorMessageFromValue(value.error, depth + 1)
    ?? errorMessageFromValue(value.message, depth + 1)
    ?? errorMessageFromValue(value.detail, depth + 1);
}

/** Extracts the actual failure from `codex exec --json` stdout events. */
export function codexErrorFromStdout(stdout: string): string | null {
  const lines = stdout.split(/\r?\n/);
  for (let index = lines.length - 1; index >= 0; index -= 1) {
    const line = lines[index];
    if (line === undefined) continue;
    let event: unknown;
    try {
      event = JSON.parse(line) as unknown;
    } catch {
      continue;
    }
    if (!isRecord(event) || typeof event.type !== 'string') continue;
    if (event.type === 'item.completed' && isRecord(event.item) && event.item.type === 'error') {
      const message = errorMessageFromValue(event.item);
      if (message) return message.slice(0, 4_000);
    }
    if (!/(?:^|[._-])(?:error|failed|failure)(?:$|[._-])/i.test(event.type)) continue;
    const message = errorMessageFromValue(event);
    if (message) return message.slice(0, 4_000);
  }
  return null;
}

function meaningfulStderr(stderr: string): string {
  return stderr
    .split(/\r?\n/)
    .filter((line) => !/^Reading additional input from stdin(?:\.\.\.)?$/i.test(line.trim()))
    .join('\n')
    .trim();
}

function diagnosticText(attempt: AttemptResult, fallback: string): string {
  const diagnostic = codexErrorFromStdout(attempt.stdout)
    ?? (meaningfulStderr(attempt.stderr) || attempt.spawnError?.message || '');
  return humanizeCodexFailure(diagnostic, fallback);
}

function attemptHasDiagnostic(attempt: AttemptResult): boolean {
  return codexErrorFromStdout(attempt.stdout) !== null ||
    meaningfulStderr(attempt.stderr).length > 0 || attempt.spawnError !== null;
}

/* Raw CLI stderr must never reach the conversation: it is log noise (timestamps,
   rust module paths, auth headers). Map known failure classes to actionable
   sentences and reduce everything else to its last meaningful line. The full
   diagnostic stays in the main-process log. */
export function humanizeCodexFailure(diagnostic: string, fallback = 'ChatGPT generation failed.'): string {
  const text = diagnostic.trim();
  if (!text) return fallback;
  console.error('[codex] run failed:', text.slice(0, 4_000));
  if (/AuthRequired|www_authenticate|Unauthorized|\b401\b/i.test(text) && /rmcp|mcp/i.test(text)) {
    return 'One of your Codex integrations (an MCP server) needs to be signed in again. Run `codex` in a terminal, re-authenticate it, then retry.';
  }
  if (/failed to initialize in-process app-server client.{0,160}Operation not permitted|attempt to write a readonly database/i.test(text)) {
    return 'Powermove was opened from a restricted development environment, so Codex cannot start. Quit Powermove, reopen it normally, then retry.';
  }
  if (/not logged in|login required|please run codex login|invalid api key|missing (?:authentication )?credentials|authentication credentials (?:were|are) not provided/i.test(text)) {
    return 'Codex CLI is not signed in. Run `codex login` in a terminal, then retry.';
  }
  if (/ENOENT|command not found|No such file/i.test(text)) {
    return 'The Codex CLI could not be launched. Check that `codex` is installed and on your PATH.';
  }
  if (/already has an active writer/i.test(text)) {
    return 'The saved agent session couldn’t reopen. Try again to reconnect the agent; if it repeats, restart Powermove.';
  }
  if (/ECONNRESET|ENOTFOUND|fetch failed|network|connection (?:closed|refused)|stream disconnected/i.test(text)) {
    return 'The connection to the agent was interrupted. Check your internet connection, then retry.';
  }
  if (/rate.?limit|\b429\b|overloaded/i.test(text)) {
    return 'The model is rate-limited right now. Wait a moment and retry.';
  }
  if (/invalid_json_schema|Invalid schema for response_format/i.test(text)) {
    return "Powermove's agent response format was rejected. Restart Powermove and retry; if it persists, update the app.";
  }
  const lines = text
    .split(/\r?\n/)
    .map((line) => line.replace(/^\d{4}-\d{2}-\d{2}T\S+\s+(ERROR|WARN|INFO)\s+\S*:?\s*/i, '').trim())
    .filter(Boolean);
  const last = lines.at(-1) ?? fallback;
  return `The agent failed: ${last.slice(0, 240)}`;
}

function attemptOutput(attempt: AttemptResult): string {
  return `${attempt.stderr}\n${attempt.stdout}`.trim();
}

function sortJsonValue(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(sortJsonValue);
  if (!isRecord(value)) return value;
  return Object.fromEntries(
    Object.keys(value).sort().map((key) => [key, sortJsonValue(value[key])])
  );
}

export function parseAgentExtensionChanges(value: unknown): AgentExtensionChange[] | undefined {
  if (!Array.isArray(value)) return undefined;
  const changes: AgentExtensionChange[] = [];
  for (const item of value) {
    if (changes.length >= 32) break;
    if (
      !isRecord(item) ||
      !isString(item.id) ||
      !EXTENSION_ID.test(item.id) ||
      !isOneOf(item.action, ['created', 'updated', 'removed'] as const)
    ) continue;
    const change: AgentExtensionChange = { id: item.id, action: item.action };
    if (isString(item.summary)) change.summary = item.summary;
    changes.push(change);
  }
  return changes;
}

function isUnknownSession(text: string): boolean {
  return /(?:unknown|invalid|missing|not found|does not exist|could not find).{0,80}(?:session|thread|rollout)|(?:session|thread|rollout).{0,80}(?:unknown|invalid|missing|not found|does not exist|could not find)|no rollout found/i.test(text);
}

/** A failed resume is safe to replace with a fresh run until Codex has actually
 * begun the turn. This also covers native child-process exits where the CLI's
 * short startup diagnostic is unavailable to Electron. */
function codexTurnStarted(stdout: string): boolean {
  for (const line of stdout.split(/\r?\n/)) {
    try {
      const event: unknown = JSON.parse(line);
      if (!isRecord(event) || typeof event.type !== 'string') continue;
      if (event.type === 'turn.started' || (event.type.startsWith('item.') &&
        !(isRecord(event.item) && event.item.type === 'error'))) return true;
    } catch {
      // Non-JSON notices are diagnostics, not evidence that a turn began.
    }
  }
  return false;
}

function appendDiagnostic(chunks: Buffer[], chunk: Buffer): void {
  let current = 0;
  for (const item of chunks) current += item.byteLength;
  if (current >= MAX_DIAGNOSTIC_BYTES) return;
  chunks.push(chunk.subarray(0, MAX_DIAGNOSTIC_BYTES - current));
}

async function writeEditorInputs(req: CodexRunRequest): Promise<{
  directory: string;
  schemaPath: string;
  outputPath: string;
  imagePaths: string[];
}> {
  const directory = await mkdtemp(path.join(tmpdir(), 'powermove-codex-'));
  const schemaPath = path.join(directory, 'schema.json');
  const outputPath = path.join(directory, 'result.json');
  await writeFile(schemaPath, JSON.stringify(req.schema ?? { type: 'object' }, null, 2));
  const imagePaths: string[] = [];
  for (const [index, image] of req.images.entries()) {
    const extension = image[0] === 0x89 && image[1] === 0x50 ? 'png' : 'jpg';
    const imagePath = path.join(directory, `frame-${index}.${extension}`);
    await writeFile(imagePath, image);
    imagePaths.push(imagePath);
  }
  return { directory, schemaPath, outputPath, imagePaths };
}

export class CodexRunner {
  private readonly active = new Map<string, ActiveRun>();
  private readonly cancelled = new Set<string>();

  async run(rawRequest: CodexRunRequest, options: CodexRunOptions): Promise<CodexRunResult> {
    if (!isCodexRunRequest(rawRequest)) return failure('Invalid Codex run request.');
    const req = rawRequest;
    if (this.active.has(req.id)) return failure(`A Codex run with id ${req.id} is already active.`);

    // Reserve the conversation before any asynchronous setup can race a second request.
    if (req.mode === 'autonomous') {
      const previous = [...this.active.values()].find(state => state.userData === options.userData &&
        state.request.mode === 'autonomous' && state.request.projectId === req.projectId &&
        state.request.threadId === req.threadId && authorityForAccess(state.request.access) === authorityForAccess(req.access));
      if (previous) {
        if (this.cancelled.has(previous.request.id)) {
          await previous.done;
          return this.run(req, options);
        }
        return failure('An agent is already running in this conversation. Stop it or wait for it to finish, then retry.');
      }
    }

    if (req.access === 'computer') {
      const consume = options.consumeConsentToken ?? consumeToken;
      if (req.consentToken === null || !consume(req.consentToken)) {
        return failure('Computer access requires fresh approval for this run.');
      }
    }

    let release!: () => void;
    const done = new Promise<void>(resolve => { release = resolve; });
    const state: ActiveRun = {
      request: req,
      child: null,
      layout: null,
      killTimer: null,
      userData: options.userData,
      sessionWrite: Promise.resolve(),
      done,
      timedOut: false
    };
    this.active.set(req.id, state);
    let editorDirectory: string | null = null;
    let repairingResult = false;
    let timeout: NodeJS.Timeout | null = null;

    try {
      if (this.cancelled.has(req.id)) return cancelledResult(state);

      const binary = options.binary ?? await discoverCodexBinary(options.codexBinaryPref ?? null);
      if (this.cancelled.has(req.id)) return cancelledResult(state);
      const [codexHome, disabledSkillPaths] = await Promise.all([
        prepareIsolatedCodexHome(options.userData),
        (options.discoverDisabledSkillPaths ?? discoverUserSkillFiles)()
      ]);
      if (this.cancelled.has(req.id)) return cancelledResult(state);

      if (req.mode === 'editor') {
        const input = await writeEditorInputs(req);
        editorDirectory = input.directory;
        const argv = buildEditorArgv({
          schemaPath: input.schemaPath,
          outputPath: input.outputPath,
          prompt: req.prompt,
          imagePaths: input.imagePaths,
          model: req.model,
          reasoningEffort: req.reasoningEffort,
          disabledSkillPaths
        });
        const attempt = await this.execute(req, state, binary, argv, input.directory, codexHome, null, options, (timer) => {
          timeout = timer;
        });
        if (this.cancelled.has(req.id)) {
          await this.cleanupCancelled(state);
          return cancelledResult(state);
        }
        if (attempt.code !== 0) return failure(diagnosticText(attempt, 'ChatGPT generation failed.'));
        try {
          return { ok: true, text: await readFile(input.outputPath, 'utf8'), access: 'editor' };
        } catch {
          return failure('ChatGPT generation completed without a result.');
        }
      }

      const authority = authorityForAccess(req.access);
      let apiPackFiles: AgentApiPackFile[] = [];
      try {
        apiPackFiles = await options.apiPackFiles();
      } catch (error) {
        options.onWarning?.(`API pack unavailable: ${String(error)}`);
      }
      const layout = await prepareAgentWorkspace(
        req,
        options.userData,
        authority,
        agentResultSchema(),
        { extensionsDir: options.extensionsDir, apiPackFiles }
      );
      state.layout = layout;
      if (this.cancelled.has(req.id)) {
        await this.cleanupCancelled(state, options.userData);
        return cancelledResult(state);
      }

      let resumeId = await readSession(layout.sessionPath);
      let attempt: AttemptResult | null = null;
      const executeAutonomous = async (prompt: string, sessionId: string | null) => {
        if (this.cancelled.has(req.id)) throw new Error('The Codex run was cancelled.');
        // A repair must produce a new report, never reuse the rejected output.
        await rm(layout.outputPath, { force: true });
        const argv = buildAutonomousArgv({
          schemaPath: layout.schemaPath,
          outputPath: layout.outputPath,
          instructions: agentInstructions({
            projectName: req.projectName,
            artifactPath: `artifacts/${layout.runId}`,
            access: authority,
            extensionsDir: layout.extensionsDir
          }),
          prompt,
          imagePaths: layout.imagePaths,
          model: req.model,
          reasoningEffort: req.reasoningEffort,
          access: authority,
          extensionsDir: layout.extensionsDir,
          sessionId,
          disabledSkillPaths,
          nativeTools: options.nativeTools
        });
        const result = await this.execute(req, state, binary, argv, layout.root, codexHome, layout, options, (timer) => {
          timeout = timer;
        });
        if (this.cancelled.has(req.id)) throw new Error('The Codex run was cancelled.');
        return result;
      };
      for (let tryIndex = 0; tryIndex < 2; tryIndex += 1) {
        const resuming = Boolean(resumeId);
        attempt = await executeAutonomous(req.prompt, resumeId);
        const diagnostic = attemptOutput(attempt);
        const silentFailureBeforeTurn = attempt.code !== 0 &&
          !codexTurnStarted(attempt.stdout) && !attemptHasDiagnostic(attempt);
        const canRetryFresh = tryIndex === 0 && attempt.code !== 0 && !codexTurnStarted(attempt.stdout) &&
          ((resuming && (isUnknownSession(diagnostic) || /already has an active writer/i.test(diagnostic))) || silentFailureBeforeTurn);
        if (canRetryFresh) {
          options.onProgress?.(resuming
            ? 'The saved agent thread could not be resumed — starting a fresh run…'
            : 'The agent stopped before it could start — retrying…');
          await state.sessionWrite;
          await clearSession(layout.sessionPath);
          await rm(layout.outputPath, { force: true });
          resumeId = null;
          continue;
        }
        break;
      }

      if (attempt === null || attempt.code !== 0) {
        await state.sessionWrite;
        await clearSession(layout.sessionPath);
        return failure(attempt ? diagnosticText(attempt, 'The autonomous agent failed.') : 'The autonomous agent failed.');
      }

      const { parsed, extensions, changeSet } = await repairAgentResult(async () => {
        if (this.cancelled.has(req.id)) throw new Error('The Codex run was cancelled.');
        let parsed: Record<string, unknown>;
        try {
          const value: unknown = JSON.parse(await readFile(layout.outputPath, 'utf8'));
          if (!isRecord(value)) throw new Error('not an object');
          parsed = value;
        } catch {
          throw new AgentResultValidationError('The autonomous agent returned an invalid result. Return a JSON object matching the result schema.');
        }
        const requested = Array.isArray(parsed.artifacts) ? parsed.artifacts : [];
        parsed.artifacts = await collectArtifacts(layout.runDirectory, layout.runId, requested);
        parsed.projectId = req.projectId;
        parsed.access = authority;
        const extensions = parseAgentExtensionChanges(parsed.extensions);
        await validateStagedExtensions(layout, extensions ?? []);
        if (this.cancelled.has(req.id)) throw new Error('The Codex run was cancelled.');
        const changeSet = await publishExtensionChanges(layout, extensions ?? []);
        return { parsed, extensions, changeSet };
      }, async prompt => {
        repairingResult = true;
        const repaired = await executeAutonomous(prompt, await readSession(layout.sessionPath));
        if (repaired.code !== 0) throw new Error(diagnosticText(repaired, 'The agent could not correct its result.'));
      }, options.onProgress);
      repairingResult = false;
      await discardExtensionStage(layout);
      return {
        ok: true,
        text: JSON.stringify(sortJsonValue(parsed)),
        access: authority,
        ...(extensions === undefined ? {} : { extensions }),
        ...(changeSet === null ? {} : { extensionChangeSetId: changeSet.id })
      };
    } catch (error) {
      if (this.cancelled.has(req.id)) {
        await this.cleanupCancelled(state, options.userData);
        return cancelledResult(state);
      }
      if ((error instanceof AgentResultValidationError || repairingResult) && state.layout) {
        await preserveCancelledRun(state.layout).catch(checkpointError => options.onWarning?.(`Could not save the agent checkpoint: ${String(checkpointError)}`));
      }
      return failure(error instanceof Error ? error.message : String(error));
    } finally {
      if (timeout) clearTimeout(timeout);
      if (state.killTimer) clearTimeout(state.killTimer);
      state.child = null;
      this.active.delete(req.id);
      this.cancelled.delete(req.id);
      release();
      if (editorDirectory) await rm(editorDirectory, { recursive: true, force: true });
    }
  }

  async cancel(id: string): Promise<boolean> {
    this.cancelled.add(id);
    const state = this.active.get(id);
    if (!state) return false;
    this.terminate(state);
    await this.cleanupCancelled(state);
    return true;
  }

  async cancelAll(): Promise<void> {
    await Promise.all([...this.active.keys()].map((id) => this.cancel(id)));
  }

  private async execute(
    req: CodexRunRequest,
    state: ActiveRun,
    binary: string,
    argv: string[],
    cwd: string,
    codexHome: string,
    layout: AgentWorkspace | null,
    options: CodexRunOptions,
    setTimeoutHandle: (timer: NodeJS.Timeout) => void
  ): Promise<AttemptResult> {
    if (this.cancelled.has(req.id)) {
      return { code: null, signal: null, stderr: '', stdout: '', spawnError: null };
    }

    const spawnProcess = options.spawnProcess ?? ((command, args, spawnOptions) =>
      spawn(command, args, spawnOptions));
    let child: ChildProcess;
    try {
      child = spawnProcess(binary, argv, {
        cwd,
        detached: true,
        env: isolatedCodexEnvironment(codexHome),
        stdio: ['ignore', 'pipe', 'pipe']
      });
    } catch (error) {
      return {
        code: null,
        signal: null,
        stderr: '',
        stdout: '',
        spawnError: error instanceof Error ? error : new Error(String(error))
      };
    }
    state.child = child;
    if (this.cancelled.has(req.id)) this.terminate(state);

    const stdoutChunks: Buffer[] = [];
    const stderrChunks: Buffer[] = [];
    const parser = new CodexEventParser({
      onProgress: (text) => options.onProgress?.(text),
      onTrace: (step) => options.onTrace?.(step),
      onThreadId: (threadId) => {
        if (layout) {
          state.sessionWrite = state.sessionWrite
            .then(() => writeSession(layout.sessionPath, threadId))
            .catch(() => undefined);
        }
      },
      onWarning: (warning) => {
        if (options.onWarning) options.onWarning(warning);
        else console.warn(`[codex] ${warning}`);
      }
    });
    child.stdout?.on('data', (data: Buffer) => {
      appendDiagnostic(stdoutChunks, data);
      parser.push(data);
    });
    child.stderr?.on('data', (data: Buffer) => appendDiagnostic(stderrChunks, data));

    const timeoutMs = Math.max(1, options.timeoutMs ?? DEFAULT_TIMEOUT_MS);
    const timer = setTimeout(() => {
      state.timedOut = true;
      this.cancelled.add(req.id);
      this.terminate(state);
    }, timeoutMs);
    timer.unref();
    setTimeoutHandle(timer);

    return await new Promise((resolve) => {
      let settled = false;
      const settle = (result: AttemptResult): void => {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        parser.finish();
        void state.sessionWrite.then(() => resolve(result));
      };
      child.once('error', (error) => settle({
        code: null,
        signal: null,
        stderr: Buffer.concat(stderrChunks).toString('utf8'),
        stdout: Buffer.concat(stdoutChunks).toString('utf8'),
        spawnError: error
      }));
      child.once('close', (code, signal) => settle({
        code,
        signal,
        stderr: Buffer.concat(stderrChunks).toString('utf8'),
        stdout: Buffer.concat(stdoutChunks).toString('utf8'),
        spawnError: null
      }));
    });
  }

  private terminate(state: ActiveRun): void {
    const pid = state.child?.pid;
    if (!pid) return;
    try {
      process.kill(-pid, 'SIGTERM');
    } catch {
      try { state.child?.kill('SIGTERM'); } catch { /* already exited */ }
    }
    if (state.killTimer) clearTimeout(state.killTimer);
    state.killTimer = setTimeout(() => {
      try { process.kill(-pid, 'SIGKILL'); } catch { /* already exited */ }
    }, 3_000);
    state.killTimer.unref();
  }

  private async cleanupCancelled(state: ActiveRun, userData = state.userData): Promise<void> {
    await state.sessionWrite;
    if (state.layout) await preserveCancelledRun(state.layout);
  }
}

const defaultRunner = new CodexRunner();

export const run = (req: CodexRunRequest, options: CodexRunOptions): Promise<CodexRunResult> =>
  defaultRunner.run(req, options);
export const cancel = (id: string): Promise<boolean> => defaultRunner.cancel(id);
export const cancelAll = (): Promise<void> => defaultRunner.cancelAll();
