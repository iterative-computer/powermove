import { basename } from 'node:path';
import { encodeCommit, encodeLoose, hashObject } from '@powermove/registry/git';
import { parseManifest, type ExtensionManifest } from '@powermove/registry/manifest';
import { walkDir } from '@powermove/registry/node';
import { scanFiles } from '@powermove/registry/scan';
import { snapshot, type SnapshotInput } from '@powermove/registry/snapshot';
import type { Snapshot } from '@powermove/registry/snapshot';

type Fetch = (input: string | URL | Request, init?: RequestInit) => Promise<Response>;
type Category = 'panels' | 'effects' | 'transitions' | 'themes' | 'layers' | 'commands' | 'tools';
export type BuiltinPlan = { coordinate: string; version: string; treeSha: string; fileCount: number; category: Category; status: 'published' | 'skipped' | 'planned' };
export interface PublishBuiltinsOptions {
  fetch: Fetch;
  dirs: string[];
  token?: string;
  origin: string;
  appVersion: string;
  notes?: string;
  dryRun?: boolean;
  readDir?: (dir: string) => Promise<SnapshotInput[]>;
}

export function builtinCategory(manifest: ExtensionManifest): Category {
  const first = manifest.contributes?.[0];
  if (first === 'panels' || first === 'effects' || first === 'transitions' || first === 'themes' || first === 'layers') return first;
  if (first === 'commands' || first === 'palette' || first === 'keybindings') return 'commands';
  return 'tools';
}

function parseBuiltin(input: SnapshotInput[], dir: string): ExtensionManifest {
  const file = input.find(item => item.path === 'manifest.json');
  if (!file) throw new Error(`${dir}: manifest.json is missing`);
  let raw: unknown;
  try { raw = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(file.bytes)); }
  catch { throw new Error(`${dir}: manifest.json is invalid JSON or UTF-8`); }
  const parsed = parseManifest(raw);
  if (!parsed.ok) throw new Error(`${dir}: ${parsed.error}`);
  return parsed.manifest;
}

function scanBuiltin(input: SnapshotInput[], coordinate: string): void {
  const decoder = new TextDecoder('utf-8', { fatal: true });
  const textFiles = input.flatMap(file => {
    try { return [{ path: file.path, text: decoder.decode(file.bytes) }]; }
    catch { return []; }
  });
  const findings = scanFiles(textFiles);
  if (findings.blocked.length || findings.waived.length) {
    throw new Error(`${coordinate}: secret scan found ${[...findings.blocked, ...findings.waived].map(f => `${f.path}:${f.line} (${f.kind})`).join(', ')}`);
  }
}

async function api(fetcher: Fetch, url: string, token: string, init: RequestInit = {}): Promise<Response> {
  return fetcher(url, { ...init, headers: { Authorization: `Bearer ${token}`, ...init.headers } });
}

async function responseBody(response: Response): Promise<Record<string, unknown>> {
  try { return await response.json() as Record<string, unknown>; }
  catch { return {}; }
}

function failed(coordinate: string, step: string, response: Response, body: Record<string, unknown>): never {
  throw new Error(`${coordinate}: ${step} failed (${response.status} ${String(body.error ?? response.statusText)}${body.detail ? `: ${body.detail}` : ''})`);
}

async function uploadObjects(fetcher: Fetch, base: string, token: string, coordinate: string, snap: Snapshot, commit: { sha: string; type: 'commit'; body: Uint8Array }, repoId?: string): Promise<void> {
  const objects = [...snap.objects, commit];
  const missingResponse = await api(fetcher, `${base}/v1/objects/missing`, token, {
    method: 'POST', headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ shas: objects.map(object => object.sha), ...(repoId ? { repoId } : {}) })
  });
  const missingBody = await responseBody(missingResponse);
  if (!missingResponse.ok) failed(coordinate, 'objects/missing', missingResponse, missingBody);
  const missing = new Set(missingBody.missing as string[]);
  const pending = objects.filter(object => missing.has(object.sha));
  for (let start = 0; start < pending.length; start += 64) {
    const form = new FormData();
    for (const object of pending.slice(start, start + 64)) {
      const loose = await encodeLoose(object.type, object.body);
      form.append(object.sha, new Blob([new Uint8Array(loose)], { type: 'application/x-git-loose-object' }), 'object');
    }
    const response = await api(fetcher, `${base}/v1/objects`, token, { method: 'POST', body: form });
    const body = await responseBody(response);
    if (!response.ok) failed(coordinate, 'objects upload', response, body);
    if (Array.isArray(body.rejected) && body.rejected.length) throw new Error(`${coordinate}: objects upload rejected ${JSON.stringify(body.rejected)}`);
  }
}

