/*
 * Publishing a folder to the Store (store plan §2.1, §2.2, §2.6; P0 Q2, Q3).
 *
 *   prepare(localId)   read and snapshot the folder, parse its manifest, run
 *                      the scanner, work out whether this is a first publish,
 *                      an update or a fork, then ask the registry what is
 *                      already there. Local checks all run before the one
 *                      network call.
 *   publish(localId)   re-read the folder, freeze its snapshot, show the
 *                      native confirmation naming coordinate, version and
 *                      that snapshot's tree hash, then upload exactly those
 *                      objects and put the release.
 *
 * A fork is created at publish: when the folder was installed from someone
 * else's release, the uploaded manifest (and only the uploaded manifest)
 * gains `forkedFrom: "<handle>/<slug>@<version>"`. The folder is never
 * rewritten for that, so the snapshot the person confirms is recomputed
 * after the rewrite and is the one uploaded.
 *
 * Nothing here logs file contents, findings' text or the session token.
 */
import * as fs from 'node:fs/promises';
import path from 'node:path';
import type { MessageBoxOptions } from 'electron';
import { encodeCommit, encodeLoose, hashObject, type GitObjectType } from '@powermove/registry/git';
import { UPLOAD_ENVELOPE } from '@powermove/registry/limits';
import { walkDir } from '@powermove/registry/node';
import { scanFiles, type ScanFinding } from '@powermove/registry/scan';
import { snapshot, SnapshotError, type Snapshot, type SnapshotInput } from '@powermove/registry/snapshot';
import {
  ApiError,
  Me,
  Objects,
  Publish,
  Store,
  MULTIPART_OBJECT_TYPE,
  type ApiErrorBody,
  type ExtensionDetailDto,
  type MeDto
} from '@powermove/registry/wire';
import type { z } from 'zod';

import { EXTENSION_ID, parseForkedFrom, parseManifest, type ExtensionManifest } from '../../shared/extensions';
import {
  HANDLE_MAIL_DOMAIN,
  compareVersions,
  findingKey,
  nextPatch,
  publishErrorText,
  versionProblem,
  waiverProblem,
  PUBLISH_LIMITS,
  type PublishErrorContext,
  type PublishFinding,
  type PublishForm,
  type PublishListing,
  type PublishPlanDto,
  type PublishProgress,
  type PublishWaiver,
  type StorePublishResult
} from '../../shared/publish';
import type { CloudClient } from './client';
import { StoreLocalError, type StoreInstallerRegistry } from './install';
import type { ProvenanceOrigin, ProvenancePublished, ProvenanceRecord, ProvenanceStore } from './provenance';
import { categoryFor } from './store-ipc';

/* ── the registry calls publishing needs ─────────────────── */

export type MissingBody = z.infer<typeof Objects.Missing.Req.shape.body>;
export type UploadResult = z.infer<typeof Objects.Upload.Res>;
export type PutReleaseBody = z.infer<typeof Publish.PutRelease.Req.shape.body>;
export type PutReleaseResult = z.infer<typeof Publish.PutRelease.Res>;
export type YankResult = z.infer<typeof Publish.Yank.Res>;
export type MyReposResult = z.infer<typeof Me.Repos.Res>;

export interface UploadPart {
  sha: string;
  /** `encodeLoose` bytes: deflated `<type> <size>\0<body>`. */
  bytes: Uint8Array;
}

export interface PublishApi {
  detail(handle: string, slug: string): Promise<ExtensionDetailDto>;
  missing(body: MissingBody): Promise<{ missing: string[] }>;
  upload(parts: UploadPart[]): Promise<UploadResult>;
  putRelease(handle: string, slug: string, body: PutReleaseBody): Promise<PutReleaseResult>;
  yank(handle: string, slug: string, version: string): Promise<YankResult>;
  myRepos(): Promise<MyReposResult>;
}

