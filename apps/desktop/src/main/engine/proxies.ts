/**
 * Whole-clip proxy jobs and machine-local generated-media locations (plan
 * section 14, Phase 4a). Main owns what the project cannot store: where
 * generated media lives on this Mac, which proxies the user deleted, and the
 * generation jobs in flight. The project stores intent only.
 *
 * Every operation that reads or changes the store runs on one serial queue.
 * Per-project locks are not enough: the store is content-addressed, so two
 * projects that use the same original share one proxy file, and a delete or
 * rebuild in one races a create in the other on the same key.
 */
import { randomUUID } from 'node:crypto';
import { mkdir, open, readFile, readdir, realpath, rename, stat, unlink } from 'node:fs/promises';
import path from 'node:path';
import type { ContentHash, EngineProcess, JobStatus, Request } from '../../shared/engine-protocol';
import {
  validGeneratedMediaLocation,
  type AssetProxyState,
  type GeneratedMediaLocation,
  type ProjectProxyIntent,
  type ProxyActivity,
  type ProxyAssetDescriptor,
  type ProxyFormat,
  type ProxySize,
  type ProxySnapshot
} from '../../shared/proxies';
import { isId, ProtocolError } from './protocol';

export type ProxySource = { path: string; contentHash: ContentHash | null };
/** Which project's binding of an asset id a source comes from. */
export type ProxySourceScope = { owner: number; project: string; sequenceHash?: string };
/** The host-call context services.ts sees for a capability mint. */
export type ProxyHostContext = { process: EngineProcess; session: number };
export type ProxyServiceOptions = {
  cache: string;
  request(request: Request): Promise<unknown>;
  /** The original as `scope.project` binds it (prefer the window `scope.owner`, then other windows
   *  bound to the same project). Never another project's binding of the same asset id. */
  resolve(asset: string, scope: ProxySourceScope): Promise<ProxySource | null>;
  changed(owner: number, snapshot: ProxySnapshot): void;
  /**
   * The file's full BLAKE3 digest (lowercase hex), hashed off main's thread
   * (media-store's worker pool). A proxy is ready, and usable for draft, only
   * once its bytes match the checksum in its record, as pm-proxy `lookup`
   * requires: a same-size corruption passes every cheaper check.
   */
  hashFile(file: string): Promise<string>;
  /**
   * "When needed" (section 14.5): whether this original can't play in real
   * time on this Mac, from a measured decode (the job engine's ProbeMedia
   * decode benchmark), never a guess. Called one asset at a time with the
   * source already resolved through `scope`. Resolve true/false; reject with
   * a sentence naming what failed. Results are cached by content hash.
   */
  assess?(asset: string, source: ProxySource, scope: ProxySourceScope): Promise<boolean>;
  /** Identifies this Mac's decode hardware; cached assessments from another machine are dropped. */
  machine?: string;
  log?(level: 'info' | 'warn' | 'error', message: string): void;
  now?(): number;
};

/** pm-proxy `StoredRecord` (serde field names; ProxyFormat/ProxySize are camelCase). */
type StoreRecord = {
  source_hash: string;
  recipe: { version: number; requested_format: ProxyFormat; requested_size: ProxySize; width: number; height: number; source_depth: number; source_has_alpha: boolean };
  byte_size: number;
  blake3: string;
};
type Candidate = {
  root: string; recordFile: string; file: string; mtimeMs: number; record: StoreRecord; problem: string | null;
  /** The media's bytes matched the record's checksum (`verify`). Unchecked candidates are never ready. */
  checked: boolean;
};
type Recipe = { format: ProxyFormat; size: ProxySize };
type Project = {
  key: string; owner: number; id: string; intent: ProjectProxyIntent;
  assets: Record<string, ProxyAssetDescriptor>; projectPath?: string; slow: Set<string>;
};
type Job = {
  id: string; alias: string; hash: string; recipe: Recipe; root: string;
  /** The frozen original and the binding it came from (re-resolved at grant time). */
  source: ProxySource; scope: ProxySourceScope & { asset: string };
  /** project id → asset ids this job makes a proxy for (projects sharing one original share a job). */
  targets: Map<string, Set<string>>;
  state: 'queued' | 'creating' | 'paused'; progress: number; paused: 'lowDisk' | 'interactive' | null;
  /** Pending jobs can be reordered without cancelling a native transcode. */
  submitted: boolean; order: number;
  /** Deleted or replaced: its output is not wanted, its statuses change nothing. */
  discarded: boolean;
};
type Deletion = { hash: string | null; format: ProxyFormat; size: ProxySize; at: number };

const FORMATS: readonly ProxyFormat[] = ['proResProxy', 'proResLt', 'hevc10'];
const SIZES: readonly ProxySize[] = ['automatic', 'half', 'quarter', 'p1080', 'p720'];
const POLICIES = ['whenNeeded', 'allVideo', 'onlyWhenAsked'] as const;
const CODEC: Record<ProxyFormat, string> = { proResProxy: 'ProRes 422 Proxy', proResLt: 'ProRes 422 LT', hevc10: 'HEVC 10-bit' };
const HASH = /^[0-9a-f]{64}$/;
const RECORD = /^[0-9a-f]{64}\.json$/;
const SHARD = /^[0-9a-f]{2}$/;
const MAX_DELETIONS = 50_000;
/** pm-proxy `ProxyRecipe::VERSION`: the engine never plays a record of another version, so neither is it ready here. */
export const RECIPE_VERSION = 2;
export const LOCATIONS_FILE = 'Proxy Locations.json';
export const DELETIONS_FILE = 'Proxy Deletions.json';
export const ASSESSMENTS_FILE = 'Proxy Assessments.json';
export const DEFAULT_ROOT = 'Generated Media';
export const PROJECT_ROOT = 'Powermove Generated Media';

const hex = (hash: ContentHash): string => Buffer.from(hash).toString('hex');
const message = (e: unknown): string => e instanceof Error ? e.message : String(e);
const recipeKey = (hash: string, recipe: Recipe): string => `${hash}:${recipe.format}:${recipe.size}`;
const target = (project: string, asset: string): string => `${project}\u0000${asset}`;
const absolute = (value: unknown): value is string =>
  typeof value === 'string' && path.isAbsolute(value) && value.length <= 4096 && !value.includes('\0');
const eligible = (meta: ProxyAssetDescriptor | undefined): boolean => meta?.kind === 'video' || !!meta?.imageSequence;

/** A project intent as IPC delivers it (shape only; ids are checked with `isId`). */
export function validProxyIntent(value: unknown): value is ProjectProxyIntent {
  if (!value || typeof value !== 'object') return false;
  const intent = value as Record<string, unknown>;
  return POLICIES.includes(intent['create'] as never) && SIZES.includes(intent['size'] as never)
    && FORMATS.includes(intent['format'] as never) && Array.isArray(intent['alwaysPlayOriginal'])
    && intent['alwaysPlayOriginal'].length <= 100_000 && intent['alwaysPlayOriginal'].every(isId);
}

