import { spawn } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { mkdir, mkdtemp, open, readFile, readdir, realpath, rename, rm, stat, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import type { AgentToolContent, CodexRunResult } from '../shared/ipc';
import { EXTENSION_ID, parseManifest } from '../shared/extensions';
import { compileExtension } from './extensions/compiler';
import { collectArtifacts, mimeTypeForPath } from './codex/artifacts';
import { publishExtensionChanges, withStageSnapshot } from './codex/change-history';
import { validateStagedExtensions } from './codex/validate-staged-extensions';
import { loginShellPath } from './login-shell-path';
import { killStrays, ProcessFamily } from './process-family';
import { agentResultSchema } from './codex/instructions';
import type { AgentWorkspace } from './codex/workspace';
import type { PowermoveAgentToolSpec } from './agent-tools/spec';
import { powershellPath } from './windows-system';
import { windowsSandboxArgs } from './windows-sandbox';
import { discoverCodexBinary } from './codex/env';

const object = (properties: Record<string, unknown>, required: string[]) => ({ type: 'object', additionalProperties: false, properties, required });
const COMMAND_TIMEOUT_MS = 600_000;
const MAX_BACKGROUND_JOBS = 4;
const JOB_OUTPUT_CHARS = 20_000;
const MAX_READ_BYTES = 8 * 1024 * 1024;
export const COMPATIBLE_WORKSPACE_TOOLS: readonly PowermoveAgentToolSpec[] = [
  { name: 'list_files', description: 'List a workspace directory. Paths may be absolute or relative to the workspace.', inputSchema: object({ path: { type: 'string' } }, ['path']) },
  { name: 'read_file', description: 'Read a UTF-8 file, or return an image for visual inspection. Use offset/limit to page large text files. Read shipped API types and samples before implementing extensions.', inputSchema: object({ path: { type: 'string' }, offset: { type: 'integer', minimum: 0 }, limit: { type: 'integer', minimum: 1, maximum: 100000 } }, ['path']) },
  { name: 'write_file', description: 'Create or replace a UTF-8 file. Creates parent directories. Write extensions only in the supplied staging directory and deliverables in the run artifact directory. Read existing files before replacing them.', inputSchema: object({ path: { type: 'string' }, text: { type: 'string', maxLength: 1000000 } }, ['path', 'text']) },
  { name: 'run_command', description: 'Run a shell command in the project workspace. Use for searching, editing, scripts, tests, downloads and web research (curl). Commands have full internet access. Project access restricts filesystem writes to this workspace; Computer access allows broader operations. Commands time out after 30 seconds unless timeoutMs allows up to 600 seconds. For longer work such as renders or installs, set background: true and follow the job with command_status. Processes a command leaves running stop when it exits; background jobs stop when complete_task runs or the run ends. Output is bounded. Never repeat a timed-out mutation without inspecting its result.', inputSchema: object({ command: { type: 'string', maxLength: 100000 }, timeoutMs: { type: 'integer', minimum: 1, maximum: COMMAND_TIMEOUT_MS }, background: { type: 'boolean' } }, ['command']) },
  { name: 'command_status', description: 'Check a background job from run_command: its state, exit code and recent output. Set waitMs to wait up to 120 seconds for it to finish, or stop: true to stop it and everything it started.', inputSchema: object({ jobId: { type: 'string' }, waitMs: { type: 'integer', minimum: 0, maximum: 120000 }, stop: { type: 'boolean' } }, ['jobId']) },
  { name: 'compile_extension', description: 'Compile a staged extension with Powermove’s real compiler. Returns compilation errors for repair. This does not activate it; after completing this run, Powermove loads it and continues the task for live verification.', inputSchema: object({ id: { type: 'string', pattern: EXTENSION_ID.source } }, ['id']) },
  { name: 'complete_task', description: 'Finish the run with its summary, typed project commands, artifacts and all staged extension changes. Validates and publishes the staged extensions. Return commands: [] for edits already applied through live tools. Powermove loads extensions before applying dependent commands and continues with live verification. If this tool fails, repair the reported problem and call it again.', inputSchema: agentResultSchema() }
];

export class CompatibleWorkspace {
  private readonly commands = new Set<WorkspaceCommand>();
  private readonly jobs = new Map<string, WorkspaceCommand>();
  private readonly startedAt = Date.now();
  /** Every Project command of this run carries it, so the run end finds what any of them left. */
  private readonly mark = sandboxMark();

  constructor(readonly layout: AgentWorkspace, readonly access: 'project' | 'computer', readonly context: 'app' | 'project' = 'project') {}

  private async start(command: string, timeoutMs: number, signal: AbortSignal,
    options: { keepTail?: boolean; input?: string } = {}): Promise<WorkspaceCommand> {
    const started = await startWorkspaceCommand(this.layout.root, this.access, command, { timeoutMs, signal, ...options, runMark: this.mark });
    this.commands.add(started);
    void started.done.finally(() => this.commands.delete(started)).catch(() => undefined);
    return started;
  }

  /**
   * Stop every command and background job of this run, with everything they
   * started, then anything that escaped a finished command. Runs before
   * complete_task validates and at run end.
   */
  async stopCommands(): Promise<void> {
    await Promise.all([...this.commands].map(command => command.stop()));
    await killStrays({ cwd: await realpath(this.layout.root), since: this.startedAt, ...(this.access === 'project' ? { marks: [this.mark] } : {}) });
  }

  /** Resolve existing ancestors too, so symlinks cannot redirect file writes. */
  private async resolve(file: unknown): Promise<string> {
    if (typeof file !== 'string' || !file || file.includes('\0')) throw new Error('Provide a valid file path.');
    const root = await realpath(this.layout.root);
    const logicalRoot = path.resolve(this.layout.root);
    let candidate = path.resolve(root, file);
    // macOS commonly supplies /var paths whose real spelling is /private/var.
    if (candidate === logicalRoot || candidate.startsWith(logicalRoot + path.sep)) {
      candidate = path.resolve(root, path.relative(logicalRoot, candidate));
    }
    if (this.access === 'computer') return candidate;
    const contained = (value: string) => value === root || value.startsWith(root + path.sep);
    if (!contained(candidate)) throw new Error('This path is outside the project workspace.');
    let ancestor = candidate;
    for (;;) {
      try {
        if (!contained(await realpath(ancestor))) throw new Error('This path leaves the project workspace through a symbolic link.');
        return candidate;
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error;
        ancestor = path.dirname(ancestor);
      }
    }
  }

  async call(name: string, args: Record<string, unknown>, signal: AbortSignal): Promise<AgentToolContent[]> {
    signal.throwIfAborted();
    const text = (value: unknown): AgentToolContent[] => [{ type: 'text', text: typeof value === 'string' ? value : JSON.stringify(value) }];
    if (name === 'list_files') {
      const directory = await this.resolve(args.path);
      const entries = (await readdir(directory, { withFileTypes: true })).map(entry => ({ name: entry.name, directory: entry.isDirectory() }));
      return text({ entries: entries.slice(0, 1000), total: entries.length });
    }
    if (name === 'read_file') {
      const file = await this.resolve(args.path);
      const info = await stat(file);
      if (!info.isFile() || info.size > MAX_READ_BYTES) throw new Error('Read a regular file no larger than 8 MB; use run_command to inspect larger files.');
      const data = await readFile(file);
      const mime = mimeTypeForPath(file);
      if (mime === 'image/png' || mime === 'image/jpeg') {
        return [{ type: 'image', mimeType: mime, data: new Uint8Array(data) }];
      }
      const source = data.toString('utf8');
      const offset = boundedInteger(args.offset, 0, 0, source.length);
      const limit = boundedInteger(args.limit, 30000, 1, 100000);
      return text({ text: source.slice(offset, offset + limit), offset, totalChars: source.length });
    }
    if (name === 'write_file') {
      if (typeof args.text !== 'string' || args.text.length > 1000000) throw new Error('Provide text no larger than 1,000,000 characters.');
      const file = await this.resolve(args.path);
      if (this.access === 'project' && process.platform === 'darwin') {
        // The sandbox writes it: a link planted after, or dangling past, the
        // check above still cannot lead outside the workspace.
        const quote = (value: string) => "'" + value.replaceAll("'", "'\\''") + "'";
        const written = await settleCommand(await this.start(`/bin/mkdir -p -- ${quote(path.dirname(file))} && /bin/cat >| ${quote(file)}`,
          30000, signal, { input: args.text }), signal);
        if (written.exitCode !== 0) throw new Error(`Could not write ${file}: ${written.output.trim()}`);
      } else {
        await mkdir(path.dirname(file), { recursive: true });
        await writeFile(file, args.text, 'utf8');
      }
      return text({ path: file, bytes: Buffer.byteLength(args.text) });
    }
    if (name === 'run_command') {
      if (typeof args.command !== 'string' || !args.command.trim() || args.command.length > 100000) throw new Error('Provide a nonempty command, at most 100,000 characters.');
      if (args.background !== undefined && typeof args.background !== 'boolean') throw new Error('background must be true or false.');
      if (!args.background) {
        return text(await settleCommand(await this.start(args.command, boundedInteger(args.timeoutMs, 30000, 1, COMMAND_TIMEOUT_MS), signal), signal));
      }
      if ([...this.jobs.values()].filter(job => job.running).length >= MAX_BACKGROUND_JOBS) {
        throw new Error(`At most ${MAX_BACKGROUND_JOBS} background jobs can run at once. Wait for or stop one with command_status.`);
      }
      const job = await this.start(args.command, boundedInteger(args.timeoutMs, COMMAND_TIMEOUT_MS, 1, COMMAND_TIMEOUT_MS), signal, { keepTail: true });
      const jobId = `job-${this.jobs.size + 1}`;
      this.jobs.set(jobId, job);
      return text({ jobId, state: 'running' });
    }
    if (name === 'command_status') {
      const job = typeof args.jobId === 'string' ? this.jobs.get(args.jobId) : undefined;
      if (!job) throw new Error('Unknown background job. Use a jobId returned by run_command.');
      if (args.stop !== undefined && typeof args.stop !== 'boolean') throw new Error('stop must be true or false.');
      const waitMs = boundedInteger(args.waitMs, 0, 0, 120000);
      if (args.stop) await job.stop();
      else if (waitMs && job.running) {
        const waited = new AbortController();
        try { await Promise.race([job.done.catch(() => undefined), delay(waitMs, undefined, { signal: AbortSignal.any([signal, waited.signal]) })]); }
        finally { waited.abort(); }
        signal.throwIfAborted();
      }
      return text({ jobId: args.jobId, ...job.status() });
    }
    if (name === 'compile_extension') return text(await this.compile(args.id));
    throw new Error(`Unknown workspace tool: ${name}`);
  }

  private async compile(id: unknown) {
    if (typeof id !== 'string' || !EXTENSION_ID.test(id)) throw new Error('Provide a valid extension id.');
    const dir = path.join(this.layout.stagingDirectory, id);
    const manifest = parseManifest(JSON.parse(await readFile(path.join(dir, 'manifest.json'), 'utf8')));
    if (!manifest.ok) throw new Error(manifest.error);
    if (manifest.manifest.id !== id) throw new Error('The manifest id must match its folder.');
    // Only the result matters; the bundle goes where commands cannot plant links.
    const outDir = await mkdtemp(path.join(os.tmpdir(), 'powermove-compile-'));
    try {
      const compiled = await compileExtension({ dir, entry: manifest.manifest.entry || 'index.ts', outDir });
      return compiled.ok ? { ok: true as const, hash: compiled.hash } : compiled;
    } finally { await rm(outDir, { recursive: true, force: true }); }
  }

  async finish(value: Record<string, unknown>, signal: AbortSignal): Promise<CodexRunResult> {
    await this.stopCommands();
    if (typeof value.summary !== 'string' || !Array.isArray(value.commands) || value.commands.length > 80
      || !value.commands.every(command => typeof command === 'string')
      || !Array.isArray(value.extensions) || value.extensions.length > 32
      || !Array.isArray(value.artifacts) || !Array.isArray(value.notes) || !Array.isArray(value.externalActions)) {
      throw new Error('complete_task requires summary, commands, artifacts, extensions, notes and externalActions matching its schema.');
    }
    if (this.context === 'app' && ((value.commands as unknown[]).length ||
      (value.artifacts as unknown[]).some((item: any) => item?.importToTimeline === true))) {
      throw new Error('No project is attached. Return commands: [] and do not import artifacts to a timeline.');
    }
    const extensions = value.extensions.map((item: any) => {
      if (!item || typeof item.id !== 'string' || !EXTENSION_ID.test(item.id)
        || !['created', 'updated', 'removed'].includes(item.action)) throw new Error('Invalid extension change.');
      return { id: item.id, action: item.action as 'created' | 'updated' | 'removed', summary: typeof item.summary === 'string' ? item.summary : '' };
    });
    // One private copy is checked and published; later stage writes cannot ship.
    return withStageSnapshot(this.layout, async snapshot => {
      signal.throwIfAborted();
      // The same manifest, credential and compile checks as Codex and Claude runs.
      await validateStagedExtensions(snapshot, extensions);
      signal.throwIfAborted();
      const artifacts = await collectArtifacts(this.layout.runDirectory, this.layout.runId, value.artifacts as unknown[]);
      signal.throwIfAborted();
      const changeSet = await publishExtensionChanges(snapshot, extensions);
      return { ok: true as const, access: this.access, text: JSON.stringify({ ...value, extensions, artifacts, projectId: this.layout.projectId }), extensions,
        ...(changeSet ? { extensionChangeSetId: changeSet.id } : {}) };
    }, extensions.map(change => change.id));
  }
}

function boundedInteger(value: unknown, fallback: number, min: number, max: number): number {
  if (value === undefined) return fallback;
  if (!Number.isInteger(value) || (value as number) < min || (value as number) > max) throw new Error(`Expected an integer between ${min} and ${max}.`);
  return value as number;
}

// macOS mktemp ignores TMPDIR for -t and bare calls and uses the per-user
// temp folder, which Project commands must not write: other tools run code
// cached there. Default those templates to $TMPDIR; keep explicit ones.
const MKTEMP_SHIM = `#!/bin/zsh -f
emulate -L zsh
local -a flags templates resolved
local dir= prefix= arg rest c named=0
while (( $# )); do
  arg=$1; shift
  case $arg in
    --) templates+=("$@"); break ;;
    --tmpdir) dir=\${TMPDIR:-/tmp} ;;
    --tmpdir=*) dir=\${arg#--tmpdir=} ;;
    --*) flags+=("$arg") ;;
    -?*)
      rest=\${arg#-}
      while [[ -n $rest ]]; do
        c=\${rest[1]}; rest=\${rest[2,-1]}
        case $c in
          t|p)
            local value=$rest; rest=
            if [[ -z $value ]] && (( $# )); then value=$1; shift; fi
            if [[ $c == t ]]; then prefix=$value; named=1; else dir=$value; fi ;;
          *) flags+=("-$c") ;;
        esac
      done ;;
    *) templates+=("$arg") ;;
  esac
done
local base=\${\${dir:-\${TMPDIR:-/tmp}}%/}
for arg in "\${templates[@]}"; do
  if [[ -n $dir && $arg != /* ]]; then resolved+=("$base/$arg"); else resolved+=("$arg"); fi
done
if (( named || ! \${#templates} )); then resolved+=("$base/\${prefix:-tmp}.XXXXXXXX"); fi
exec /usr/bin/mktemp "\${flags[@]}" -- "\${resolved[@]}"
`;

// HOME stays unwritable in Project access so a run cannot poison packages
// the user's other projects share; package tools cache in the workspace.
const PROJECT_CACHES = [['BUN_INSTALL_CACHE_DIR', 'bun'], ['npm_config_cache', 'npm'], ['XDG_CACHE_HOME', 'xdg'],
  ['PIP_CACHE_DIR', 'pip'], ['CLANG_MODULE_CACHE_PATH', 'clang']] as const;

const shimWrites = new Map<string, Promise<string>>();

/**
 * Tool shims live beside Agent Workspaces, never inside one: Project commands
 * cannot rewrite them, and main never writes where commands can, since a
 * planted link would redirect the write. Written once per launch; the rename
 * replaces whatever is at the name instead of following it.
 */
function toolShims(root: string): Promise<string> {
  const bin = path.join(path.dirname(path.dirname(root)), 'Agent Tools', 'bin');
  let written = shimWrites.get(bin);
  if (!written) {
    written = (async () => {
      await mkdir(bin, { recursive: true });
      const temporary = path.join(bin, `.mktemp-${randomUUID()}`);
      try {
        const file = await open(temporary, 'wx', 0o755);
        try { await file.writeFile(MKTEMP_SHIM); await file.chmod(0o755); } finally { await file.close(); }
        await rename(temporary, path.join(bin, 'mktemp'));
      } finally { await rm(temporary, { force: true }); }
      return bin;
    })();
    shimWrites.set(bin, written);
    written.catch(() => shimWrites.delete(bin));
  }
  return written;
}

/** Scratch inside the workspace and tool shims beside it. */
async function commandEnvironment(root: string, access: 'project' | 'computer'): Promise<NodeJS.ProcessEnv> {
  const scratch = path.join(root, '.powermove', 'tmp');
  const bin = process.platform === 'darwin' ? await toolShims(root) : null;
  const PATH = await loginShellPath();
  // Keep account keys and provider configuration out of subprocess environments.
  const env: NodeJS.ProcessEnv = { PATH: bin ? `${bin}:${PATH}` : PATH, HOME: process.env.HOME, LANG: 'en_US.UTF-8',
    TMPDIR: scratch, TMP: scratch, TEMP: scratch, TMPPREFIX: path.join(scratch, 'zsh') };
  if (process.platform === 'win32') {
    for (const name of ['SystemRoot', 'SYSTEMROOT', 'WINDIR', 'COMSPEC', 'PATHEXT', 'USERPROFILE', 'APPDATA', 'LOCALAPPDATA']) env[name] = process.env[name];
    env.PATH = PATH;
  }
  if (access === 'project') for (const [name, tool] of PROJECT_CACHES) env[name] = path.join(root, '.powermove', 'cache', tool);
  return env;
}

export type WorkspaceCommandResult = { output: string; exitCode: number | null; truncated: boolean };
type CommandEnding = 'timeout' | 'abort' | 'stop';

/** A started command. `done` settles once it and everything it started are gone. */
export interface WorkspaceCommand {
  readonly done: Promise<WorkspaceCommandResult & { ending: CommandEnding | null }>;
  readonly running: boolean;
  status(): { state: 'running' | 'exited' | 'timed out' | 'stopped'; exitCode: number | null; output: string; truncated: boolean };
  stop(): Promise<void>;
}

/** A mach service name no one registers; a sandbox that denies it marks its processes. */
const sandboxMark = () => `com.powermove.command.${randomUUID()}`;

export async function startWorkspaceCommand(root: string, access: 'project' | 'computer', command: string,
  options: { timeoutMs: number; signal: AbortSignal; keepTail?: boolean; input?: string; runMark?: string }): Promise<WorkspaceCommand> {
  const { timeoutMs, signal } = options;
  signal.throwIfAborted();
  const real = await realpath(root);
  const env = await commandEnvironment(real, access);
  // Its own mark finds what this command started, even processes that left
  // its group and folder; the run's mark finds them at the run end.
  const mark = sandboxMark();
  const marks = [mark, ...(options.runMark ? [options.runMark] : [])];
  // Project access keeps unrestricted outbound network for research,
  // downloads and dev servers; only the filesystem is confined.
  const profile = `(version 1)(allow default)(allow network-outbound)(deny appleevent-send)`
    + `(deny mach-lookup ${marks.map(name => `(global-name ${JSON.stringify(name)})`).join(' ')})`
    + `(deny file-write*)(allow file-write* (subpath ${JSON.stringify(await realpath(root))}) (literal "/dev/null") (literal "/dev/tty")`
    // Inherited stdio only; a broad /dev subpath would expose devices.
    + ' (literal "/dev/stdout") (literal "/dev/stderr") (regex #"^/dev/fd/[0-9]+$"))';
  if (access === 'project' && !['darwin', 'win32'].includes(process.platform)) throw new Error('Project command sandbox is available on macOS and Windows.');
  const startedAt = Date.now();
  // The command creates its own scratch folder, so the sandbox, not main,
  // decides where a planted link may lead.
  const windows = process.platform === 'win32';
  const script = 'New-Item -ItemType Directory -Force -Path $env:TMPDIR | Out-Null; ' + command;
  const shell = windows
    ? [powershellPath(), '-NoLogo', '-NoProfile', '-NonInteractive', '-EncodedCommand', Buffer.from(script, 'utf16le').toString('base64')]
    : ['/bin/sh', '-c', `mkdir -p -- "$TMPDIR" 2>/dev/null; exec ${process.platform === 'darwin' ? '/bin/zsh' : '/bin/sh'} -c "$1"`, 'shell', command];
  const sandbox = windows && access === 'project' ? await discoverCodexBinary(null) : null;
  // A configured runtime home carries the user's Windows sandbox choice.
  // Forward only the directory, never provider keys or account environment.
  if (sandbox && process.env.CODEX_HOME) env.CODEX_HOME = process.env.CODEX_HOME;
  const child = spawn(sandbox ?? (access === 'project' ? '/usr/bin/sandbox-exec' : shell[0]!),
    sandbox ? windowsSandboxArgs(real, shell) : access === 'project' ? ['-p', profile, ...shell] : shell.slice(1),
    { cwd: root, env, detached: !windows, windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
  // Closed at once, so stdin reads end as they did from /dev/null.
  child.stdin.on('error', () => undefined); child.stdin.end(options.input);
  let output = '', truncated = false, running = true, exitCode: number | null = null;
  let ending: CommandEnding | null = null, exited = false;
  // Descendants are recorded while it runs, so one that left the group and
  // lost its parent is still killed with the command.
  const family = new ProcessFamily(child.pid ?? null, { cwd: real, since: startedAt, ...(access === 'project' ? { marks: [mark] } : {}) });
  if (child.pid) family.watch();
  const sweep = () => child.pid ? family.kill() : Promise.resolve();
  // A command that already exited on its own is not reported as stopped.
  const end = (reason: CommandEnding) => { if (!exited) ending ??= reason; return sweep(); };
  const onTimeout = () => void end('timeout');
  const onAbort = () => void end('abort');
  const timer = setTimeout(onTimeout, timeoutMs);
  signal.addEventListener('abort', onAbort, { once: true });
  if (signal.aborted) onAbort();
  // Foreground output keeps its start; background jobs keep their latest lines.
  const append = (chunk: Buffer) => {
    const next = chunk.toString('utf8');
    if (options.keepTail) {
      truncated ||= output.length + next.length > JOB_OUTPUT_CHARS;
      output = (output + next).slice(-JOB_OUTPUT_CHARS);
      return;
    }
    truncated ||= output.length + next.length > 100000;
    output += next.slice(0, Math.max(0, 100000 - output.length));
  };
  child.stdout.on('data', append); child.stderr.on('data', append);
  const done = new Promise<WorkspaceCommandResult & { ending: CommandEnding | null }>((resolve, reject) => {
    const cleanup = () => { clearTimeout(timer); signal.removeEventListener('abort', onAbort); running = false; };
    child.on('error', error => { cleanup(); void sweep(); reject(error); });
    // Nothing a command starts outlives it. A process that escaped the kill
    // may still hold the output pipes, so stop waiting on them shortly after.
    child.on('exit', () => {
      exited = true;
      void sweep().finally(() => setTimeout(() => { child.stdout.destroy(); child.stderr.destroy(); }, 250).unref());
    });
    child.on('close', code => { cleanup(); exitCode = code; resolve({ output, exitCode: code, truncated, ending }); });
  });
  done.catch(() => undefined);
  return {
    done,
    get running() { return running; },
    status: () => ({ state: running ? 'running' : ending === 'timeout' ? 'timed out' : ending ? 'stopped' : 'exited', exitCode, output, truncated }),
    stop: async () => { if (running) await end('stop'); await done.catch(() => undefined); }
  };
}

/** Wait for a foreground command; a timeout or Stop is an error the model must inspect. */
async function settleCommand(command: WorkspaceCommand, signal: AbortSignal): Promise<WorkspaceCommandResult> {
  const { ending, ...result } = await command.done;
  if (signal.aborted) throw signal.reason;
  if (ending) throw new Error(`Command ${ending === 'timeout' ? 'timed out' : 'was stopped'}. Inspect its effects before retrying. Output: ${result.output}`);
  return result;
}

export async function runWorkspaceCommand(root: string, access: 'project' | 'computer', command: string, timeoutMs: number, signal: AbortSignal): Promise<WorkspaceCommandResult> {
  return settleCommand(await startWorkspaceCommand(root, access, command, { timeoutMs, signal }), signal);
}