export function createPublishApi(client: () => CloudClient): PublishApi {
  return {
    detail: (handle, slug) =>
      client().request(Store.Detail.Res, (api) => api.v1.store.x[':handle'][':slug'].$get({ param: { handle, slug } })),
    missing: (body) => {
      const args = { json: body };
      return client().request(Objects.Missing.Res, (api) => api.v1.objects.missing.$post(args));
    },
    upload: (parts) => {
      const form = new FormData();
      for (const part of parts) form.append(part.sha, new Blob([new Uint8Array(part.bytes)], { type: MULTIPART_OBJECT_TYPE }), part.sha);
      /* The route parses the multipart stream itself, so the typed client
         carries no body type for it; the form goes in as the request body. */
      return client().request(Objects.Upload.Res, (api) => api.v1.objects.$post({}, { init: { body: form } }));
    },
    putRelease: (handle, slug, body) => {
      const args = { param: { handle, slug }, json: body };
      return client().request(Publish.PutRelease.Res, (api) => api.v1.repos[':handle'][':slug'].releases.$put(args));
    },
    yank: (handle, slug, version) =>
      client().request(Publish.Yank.Res, (api) =>
        api.v1.repos[':handle'][':slug'].releases[':version'].yank.$post({ param: { handle, slug, version } })),
    myRepos: () => client().request(Me.Repos.Res, (api) => api.v1.me.repos.$get())
  };
}

/* ── pure helpers ────────────────────────────────────────── */

export interface UploadBatchLimits {
  parts: number;
  bytes: number;
}

/**
 * Multipart framing per part (boundary, two headers, CRLFs), counted against
 * the compressed envelope so a full batch never trips the server's
 * `Content-Length` check.
 */
const PART_OVERHEAD = 256;

export const UPLOAD_BATCH: UploadBatchLimits = {
  parts: UPLOAD_ENVELOPE.partsPerRequest,
  bytes: UPLOAD_ENVELOPE.compressedBytesPerRequest - 4096
};

/** Split parts into requests of at most `limits.parts` parts and `limits.bytes` bytes, in order. */
export function batchParts(parts: UploadPart[], limits: UploadBatchLimits = UPLOAD_BATCH): UploadPart[][] {
  const batches: UploadPart[][] = [];
  let current: UploadPart[] = [];
  let size = 0;
  for (const part of parts) {
    const cost = part.bytes.length + PART_OVERHEAD;
    if (current.length && (current.length >= limits.parts || size + cost > limits.bytes)) {
      batches.push(current);
      current = [];
      size = 0;
    }
    current.push(part);
    size += cost;
  }
  if (current.length) batches.push(current);
  return batches;
}

/** Bytes as a person reads them: "820 bytes", "14 KB", "1.2 MB". */
export function formatSize(bytes: number): string {
  if (bytes < 1000) return `${bytes} ${bytes === 1 ? 'byte' : 'bytes'}`;
  if (bytes < 1_000_000) return `${Math.round(bytes / 1000)} KB`;
  return `${(bytes / 1_000_000).toFixed(1)} MB`;
}

/** The manifest as uploaded or written: two-space JSON and a trailing newline, so diffs stay stable. */
export function manifestText(raw: Record<string, unknown>): string {
  return `${JSON.stringify(raw, null, 2)}\n`;
}

const PNG_SIGNATURE = [137, 80, 78, 71, 13, 10, 26, 10];

/** Width and height from a PNG's IHDR, or null when it isn't one. */
export function pngSize(bytes: Uint8Array): { width: number; height: number } | null {
  if (bytes.length < 24 || !PNG_SIGNATURE.every((value, index) => bytes[index] === value)) return null;
  if (String.fromCharCode(bytes[12]!, bytes[13]!, bytes[14]!, bytes[15]!) !== 'IHDR') return null;
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  return { width: view.getUint32(16), height: view.getUint32(20) };
}

function snapshotText(error: SnapshotError): string {
  const file = error.path ? `“${error.path}”` : 'A file';
  switch (error.code) {
    case 'bad_path': return `${file} has a name that can’t be published. Rename it and try again.`;
    case 'path_too_long': return `The path ${file} is too long. Keep paths under 200 characters.`;
    case 'duplicate_path':
    case 'case_collision': return `${file} clashes with another file whose name differs only in capitals. Rename one of them.`;
    case 'file_dir_collision': return `${file} is both a file and a folder. Rename one of them.`;
    case 'too_many_files': return 'Extensions can have at most 400 files. Remove some and try again.';
    case 'file_too_large': return `${file} is larger than 2 MB. Remove it or make it smaller.`;
    case 'tree_too_large': return 'Extensions can be at most 8 MB. Remove large files and try again.';
    case 'empty_tree': return 'This extension’s folder is empty.';
    case 'symlink': return `${file} is a shortcut to another file. Replace it with the file itself.`;
    case 'not_regular': return `${file} isn’t a regular file. Remove it and try again.`;
  }
}