/** Why a store record cannot be used, or null when it is well formed. */
function recordProblem(value: unknown): string | null {
  if (!value || typeof value !== 'object') return 'it is not an object';
  const r = value as Record<string, any>;
  if (typeof r['source_hash'] !== 'string' || !HASH.test(r['source_hash'].toLowerCase())) return 'its source hash is missing';
  const recipe = r['recipe'];
  if (!recipe || typeof recipe !== 'object') return 'its recipe is missing';
  if (!FORMATS.includes(recipe.requested_format) || !SIZES.includes(recipe.requested_size)) return 'its recipe names an unknown format or size';
  for (const field of ['version', 'width', 'height', 'source_depth']) {
    if (!Number.isSafeInteger(recipe[field]) || recipe[field] < 1) return `its recipe ${field} is invalid`;
  }
  if (typeof recipe.source_has_alpha !== 'boolean') return 'its recipe alpha flag is invalid';
  if (!Number.isSafeInteger(r['byte_size']) || r['byte_size'] < 0) return 'its byte size is invalid';
  if (typeof r['blake3'] !== 'string' || !HASH.test(r['blake3'])) return 'its checksum is invalid';
  return null;
}

export class EngineProxyService {
  private readonly defaultRoot: string;
  readonly locationsFile: string;
  readonly deletionsFile: string;
  readonly assessmentsFile: string;
  private projects = new Map<string, Project>();
  private jobs = new Map<string, Job>();
  private aliases = new Map<string, string>();
  private pendingGeneration: string[] = [];
  private activeGeneration: string | null = null;
  private nextGenerationOrder = 0;
  private generationTurn = 0;
  private lastServed = new Map<string, number>();
  /** Project id → nearest-first asset ids (the most recent editor view wins). */
  private priority = new Map<string, { owner: number; ranks: Map<string, number> }>();
  /** The generation lane is serial: the job whose original was granted last is the one minting its store. */
  private current: { job: string; root: string } | null = null;
  private roots: string[] = [];
  private setting: GeneratedMediaLocation = { kind: 'cache' };
  private deletions = new Map<string, Map<string, Deletion>>();
  private failures = new Map<string, { hash: string | null; recipe: Recipe; error: string }>();
  private batches = new Map<string, { done: number; total: number }>();
  private index = new Map<string, Candidate[]>();
  private parsed = new Map<string, { mtimeMs: number; size: number; record: unknown; error: string | null }>();
  /** Media file → its checksum verdict, valid while the file's identity and the expected digest are unchanged. */
  private checksums = new Map<string, { identity: string; expected: string; problem: string | null }>();
  /** content hash → needs a proxy on this Mac (measured). */
  private assessments = new Map<string, { needsProxy: boolean; at: number }>();
  /** content hash → why measuring failed (this launch only; retried after relaunch). */
  private assessFailures = new Map<string, string>();
  private assessing = new Set<string>();
  private assessQueue: Promise<void> = Promise.resolve();
  private persistedRoots = '';
  private playing = false;
  private exporting = new Set<string>();
  private readonly ready: Promise<void>;
  private queue: Promise<unknown>;
  private writes: Promise<unknown> = Promise.resolve();
  private stopped = false;

  constructor(private options: ProxyServiceOptions) {
    this.defaultRoot = path.join(options.cache, DEFAULT_ROOT);
    this.locationsFile = path.join(options.cache, LOCATIONS_FILE);
    this.deletionsFile = path.join(options.cache, DELETIONS_FILE);
    this.assessmentsFile = path.join(options.cache, ASSESSMENTS_FILE);
    this.roots = [this.defaultRoot];
    this.ready = this.load();
    // A failed load is reported by every call (see `serial`), never swallowed.
    this.queue = this.ready.catch(() => {});
  }

  /* ── persistence ── */

  private async readJson(file: string, what: string): Promise<Record<string, unknown> | null> {
    let text: string;
    try { text = await readFile(file, 'utf8'); } catch (e) {
      if ((e as NodeJS.ErrnoException).code === 'ENOENT') return null;
      throw new Error(`Powermove couldn't read its ${what} (${file}): ${message(e)}.`);
    }
    let value: unknown;
    try { value = JSON.parse(text); } catch (e) {
      throw new Error(`Powermove's ${what} file is damaged (${file}: ${message(e)}). Move that file out of the folder to reset it.`);
    }
    if (!value || typeof value !== 'object' || Array.isArray(value)) {
      throw new Error(`Powermove's ${what} file is damaged (${file} is not an object). Move that file out of the folder to reset it.`);
    }
    return value as Record<string, unknown>;
  }

  private async load(): Promise<void> {
    await mkdir(this.options.cache, { recursive: true, mode: 0o700 });
    const damaged = (file: string, what: string, why: string) =>
      new Error(`Powermove's ${what} file is damaged (${file}: ${why}). Move that file out of the folder to reset it.`);
    const locations = await this.readJson(this.locationsFile, 'generated-media locations');
    if (locations) {
      const roots = locations['roots'];
      if (!Array.isArray(roots) || !roots.every(absolute)) throw damaged(this.locationsFile, 'generated-media locations', 'roots must be absolute folders');
      if (!validGeneratedMediaLocation(locations['location'])) throw damaged(this.locationsFile, 'generated-media locations', 'the location is invalid');
      this.setting = structuredClone(locations['location']);
      for (const root of roots) if (!this.roots.includes(root)) this.roots.push(root);
      this.persistedRoots = JSON.stringify(roots);
    }
    const deletions = await this.readJson(this.deletionsFile, 'deleted-proxy list');
    if (deletions) {
      const projects = deletions['deleted'];
      if (deletions['version'] !== 1 || !projects || typeof projects !== 'object') throw damaged(this.deletionsFile, 'deleted-proxy list', 'unknown version');
      for (const [project, assets] of Object.entries(projects as Record<string, unknown>)) {
        if (!isId(project) || !assets || typeof assets !== 'object') throw damaged(this.deletionsFile, 'deleted-proxy list', 'invalid project entry');
        const map = new Map<string, Deletion>();
        for (const [asset, d] of Object.entries(assets as Record<string, any>)) {
          if (!isId(asset) || !d || !(d.hash === null || typeof d.hash === 'string' && HASH.test(d.hash))
            || !FORMATS.includes(d.format) || !SIZES.includes(d.size) || !Number.isFinite(d.at)) {
            throw damaged(this.deletionsFile, 'deleted-proxy list', 'invalid asset entry');
          }
          map.set(asset, { hash: d.hash, format: d.format, size: d.size, at: d.at });
        }
        if (map.size) this.deletions.set(project, map);
      }
    }
    const assessed = await this.readJson(this.assessmentsFile, 'decode measurement cache');
    if (assessed) {
      const results = assessed['results'];
      if (assessed['version'] !== 1 || !results || typeof results !== 'object' || Array.isArray(results)) throw damaged(this.assessmentsFile, 'decode measurement cache', 'unknown version');
      if ((assessed['machine'] ?? null) === (this.options.machine ?? null)) {
        for (const [hash, r] of Object.entries(results as Record<string, any>)) {
          if (!HASH.test(hash) || !r || typeof r.needsProxy !== 'boolean' || !Number.isFinite(r.at)) throw damaged(this.assessmentsFile, 'decode measurement cache', 'invalid entry');
          this.assessments.set(hash, { needsProxy: r.needsProxy, at: r.at });
        }
      }
    }
  }

