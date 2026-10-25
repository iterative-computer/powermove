/*
 * Output destinations and atomic commits (contract 14 invariants 4 and 5,
 * plan section 17 "atomic output").
 *
 * The job engine never chooses a path. It asks main for a destination; main
 * creates a temporary file in the destination folder owner-only and
 * exclusively (`open(wx, 0600)`) and answers with a write handle. The
 * engine encodes and verifies into it, then asks main to commit: main moves
 * the file into place under the job's overwrite policy and only then
 * returns the real path. Empty and unverified outputs are removed, and so
 * is every temporary file of an engine launch that crashed.
 *
 * Agent outputs go to the host's automatic destination (the folder agents
 * export to today) with a unique name and no dialog; user outputs go where
 * the user chose, already turned into a handle by main, under the policy main
 * bound to that handle (the export sheet's save panel confirmed a replace;
 * render queue jobs keep both, replace, or stop).
 *
 * PNG sequences are folders: the temporary output is an owner-only folder,
 * and the commit renames it into place (reserving the name by creating an
 * empty folder first, so nothing is ever replaced) or, under replace, moves
 * each frame into the existing folder, then removes the frames of an earlier,
 * longer export of the same sequence (same `<name>_<digits>.<ext>` name, digit
 * count and extension), so the folder never mixes old and new frames; its
 * other files stay. pm_export's AtomicOutput::commit_folder does the same.
 */
import { randomBytes } from 'node:crypto';
import { link, lstat, mkdir, open, readdir, rename, rm, rmdir, stat, unlink } from 'node:fs/promises';
import path from 'node:path';

import type { CommitOutput, OutputCommitted, OutputDestination, OutputDestinationRequest } from '../../shared/engine-protocol';
import { HandleTable } from './handles';

export class OutputError extends Error {
  override name = 'OutputError';
  constructor(readonly code: 'invalidMessage' | 'capabilityRejected' | 'exportFailed', message: string) {
    super(message);
  }
}

const TEMP_PREFIX = '.powermove-';
const TEMP_SUFFIX = '.part';
const NAME_MAX = 255;
/** Our temporary outputs, and the job engine's work files beside them
 *  (`pm_export::AtomicOutput::in_place`: `<temporary>.<16 hex>.partial.<ext>`),
 *  which a crashed or cancelled encode can leave behind. */
const TEMPORARY_NAME = /^\.powermove-[0-9a-f]{16}\.part(?:\.[0-9a-f]{16}\.partial(?:\.[A-Za-z0-9]{1,16})?)?$/;

/** `<name>_<digits>.<ext>` as a sequence key (name, digit count, ext), or null. */
function frameSequence(file: string): string | null {
  const match = /^(.*)_(\d+)\.([^.]+)$/.exec(file);
  return match ? `${match[1]}\u0000${match[2]!.length}\u0000${match[3]}` : null;
}

/** Removes files of the sequences in `written` that this export did not write. */
export async function removeStaleFrames(folder: string, written: readonly string[]): Promise<void> {
  const sequences = new Set(written.map(frameSequence).filter((key): key is string => key !== null));
  if (!sequences.size) return;
  const mine = new Set(written);
  for (const entry of await readdir(folder, { withFileTypes: true })) {
    if (!entry.isFile() || mine.has(entry.name)) continue;
    const key = frameSequence(entry.name);
    if (key && sequences.has(key)) await unlink(path.join(folder, entry.name));
  }
}

/** Removes one of our temporary names: a file, or a folder and its frames. */
async function removeOwn(file: string): Promise<void> {
  const info = await lstat(file).catch(() => null);
  if (info?.isDirectory()) await rm(file, { recursive: true, force: true }).catch(() => {});
  else if (info) await unlink(file).catch(() => {});
}

/** Removes a temporary output and every work file named after it. */
async function removeTemporary(temporary: string): Promise<void> {
  await removeOwn(temporary);
  const base = path.basename(temporary);
  const entries = await readdir(path.dirname(temporary)).catch(() => [] as string[]);
  for (const name of entries) {
    if (name.startsWith(`${base}.`) && TEMPORARY_NAME.test(name)) await removeOwn(path.join(path.dirname(temporary), name));
  }
}

