import { spawn, type ChildProcess, type SpawnOptions } from 'node:child_process';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';

import type { CodexAnswerRequest, CodexQuestion, CodexRunRequest, CodexRunResult, CodexTraceEvent } from '../../shared/ipc';
import { isRecord, isString } from '../../shared/guards';
import { collectArtifacts } from '../codex/artifacts';
import { publishExtensionChanges, withStageSnapshot } from '../codex/change-history';
import { AgentResultValidationError, repairAgentResult } from '../codex/result-repair';
import { validateStagedExtensions } from '../codex/validate-staged-extensions';
import { consumeToken } from '../codex/consent';
import { agentInstructions, agentResultSchema } from '../codex/instructions';
import { parseAgentExtensionChanges } from '../codex/runner';
import {
  agentWorkspaceRoot,
  clearSession,
  discardExtensionStage,
  preserveCancelledRun,
  removeEmptyRunDirectory,
  prepareAgentWorkspace,
  readSession,
  sessionPathFor,
  writeSession,
  type AgentApiPackFile,
  type AgentWorkspace,
  type CodexAuthority
} from '../codex/workspace';
import { buildClaudeArgv, claudeUserMessage } from './adapter';
import { loadUserMcpServers, type UserMcpServers } from '../agent-tools/user-mcp';
import { discoverClaudeBinary } from './env';
import { ClaudeEventParser } from './events';
import { isolatedClaudeEnvironment, prepareIsolatedClaudeHome } from './isolation';
import type { NativeMcpServerConfig } from '../agent-tools/spec';
import { imageExtension } from '../image-extension';
import type { ApprovalRequest, RequestApproval } from '../agent-approvals';

const DEFAULT_TIMEOUT_MS = 3_600_000;
const MAX_DIAGNOSTIC_BYTES = 2 * 1024 * 1024;
const QUESTION_LIMITS = { questions: 6, options: 8, header: 80, question: 1_000, label: 160, description: 400 } as const;

function clip(value: unknown, limit: number): string {
  return typeof value === 'string' ? value.replace(/[\u0000-\u0008\u000b-\u001f\u007f]/g, '').trim().slice(0, limit) : '';
}

/** A Claude permission prompt, as the person will read it. */
function describePermission(request: Record<string, unknown>, input: Record<string, unknown>): ApprovalRequest {
  const tool = isString(request.display_name, 80) ? request.display_name : isString(request.tool_name, 80) ? request.tool_name : 'a tool';
  if (request.tool_name === 'Bash' && isString(input.command)) {
    const reason = clip(input.description, 600) || undefined;
    return {
      title: input.dangerouslyDisableSandbox === true ? 'Run a command outside the project sandbox?' : 'Run this command?',
      detail: input.command,
      ...(reason ? { reason } : {})
    };
  }
  const file = isString(input.file_path) ? input.file_path : isString(input.notebook_path) ? input.notebook_path : null;
  if (file) return { title: `Allow ${tool} to change a file outside the project?`, detail: file };
  return { title: `Allow ${tool}?`, detail: JSON.stringify(input, null, 2) };
}

/** AskUserQuestion input, bounded for the renderer. Claude always accepts a typed "Other". */
function askUserQuestions(itemId: string, input: Record<string, unknown>): { questions: CodexQuestion[]; texts: Map<string, string> } {
  const texts = new Map<string, string>();
  const entries = Array.isArray(input.questions) ? input.questions.slice(0, QUESTION_LIMITS.questions) : [];
  const questions = entries.flatMap((entry, index): CodexQuestion[] => {
    if (!isRecord(entry) || !isString(entry.question)) return [];
    const question = clip(entry.question, QUESTION_LIMITS.question);
    if (!question) return [];
    const id = `${itemId}-${index}`;
    texts.set(id, entry.question);
    const options = (Array.isArray(entry.options) ? entry.options : []).slice(0, QUESTION_LIMITS.options).flatMap((option) => {
      const label = isRecord(option) ? clip(option.label, QUESTION_LIMITS.label) : '';
      return label ? [{ label, description: clip((option as Record<string, unknown>).description, QUESTION_LIMITS.description) }] : [];
    });
    return [{
      id, header: clip(entry.header, QUESTION_LIMITS.header), question, options,
      allowOther: true, secret: false, ...(entry.multiSelect === true ? { multiSelect: true } : {})
    }];
  });
  return { questions, texts };
}