  /** Atomic, fsynced replace; writes run in order so an older state never lands last. */
  private write(file: string, contents: () => string, written?: () => void): Promise<void> {
    const run = this.writes.then(async () => {
      const temp = path.join(path.dirname(file), `.${path.basename(file)}.${randomUUID()}.tmp`);
      let renamed = false;
      try {
        const handle = await open(temp, 'wx', 0o600);
        try { await handle.writeFile(contents()); await handle.sync(); } finally { await handle.close(); }
        await rename(temp, file);
        renamed = true;
        written?.();
      } finally { if (!renamed) await unlink(temp).catch(() => {}); }
    });
    this.writes = run.catch(() => {});
    return run;
  }
  private persistLocations(): Promise<void> {
    let roots = '';
    return this.write(this.locationsFile, () => {
      roots = JSON.stringify(this.roots);
      return JSON.stringify({ location: this.setting, roots: this.roots });
    }, () => { this.persistedRoots = roots; });
  }
  private persistDeletions(): Promise<void> {
    return this.write(this.deletionsFile, () => {
      const all = [...this.deletions].flatMap(([project, assets]) => [...assets].map(([asset, d]) => ({ project, asset, d })));
      const kept = all.sort((a, b) => b.d.at - a.d.at).slice(0, MAX_DELETIONS);
      const deleted: Record<string, Record<string, Deletion>> = {};
      for (const { project, asset, d } of kept) (deleted[project] ??= {})[asset] = d;
      return JSON.stringify({ version: 1, deleted });
    });
  }
  private persistAssessments(): Promise<void> {
    return this.write(this.assessmentsFile, () => {
      const kept = [...this.assessments].sort((a, b) => b[1].at - a[1].at).slice(0, MAX_DELETIONS);
      return JSON.stringify({ version: 1, machine: this.options.machine ?? null, results: Object.fromEntries(kept) });
    });
  }
  /** The engine finds proxies through the persisted roots: a root is written before a job may use it. */
  private async ensureRoot(root: string): Promise<void> {
    await mkdir(root, { recursive: true, mode: 0o700 });
    if (!this.roots.includes(root)) this.roots.push(root);
    if (this.persistedRoots !== JSON.stringify(this.roots)) await this.persistLocations();
  }

  /** One queue for every store operation; a damaged state file fails each call by name. */
  private serial<T>(work: () => Promise<T>): Promise<T> {
    if (this.stopped) return Promise.reject(new Error('The proxy service has stopped.'));
    const run = this.queue.then(() => this.ready).then(work);
    this.queue = run.catch(() => {});
    return run;
  }

  /* ── the store ── */

  /**
   * Reads every root's records once, in the engine's search order (roots
   * order, newest first), then verifies the checksums of the sources open
   * projects use plus `extra`.
   */
  private async rescan(extra: (string | null)[] = []): Promise<Map<string, Candidate[]>> {
    const index = new Map<string, Candidate[]>();
    const seen = new Set<string>();
    for (const root of this.roots) {
      const shards = await readdir(root, { withFileTypes: true }).catch(() => []);
      const found: Candidate[] = [];
      for (const shard of shards) {
        if (!shard.isDirectory() || !SHARD.test(shard.name)) continue;
        const entries = await readdir(path.join(root, shard.name)).catch(() => [] as string[]);
        for (const entry of entries) {
          if (!RECORD.test(entry)) continue;
          const recordFile = path.join(root, shard.name, entry);
          seen.add(recordFile);
          const info = await stat(recordFile).catch(() => null);
          if (!info?.isFile()) continue;
          let cached = this.parsed.get(recordFile);
          if (!cached || cached.mtimeMs !== info.mtimeMs || cached.size !== info.size) {
            let record: unknown = null, error: string | null = null;
            try { record = JSON.parse(await readFile(recordFile, 'utf8')); error = recordProblem(record); } catch (e) { error = message(e); }
            cached = { mtimeMs: info.mtimeMs, size: info.size, record, error };
            this.parsed.set(recordFile, cached);
            if (error) this.options.log?.('warn', `[proxies] ignoring damaged proxy record ${recordFile}: ${error}`);
          }
          const hash = (cached.record as { source_hash?: unknown } | null)?.source_hash;
          // A record that cannot name its source is unusable by the engine too.
          if (cached.error && !(typeof hash === 'string' && HASH.test(hash.toLowerCase()))) continue;
          const record = cached.record as StoreRecord;
          const file = recordFile.slice(0, -'.json'.length) + '.mov';
          let problem = cached.error;
          if (!problem) {
            const media = await stat(file).catch(() => null);
            if (!media) continue; // the media went: a miss, as in pm-proxy
            if (media.size !== record.byte_size) problem = `the file is ${media.size} bytes but its record says ${record.byte_size}`;
          }
          found.push({ root, recordFile, file, mtimeMs: info.mtimeMs, record, problem: problem && `The proxy file is damaged (${problem}). Rebuild the proxy.`, checked: false });
        }
      }
      found.sort((a, b) => b.mtimeMs - a.mtimeMs);
      for (const c of found) {
        const key = c.record.source_hash.toLowerCase();
        index.set(key, [...index.get(key) ?? [], c]);
      }
    }
    for (const file of this.parsed.keys()) if (!seen.has(file)) this.parsed.delete(file);
    for (const file of this.checksums.keys()) if (!seen.has(file.slice(0, -'.mov'.length) + '.json')) this.checksums.delete(file);
    const used = [...this.projects.values()].flatMap((p) => Object.keys(p.assets).filter((a) => eligible(p.assets[a])).map((a) => this.hashOf(p, a)));
    await this.verify([...used, ...extra], index);
    // Published only once verified: job statuses emit from `this.index` outside the queue.
    this.index = index;
    return index;
  }

  /**
   * Checks every playable (current recipe version, well-formed) candidate of
   * these sources against its record's BLAKE3, as pm-proxy `lookup` does. A
   * mismatch is a named failure that Rebuild clears; media that went is a miss.
   * Works on one scan's `index` so a draft lookup outside the queue never
   * writes into a newer scan.
   */
  private async verify(hashes: Iterable<string | null | undefined>, index = this.index): Promise<void> {
    for (const hash of new Set(hashes)) {
      const all = hash ? index.get(hash) : undefined;
      if (!hash || !all) continue;
      const kept: Candidate[] = [];
      for (const c of all) {
        if (!c.checked && !c.problem && c.record.recipe.version === RECIPE_VERSION) {
          const problem = await this.checksum(c);
          if (problem === undefined) continue;
          c.problem = problem; c.checked = true;
        }
        kept.push(c);
      }
      if (kept.length) index.set(hash, kept); else index.delete(hash);
    }
  }