/** Bytes of a temporary output: a file's length, or the sum of a folder's
 *  files; 0 when a folder is empty or holds an empty or non-file entry. */
export async function outputBytes(temporary: string): Promise<number> {
  const info = await lstat(temporary).catch(() => null);
  if (!info) return 0;
  if (!info.isDirectory()) return info.size;
  let total = 0;
  for (const name of await readdir(temporary).catch(() => [] as string[])) {
    const entry = await lstat(path.join(temporary, name)).catch(() => null);
    if (!entry?.isFile() || entry.size === 0) return 0;
    total += entry.size;
  }
  return total;
}

/** Rust's `OutputDestinationRequest::validate`: a bare file name part. */
function bareName(value: string): boolean {
  return value.length > 0 && Buffer.byteLength(value) <= NAME_MAX && !/[/\\\0]/.test(value) && value !== '.' && value !== '..';
}

export function validateDestinationRequest(request: OutputDestinationRequest): void {
  if (typeof request.job !== 'string' || request.job.length === 0 || Buffer.byteLength(request.job) > 256) {
    throw new OutputError('invalidMessage', 'invalid job id');
  }
  if (request.choice.kind === 'agentAutomatic') {
    if (request.overwrite !== 'uniqueName') throw new OutputError('invalidMessage', 'agent outputs always get unique names');
    if (!bareName(request.choice.suggestedName)) throw new OutputError('invalidMessage', 'suggested name must be a bare file name');
    if (!bareName(request.choice.extension) || !/^[A-Za-z0-9]+$/.test(request.choice.extension)) {
      throw new OutputError('invalidMessage', 'extension must be alphanumeric');
    }
  }
}

/** `Name.ext`, `Name (2).ext`, … trimmed so every candidate fits NAME_MAX. */
export function candidateName(base: string, extension: string, attempt: number): string {
  const suffix = attempt ? ` (${attempt + 1})` : '';
  const tail = `${suffix}${extension ? `.${extension}` : ''}`;
  let trimmed = base;
  // Trim whole code points, never half of a surrogate pair.
  while (Buffer.byteLength(trimmed) + Buffer.byteLength(tail) > NAME_MAX) trimmed = Array.from(trimmed).slice(0, -1).join('');
  return `${trimmed}${tail}`;
}

type Pending = {
  job: string;
  session: number;
  port: number;
  temporary: string;
  directory: string;
  /** Final name without the attempt suffix, and its extension. */
  base: string;
  extension: string;
  overwrite: OutputDestinationRequest['overwrite'];
  /** For user-chosen destinations, the exact final path. */
  exact: string | null;
  /** A folder output (PNG sequence). */
  folder: boolean;
};

export type OutputDestinationsOptions = {
  handles: HandleTable;
  /** Where agent outputs go (today `~/Powermove`). */
  automaticDirectory: string;
};

export class OutputDestinations {
  private pending = new Map<string, Pending>();
  /** The overwrite policy main chose for a user destination handle. */
  private policies = new Map<string, OutputDestinationRequest['overwrite']>();

  constructor(private readonly options: OutputDestinationsOptions) {}

  /** The export sheet, the render queue, or the CLI: a destination the user
   *  chose, as a handle bound to the job engine session and port that will
   *  write it. `overwrite` replaces the engine's default for chosen files
   *  (replace: the save panel confirmed it). */
  chooseDestination(file: string, job: string, session: number, port: number, overwrite?: OutputDestinationRequest['overwrite']): string {
    if (!path.isAbsolute(file)) throw new OutputError('invalidMessage', 'A chosen destination is an absolute path.');
    const handle = this.options.handles.mint({ kind: { kind: 'outputWrite', job }, session, port, path: file });
    if (overwrite) this.policies.set(handle, overwrite);
    return handle;
  }

