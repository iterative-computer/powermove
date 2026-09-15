import { spawn } from 'node:child_process';
import { mkdir, readFile, readdir, realpath, stat, writeFile } from 'node:fs/promises';
import path from 'node:path';
import type { AgentToolContent, CodexRunResult } from '../shared/ipc';
import { EXTENSION_ID, parseManifest } from '../shared/extensions';
import { compileExtension } from './extensions/compiler';
import { collectArtifacts, mimeTypeForPath } from './codex/artifacts';
import { publishExtensionChanges } from './codex/change-history';
import { agentResultSchema } from './codex/instructions';
import type { AgentWorkspace } from './codex/workspace';
import type { PowermoveAgentToolSpec } from './agent-tools/spec';

const object = (properties: Record<string, unknown>, required: string[]) => ({ type: 'object', additionalProperties: false, properties, required });
export const COMPATIBLE_WORKSPACE_TOOLS: readonly PowermoveAgentToolSpec[] = [
  { name: 'list_files', description: 'List a workspace directory. Paths may be absolute or relative to the workspace.', inputSchema: object({ path: { type: 'string' } }, ['path']) },
  { name: 'read_file', description: 'Read a UTF-8 file, or return an image for visual inspection. Use offset/limit to page large text files. Read shipped API types and samples before implementing extensions.', inputSchema: object({ path: { type: 'string' }, offset: { type: 'integer', minimum: 0 }, limit: { type: 'integer', minimum: 1, maximum: 100000 } }, ['path']) },
  { name: 'write_file', description: 'Create or replace a UTF-8 file. Creates parent directories. Write extensions only in the supplied staging directory and deliverables in the run artifact directory. Read existing files before replacing them.', inputSchema: object({ path: { type: 'string' }, text: { type: 'string', maxLength: 1000000 } }, ['path', 'text']) },
  { name: 'run_command', description: 'Run a shell command in the project workspace. Use for searching, editing, scripts, tests, downloads and web research (curl). Project access restricts filesystem writes to this workspace; Computer access allows broader operations. Commands time out after at most 120 seconds. Output is bounded. Never repeat a timed-out mutation without inspecting its result.', inputSchema: object({ command: { type: 'string', maxLength: 100000 }, timeoutMs: { type: 'integer', minimum: 1, maximum: 120000 } }, ['command']) },
  { name: 'compile_extension', description: 'Compile a staged extension with Powermove’s real compiler. Returns compilation errors for repair. This does not activate it; after completing this run, Powermove loads it and continues the task for live verification.', inputSchema: object({ id: { type: 'string', pattern: EXTENSION_ID.source } }, ['id']) },
  { name: 'complete_task', description: 'Finish the run with its summary, typed project commands, artifacts and all staged extension changes. Validates and publishes the staged extensions. Return commands: [] for edits already applied through live tools. Powermove loads extensions before applying dependent commands and continues with live verification. If this tool fails, repair the reported problem and call it again.', inputSchema: agentResultSchema() }
];