  /** dev, inode, size, mtime and ctime (ns): any rewrite or replacement of the file changes it. */
  private async identity(file: string): Promise<string | null> {
    const s = await stat(file, { bigint: true }).catch(() => null);
    return s?.isFile() ? `${s.dev}:${s.ino}:${s.size}:${s.mtimeNs}:${s.ctimeNs}` : null;
  }

  /** null when the bytes match, a named problem when not, undefined when the media went. Hashes a file once per identity. */
  private async checksum(c: Candidate): Promise<string | null | undefined> {
    const expected = c.record.blake3;
    for (let attempt = 0; attempt < 3; attempt++) {
      const before = await this.identity(c.file);
      if (!before) return undefined;
      const known = this.checksums.get(c.file);
      if (known && known.identity === before && known.expected === expected) return known.problem;
      let digest: string;
      try { digest = String(await this.options.hashFile(c.file)).toLowerCase(); } catch (e) {
        if ((e as NodeJS.ErrnoException).code === 'ENOENT') return undefined;
        this.options.log?.('warn', `[proxies] checking ${c.file}: ${message(e)}`);
        return `Powermove couldn't check the proxy file (${message(e)}). Rebuild the proxy.`;
      }
      // Rewritten while it was read: the digest describes neither version.
      if (await this.identity(c.file) !== before) continue;
      const problem = digest === expected ? null : 'The proxy file is damaged (its contents do not match its checksum). Rebuild the proxy.';
      if (problem) this.options.log?.('warn', `[proxies] ${c.file} does not match its checksum (BLAKE3 ${digest}, record ${expected})`);
      this.checksums.set(c.file, { identity: before, expected, problem });
      return problem;
    }
    return 'The proxy file kept changing while Powermove checked it. Rebuild the proxy.';
  }

  /** What the engine would play for this source and recipe (pm-proxy `find_for_source`). */
  private stored(hash: string | null | undefined, recipe?: Recipe, index = this.index): { ready: Candidate | null; damaged: Candidate | null } {
    const all = (hash && index.get(hash)) || [];
    // Well-formed records of another recipe version are stale, not damaged: the engine skips them too.
    const current = all.filter((c) => c.record.recipe?.version === RECIPE_VERSION || c.problem && !Number.isSafeInteger(c.record.recipe?.version));
    const matching = recipe ? current.filter((c) => c.record.recipe?.requested_format === recipe.format && c.record.recipe?.requested_size === recipe.size) : current;
    return { ready: matching.find((c) => c.checked && !c.problem) ?? null, damaged: matching.find((c) => c.problem) ?? null };
  }

  /* ── projects ── */

  private recipe(intent: ProjectProxyIntent): Recipe { return { format: intent.format, size: intent.size }; }
  private hashOf(p: Project, asset: string): string | null {
    const meta = p.assets[asset];
    const hash = meta?.originalSequenceHash ?? meta?.contentHash;
    return hash && HASH.test(hash) ? hash : null;
  }
  private entries(project: string): Project[] { return [...this.projects.values()].filter((p) => p.id === project); }
  private project(owner: number, id: string): Project {
    const project = this.projects.get(`${owner}:${id}`);
    if (!project) throw new Error('Open the project before managing its proxies.');
    return project;
  }
  private activeJob(project: string, asset: string): Job | undefined {
    for (const job of this.jobs.values()) if (!job.discarded && job.targets.get(project)?.has(asset)) return job;
    return undefined;
  }
  private suppressed(p: Project, asset: string, hash: string | null): boolean {
    const d = this.deletions.get(p.id)?.get(asset);
    return !!d && d.format === p.intent.format && d.size === p.intent.size && (!d.hash || !hash || d.hash === hash);
  }
  private failed(p: Project, asset: string, hash: string | null): string | null {
    const f = this.failures.get(target(p.id, asset));
    return f && f.recipe.format === p.intent.format && f.recipe.size === p.intent.size && (!f.hash || !hash || f.hash === hash) ? f.error : null;
  }
  private originalOnly(p: Project, asset: string): boolean {
    return p.intent.alwaysPlayOriginal.includes(asset) || !!p.assets[asset]?.alwaysPlayOriginal;
  }
  private rootFor(p: Project): string {
    if (this.setting.kind === 'folder') return this.setting.path;
    if (this.setting.kind === 'project') {
      if (!p.projectPath) throw new Error('Save this project before creating media next to it, or choose another generated media location.');
      return path.join(path.dirname(p.projectPath), PROJECT_ROOT);
    }
    return this.defaultRoot;
  }

  private stateOf(p: Project, asset: string): AssetProxyState {
    const meta = p.assets[asset];
    if (!eligible(meta)) return { state: 'unsupported' };
    const job = this.activeJob(p.id, asset);
    if (job) return job.state === 'queued' ? { state: 'queued' } : { state: 'creating', progress: job.progress };
    const hash = this.hashOf(p, asset);
    const error = this.failed(p, asset, hash);
    if (error) return { state: 'failed', error };
    const { ready, damaged } = this.stored(hash, this.recipe(p.intent));
    if (ready) {
      const r = ready.record.recipe;
      return { state: 'ready', width: r.width, height: r.height, codec: r.source_has_alpha || r.source_depth > 10 ? 'ProRes 4444' : CODEC[r.requested_format], bytes: ready.record.byte_size };
    }
    if (damaged?.problem) return { state: 'failed', error: damaged.problem };
    return { state: 'none' };
  }

  private value(p: Project): ProxySnapshot {
    const assets: Record<string, AssetProxyState> = {};
    for (const asset of Object.keys(p.assets)) assets[asset] = this.stateOf(p, asset);
    const active = [...this.jobs.values()].filter((j) => !j.discarded && j.targets.has(p.id));
    const batch = this.batches.get(p.id);
    let paused: ProxyActivity['paused'] = null;
    if (active.some((j) => j.paused === 'lowDisk')) paused = 'lowDisk';
    else if (active.some((j) => j.paused === 'interactive')) paused = this.playing || !this.exporting.size ? 'playback' : 'export';
    return { project: p.id, assets, activity: {
      creating: active.length ? { done: batch?.done ?? 0, total: Math.max(batch?.total ?? 0, (batch?.done ?? 0) + active.length) } : null,
      paused
    } };
  }