async function snapshotOrExplain(inputs: SnapshotInput[]): Promise<Snapshot> {
  try {
    return await snapshot(inputs);
  } catch (error) {
    if (error instanceof SnapshotError) throw new StoreLocalError('folder_invalid', snapshotText(error));
    throw error;
  }
}

const utf8 = new TextDecoder('utf-8', { fatal: true });
const encoder = new TextEncoder();

/** Hard findings block; soft ones need a reason each, one per file and line (the registry's waiver key). */
function findingsOf(inputs: SnapshotInput[]): { blocked: PublishFinding[]; waivable: PublishFinding[] } {
  const texts = inputs.flatMap((input) => {
    try {
      return [{ path: input.path, text: utf8.decode(input.bytes) }];
    } catch {
      return [];
    }
  });
  const result = scanFiles(texts);
  const all: ScanFinding[] = [...result.blocked, ...result.waived];
  const blocked: PublishFinding[] = [];
  const waivable = new Map<string, PublishFinding>();
  for (const finding of all) {
    if (finding.hard) {
      blocked.push({ path: finding.path, line: finding.line, kind: finding.kind });
      continue;
    }
    const key = findingKey(finding);
    if (waivable.has(key)) continue;
    waivable.set(key, finding.waived
      ? { path: finding.path, line: finding.line, kind: finding.kind, reason: finding.waived.slice(0, PUBLISH_LIMITS.waiverReasonMax) }
      : { path: finding.path, line: finding.line, kind: finding.kind });
  }
  const order = (a: PublishFinding, b: PublishFinding): number => a.path.localeCompare(b.path) || a.line - b.line;
  return { blocked: blocked.sort(order), waivable: [...waivable.values()].sort(order) };
}

/* ── the publisher ───────────────────────────────────────── */

export interface PublisherOptions {
  registry: Pick<StoreInstallerRegistry, 'userDir' | 'list' | 'refresh' | 'emitChanged'>;
  provenance: ProvenanceStore;
  api: PublishApi;
  me(): MeDto | null;
  signedIn(): boolean;
  /** The native, main-owned confirmation. Resolves true for Publish. */
  confirm(options: MessageBoxOptions): Promise<boolean>;
  /** main → renderers: `store:publish-progress`. */
  notifyProgress?(progress: PublishProgress): void;
  /** main → renderers: `store:library-changed`. */
  notifyLibrary?(): void;
  now?(): number;
  log?(message: string, error?: unknown): void;
}

export interface Publisher {
  prepare(localId: string): Promise<PublishPlanDto>;
  publish(localId: string, form: PublishForm): Promise<StorePublishResult>;
  /** Withdraw one version: by repo and version, or a folder's last published version. */
  yank(target: string | { repoId: string; version: string }): Promise<{ version: string }>;
}

interface Identity {
  handle: string;
  publisherId: string;
}

interface LocalFolder {
  localId: string;
  folder: string;
  inputs: SnapshotInput[];
  /** The folder's manifest.json as read. */
  manifestBytes: Uint8Array;
  raw: Record<string, unknown>;
  manifest: ExtensionManifest;
  treeSha: string;
}

/** Where this publish lands, from provenance and the account; no network. */
interface Lineage {
  record: ProvenanceRecord | null;
  origin?: ProvenanceOrigin;
  /** The folder's `published`, when it is this account's. */
  published?: ProvenancePublished;
  /** Installed from someone else's release: the uploaded manifest names it. */
  fork: boolean;
  /** An existing repo of mine, and the commit the next one follows. */
  existing?: { repoId: string; parent?: string; releaseId: string };
}

/** Everything `prepare` learned that `publish` must reuse, held in main per folder. */
interface Prepared {
  identity: Identity;
  coordinate: string;
  lineage: Lineage;
  firstPublish: boolean;
  lastVersion: string | null;
  listing: PublishListing;
  parent?: string;
  originReleaseId?: string;
  basedOnReleaseId?: string;
}

interface Frozen {
  snap: Snapshot;
  /** The folder's tree once the new version is written back; the fork's `forkedFrom` is not in it. */
  localTreeSha: string;
  /** manifest.json as it will be in the folder after publish; null when it stays as it is. */
  localManifest: Uint8Array | null;
  findings: { blocked: PublishFinding[]; waivable: PublishFinding[] };
  inputs: SnapshotInput[];
}