export class CompatibleWorkspace {
  constructor(readonly layout: AgentWorkspace, readonly access: 'project' | 'computer') {}

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
      const entries = await readdir(await this.resolve(args.path), { withFileTypes: true });
      return text({ entries: entries.slice(0, 1000).map(entry => ({ name: entry.name, directory: entry.isDirectory() })), total: entries.length });
    }
    if (name === 'read_file') {
      const file = await this.resolve(args.path);
      const info = await stat(file);
      if (!info.isFile() || info.size > 8 * 1024 * 1024) throw new Error('Read a regular file no larger than 8 MB; use run_command to inspect larger files.');
      const mime = mimeTypeForPath(file);
      if (mime === 'image/png' || mime === 'image/jpeg') {
        return [{ type: 'image', mimeType: mime, data: new Uint8Array(await readFile(file)) }];
      }
      const source = await readFile(file, 'utf8');
      const offset = boundedInteger(args.offset, 0, 0, source.length);
      const limit = boundedInteger(args.limit, 30000, 1, 100000);
      return text({ text: source.slice(offset, offset + limit), offset, totalChars: source.length });
    }
    if (name === 'write_file') {
      if (typeof args.text !== 'string' || args.text.length > 1000000) throw new Error('Provide text no larger than 1,000,000 characters.');
      const file = await this.resolve(args.path);
      await mkdir(path.dirname(file), { recursive: true });
      await writeFile(file, args.text, 'utf8');
      return text({ path: file, bytes: Buffer.byteLength(args.text) });
    }
    if (name === 'run_command') {
      if (typeof args.command !== 'string' || !args.command.trim() || args.command.length > 100000) throw new Error('Provide a nonempty command, at most 100,000 characters.');
      return text(await runWorkspaceCommand(this.layout.root, this.access, args.command, boundedInteger(args.timeoutMs, 30000, 1, 120000), signal));
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
    return compileExtension({ dir, entry: manifest.manifest.entry || 'index.ts', outDir: path.join(this.layout.runDirectory, '.compiled') });
  }

  async finish(value: Record<string, unknown>, signal: AbortSignal): Promise<CodexRunResult> {
    if (typeof value.summary !== 'string' || !Array.isArray(value.commands) || value.commands.length > 80
      || !value.commands.every(command => typeof command === 'string')
      || !Array.isArray(value.extensions) || value.extensions.length > 32
      || !Array.isArray(value.artifacts) || !Array.isArray(value.notes) || !Array.isArray(value.externalActions)) {
      throw new Error('complete_task requires summary, commands, artifacts, extensions, notes and externalActions matching its schema.');
    }
    const extensions = value.extensions.map((item: any) => {
      if (!item || typeof item.id !== 'string' || !EXTENSION_ID.test(item.id)
        || !['created', 'updated', 'removed'].includes(item.action)) throw new Error('Invalid extension change.');
      return { id: item.id, action: item.action as 'created' | 'updated' | 'removed', summary: typeof item.summary === 'string' ? item.summary : '' };
    });
    for (const change of extensions) {
      signal.throwIfAborted();
      if (change.action === 'removed') continue;
      const compiled = await this.compile(change.id);
      if (!compiled.ok) throw new Error(`${change.id} failed compilation: ${compiled.error}`);
    }
    const artifacts = await collectArtifacts(this.layout.runDirectory, this.layout.runId, value.artifacts);
    signal.throwIfAborted();
    const changeSet = await publishExtensionChanges(this.layout, extensions);
    return { ok: true, access: this.access, text: JSON.stringify({ ...value, extensions, artifacts, projectId: this.layout.projectId }), extensions,
      ...(changeSet ? { extensionChangeSetId: changeSet.id } : {}) };
  }
}

function boundedInteger(value: unknown, fallback: number, min: number, max: number): number {
  if (value === undefined) return fallback;
  if (!Number.isInteger(value) || (value as number) < min || (value as number) > max) throw new Error(`Expected an integer between ${min} and ${max}.`);
  return value as number;
}

export async function runWorkspaceCommand(root: string, access: 'project' | 'computer', command: string, timeoutMs: number, signal: AbortSignal): Promise<{ output: string; exitCode: number | null; truncated: boolean }> {
  signal.throwIfAborted();
  const scratch = path.join(await realpath(root), '.powermove', 'tmp');
  await mkdir(scratch, { recursive: true });
  // Keep account keys and provider configuration out of subprocess environments.
  const env: NodeJS.ProcessEnv = { PATH: process.env.PATH || '/usr/bin:/bin:/usr/sbin:/sbin', HOME: process.env.HOME, LANG: 'en_US.UTF-8', TMPDIR: scratch };
  const profile = `(version 1)(allow default)(deny appleevent-send)(deny file-write*)(allow file-write* (subpath ${JSON.stringify(await realpath(root))}) (literal "/dev/null") (literal "/dev/tty"))`;
  if (access === 'project' && process.platform !== 'darwin') throw new Error('Project command sandbox is only available on macOS.');
  return new Promise((resolve, reject) => {
    const child = spawn(access === 'project' ? '/usr/bin/sandbox-exec' : '/bin/zsh',
      access === 'project' ? ['-p', profile, '/bin/zsh', '-c', command] : ['-c', command],
      { cwd: root, env, detached: true, stdio: ['ignore', 'pipe', 'pipe'] });
    let output = '', truncated = false, stopped = false;
    const stop = () => {
      stopped = true;
      if (child.pid) { try { process.kill(-child.pid, 'SIGKILL'); } catch { /* Already exited. */ } }
    };
    const timer = setTimeout(stop, timeoutMs);
    signal.addEventListener('abort', stop, { once: true });
    if (signal.aborted) stop();
    const append = (chunk: Buffer) => {
      const next = chunk.toString('utf8');
      truncated ||= output.length + next.length > 100000;
      output += next.slice(0, Math.max(0, 100000 - output.length));
    };
    child.stdout.on('data', append); child.stderr.on('data', append);
    const cleanup = () => { clearTimeout(timer); signal.removeEventListener('abort', stop); };
    child.on('error', error => { cleanup(); reject(error); });
    child.on('close', code => {
      cleanup();
      if (signal.aborted) reject(signal.reason);
      else if (stopped) reject(new Error(`Command timed out. Inspect its effects before retrying. Output: ${output}`));
      else resolve({ output, exitCode: code, truncated });
    });
  });
}