  private emit(projects: Iterable<string>): void {
    if (this.stopped) return;
    for (const id of new Set(projects)) {
      if (![...this.jobs.values()].some((j) => !j.discarded && j.targets.has(id))) this.batches.delete(id);
      for (const p of this.entries(id)) {
        try { this.options.changed(p.owner, this.value(p)); } catch (e) { this.options.log?.('warn', `[proxies] could not send proxy state: ${message(e)}`); }
      }
    }
  }

  private countTarget(project: string, delta: 1 | -1): void {
    const batch = this.batches.get(project) ?? { done: 0, total: 0 };
    batch.total = Math.max(batch.done, batch.total + delta);
    this.batches.set(project, batch);
  }

  /**
   * Queues a proxy for one asset. Explicit (the user asked) throws a message
   * the asset menu can show; automatic (the policy) skips quietly.
   */
  private async submit(p: Project, asset: string, explicit: boolean): Promise<void> {
    const meta = p.assets[asset];
    const name = meta?.name || asset;
    const skip = (error: string): void => { if (explicit) throw new Error(error); };
    if (!meta) return skip('This media is not in the project.');
    if (!eligible(meta)) return skip(`${name} is not video or an image sequence, so it plays without a proxy.`);
    if (this.activeJob(p.id, asset)) return;
    const recipe = this.recipe(p.intent);
    let hash = this.hashOf(p, asset);
    if (!explicit && (this.suppressed(p, asset, hash) || this.failed(p, asset, hash) || this.originalOnly(p, asset))) return;
    if (this.stored(hash, recipe).ready) return;
    const scope: ProxySourceScope = { owner: p.owner, project: p.id,
      ...(meta.originalSequenceHash ? { sequenceHash: meta.originalSequenceHash } : {}) };
    const source = await this.options.resolve(asset, scope).catch((e) => {
      this.options.log?.('warn', `[proxies] resolving ${asset}: ${message(e)}`);
      return null;
    });
    if (!source) return skip(`The original for ${name} is offline. Locate it before creating a proxy.`);
    if (!source.contentHash) return skip(`Powermove doesn't know the content of ${name}'s original yet. Try again once it has been imported.`);
    const resolved = hex(source.contentHash);
    if (hash && resolved !== hash) return skip(`The original for ${name} changed. Reopen the project before creating its proxy.`);
    hash = resolved;
    if (!explicit && (this.suppressed(p, asset, hash) || this.failed(p, asset, hash))) return;
    await this.verify([hash]);
    if (this.stored(hash, recipe).ready) return;
    let root: string;
    try { root = this.rootFor(p); } catch (e) { return skip(message(e)); }
    try { await this.ensureRoot(root); } catch (e) { return skip(`Powermove can't write generated media to ${root}: ${message(e)}.`); }
    this.failures.delete(target(p.id, asset));
    // Projects that share an original share its job when it writes to the same store.
    for (const job of this.jobs.values()) {
      if (!job.discarded && job.root === root && recipeKey(job.hash, job.recipe) === recipeKey(hash, recipe)) {
        const assets = job.targets.get(p.id) ?? new Set<string>();
        assets.add(asset); job.targets.set(p.id, assets);
        this.countTarget(p.id, 1);
        return;
      }
    }
    const job: Job = {
      id: `proxy-${randomUUID()}`, alias: `proxy-${randomUUID()}`, hash, recipe, root,
      source: { path: source.path, contentHash: Uint8Array.from(source.contentHash) },
      scope: { ...scope, asset },
      targets: new Map([[p.id, new Set([asset])]]), state: 'queued', progress: 0, paused: null,
      submitted: false, order: this.nextGenerationOrder++, discarded: false
    };
    this.jobs.set(job.id, job); this.aliases.set(job.alias, job.id);
    this.pendingGeneration.push(job.id);
    this.countTarget(p.id, 1);
    this.emit([p.id]);
  }

  /** A single native proxy lane: the waiting jobs remain reorderable here. */
  private async startNext(): Promise<{ job: string; error: string } | null> {
    if (this.activeGeneration || this.stopped) return null;
    const ranked = this.pendingGeneration
      .map((id) => this.jobs.get(id))
      .filter((job): job is Job => !!job && !job.discarded && !job.submitted)
      .sort((a, b) => this.rank(a) - this.rank(b) || a.order - b.order);
    // Select the least recently served project, then its nearest clip. This
    // prevents one project's long batch from starving another open project.
    const projects = [...new Set(ranked.flatMap((job) => [...job.targets.keys()]))];
    projects.sort((a, b) => (this.lastServed.get(a) ?? -1) - (this.lastServed.get(b) ?? -1));
    const chosen = projects[0];
    const job = chosen ? ranked.find((candidate) => candidate.targets.has(chosen)) : undefined;
    if (!job) return null;
    for (const project of job.targets.keys()) this.lastServed.set(project, ++this.generationTurn);
    this.pendingGeneration = this.pendingGeneration.filter((id) => id !== job.id);
    this.activeGeneration = job.id;
    job.submitted = true;
    try {
      await this.options.request({ type: 'submitJob', job: job.id, class: 'generation', run: null,
        spec: { kind: 'generateProxy', asset: job.alias, format: job.recipe.format, size: job.recipe.size }, output: null } as Request);
      return null;
    } catch (e) {
      this.forget(job);
      const error = `Couldn't start creating the proxy: ${message(e)}`;
      for (const [project, assets] of job.targets) for (const asset of assets) {
        this.failures.set(target(project, asset), { hash: job.hash, recipe: job.recipe, error });
      }
      this.emit(job.targets.keys());
      // A failed submit must not strand the rest of the queue.
      void this.serial(() => this.startNext()).catch((nextError) => this.options.log?.('warn', `[proxies] starting the next job: ${message(nextError)}`));
      return { job: job.id, error };
    }
  }

  private rank(job: Job): number {
    let nearest = Number.POSITIVE_INFINITY;
    for (const [project, assets] of job.targets) {
      const priority = this.priority.get(project)?.ranks;
      for (const asset of assets) nearest = Math.min(nearest, priority?.get(asset) ?? Number.POSITIVE_INFINITY);
    }
    return nearest;
  }

  /** The renderer sends asset ids in nearest-to-playhead order. Running work is not interrupted. */
  async prioritize(owner: number, id: string, assets: string[]): Promise<void> {
    return this.serial(async () => {
      // A new project may send its order before its first setIntent; the next
      // setIntent will use it when choosing the first job.
      if (!isId(id) || assets.length > 100_000 || !assets.every(isId)) throw new Error('Invalid proxy priority.');
      const latest = new Map<string, number>();
      for (const asset of assets) if (!latest.has(asset)) latest.set(asset, latest.size);
      this.priority.set(id, { owner, ranks: latest });
      await this.startNext();
    });
  }

  private forget(job: Job): void {
    this.jobs.delete(job.id); this.aliases.delete(job.alias);
    this.pendingGeneration = this.pendingGeneration.filter((id) => id !== job.id);
    if (this.activeGeneration === job.id) this.activeGeneration = null;
    if (this.current?.job === job.id) this.current = null;
  }

