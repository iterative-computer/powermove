import { createHash } from 'node:crypto';
import { mkdir, readFile, readdir, realpath, rename, rm, stat, utimes, writeFile } from 'node:fs/promises';
import path from 'node:path';

import type { Transcript } from '../../shared/transcription';

/*
 * Finished transcripts, so asking again is instant. The key is the file's
 * identity (real path, size, modification time, inode), the model (its id
 * and file checksum), the language it was told and the span: editing or
 * replacing the file, switching or re-pinning models, or choosing another
 * language for a model that must be told, misses.
 */

/** Bumped when the engine's output changes, so old transcripts miss.
 *  3: transcribe.cpp replaced sherpa-onnx. 4: streaming models' word lag. */
const VERSION = 4;

export interface CacheSpan { start?: number; end?: number; language?: string }

export async function transcriptKey(file: string, modelId: string, span: CacheSpan): Promise<string> {
  const real = await realpath(file);
  const info = await stat(real);
  if (!info.isFile()) throw new Error('Only files can be transcribed.');
  const identity = [VERSION, real, info.size, Math.round(info.mtimeMs), info.ino, modelId, span.start ?? 0, span.end ?? null, span.language ?? null];
  return createHash('sha256').update(JSON.stringify(identity)).digest('hex');
}

export class TranscriptCache {
  private readonly memory = new Map<string, Transcript>();
  private writes = 0;

  constructor(private readonly dir: string, private readonly limit = 400, private readonly memoryLimit = 24) {}

  async get(key: string): Promise<Transcript | null> {
    const hit = this.memory.get(key);
    if (hit) {
      this.memory.delete(key);
      this.memory.set(key, hit);
      return structuredClone(hit);
    }
    const file = path.join(this.dir, `${key}.json`);
    try {
      const transcript = JSON.parse(await readFile(file, 'utf8')) as Transcript;
      if (!transcript || !Array.isArray(transcript.segments)) return null;
      const now = new Date();
      await utimes(file, now, now).catch(() => undefined);
      this.remember(key, transcript);
      return structuredClone(transcript);
    } catch {
      return null;
    }
  }

  async set(key: string, transcript: Transcript): Promise<void> {
    this.remember(key, transcript);
    await mkdir(this.dir, { recursive: true });
    const file = path.join(this.dir, `${key}.json`);
    const temporary = `${file}.${process.pid}.tmp`;
    await writeFile(temporary, JSON.stringify(transcript), 'utf8');
    await rename(temporary, file);
    if (++this.writes % 20 === 1) await this.prune().catch(() => undefined);
  }

  async clear(): Promise<void> {
    this.memory.clear();
    await rm(this.dir, { recursive: true, force: true });
  }

  private remember(key: string, transcript: Transcript): void {
    this.memory.delete(key);
    this.memory.set(key, structuredClone(transcript));
    while (this.memory.size > this.memoryLimit) this.memory.delete(this.memory.keys().next().value!);
  }

  /** Keeps the most recently used `limit` transcripts on disk. */
  async prune(): Promise<void> {
    const entries = await readdir(this.dir).catch(() => [] as string[]);
    const files = await Promise.all(entries.filter((name) => name.endsWith('.json')).map(async (name) => {
      const full = path.join(this.dir, name);
      return { full, time: (await stat(full).catch(() => null))?.mtimeMs ?? 0 };
    }));
    if (files.length <= this.limit) return;
    files.sort((a, b) => b.time - a.time);
    await Promise.all(files.slice(this.limit).map((file) => rm(file.full, { force: true })));
  }
}