export async function publishBuiltins(options: PublishBuiltinsOptions): Promise<BuiltinPlan[]> {
  const { fetch: fetcher, dirs, dryRun = false, readDir = walkDir } = options;
  if (!dryRun && !options.token) throw new Error('POWERMOVE_REGISTRY_TOKEN is required');
  const base = options.origin.replace(/\/+$/, '');
  const plans: BuiltinPlan[] = [];
  for (const dir of dirs) {
    const input = await readDir(dir);
    const manifest = parseBuiltin(input, dir);
    const coordinate = `powermove/${manifest.id}`;
    if (basename(dir) !== manifest.id) throw new Error(`${dir}: manifest id ${manifest.id} does not match the directory name`);
    scanBuiltin(input, coordinate);
    const snap = await snapshot(input);
    const category = builtinCategory(manifest);
    const plan: BuiltinPlan = { coordinate, version: manifest.version, treeSha: snap.treeSha, fileCount: snap.files.length, category, status: 'planned' };
    if (dryRun) { plans.push(plan); continue; }

    const detailResponse = await api(fetcher, `${base}/v1/store/x/${coordinate}`, options.token!);
    let repoId: string | undefined;
    let parent: string | undefined;
    if (detailResponse.ok) {
      const detail = await responseBody(detailResponse);
      repoId = detail.repoId as string;
      const releases = detail.releases as Array<{ version: string; treeSha: string; commitSha: string }>;
      const currentVersion = releases.find(release => release.version === manifest.version);
      if (currentVersion) {
        if (currentVersion.treeSha !== snap.treeSha) throw new Error(`${coordinate}@${manifest.version}: bump the version in manifest.json`);
        plans.push({ ...plan, status: 'skipped' });
        continue;
      }
      parent = releases[0]?.commitSha;
      if (!parent) throw new Error(`${coordinate}: existing repo has no release to use as parent`);
    } else if (detailResponse.status !== 404) failed(coordinate, 'repo lookup', detailResponse, await responseBody(detailResponse));

    const identity = { name: 'powermove', email: 'powermove@users.trypowermove.com', time: Math.floor(Date.now() / 1000), tz: '+0000' };
    const commitBody = encodeCommit({ tree: snap.treeSha, parents: parent ? [parent] : [], author: identity, committer: identity, message: `Publish ${coordinate}@${manifest.version}` });
    const commit = { sha: await hashObject('commit', commitBody), type: 'commit' as const, body: commitBody };
    await uploadObjects(fetcher, base, options.token!, coordinate, snap, commit, repoId);
    const response = await api(fetcher, `${base}/v1/repos/${coordinate}/releases`, options.token!, {
      method: 'PUT', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ version: manifest.version, commitSha: commit.sha, notes: options.notes ?? `Powermove ${options.appVersion}`,
        listing: { name: manifest.name, tagline: manifest.description ?? manifest.name, category, licence: 'AGPL-3.0-or-later' }, waivers: [] })
    });
    if (!response.ok) {
      const body = await responseBody(response);
      if (body.error === 'version_exists') throw new Error(`${coordinate}@${manifest.version}: bump the version in manifest.json`);
      failed(coordinate, 'release publish', response, body);
    }
    plans.push({ ...plan, status: 'published' });
  }
  return plans;
}