  /** Assets of other open projects that would play this proxy keep it. */
  private inUseElsewhere(project: string, hash: string, recipe: Recipe): boolean {
    for (const p of this.projects.values()) {
      if (p.id === project || p.intent.format !== recipe.format || p.intent.size !== recipe.size) continue;
      for (const asset of Object.keys(p.assets)) {
        if (eligible(p.assets[asset]) && this.hashOf(p, asset) === hash && !this.suppressed(p, asset, hash)) return true;
      }
    }
    return false;
  }

  private async deleteCandidate(c: Candidate): Promise<void> {
    // Record first: a record without media is a miss, never a broken proxy.
    for (const file of [c.recordFile, c.file]) {
      await unlink(file).catch((e) => { if ((e as NodeJS.ErrnoException).code !== 'ENOENT') throw e; });
    }
  }

  /** Cancels this asset's job and deletes its files. Delete keeps a file another open project plays; Rebuild (`force`) replaces it for everyone. */
  private async discard(p: Project, asset: string, force = false): Promise<void> {
    const hash = this.hashOf(p, asset);
    const job = this.activeJob(p.id, asset);
    if (job) {
      const assets = job.targets.get(p.id)!;
      assets.delete(asset);
      if (!assets.size) job.targets.delete(p.id);
      this.countTarget(p.id, -1);
      if (!job.targets.size) {
        job.discarded = true;
        if (job.submitted) {
          await this.options.request({ type: 'cancelJobs', kind: 'job', job: job.id } as Request)
            .catch((e) => this.options.log?.('warn', `[proxies] cancelling ${job.id}: ${message(e)}`));
        } else this.forget(job);
      }
    }
    this.failures.delete(target(p.id, asset));
    const sourceHash = hash ?? job?.hash ?? null;
    if (!sourceHash) return;
    for (const c of this.index.get(sourceHash) ?? []) {
      const recipe = c.problem && !c.record.recipe ? null : { format: c.record.recipe.requested_format, size: c.record.recipe.requested_size };
      const playable = c.record.recipe?.version === RECIPE_VERSION;
      if (!force && playable && recipe && this.inUseElsewhere(p.id, sourceHash, recipe)) continue;
      await this.deleteCandidate(c);
    }
  }

  /* ── renderer calls (IPC) ── */

  async setIntent(owner: number, id: string, intent: ProjectProxyIntent, assets: Record<string, ProxyAssetDescriptor>, projectPath?: string): Promise<void> {
    if (!isId(id) || !validProxyIntent(intent)) throw new Error('Invalid project proxy settings.');
    return this.serial(async () => {
      const key = `${owner}:${id}`, old = this.projects.get(key);
      const p: Project = { key, owner, id, intent: structuredClone(intent), assets: structuredClone(assets),
        ...(absolute(projectPath) ? { projectPath } : {}), slow: old?.slow ?? new Set() };
      this.projects.set(key, p);
      // A recipe change ends "deleted" for the old recipe (appendix D).
      const deleted = this.deletions.get(id);
      if (deleted) {
        let changed = false;
        for (const [asset, d] of deleted) if (d.format !== intent.format || d.size !== intent.size) { deleted.delete(asset); changed = true; }
        if (!deleted.size) this.deletions.delete(id);
        if (changed) await this.persistDeletions();
      }
      await this.rescan();
      if (intent.create === 'whenNeeded') {
        for (const asset of Object.keys(p.assets)) {
          const hash = this.hashOf(p, asset);
          if (eligible(p.assets[asset]) && hash && this.assessments.get(hash)?.needsProxy) p.slow.add(asset);
        }
      }
      // An image sequence is not a single decoder input for the cold ProbeMedia
      // measurement. Its multi-file original needs a proxy under When needed.
      const wanted = intent.create === 'allVideo' ? Object.keys(p.assets) : intent.create === 'whenNeeded'
        ? [...new Set([...p.slow, ...Object.keys(p.assets).filter((asset) => p.assets[asset]?.imageSequence)])] : [];
      for (const asset of wanted) await this.submit(p, asset, false);
      await this.startNext();
      this.emit([id]);
      // Measuring decodes takes seconds per clip: it never holds up the intent.
      if (intent.create === 'whenNeeded') this.scheduleAssessments(p);
    });
  }

  /** Why measuring an original failed, if it did (named, never a modal; the asset stays on its original). */
  assessmentError(hash: string): string | null { return this.assessFailures.get(hash) ?? null; }

  /** Resolves once queued measurements and the operations they started have run (tests, shutdown). */
  async settled(): Promise<void> {
    for (let previous: unknown; previous !== this.assessQueue;) { previous = this.assessQueue; await this.assessQueue; }
    await this.queue;
  }

  private needsAssessment(p: Project, asset: string, hash: string | null): boolean {
    if (!this.options.assess || p.intent.create !== 'whenNeeded' || !eligible(p.assets[asset]) || p.assets[asset]?.imageSequence || p.slow.has(asset)) return false;
    if (hash && (this.assessments.has(hash) || this.assessFailures.has(hash) || this.assessing.has(hash))) return false;
    if (this.originalOnly(p, asset) || this.suppressed(p, asset, hash) || this.failed(p, asset, hash) || this.activeJob(p.id, asset)) return false;
    return !this.stored(hash, this.recipe(p.intent)).ready;
  }

  private scheduleAssessments(p: Project): void {
    for (const asset of Object.keys(p.assets)) {
      const hash = this.hashOf(p, asset);
      if (!this.needsAssessment(p, asset, hash)) continue;
      if (hash) this.assessing.add(hash);
      // One at a time: a cold sequential decode is the measurement.
      this.assessQueue = this.assessQueue.then(() => this.assessOne(p.key, asset, hash)).catch((e) => {
        this.options.log?.('warn', `[proxies] measuring ${asset}: ${message(e)}`);
      });
    }
  }