type SpawnLike = (command: string, args: readonly string[], options: SpawnOptions) => ChildProcess;

export interface ClaudeRunOptions {
  userData: string;
  extensionsDir: string;
  apiPackFiles: () => Promise<AgentApiPackFile[]>;
  claudeBinaryPref?: string | null;
  binary?: string;
  timeoutMs?: number;
  onProgress?: (text: string) => void;
  onTrace?: (step: CodexTraceEvent) => void;
  onWarning?: (text: string) => void;
  spawnProcess?: SpawnLike;
  consumeConsentToken?: (token: string) => boolean;
  nativeTools?: NativeMcpServerConfig;
  workspaceId?: string;
  additionalInstructions?: string;
  externalMcpServers?: UserMcpServers;
  /** Asks the person before a Project run steps outside its sandbox. */
  requestApproval?: RequestApproval;
}

/** An AskUserQuestion call held open on the CLI's permission prompt. */
interface HeldQuestion {
  controlId: string;
  input: Record<string, unknown>;
  /** Question id → the question text Claude keys its answers by. */
  texts: Map<string, string>;
}

interface ActiveRun {
  request: CodexRunRequest;
  child: ChildProcess | null;
  /** Held questions by tool_use id (the trace item id). */
  questions: Map<string, HeldQuestion>;
  /** Permission prompts waiting on the person, by control request id. */
  approvals: Map<string, AbortController>;
  requestApproval: RequestApproval | null;
  layout: AgentWorkspace | null;
  killTimer: NodeJS.Timeout | null;
  userData: string;
  sessionWrite: Promise<void>;
}

interface Attempt {
  code: number | null;
  signal: NodeJS.Signals | null;
  stdout: string;
  stderr: string;
  spawnError: Error | null;
  output: unknown;
  resultText: string;
  resultError: string | null;
}

function failure(error: string, cancelled = false): CodexRunResult {
  return { ok: false, error, cancelled };
}

function authorityForAccess(access: CodexRunRequest['access']): CodexAuthority {
  return access === 'computer' ? 'computer' : 'project';
}

function appendBounded(chunks: Buffer[], value: Buffer): void {
  const used = chunks.reduce((sum, chunk) => sum + chunk.byteLength, 0);
  if (used >= MAX_DIAGNOSTIC_BYTES) return;
  chunks.push(value.subarray(0, MAX_DIAGNOSTIC_BYTES - used));
}

function sortJson(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(sortJson);
  if (!isRecord(value)) return value;
  return Object.fromEntries(Object.keys(value).sort().map((key) => [key, sortJson(value[key])]));
}

function humanizeFailure(attempt: Attempt, fallback: string): string {
  const detail = (attempt.resultError || attempt.stderr || attempt.spawnError?.message || '').trim();
  if (!detail) return fallback;
  console.error('[claude] run failed:', detail.slice(0, 4_000));
  if (/not logged in|login required|authentication|oauth|unauthorized|\b401\b/iu.test(detail)) {
    return 'Claude is not connected to Powermove. Connect your Claude subscription and retry.';
  }
  if (/ENOENT|command not found|No such file/iu.test(detail)) {
    return "Powermove's built-in Claude runtime could not be launched. Reinstall Powermove and retry.";
  }
  if (/rate.?limit|\b429\b|overloaded/iu.test(detail)) {
    return 'Claude is rate-limited right now. Wait a moment and retry.';
  }
  return `Claude failed: ${(detail.split(/\r?\n/u).filter(Boolean).at(-1) ?? fallback).slice(0, 240)}`;
}