  /** A caller-owned choice is no longer needed after its job has stopped. */
  releaseChoice(handle: string): void {
    this.policies.delete(handle);
    this.options.handles.revoke(handle);
  }

  /** `outputDestination`: creates the temporary file and binds a write handle. */
  async destination(request: OutputDestinationRequest, session: number, port: number): Promise<OutputDestination> {
    validateDestinationRequest(request);
    let directory: string, base: string, extension: string, exact: string | null = null;
    let overwrite = request.overwrite;
    const folder = request.folder === true;
    if (request.choice.kind === 'agentAutomatic') {
      directory = this.options.automaticDirectory;
      base = request.choice.suggestedName;
      extension = request.choice.extension;
    } else {
      let binding;
      try {
        binding = this.options.handles.check(request.choice.destination, session, port);
      } catch (error) {
        throw new OutputError('capabilityRejected', error instanceof Error ? error.message : String(error));
      }
      if (binding.kind.kind !== 'outputWrite' || binding.kind.job !== request.job) {
        throw new OutputError('capabilityRejected', 'The destination handle is not this job\'s output.');
      }
      exact = binding.path;
      directory = path.dirname(binding.path);
      const name = path.basename(binding.path);
      extension = folder ? '' : path.extname(name).slice(1);
      base = extension ? name.slice(0, -(extension.length + 1)) : name;
      overwrite = this.policies.get(request.choice.destination) ?? overwrite;
      this.policies.delete(request.choice.destination);
    }
    if (folder && request.choice.kind === 'agentAutomatic') extension = '';
    await mkdir(directory, { recursive: true });
    let temporary = '';
    for (let attempt = 0; ; attempt++) {
      temporary = path.join(directory, `${TEMP_PREFIX}${randomBytes(8).toString('hex')}${TEMP_SUFFIX}`);
      try {
        if (folder) {
          // mkdir is exclusive: it fails when the name exists.
          await mkdir(temporary, { mode: 0o700 });
        } else {
          const file = await open(temporary, 'wx', 0o600);
          await file.close();
        }
        break;
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code !== 'EEXIST' || attempt > 8) throw error;
      }
    }
    const handle = this.options.handles.mint({ kind: { kind: 'outputWrite', job: request.job }, session, port, path: temporary });
    this.pending.set(handle, { job: request.job, session, port, temporary, directory, base, extension, overwrite, exact, folder });
    return { job: request.job, temporary: handle };
  }

  /** `commitOutput`: moves a verified output into place, or removes it. */
  async commit(request: CommitOutput, session: number, port: number): Promise<OutputCommitted> {
    let binding;
    try {
      binding = this.options.handles.check(request.temporary, session, port);
    } catch (error) {
      throw new OutputError('capabilityRejected', error instanceof Error ? error.message : String(error));
    }
    const pending = this.pending.get(request.temporary);
    if (!pending || binding.kind.kind !== 'outputWrite' || binding.kind.job !== request.job || pending.job !== request.job) {
      throw new OutputError('capabilityRejected', 'The handle is not this job\'s temporary output.');
    }
    this.pending.delete(request.temporary);
    this.options.handles.revoke(request.temporary);
    const discard = (): Promise<void> => removeTemporary(pending.temporary);
    if (!request.commit) {
      await discard();
      return { job: request.job, path: null };
    }
    const size = pending.folder ? await outputBytes(pending.temporary) : (await stat(pending.temporary).catch(() => null))?.size ?? 0;
    if (size === 0) {
      await discard();
      throw new OutputError('exportFailed', 'The output is empty, so it was removed.');
    }
    if (!request.verification || request.verification.bytes !== size) {
      await discard();
      throw new OutputError('exportFailed', 'The output was not verified, so it was removed.');
    }
    try {
      const placed = await this.place(pending);
      return { job: request.job, path: placed };
    } catch (error) {
      await discard();
      throw error;
    }
  }

  private async place(pending: Pending): Promise<string> {
    if (pending.folder) return this.placeFolder(pending);
    if (pending.exact && pending.overwrite === 'replace') {
      await rename(pending.temporary, pending.exact);
      return pending.exact;
    }
    const attempts = pending.exact && pending.overwrite === 'fail' ? 1 : 10_000;
    for (let attempt = 0; attempt < attempts; attempt++) {
      const candidate = pending.exact && attempt === 0
        ? pending.exact
        : path.join(pending.directory, candidateName(pending.base, pending.extension, attempt));
      if (await this.placeNoClobber(pending.temporary, candidate)) return candidate;
    }
    if (attempts === 1) throw new OutputError('exportFailed', 'A file with that name already exists.');
    throw new OutputError('exportFailed', 'Could not choose a free export file name.');
  }

  private async placeFolder(pending: Pending): Promise<string> {
    if (pending.exact && pending.overwrite === 'replace') {
      const existing = await lstat(pending.exact).catch(() => null);
      if (existing?.isDirectory()) {
        // Frames of the same name are replaced; the folder's other files stay.
        const written = await readdir(pending.temporary);
        for (const name of written) await rename(path.join(pending.temporary, name), path.join(pending.exact, name));
        await rmdir(pending.temporary).catch(() => {});
        // Only once every new frame is in place: the earlier export's extra frames.
        await removeStaleFrames(pending.exact, written);
      } else {
        if (existing) throw new OutputError('exportFailed', 'A file with that name already exists.');
        await rename(pending.temporary, pending.exact);
      }
      return pending.exact;
    }
    const attempts = pending.exact && pending.overwrite === 'fail' ? 1 : 10_000;
    for (let attempt = 0; attempt < attempts; attempt++) {
      const candidate = pending.exact && attempt === 0
        ? pending.exact
        : path.join(pending.directory, candidateName(pending.base, '', attempt));
      try {
        // Reserve the name: an empty folder, which rename then replaces.
        await mkdir(candidate);
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code === 'EEXIST') continue;
        throw error;
      }
      await rename(pending.temporary, candidate);
      return candidate;
    }
    if (attempts === 1) throw new OutputError('exportFailed', 'A folder with that name already exists.');
    throw new OutputError('exportFailed', 'Could not choose a free export folder name.');
  }

  /** Moves without replacing anything: a hard link fails if the name exists.
   *  Volumes without hard links reserve the name exclusively, then rename. */
  private async placeNoClobber(temporary: string, candidate: string): Promise<boolean> {
    try {
      await link(temporary, candidate);
      await unlink(temporary);
      return true;
    } catch (error) {
      const code = (error as NodeJS.ErrnoException).code;
      if (code === 'EEXIST') return false;
      if (code !== 'EPERM' && code !== 'ENOTSUP' && code !== 'EOPNOTSUPP' && code !== 'EXDEV') throw error;
    }
    try {
      const reserved = await open(candidate, 'wx', 0o600);
      await reserved.close();
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === 'EEXIST') return false;
      throw error;
    }
    await rename(temporary, candidate);
    return true;
  }

  /** Removes every temporary output of an engine launch (it crashed or
   *  exited) and revokes their handles. */
  async abandonPort(port: number): Promise<number> {
    let removed = 0;
    for (const [handle, pending] of [...this.pending]) {
      if (pending.port !== port) continue;
      this.pending.delete(handle);
      this.options.handles.revoke(handle);
      await removeTemporary(pending.temporary);
      removed++;
    }
    return removed;
  }

  /** Removes temporary outputs left in the automatic folder by a previous
   *  run that ended without cleanup. Only our exact temporary names. */
  async sweepAutomaticDirectory(): Promise<number> {
    const entries = await readdir(this.options.automaticDirectory).catch(() => [] as string[]);
    let removed = 0;
    for (const name of entries) {
      if (!TEMPORARY_NAME.test(name)) continue;
      await removeOwn(path.join(this.options.automaticDirectory, name));
      removed++;
    }
    return removed;
  }

  get size(): number {
    return this.pending.size;
  }
}