  private async assessOne(key: string, asset: string, expected: string | null): Promise<void> {
    try {
      const p = this.projects.get(key);
      if (this.stopped || !p || p.intent.create !== 'whenNeeded') return;
      if (expected && this.hashOf(p, asset) !== expected) return;
      const scope: ProxySourceScope = { owner: p.owner, project: p.id,
        ...(p.assets[asset]?.originalSequenceHash ? { sequenceHash: p.assets[asset].originalSequenceHash } : {}) };
      const source = await this.options.resolve(asset, scope).catch(() => null);
      if (!source?.contentHash) return; // offline or unknown: measured when it comes back
      const hash = hex(source.contentHash);
      if (expected && hash !== expected) return;
      let result = this.assessments.get(hash)?.needsProxy;
      if (result === undefined) {
        if (this.assessFailures.has(hash) || this.stopped) return;
        try {
          result = await this.options.assess!(asset, source, scope);
          if (typeof result !== 'boolean') throw new Error('the decode measurement gave no answer');
        } catch (e) {
          const error = `Couldn't measure how ${p.assets[asset]?.name || asset} decodes on this Mac: ${message(e)}`;
          this.assessFailures.set(hash, error);
          this.options.log?.('warn', `[proxies] ${error}`);
          return;
        }
        this.assessments.set(hash, { needsProxy: result, at: this.options.now?.() ?? Date.now() });
        await this.persistAssessments().catch((e) => this.options.log?.('warn', `[proxies] saving decode measurements: ${message(e)}`));
      }
      if (!result || this.stopped) return;
      // Every open "When needed" project playing this original gets the proxy.
      void this.serial(async () => {
        await this.rescan();
        const touched: string[] = [];
        for (const q of this.projects.values()) {
          if (q.intent.create !== 'whenNeeded') continue;
          for (const a of Object.keys(q.assets)) {
            if ((a === asset && q.key === key) || this.hashOf(q, a) === hash) {
              if (!eligible(q.assets[a])) continue;
              q.slow.add(a);
              await this.submit(q, a, false);
              touched.push(q.id);
            }
          }
        }
        await this.startNext();
        this.emit(touched);
      }).catch((e) => this.options.log?.('warn', `[proxies] queuing a measured proxy: ${message(e)}`));
    } finally {
      if (expected) this.assessing.delete(expected);
    }
  }

  async snapshot(owner: number, id: string): Promise<ProxySnapshot> {
    return this.serial(async () => {
      const p = this.projects.get(`${owner}:${id}`);
      if (!p) return { project: id, assets: {}, activity: { creating: null, paused: null } };
      await this.rescan();
      return this.value(p);
    });
  }

  /** Create Proxy: a no-op where one exists or is being made. */
  async create(owner: number, id: string, assets: string[]): Promise<void> {
    return this.serial(async () => {
      const p = this.project(owner, id);
      await this.rescan();
      const errors: string[] = [];
      for (const asset of new Set(assets)) {
        await this.clearDeletion(p.id, asset);
        await this.submit(p, asset, true).catch((e) => { errors.push(message(e)); });
      }
      const started = await this.startNext();
      if (started && [...new Set(assets)].some((asset) => this.failed(p, asset, this.hashOf(p, asset)) === started.error)) errors.push(started.error);
      this.emit([p.id]);
      if (errors.length) throw new Error(errors.length > 1 ? `${errors[0]} (${errors.length - 1} more)` : errors[0]);
    });
  }

  /** Delete Proxy: generated files only; the policy does not bring it back until Create, Rebuild, or a recipe change. */
  async remove(owner: number, id: string, assets: string[]): Promise<void> {
    return this.serial(async () => {
      const p = this.project(owner, id);
      await this.rescan();
      for (const asset of new Set(assets)) {
        if (!eligible(p.assets[asset])) continue;
        await this.discard(p, asset);
        const map = this.deletions.get(p.id) ?? new Map<string, Deletion>();
        map.set(asset, { hash: this.hashOf(p, asset), format: p.intent.format, size: p.intent.size, at: this.options.now?.() ?? Date.now() });
        this.deletions.set(p.id, map);
      }
      await this.persistDeletions();
      await this.rescan();
      this.emit([p.id]);
    });
  }

  /** Rebuild Proxy: discard (cancelling a job in flight) and regenerate, in one step. */
  async rebuild(owner: number, id: string, assets: string[]): Promise<void> {
    return this.serial(async () => {
      const p = this.project(owner, id);
      await this.rescan();
      const errors: string[] = [];
      for (const asset of new Set(assets)) {
        try {
          await this.discard(p, asset, true);
          await this.clearDeletion(p.id, asset);
          await this.rescan();
          await this.submit(p, asset, true);
        } catch (e) { errors.push(message(e)); }
      }
      const started = await this.startNext();
      if (started && [...new Set(assets)].some((asset) => this.failed(p, asset, this.hashOf(p, asset)) === started.error)) errors.push(started.error);
      this.emit([p.id]);
      if (errors.length) throw new Error(errors.length > 1 ? `${errors[0]} (${errors.length - 1} more)` : errors[0]);
    });
  }

  /** Decode measurements (`slowDecodes`, `Event::ProxyNeeded`): "When needed" creates proxies for these. */
  async slow(owner: number, id: string, assets: string[]): Promise<void> {
    return this.serial(async () => {
      const p = this.project(owner, id);
      for (const asset of assets) if (eligible(p.assets[asset])) p.slow.add(asset);
      if (p.intent.create !== 'whenNeeded') return;
      await this.rescan();
      for (const asset of new Set(assets)) await this.submit(p, asset, false);
      await this.startNext();
      this.emit([p.id]);
    });
  }
  needed(owner: number, id: string, assets: string[]): Promise<void> { return this.slow(owner, id, assets); }

  private async clearDeletion(project: string, asset: string): Promise<void> {
    const map = this.deletions.get(project);
    if (!map?.delete(asset)) return;
    if (!map.size) this.deletions.delete(project);
    await this.persistDeletions();
  }

  async location(): Promise<GeneratedMediaLocation> {
    return this.serial(async () => structuredClone(this.setting));
  }

  /** Changes where new proxies go. Existing ones stay findable (their root stays listed); queued jobs keep their store. */
  async setLocation(location: GeneratedMediaLocation): Promise<GeneratedMediaLocation> {
    if (!validGeneratedMediaLocation(location)) throw new Error('Choose a generated media location.');
    return this.serial(async () => {
      if (location.kind === 'folder') {
        try {
          await mkdir(location.path, { recursive: true, mode: 0o700 });
          if (!(await stat(location.path)).isDirectory()) throw new Error('it is not a folder');
        } catch (e) { throw new Error(`Powermove can't use ${location.path} for generated media: ${message(e)}.`); }
        if (!this.roots.includes(location.path)) this.roots.push(location.path);
      }
      this.setting = structuredClone(location);
      await this.persistLocations();
      return structuredClone(this.setting);
    });
  }

  /* ── engine callbacks ── */

  /**
   * The generation lane's `mediaRead` for a job alias (job process only).
   * The original is re-resolved through the project that queued the job so a
   * relink is honoured, and must still be the content frozen at Create; with
   * no window bound to that project (closed) the frozen source is used.
   */
  async resolveAlias(asset: string, context?: ProxyHostContext): Promise<ProxySource | null> {
    if (context?.process !== 'job') return null;
    const id = this.aliases.get(asset);
    const job = id ? this.jobs.get(id) : undefined;
    if (!job) return null;
    if (job.discarded) throw new ProtocolError('cancelled', 'This proxy was deleted before it was created.');
    const fresh = await this.options.resolve(job.scope.asset, job.scope).catch(() => null);
    if (fresh?.contentHash && hex(fresh.contentHash) !== job.hash) {
      throw new ProtocolError('mediaOffline', 'The original changed after the proxy was queued. Create the proxy again.');
    }
    const source = fresh?.contentHash ? fresh : job.source;
    this.current = { job: job.id, root: job.root };
    return { path: source.path, contentHash: Uint8Array.from(job.source.contentHash!) };
  }