function unknownSession(text: string): boolean {
  return /(?:unknown|invalid|missing|not found|does not exist|could not find).{0,80}(?:session|conversation)|(?:session|conversation).{0,80}(?:unknown|invalid|missing|not found|does not exist|could not find)/iu.test(text);
}

async function writeEditorImages(req: CodexRunRequest): Promise<{ directory: string; imagePaths: string[] }> {
  const directory = await mkdtemp(path.join(tmpdir(), 'powermove-claude-'));
  const imagePaths: string[] = [];
  for (const [index, image] of req.images.entries()) {
    const extension = imageExtension(image);
    const file = path.join(directory, `reference-${index}.${extension}`);
    await writeFile(file, image);
    imagePaths.push(file);
  }
  return { directory, imagePaths };
}

export class ClaudeRunner {
  private readonly active = new Map<string, ActiveRun>();
  private readonly cancelled = new Set<string>();

  async run(req: CodexRunRequest, options: ClaudeRunOptions): Promise<CodexRunResult> {
    if (this.active.has(req.id)) return failure(`A Claude run with id ${req.id} is already active.`);
    if (req.access === 'computer') {
      const consume = options.consumeConsentToken ?? consumeToken;
      if (req.consentToken === null || !consume(req.consentToken)) {
        return failure('Computer access requires fresh approval for this run.');
      }
    }

    const state: ActiveRun = {
      request: req, child: null, questions: new Map(), approvals: new Map(),
      // Only Project runs ask; Edit runs and Full access never prompt.
      requestApproval: req.mode === 'autonomous' && req.access === 'project' ? options.requestApproval ?? null : null,
      layout: null, killTimer: null,
      userData: options.userData, sessionWrite: Promise.resolve()
    };
    this.active.set(req.id, state);
    let editorDirectory: string | null = null;
    let repairingResult = false;
    try {
      const [binary, configDirectory, externalMcpServers] = await Promise.all([
        options.binary ?? discoverClaudeBinary(options.claudeBinaryPref ?? null),
        prepareIsolatedClaudeHome(options.userData),
        options.externalMcpServers ?? loadUserMcpServers('claude')
      ]);
      if (this.cancelled.has(req.id)) return failure('The Claude run was cancelled.', true);

      if (req.mode === 'editor') {
        const editor = await writeEditorImages(req);
        editorDirectory = editor.directory;
        const attempt = await this.execute(req, state, binary, configDirectory, editor.directory, buildClaudeArgv({
          schema: req.schema ?? { type: 'object' },
          prompt: req.prompt,
          imagePaths: editor.imagePaths,
          model: req.model,
          reasoningEffort: req.reasoningEffort,
          sessionId: null,
          access: 'editor',
          nativeTools: options.nativeTools,
          additionalInstructions: options.additionalInstructions,
          externalMcpServers
        }), claudeUserMessage(req.prompt, editor.imagePaths), null, options);
        if (this.cancelled.has(req.id)) return failure('The Claude run was cancelled.', true);
        if (attempt.code !== 0 || attempt.resultError) return failure(humanizeFailure(attempt, 'Claude generation failed.'));
        const output = attempt.output ?? (() => {
          try { return JSON.parse(attempt.resultText); } catch { return undefined; }
        })();
        return output === undefined
          ? failure('Claude completed without a structured result.')
          : { ok: true, text: JSON.stringify(output), access: 'editor' };
      }

      const authority = authorityForAccess(req.access);
      let apiPackFiles: AgentApiPackFile[] = [];
      try { apiPackFiles = await options.apiPackFiles(); }
      catch (error) { options.onWarning?.(`API pack unavailable: ${String(error)}`); }
      const layout = await prepareAgentWorkspace(req, options.userData, authority, agentResultSchema(), {
        extensionsDir: options.extensionsDir,
        workspaceId: options.workspaceId,
        apiPackFiles
      });
      state.layout = layout;
      let sessionId = await readSession(layout.sessionPath);
      let attempt: Attempt | null = null;
      const executeAutonomous = async (prompt: string, resumeId: string | null) => {
        if (this.cancelled.has(req.id)) throw new Error('The Claude run was cancelled.');
        const result = await this.execute(req, state, binary, configDirectory, layout.root, buildClaudeArgv({
          schema: agentResultSchema(),
          prompt,
          imagePaths: layout.imagePaths,
          model: req.model,
          reasoningEffort: req.reasoningEffort,
          sessionId: resumeId,
          access: req.access,
          extensionsDir: layout.extensionsDir,
          instructions: agentInstructions({
            projectName: req.projectName,
            artifactPath: `artifacts/${layout.runId}`,
            access: authority,
            context: req.context,
            request: prompt,
            extensionsDir: layout.extensionsDir
          }),
          nativeTools: options.nativeTools,
          additionalInstructions: options.additionalInstructions,
          externalMcpServers,
          askOutsideSandbox: state.requestApproval !== null
        }), claudeUserMessage(prompt, layout.imagePaths), layout, options);
        if (this.cancelled.has(req.id)) throw new Error('The Claude run was cancelled.');
        return result;
      };
      for (let index = 0; index < 2; index += 1) {
        attempt = await executeAutonomous(req.prompt, sessionId);
        const diagnostic = `${attempt.stderr}\n${attempt.resultError ?? ''}`;
        if (index === 0 && sessionId && attempt.code !== 0 && unknownSession(diagnostic)) {
          options.onProgress?.('The saved Claude thread could not be resumed — starting a fresh run…');
          await state.sessionWrite;
          await clearSession(layout.sessionPath);
          sessionId = null;
          continue;
        }
        break;
      }
      if (!attempt || attempt.code !== 0 || attempt.resultError) {
        await state.sessionWrite;
        await clearSession(layout.sessionPath);
        return failure(attempt ? humanizeFailure(attempt, 'The Claude agent failed.') : 'The Claude agent failed.');
      }
      const { parsed, extensions, changeSet } = await repairAgentResult(async () => {
        if (this.cancelled.has(req.id)) throw new Error('The Claude run was cancelled.');
        const output = attempt!.output ?? (() => {
          try { return JSON.parse(attempt!.resultText); } catch { return undefined; }
        })();
        if (!isRecord(output)) throw new AgentResultValidationError('Claude returned an invalid autonomous result. Return a JSON object matching the result schema.');
        const parsed: Record<string, unknown> = { ...output };
        if (req.context === 'app' && (Array.isArray(parsed.commands) && parsed.commands.length
          || Array.isArray(parsed.artifacts) && parsed.artifacts.some((item: any) => item?.importToTimeline === true))) {
          throw new AgentResultValidationError('No project is attached. Return commands: [] and do not import artifacts to a timeline.');
        }
        parsed.artifacts = await collectArtifacts(
          layout.runDirectory,
          layout.runId,
          Array.isArray(parsed.artifacts) ? parsed.artifacts : []
        );
        parsed.projectId = req.projectId;
        parsed.access = authority;
        const extensions = parseAgentExtensionChanges(parsed.extensions);
        // One private copy is checked and published; later stage writes cannot ship.
        const changeSet = await withStageSnapshot(layout, async snapshot => {
          await validateStagedExtensions(snapshot, extensions ?? []);
          if (this.cancelled.has(req.id)) throw new Error('The Claude run was cancelled.');
          return publishExtensionChanges(snapshot, extensions ?? []);
        }, (extensions ?? []).map(change => change.id));
        return { parsed, extensions, changeSet };
      }, async prompt => {
        repairingResult = true;
        attempt = await executeAutonomous(prompt, await readSession(layout.sessionPath));
        if (attempt.code !== 0 || attempt.resultError) throw new Error(humanizeFailure(attempt, 'Claude could not correct its result.'));
      }, options.onProgress);
      repairingResult = false;
      await discardExtensionStage(layout);
      return {
        ok: true,
        text: JSON.stringify(sortJson(parsed)),
        access: authority,
        ...(extensions === undefined ? {} : { extensions }),
        ...(changeSet === null ? {} : { extensionChangeSetId: changeSet.id })
      };
    } catch (error) {
      if (this.cancelled.has(req.id)) {
        await this.cleanupCancelled(state, options.userData);
        return failure('The Claude run was cancelled.', true);
      }
      if ((error instanceof AgentResultValidationError || repairingResult) && state.layout) {
        await preserveCancelledRun(state.layout).catch(checkpointError => options.onWarning?.(`Could not save the agent checkpoint: ${String(checkpointError)}`));
      }
      return failure(error instanceof Error ? error.message : String(error));
    } finally {
      if (state.killTimer) clearTimeout(state.killTimer);
      this.active.delete(req.id);
      this.cancelled.delete(req.id);
      if (state.layout) await removeEmptyRunDirectory(state.layout);
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

  /** Settles a held AskUserQuestion with the person's answers; none means skipped. */
  answer(req: CodexAnswerRequest): boolean {
    const state = this.active.get(req.id);
    const held = state?.questions.get(req.itemId);
    if (!state || !held) return false;
    state.questions.delete(req.itemId);
    const answers = Object.fromEntries([...held.texts].flatMap(([id, text]) => {
      const chosen = (req.answers[id] ?? []).map((answer) => answer.trim()).filter(Boolean);
      return chosen.length ? [[text, chosen.join(', ')]] : [];
    }));
    this.respondControl(state, held.controlId, Object.keys(answers).length
      ? { behavior: 'allow', updatedInput: { ...held.input, answers } }
      : { behavior: 'deny', message: 'The user skipped the question. Continue with your best judgement.' });
    return true;
  }

  private respondControl(state: ActiveRun, requestId: string, response: Record<string, unknown>): void {
    const stdin = state.child?.stdin;
    if (!stdin || stdin.destroyed || stdin.writableEnded) return;
    stdin.write(`${JSON.stringify({ type: 'control_response', response: { subtype: 'success', request_id: requestId, response } })}\n`);
  }

  /* The stdio permission prompt. AskUserQuestion becomes a question card and
     waits for the person. In a Project run that asks before leaving its
     sandbox, every other prompt (a command retried outside the sandbox, a
     write outside the workspace) becomes an approval card; otherwise it is
     denied, as --print would without a prompt tool. */
  private control(state: ActiveRun, event: Record<string, unknown>, onTrace?: (step: CodexTraceEvent) => void): void {
    const requestId = isString(event.request_id, 200) ? event.request_id : null;
    if (!requestId) return;
    if (event.type === 'control_cancel_request') {
      state.approvals.get(requestId)?.abort();
      for (const [itemId, held] of state.questions) {
        if (held.controlId !== requestId) continue;
        state.questions.delete(itemId);
        onTrace?.({ kind: 'question-closed', itemId });
      }
      return;
    }
    const request = isRecord(event.request) ? event.request : {};
    if (request.subtype !== 'can_use_tool') {
      const stdin = state.child?.stdin;
      if (stdin && !stdin.destroyed && !stdin.writableEnded) {
        stdin.write(`${JSON.stringify({ type: 'control_response', response: { subtype: 'error', request_id: requestId, error: 'Unsupported by Powermove.' } })}\n`);
      }
      return;
    }
    const itemId = isString(request.tool_use_id, 120) ? request.tool_use_id : null;
    const input = isRecord(request.input) ? request.input : {};
    if (request.tool_name === 'AskUserQuestion' && itemId && onTrace && !state.questions.has(itemId)) {
      const { questions, texts } = askUserQuestions(itemId, input);
      if (questions.length) {
        state.questions.set(itemId, { controlId: requestId, input, texts });
        onTrace({ kind: 'question', itemId, questions, transport: 'reply', blocking: true });
        return;
      }
    }
    if (state.requestApproval && !state.approvals.has(requestId)) {
      const controller = new AbortController();
      state.approvals.set(requestId, controller);
      void state.requestApproval(describePermission(request, input), controller.signal).then(decision => {
        state.approvals.delete(requestId);
        if (controller.signal.aborted) return;
        this.respondControl(state, requestId, decision.allowed
          ? { behavior: 'allow', updatedInput: input }
          : { behavior: 'deny', message: decision.message });
      });
      return;
    }
    this.respondControl(state, requestId, {
      behavior: 'deny',
      message: `Powermove does not allow ${isString(request.tool_name, 80) ? request.tool_name : 'this tool'} in this run.`
    });
  }

  private async execute(
    req: CodexRunRequest,
    state: ActiveRun,
    binary: string,
    configDirectory: string,
    cwd: string,
    argv: string[],
    message: string,
    layout: AgentWorkspace | null,
    options: ClaudeRunOptions
  ): Promise<Attempt> {
    const spawnProcess = options.spawnProcess ?? ((command, args, spawnOptions) => spawn(command, args, spawnOptions));
    let child: ChildProcess;
    try {
      child = spawnProcess(binary, argv, {
        cwd,
        detached: true,
        env: isolatedClaudeEnvironment(configDirectory),
        stdio: ['pipe', 'pipe', 'pipe']
      });
    } catch (error) {
      return {
        code: null, signal: null, stdout: '', stderr: '',
        spawnError: error instanceof Error ? error : new Error(String(error)),
        output: undefined, resultText: '', resultError: null
      };
    }
    state.child = child;
    // A CLI that exits early closes its end; the exit status reports why.
    child.stdin?.on('error', () => undefined);
    child.stdin?.write(message);
    const stdout: Buffer[] = [];
    const stderr: Buffer[] = [];
    const parser = new ClaudeEventParser({
      onProgress: options.onProgress,
      onTrace: options.onTrace,
      onWarning: options.onWarning,
      projectCwd: cwd,
      onControl: (event) => this.control(state, event, options.onTrace),
      // The CLI reads more turns until stdin closes; one prompt is one run.
      onResult: () => { child.stdin?.end(); },
      onSessionId: (sessionId) => {
        if (!layout) return;
        state.sessionWrite = state.sessionWrite
          .then(() => writeSession(layout.sessionPath, sessionId))
          .catch(() => undefined);
      }
    });
    child.stdout?.on('data', (chunk: Buffer) => { appendBounded(stdout, chunk); parser.push(chunk); });
    child.stderr?.on('data', (chunk: Buffer) => appendBounded(stderr, chunk));
    const timer = setTimeout(() => {
      this.cancelled.add(req.id);
      this.terminate(state);
    }, Math.max(1, options.timeoutMs ?? DEFAULT_TIMEOUT_MS));
    timer.unref();
    return await new Promise((resolve) => {
      let settled = false;
      const finish = (code: number | null, signal: NodeJS.Signals | null, spawnError: Error | null): void => {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        parser.finish();
        // Held questions die with the process; the renderer closes their cards.
        state.questions.clear();
        for (const approval of state.approvals.values()) approval.abort();
        state.approvals.clear();
        void state.sessionWrite.then(() => resolve({
          code,
          signal,
          stdout: Buffer.concat(stdout).toString('utf8'),
          stderr: Buffer.concat(stderr).toString('utf8'),
          spawnError,
          output: parser.output,
          resultText: parser.text,
          resultError: parser.error
        }));
      };
      child.once('error', (error) => finish(null, null, error));
      child.once('close', (code, signal) => finish(code, signal, null));
    });
  }

  private terminate(state: ActiveRun): void {
    const pid = state.child?.pid;
    if (!pid) return;
    try { process.kill(-pid, 'SIGINT'); }
    catch { try { state.child?.kill('SIGINT'); } catch { /* already exited */ } }
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