export function createPublisher(options: PublisherOptions): Publisher {
  const { registry, provenance, api } = options;
  const now = options.now ?? Date.now;
  const log = options.log ?? ((message: string, error?: unknown) => console.warn(`[publish] ${message}`, error instanceof Error ? error.message : ''));
  const prepared = new Map<string, Prepared>();
  /* One publish at a time: they share the provenance file and the
     confirmation, and a double click must not upload twice. */
  let queue: Promise<unknown> = Promise.resolve();
  const serial = <T>(task: () => Promise<T>): Promise<T> => {
    const run = queue.then(task);
    queue = run.catch(() => undefined);
    return run;
  };

  function identity(): Identity {
    const me = options.me();
    if (!options.signedIn() || !me) throw new ApiError({ error: 'unauthorized' });
    if (!me.publisher || me.publisher.tombstoned) throw new ApiError({ error: 'forbidden' });
    return { handle: me.publisher.handle, publisherId: me.publisher.id };
  }

  /** Registry failures, rewritten with the sentence the sheet shows. */
  function explain(error: unknown, context: PublishErrorContext): unknown {
    if (error instanceof ApiError) {
      const body: ApiErrorBody = { ...error.body, detail: publishErrorText(error.body, context) };
      return new ApiError(body);
    }
    return error;
  }

  async function readFolder(localId: string): Promise<LocalFolder> {
    if (!EXTENSION_ID.test(localId)) throw new StoreLocalError('not_installed', 'That extension isn’t on this Mac.');
    const record = registry.list().find((candidate) => candidate.id === localId);
    if (!record || record.scope !== 'user') throw new StoreLocalError('not_installed', 'Only extensions in your Library can be published.');
    const folder = path.join(registry.userDir, localId);
    let inputs: SnapshotInput[];
    try {
      inputs = await walkDir(folder);
    } catch (error) {
      if (error instanceof SnapshotError) throw new StoreLocalError('folder_invalid', snapshotText(error));
      throw new StoreLocalError('not_installed', 'Powermove couldn’t read this extension’s folder.');
    }
    const snap = await snapshotOrExplain(inputs);
    const manifestInput = inputs.find((input) => input.path === 'manifest.json');
    if (!manifestInput) throw new StoreLocalError('folder_invalid', 'This extension has no manifest.json.');
    let raw: unknown;
    try {
      raw = JSON.parse(utf8.decode(manifestInput.bytes));
    } catch {
      throw new StoreLocalError('folder_invalid', 'manifest.json isn’t valid JSON. Fix it and try again.');
    }
    const parsed = parseManifest(raw);
    if (!parsed.ok || !raw || typeof raw !== 'object' || Array.isArray(raw)) {
      throw new StoreLocalError('folder_invalid', `manifest.json isn’t valid: ${parsed.ok ? 'it must be an object' : parsed.error}.`);
    }
    if (parsed.manifest.id !== localId) {
      throw new StoreLocalError('folder_invalid', `manifest.json says its id is “${parsed.manifest.id}”, but its folder is “${localId}”. Make them match.`);
    }
    return { localId, folder, inputs, manifestBytes: manifestInput.bytes, raw: raw as Record<string, unknown>, manifest: parsed.manifest, treeSha: snap.treeSha };
  }

  async function lineageOf(localId: string, who: Identity): Promise<Lineage> {
    const record = await provenance.get(localId);
    const origin = record?.origin;
    const published = record?.published && record.published.ownerPublisherId === who.publisherId ? record.published : undefined;
    const fork = !!origin && origin.ownerPublisherId !== who.publisherId;
    const lineage: Lineage = { record, fork };
    if (origin) lineage.origin = origin;
    if (published) {
      lineage.published = published;
      lineage.existing = { repoId: published.repoId, releaseId: published.releaseId, ...(published.commitSha ? { parent: published.commitSha } : {}) };
    } else if (origin && !fork) {
      // My own extension, installed from the store: its release is the head I build on.
      lineage.existing = { repoId: origin.repoId, releaseId: origin.releaseId, parent: origin.commitSha };
    }
    return lineage;
  }

  const forkedFromOf = (origin: ProvenanceOrigin): string => `${origin.coordinate}@${origin.version}`;

  /**
   * The tree to upload for `version`: the folder with manifest.json rewritten
   * for the new version and, on a fork, `forkedFrom`. Recomputed here so the
   * hash shown is the hash uploaded.
   */
  async function freeze(local: LocalFolder, version: string, lineage: Lineage): Promise<Frozen> {
    const forkedFrom = lineage.fork && lineage.origin ? forkedFromOf(lineage.origin) : undefined;
    const versionChanged = local.raw['version'] !== version;
    const forkChanged = forkedFrom !== undefined && local.raw['forkedFrom'] !== forkedFrom;
    const localRaw = versionChanged ? { ...local.raw, version } : local.raw;
    const uploadRaw = forkedFrom !== undefined ? { ...localRaw, forkedFrom } : localRaw;
    const check = parseManifest(uploadRaw);
    if (!check.ok) throw new StoreLocalError('folder_invalid', `manifest.json isn’t valid for publishing: ${check.error}.`);
    const localManifest = versionChanged ? encoder.encode(manifestText(localRaw)) : null;
    const uploadManifest = versionChanged || forkChanged ? encoder.encode(manifestText(uploadRaw)) : local.manifestBytes;
    const withManifest = (bytes: Uint8Array): SnapshotInput[] =>
      local.inputs.map((input) => (input.path === 'manifest.json' ? { path: input.path, bytes } : input));
    const inputs = withManifest(uploadManifest);
    const snap = await snapshotOrExplain(inputs);
    const localTreeSha = !forkChanged
      ? snap.treeSha
      : localManifest ? (await snapshotOrExplain(withManifest(localManifest))).treeSha : local.treeSha;
    return { snap, localTreeSha, localManifest, findings: findingsOf(inputs), inputs };
  }

  function lastVersionOf(detail: ExtensionDetailDto | null, lineage: Lineage): string | null {
    const versions = (detail?.releases ?? []).map((release) => release.version);
    if (lineage.published) versions.push(lineage.published.version);
    return versions.reduce<string | null>((max, version) => (max === null || compareVersions(version, max) > 0 ? version : max), null);
  }

  async function prepareLocked(localId: string): Promise<{ plan: PublishPlanDto; context: Prepared }> {
    const who = identity();
    const local = await readFolder(localId);
    const lineage = await lineageOf(localId, who);
    const coordinate = `${who.handle}/${localId}`;
    /* A fork has to differ from what was installed. Checked against the
       folder itself, before anything is sent anywhere. */
    if (lineage.fork && !lineage.existing && lineage.origin && local.treeSha === lineage.origin.treeSha) {
      throw new ApiError({ error: 'same_as_origin' });
    }
    const declared = local.manifest.forkedFrom ? parseForkedFrom(local.manifest.forkedFrom) : null;
    if (declared?.kind === 'store' && !lineage.fork && !lineage.existing) {
      throw new StoreLocalError('folder_invalid', `manifest.json says this was forked from ${local.manifest.forkedFrom}, but it wasn’t installed from there on this Mac. Remove “forkedFrom” and try again.`);
    }
    // Scanner and snapshot first; the network only after.
    const frozen = await freeze(local, local.manifest.version, lineage);

    let detail: ExtensionDetailDto | null = null;
    try {
      detail = await api.detail(who.handle, localId);
    } catch (error) {
      if (!(error instanceof ApiError) || error.body.error !== 'not_found') throw error;
    }
    if (detail && detail.owner.id !== who.publisherId) detail = null;
    if (detail && !lineage.existing) {
      throw new StoreLocalError('folder_invalid', `You already have ${coordinate} on the store. Remove this folder and install ${coordinate} from the store to publish updates from this Mac.`);
    }
    if (!detail && lineage.existing) {
      /* The repo this folder was published to isn't there (another
         registry, or never created): publish it as new. */
      delete lineage.existing;
      delete lineage.published;
    }

    let parent = lineage.existing?.parent;
    if (lineage.existing && !parent) {
      parent = detail?.releases.find((release) => release.id === lineage.existing?.releaseId)?.commitSha;
      if (!parent) throw new ApiError({ error: 'head_moved', head: '0'.repeat(40) });
    }
    if (!lineage.existing && lineage.fork && lineage.origin) parent = lineage.origin.commitSha;

    const lastVersion = lastVersionOf(detail, lineage);
    const firstPublish = !lineage.existing;
    const listing: PublishListing = detail
      ? {
          name: detail.name,
          tagline: detail.tagline,
          category: detail.category,
          licence: detail.licence,
          ...(detail.about ? { about: detail.about } : {})
        }
      : {
          name: local.manifest.name,
          tagline: (local.manifest.description ?? '').slice(0, PUBLISH_LIMITS.taglineChars),
          category: categoryFor(local.manifest.contributes ?? []),
          licence: 'MIT'
        };
    const context: Prepared = { identity: who, coordinate, lineage, firstPublish, lastVersion, listing };
    if (parent) context.parent = parent;
    if (firstPublish && lineage.fork && lineage.origin) context.originReleaseId = lineage.origin.releaseId;
    /* A later publish of a fork names the release it was merged from, when
       that moved past the one it was forked from (Store 1.1 merges). */
    const upstream = lineage.record?.upstream;
    if (!firstPublish && lineage.fork && lineage.origin && upstream && upstream.releaseId !== lineage.origin.releaseId) {
      context.basedOnReleaseId = upstream.releaseId;
    }

    const version = local.manifest.version;
    const plan: PublishPlanDto = {
      localId,
      coordinate,
      version,
      suggestedVersion: lastVersion && compareVersions(version, lastVersion) <= 0 ? nextPatch(lastVersion) : version,
      lastVersion,
      firstPublish,
      treeSha: frozen.snap.treeSha,
      fileCount: frozen.snap.files.length,
      sizeBytes: frozen.snap.totalBytes,
      isFork: lineage.fork,
      blockedFindings: frozen.findings.blocked,
      waivableFindings: frozen.findings.waivable,
      manifest: { id: local.manifest.id, name: local.manifest.name, description: local.manifest.description ?? null },
      listing
    };
    if (lineage.fork && lineage.origin) plan.origin = { coordinate: lineage.origin.coordinate, version: lineage.origin.version };
    if (parent) plan.parent = parent;
    return { plan, context };
  }

  function prepare(localId: string): Promise<PublishPlanDto> {
    return serial(async () => {
      try {
        const { plan, context } = await prepareLocked(localId);
        prepared.set(localId, context);
        return plan;
      } catch (error) {
        prepared.delete(localId);
        throw explain(error, { coordinate: `${options.me()?.publisher?.handle ?? ''}/${localId}` });
      }
    });
  }

  /** The waivers to send: one per soft finding in the frozen tree, each with a reason. */
  function waiversFor(findings: PublishFinding[], given: PublishWaiver[]): PublishWaiver[] {
    const byKey = new Map(given.map((waiver) => [findingKey(waiver), waiver]));
    return findings.map((finding) => {
      const waiver = byKey.get(findingKey(finding));
      if (!waiver || waiverProblem(waiver.reason)) {
        throw new StoreLocalError('folder_invalid', `Say why ${finding.path} line ${finding.line} is safe to publish.`);
      }
      return { path: finding.path, line: finding.line, reason: waiver.reason.trim() };
    });
  }

  function checkIcon(iconPng: string | undefined): void {
    if (iconPng === undefined) return;
    let bytes: Uint8Array;
    try {
      bytes = Uint8Array.from(Buffer.from(iconPng, 'base64'));
    } catch {
      throw new StoreLocalError('folder_invalid', 'The icon must be a PNG file.');
    }
    if (bytes.length > PUBLISH_LIMITS.iconBytes) throw new StoreLocalError('folder_invalid', 'The icon must be 256 KB or smaller.');
    const size = pngSize(bytes);
    if (!size) throw new StoreLocalError('folder_invalid', 'The icon must be a PNG file.');
    if (size.width !== size.height) throw new StoreLocalError('folder_invalid', 'The icon must be square.');
  }

  async function uploadMissing(localId: string, objects: Array<{ sha: string; type: GitObjectType; body: Uint8Array }>, context: Prepared): Promise<void> {
    const report = (phase: PublishProgress['phase'], done = 0, total = 0): void =>
      options.notifyProgress?.({ localId, phase, done, total });
    report('checking');
    const scope: Omit<MissingBody, 'shas'> = {};
    if (context.lineage.existing) scope.repoId = context.lineage.existing.repoId;
    if (context.originReleaseId) scope.originReleaseId = context.originReleaseId;
    if (context.basedOnReleaseId) scope.basedOnReleaseId = context.basedOnReleaseId;
    const missing = new Set<string>();
    const shas = objects.map((object) => object.sha);
    for (let start = 0; start < shas.length; start += 1000) {
      const answer = await api.missing({ shas: shas.slice(start, start + 1000), ...scope });
      for (const sha of answer.missing) missing.add(sha);
    }
    const wanted = objects.filter((object) => missing.has(object.sha));
    if (!wanted.length) return;
    // Progress counts files (blobs); folders and the commit ride along.
    const blobs = new Set(wanted.filter((object) => object.type === 'blob').map((object) => object.sha));
    const total = blobs.size;
    const parts: UploadPart[] = [];
    for (const object of wanted) parts.push({ sha: object.sha, bytes: await encodeLoose(object.type, object.body) });
    let done = 0;
    report('uploading', done, total);
    for (const batch of batchParts(parts)) {
      const result = await api.upload(batch);
      const rejected = result.rejected[0];
      if (rejected) {
        throw new StoreLocalError('upload_rejected', `The store refused one of the files (${rejected.code}). Try again.`);
      }
      done += batch.filter((part) => blobs.has(part.sha)).length;
      report('uploading', done, total);
    }
  }

  async function writeBack(local: LocalFolder, bytes: Uint8Array): Promise<void> {
    /* Only when the folder's manifest is still the one that was published
       from; an edit made meanwhile wins. */
    const target = path.join(local.folder, 'manifest.json');
    const current = new Uint8Array(await fs.readFile(target));
    if (Buffer.compare(Buffer.from(current), Buffer.from(local.manifestBytes)) !== 0) return;
    const temporary = path.join(local.folder, `.manifest.json.${process.pid}.tmp`);
    await fs.writeFile(temporary, bytes);
    await fs.rename(temporary, target);
  }

  async function publishLocked(localId: string, form: PublishForm): Promise<StorePublishResult> {
    let context = prepared.get(localId);
    const who = identity();
    if (!context || context.identity.publisherId !== who.publisherId) {
      context = (await prepareLocked(localId)).context;
    }
    const { coordinate, lineage } = context;
    const version = form.version.trim();
    const problem = versionProblem(version, context.lastVersion);
    if (problem) throw new StoreLocalError('folder_invalid', problem);
    if (context.firstPublish && !form.listing) throw new StoreLocalError('folder_invalid', 'Add a name for the store.');
    if (context.firstPublish) checkIcon(form.iconPng);

    // The frozen snapshot: read now, shown in the confirmation, uploaded as is.
    const local = await readFolder(localId);
    if (lineage.fork && !lineage.existing && lineage.origin && local.treeSha === lineage.origin.treeSha) {
      throw new ApiError({ error: 'same_as_origin' });
    }
    const frozen = await freeze(local, version, lineage);
    const blocked = frozen.findings.blocked;
    if (blocked.length) throw new ApiError({ error: 'scan_blocked', findings: blocked.map(({ path: file, line, kind }) => ({ path: file, line, kind })) });
    const waivers = waiversFor(frozen.findings.waivable, form.waivers);

    const time = Math.floor(now() / 1000);
    const person = { name: context.identity.handle, email: `${context.identity.handle}@${HANDLE_MAIL_DOMAIN}`, time, tz: '+0000' };
    const commitBody = encodeCommit({
      tree: frozen.snap.treeSha,
      parents: context.parent ? [context.parent] : [],
      author: person,
      committer: person,
      message: `${coordinate} ${version}`
    });
    const commitSha = await hashObject('commit', commitBody);

    const { snap } = frozen;
    const fileCount = snap.files.length;
    const confirmed = await options.confirm({
      type: 'question',
      message: `Publish ${coordinate} ${version}?`,
      detail: [
        `Snapshot ${snap.treeSha.slice(0, 12)}. ${fileCount} ${fileCount === 1 ? 'file' : 'files'}, ${formatSize(snap.totalBytes)}.`,
        lineage.fork && lineage.origin ? `Forked from ${lineage.origin.coordinate} ${lineage.origin.version}.` : null
      ].filter(Boolean).join(' '),
      buttons: ['Publish', 'Cancel'],
      defaultId: 0,
      cancelId: 1,
      noLink: true
    });
    if (!confirmed) return { published: false };

    const objects = [...snap.objects, { sha: commitSha, type: 'commit' as const, body: commitBody }];
    const body: PutReleaseBody = { version, commitSha, listing: context.listing, waivers };
    const notes = form.notes?.trim();
    if (notes) body.notes = notes;
    if (context.firstPublish && form.listing) {
      body.listing = { name: form.listing.name.trim(), tagline: form.listing.tagline.trim(), category: form.listing.category, licence: form.listing.licence };
      body.visibility = form.visibility ?? 'public';
      if (form.iconPng) body.iconPng = form.iconPng;
    }
    if (context.originReleaseId) body.originReleaseId = context.originReleaseId;
    if (context.basedOnReleaseId) body.basedOnReleaseId = context.basedOnReleaseId;

    const [handle] = coordinate.split('/');
    let result: PutReleaseResult | null = null;
    for (let attempt = 0; attempt < 2 && !result; attempt++) {
      await uploadMissing(localId, objects, context);
      options.notifyProgress?.({ localId, phase: 'publishing', done: 0, total: 0 });
      try {
        result = await api.putRelease(handle ?? context.identity.handle, localId, body);
      } catch (error) {
        const code = error instanceof ApiError ? error.body.error : null;
        // Objects can be collected between upload and put: upload again, once.
        if (attempt === 0 && (code === 'object_missing' || code === 'commit_missing')) continue;
        throw error;
      }
    }
    if (!result) throw new ApiError({ error: 'object_missing', shas: [] });
    const { repo, release } = result;
    if (release.commitSha !== commitSha || release.treeSha !== snap.treeSha) {
      log('the registry recorded a different commit than the one uploaded');
    }

    await provenance.ensureEnvKey(localId);
    const firstFork = !lineage.existing && lineage.fork && lineage.origin ? lineage.origin : null;
    await provenance.update(localId, (current) => {
      if (!current) return null;
      const next: ProvenanceRecord = {
        ...current,
        published: {
          repoId: repo.repoId,
          releaseId: release.id,
          version: release.version,
          commitSha,
          ownerPublisherId: context.identity.publisherId,
          coordinate,
          publishedTreeSha: snap.treeSha,
          localTreeSha: frozen.localTreeSha
        }
      };
      if (firstFork) next.upstream = { releaseId: firstFork.releaseId, treeSha: firstFork.treeSha };
      return next;
    });
    if (frozen.localManifest) {
      try {
        await writeBack(local, frozen.localManifest);
        await registry.refresh([localId]);
        registry.emitChanged({ ids: [localId], reason: 'reload' });
      } catch (error) {
        log('could not write the published version into manifest.json', error);
      }
    }
    prepared.delete(localId);
    options.notifyLibrary?.();
    return { published: true, coordinate, version: release.version, repoId: repo.repoId, releaseId: release.id };
  }

  function publish(localId: string, form: PublishForm): Promise<StorePublishResult> {
    return serial(async () => {
      try {
        return await publishLocked(localId, form);
      } catch (error) {
        throw explain(error, { coordinate: prepared.get(localId)?.coordinate ?? localId, version: form.version });
      }
    });
  }

  async function coordinateOf(repoId: string): Promise<{ handle: string; slug: string }> {
    const who = identity();
    const records = Object.values(await provenance.read());
    for (const record of records) {
      const published = record.published;
      if (published?.repoId === repoId && published.ownerPublisherId === who.publisherId && published.coordinate) {
        const [handle, slug] = published.coordinate.split('/');
        if (handle && slug) return { handle, slug };
      }
      const origin = record.origin;
      if (origin?.repoId === repoId && origin.ownerPublisherId === who.publisherId) {
        const [handle, slug] = origin.coordinate.split('/');
        if (handle && slug) return { handle, slug };
      }
    }
    const mine = (await api.myRepos()).items.find((item) => item.repoId === repoId);
    if (!mine) throw new ApiError({ error: 'not_owner' });
    return { handle: mine.owner.handle, slug: mine.slug };
  }

  function yank(target: string | { repoId: string; version: string }): Promise<{ version: string }> {
    return serial(async () => {
      let repoId: string;
      let version: string;
      if (typeof target === 'string') {
        const published = (await provenance.get(target))?.published;
        if (!published) throw new StoreLocalError('not_installed', 'This extension hasn’t been published from this Mac.');
        repoId = published.repoId;
        version = published.version;
      } else {
        ({ repoId, version } = target);
      }
      try {
        const { handle, slug } = await coordinateOf(repoId);
        const result = await api.yank(handle, slug, version);
        options.notifyLibrary?.();
        return { version: result.release.version };
      } catch (error) {
        if (error instanceof ApiError && (error.body.error === 'not_owner' || error.body.error === 'forbidden')) {
          throw new ApiError({ error: error.body.error, detail: 'You can only withdraw versions of your own extensions.' });
        }
        if (error instanceof ApiError && error.body.error === 'not_found') {
          throw new ApiError({ error: 'not_found', detail: `Version ${version} isn’t on the store.` });
        }
        throw explain(error, { version });
      }
    });
  }

  return { prepare, publish, yank };
}