  /** The `generatedMedia` store: the running generation job's frozen root, else the default cache. */
  generatedDirectory(context?: ProxyHostContext): string {
    if (context && context.process !== 'job') return this.defaultRoot;
    return this.current?.root ?? this.defaultRoot;
  }

  /**
   * Draft renders ("Render using: Proxies (draft)") for one project's binding
   * of an asset. Without the frozen hash it uses the one every open window of
   * that project agrees on; never another project's.
   */
  async resolveDraft(asset: string, scope: { project: string; contentHash?: string | null }): Promise<ProxySource | null> {
    if (this.stopped || !scope || !isId(scope.project)) return null;
    if (!await this.ready.then(() => true, () => false)) return null;
    const entries = this.entries(scope.project);
    let hash = typeof scope.contentHash === 'string' && HASH.test(scope.contentHash) ? scope.contentHash : null;
    if (hash) {
      // A sequence's stored playback movie and its ordered original frames
      // have different identities. Draft jobs freeze the movie binding; the
      // generated proxy is keyed by the original sequence's aggregate hash.
      // A frozen job may name a binding that no open window still describes;
      // in that case its exact hash remains authoritative.
      const bound = entries.find((p) => p.assets[asset]?.contentHash === hash || p.assets[asset]?.originalSequenceHash === hash);
      if (bound?.assets[asset]?.originalSequenceHash) hash = bound.assets[asset].originalSequenceHash;
    } else {
      const hashes = new Set(entries.map((p) => this.hashOf(p, asset)));
      if (hashes.size !== 1) return null;
      hash = [...hashes][0] ?? null;
    }
    if (!hash) return null;
    const index = await this.rescan([hash]);
    const intent = entries[0]?.intent;
    const found = (intent && this.stored(hash, this.recipe(intent), index).ready) || this.stored(hash, undefined, index).ready;
    return found ? { path: found.file, contentHash: Uint8Array.from(Buffer.from(found.record.blake3, 'hex')) } : null;
  }

  /** Job statuses from the job engine (all classes). */
  status(status: JobStatus): void {
    const job = this.jobs.get(status.job);
    if (!job) {
      if (status.class !== 'generation') {
        if (status.state === 'running' || status.state === 'paused') this.exporting.add(status.job); else this.exporting.delete(status.job);
      }
      return;
    }
    if (status.state === 'done' || status.state === 'failed' || status.state === 'cancelled') {
      this.forget(job);
      void this.serial(async () => {
        try { await this.finish(job, status); }
        finally { await this.startNext(); }
      }).catch((e) => this.options.log?.('warn', `[proxies] finishing ${job.id}: ${message(e)}`));
      return;
    }
    if (job.discarded) return;
    if (status.state === 'paused') {
      job.state = job.progress > 0 || status.progress > 0 ? 'creating' : job.state;
      job.paused = /low disk/i.test(status.error?.message ?? '') ? 'lowDisk' : 'interactive';
    } else if (status.state === 'running') {
      job.state = 'creating'; job.paused = null;
    } else {
      job.state = 'queued'; job.paused = null;
    }
    job.progress = Math.max(job.progress, Math.min(1, Math.max(0, status.progress || 0)));
    this.emit(job.targets.keys());
  }

  private async finish(job: Job, status: JobStatus): Promise<void> {
    if (job.discarded) {
      // A deleted proxy that finished anyway: remove it unless something else wants that file.
      if (status.state !== 'done' || !status.path) return;
      await this.rescan();
      const wanted = [...this.jobs.values()].some((j) => !j.discarded && recipeKey(j.hash, j.recipe) === recipeKey(job.hash, job.recipe))
        || this.inUseElsewhere(job.scope.project, job.hash, job.recipe);
      if (wanted) return;
      const output = await realpath(status.path).catch(() => null);
      if (!output) return;
      // Only a file of this store with this job's identity; never a path the status merely names.
      for (const c of this.index.get(job.hash) ?? []) {
        if (c.record.recipe?.version !== RECIPE_VERSION || c.record.recipe?.requested_format !== job.recipe.format || c.record.recipe?.requested_size !== job.recipe.size) continue;
        if (await realpath(c.file).catch(() => null) === output) await this.deleteCandidate(c);
      }
      await this.rescan();
      return;
    }
    for (const project of job.targets.keys()) {
      const batch = this.batches.get(project);
      if (batch) batch.done = Math.min(batch.total, batch.done + 1);
    }
    if (status.state === 'failed') {
      const error = status.error?.message || 'Proxy creation failed. Try rebuilding it.';
      for (const [project, assets] of job.targets) for (const a of assets) this.failures.set(target(project, a), { hash: job.hash, recipe: job.recipe, error });
    }
    await this.rescan();
    this.emit(job.targets.keys());
  }

  /** Transport state: tells a paused job's reason apart (playback or export). */
  playback(playing: boolean): void {
    if (this.playing === playing) return;
    this.playing = playing;
    const paused = [...this.jobs.values()].filter((j) => !j.discarded && j.paused === 'interactive');
    this.emit(paused.flatMap((j) => [...j.targets.keys()]));
  }

  /** A window closed: its project state goes. Jobs keep running (their output is shared) and later statuses are harmless. */
  release(owner: number): void {
    for (const [key, p] of this.projects) if (p.owner === owner) this.projects.delete(key);
    for (const [project, priority] of this.priority) if (priority.owner === owner || !this.entries(project).length) this.priority.delete(project);
    for (const project of [...this.lastServed.keys()]) if (!this.entries(project).length) this.lastServed.delete(project);
  }

  /** The job engine launch ended: jobs in flight never report again. */
  jobsLost(reason = 'The engine stopped while creating this proxy. Create it again.'): void {
    const affected = new Set<string>();
    for (const job of [...this.jobs.values()]) {
      this.forget(job);
      if (job.discarded) continue;
      for (const [project, assets] of job.targets) {
        affected.add(project);
        for (const a of assets) this.failures.set(target(project, a), { hash: job.hash, recipe: job.recipe, error: reason });
      }
    }
    this.current = null;
    this.activeGeneration = null;
    this.pendingGeneration = [];
    this.lastServed.clear();
    this.batches.clear();
    this.exporting.clear();
    this.emit(affected);
  }

  async dispose(): Promise<void> {
    if (this.stopped) return;
    this.stopped = true;
    const jobs = [...this.jobs.values()].filter((job) => job.submitted).map((job) => job.id);
    this.jobs.clear(); this.aliases.clear(); this.current = null;
    this.activeGeneration = null; this.pendingGeneration = []; this.lastServed.clear();
    await Promise.allSettled(jobs.map((job) => this.options.request({ type: 'cancelJobs', kind: 'job', job } as Request)));
    await this.writes;
  }
}
