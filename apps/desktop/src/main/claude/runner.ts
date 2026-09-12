import { spawn, type ChildProcess, type SpawnOptions } from 'node:child_process';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';

import type { CodexRunRequest, CodexRunResult, CodexTraceEvent } from '../../shared/ipc';
import { isRecord } from '../../shared/guards';
import { collectArtifacts } from '../codex/artifacts';
import { publishExtensionChanges } from '../codex/change-history';
import { consumeToken } from '../codex/consent';
import { agentInstructions, agentResultSchema } from '../codex/instructions';
import { parseAgentExtensionChanges } from '../codex/runner';
import {
  agentWorkspaceRoot,
  clearSession,
  discardExtensionStage,
  preserveCancelledRun,
  prepareAgentWorkspace,
  readSession,
  sessionPathFor,
  writeSession,
  type AgentApiPackFile,
  type AgentWorkspace,
  type CodexAuthority
} from '../codex/workspace';
import { buildClaudeArgv } from './adapter';
import { discoverClaudeBinary } from './env';
import { ClaudeEventParser } from './events';
import { isolatedClaudeEnvironment, prepareIsolatedClaudeHome } from './isolation';
import type { NativeMcpServerConfig } from '../agent-tools/spec';

const DEFAULT_TIMEOUT_MS = 3_600_000;
const MAX_DIAGNOSTIC_BYTES = 2 * 1024 * 1024;

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
}

interface ActiveRun {
  request: CodexRunRequest;
  child: ChildProcess | null;
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
    const extension = image[0] === 0x89 && image[1] === 0x50 ? 'png' : 'jpg';
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
      request: req, child: null, layout: null, killTimer: null,
      userData: options.userData, sessionWrite: Promise.resolve()
    };
    this.active.set(req.id, state);
    let editorDirectory: string | null = null;
    try {
      const [binary, configDirectory] = await Promise.all([
        options.binary ?? discoverClaudeBinary(options.claudeBinaryPref ?? null),
        prepareIsolatedClaudeHome(options.userData)
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
          access: 'editor'
        }), null, options);
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
        apiPackFiles
      });
      state.layout = layout;
      let sessionId = await readSession(layout.sessionPath);
      let attempt: Attempt | null = null;
      for (let index = 0; index < 2; index += 1) {
        attempt = await this.execute(req, state, binary, configDirectory, layout.root, buildClaudeArgv({
          schema: agentResultSchema(),
          prompt: req.prompt,
          imagePaths: layout.imagePaths,
          model: req.model,
          reasoningEffort: req.reasoningEffort,
          sessionId,
          access: req.access,
          extensionsDir: layout.extensionsDir,
          instructions: agentInstructions({
            projectName: req.projectName,
            artifactPath: `artifacts/${layout.runId}`,
            access: authority,
            extensionsDir: layout.extensionsDir
          }),
          nativeTools: options.nativeTools
        }), layout, options);
        if (this.cancelled.has(req.id)) {
          await this.cleanupCancelled(state, options.userData);
          return failure('The Claude run was cancelled.', true);
        }
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
      const output = attempt.output ?? (() => {
        try { return JSON.parse(attempt.resultText); } catch { return undefined; }
      })();
      if (!isRecord(output)) return failure('Claude returned an invalid autonomous result.');
      const parsed: Record<string, unknown> = { ...output };
      parsed.artifacts = await collectArtifacts(
        layout.runDirectory,
        layout.runId,
        Array.isArray(parsed.artifacts) ? parsed.artifacts : []
      );
      parsed.projectId = req.projectId;
      parsed.access = authority;
      const extensions = parseAgentExtensionChanges(parsed.extensions);
      const changeSet = await publishExtensionChanges(layout, extensions ?? []);
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
      return failure(error instanceof Error ? error.message : String(error));
    } finally {
      if (state.killTimer) clearTimeout(state.killTimer);
      this.active.delete(req.id);
      this.cancelled.delete(req.id);
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
    configDirectory: string,
    cwd: string,
    argv: string[],
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
        stdio: ['ignore', 'pipe', 'pipe']
      });
    } catch (error) {
      return {
        code: null, signal: null, stdout: '', stderr: '',
        spawnError: error instanceof Error ? error : new Error(String(error)),
        output: undefined, resultText: '', resultError: null
      };
    }
    state.child = child;
    const stdout: Buffer[] = [];
    const stderr: Buffer[] = [];
    const parser = new ClaudeEventParser({
      onProgress: options.onProgress,
      onTrace: options.onTrace,
      onWarning: options.onWarning,
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
