import { execFile } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createWriteStream, readFileSync } from 'node:fs';
import { chmod, cp, lstat, mkdir, mkdtemp, readdir, rename, rm, symlink, writeFile } from 'node:fs/promises';
import { Readable } from 'node:stream';
import { pipeline } from 'node:stream/promises';
import os from 'node:os';
import path from 'node:path';

/* The agent runtimes ship inside the signed app bundle, which Powermove can't
   rewrite. When a provider needs a newer runtime than the bundle carries, the
   latest release is downloaded from npm into userData, laid out exactly like
   the bundled resources, and discovery prefers it until the app itself is
   updated (the new bundle is then the fresher copy). */

export type RuntimeProvider = 'claude' | 'codex';

export interface RuntimeUpdateResult {
  provider: RuntimeProvider;
  version: string;
}

interface InstalledRuntime {
  version: string;
  appVersion: string;
}

interface Release {
  version: string;
  tarball: string;
  integrity: string;
}

const REGISTRY = 'https://registry.npmjs.org';
const RELATIVE_BINARY: Record<RuntimeProvider, string> = {
  claude: path.join('claude', 'bin', 'claude'),
  codex: path.join('codex', 'bin', 'codex')
};

let root: string | null = null;
let appVersion = '';
let scratchSweep: Promise<void> = Promise.resolve();

/** Recover downloads left behind when the process exits before finally runs.
    Only old runtime scratch directories with our archive/extraction layout
    qualify; never follow symlinks or remove other temporary directories. */
export async function sweepRuntimeScratch(tmpDir = os.tmpdir(), now = Date.now()): Promise<void> {
  const cutoff = now - 60 * 60 * 1000;
  for (const entry of await readdir(tmpDir, { withFileTypes: true })) {
    if (!entry.isDirectory() || !/^powermove-(claude|codex)-.+$/.test(entry.name)) continue;
    const scratch = path.join(tmpDir, entry.name);
    try {
      const info = await lstat(scratch);
      if (!info.isDirectory() || info.mtimeMs >= cutoff) continue;
      const archive = await lstat(path.join(scratch, 'package.tgz')).catch(() => null);
      const extracted = await lstat(path.join(scratch, 'package')).catch(() => null);
      if (archive?.isFile() || extracted?.isDirectory()) {
        await rm(scratch, { recursive: true, force: true });
      }
    } catch {
      // A concurrent cleanup or an inaccessible entry must not stop the sweep.
    }
  }
}

export function configureRuntimeUpdates(options: { root: string; appVersion: string }): void {
  root = options.root;
  appVersion = options.appVersion;
  // Start in the background without delaying boot. New installs wait for the
  // sweep so it cannot race their scratch directories.
  scratchSweep = sweepRuntimeScratch().catch(() => {});
}

function installed(provider: RuntimeProvider): InstalledRuntime | null {
  if (root === null) return null;
  try {
    const value = JSON.parse(readFileSync(path.join(root, `${provider}.json`), 'utf8')) as InstalledRuntime;
    return typeof value.version === 'string' && value.appVersion === appVersion ? value : null;
  } catch {
    return null;
  }
}

/** The downloaded runtime, when one exists for this app version. Synchronous so
    the bundled-candidate lists can include it without changing their shape. */
export function updatedRuntimeCandidates(provider: RuntimeProvider): string[] {
  return root !== null && installed(provider) ? [path.join(root, RELATIVE_BINARY[provider])] : [];
}

function darwinArch(): 'arm64' | 'x64' {
  return process.arch === 'x64' ? 'x64' : 'arm64';
}

async function registry(name: string, tag: string): Promise<Release> {
  const response = await fetch(`${REGISTRY}/${name.replace('/', '%2F')}/${encodeURIComponent(tag)}`);
  if (!response.ok) throw new Error(`Couldn’t check for the latest ${name} (${response.status}).`);
  const body = await response.json() as { version?: unknown; dist?: { tarball?: unknown; integrity?: unknown } };
  const { version, dist } = body;
  if (typeof version !== 'string' || typeof dist?.tarball !== 'string' || typeof dist.integrity !== 'string'
    || !dist.tarball.startsWith(`${REGISTRY}/`)) {
    throw new Error(`The registry returned an unexpected release for ${name}.`);
  }
  return { version, tarball: dist.tarball, integrity: dist.integrity };
}

async function latestRelease(provider: RuntimeProvider): Promise<Release> {
  const arch = darwinArch();
  if (provider === 'claude') return registry(`@anthropic-ai/claude-code-darwin-${arch}`, 'latest');
  // Codex publishes each platform build as a prerelease tag of the main package.
  const { version } = await registry('@openai/codex', 'latest');
  const release = await registry('@openai/codex', `${version}-darwin-${arch}`);
  return { ...release, version };
}

async function download(release: Release, file: string): Promise<void> {
  const response = await fetch(release.tarball);
  if (!response.ok || response.body === null) throw new Error(`The download failed (${response.status}).`);
  // Stream to disk and hash incrementally so a large runtime never sits in
  // memory or blocks the main process in one long hash call.
  const hash = createHash('sha512');
  await pipeline(
    Readable.fromWeb(response.body as Parameters<typeof Readable.fromWeb>[0]),
    async function* (source: AsyncIterable<Buffer>) {
      for await (const chunk of source) {
        hash.update(chunk);
        yield chunk;
      }
    },
    createWriteStream(file)
  );
  const [algorithm, expected] = release.integrity.split('-', 2);
  if (algorithm !== 'sha512' || hash.digest('base64') !== expected) {
    throw new Error('The downloaded runtime didn’t match its published checksum.');
  }
}

function run(file: string, args: readonly string[], timeout = 60_000): Promise<string> {
  return new Promise((resolve, reject) => {
    execFile(file, [...args], { encoding: 'utf8', timeout, maxBuffer: 4 * 1024 * 1024 }, (error, stdout) => {
      if (error) reject(error);
      else resolve(stdout);
    });
  });
}

function versionParts(version: string): number[] {
  return (version.match(/\d+/g) ?? []).slice(0, 3).map(Number);
}

export function isNewerVersion(candidate: string, current: string): boolean {
  const a = versionParts(candidate);
  const b = versionParts(current);
  for (let i = 0; i < 3; i++) {
    if ((a[i] ?? 0) !== (b[i] ?? 0)) return (a[i] ?? 0) > (b[i] ?? 0);
  }
  return false;
}

/** Install the newest runtime only when it is ahead of the binary in use.
    Returns null when the runtime is already current. */
export async function installRuntimeIfNewer(
  provider: RuntimeProvider,
  currentBinary: string
): Promise<RuntimeUpdateResult | null> {
  if (root === null || process.platform !== 'darwin') return null;
  const current = installed(provider)?.version
    ?? (await run(currentBinary, ['--version'], 15_000)).match(/\d+\.\d+\.\d+\S*/)?.[0];
  const release = await latestRelease(provider);
  if (current && !isNewerVersion(release.version, current)) return null;
  return installRelease(provider, release);
}

/** Download, verify, and activate the newest runtime for a provider. */
export async function installLatestRuntime(provider: RuntimeProvider): Promise<RuntimeUpdateResult> {
  if (root === null) throw new Error('Runtime updates aren’t available in this build.');
  if (process.platform !== 'darwin') throw new Error('Runtime updates are only available on macOS.');
  return installRelease(provider, await latestRelease(provider));
}

async function installRelease(provider: RuntimeProvider, release: Release): Promise<RuntimeUpdateResult> {
  if (root === null) throw new Error('Runtime updates aren’t available in this build.');
  await scratchSweep;
  const scratch = await mkdtemp(path.join(os.tmpdir(), `powermove-${provider}-`));
  try {
    const archive = path.join(scratch, 'package.tgz');
    // Leave a recognizable marker even if fetching is interrupted before the
    // response arrives. Startup can reclaim this path after an unclean exit.
    await writeFile(archive, '');
    await download(release, archive);
    await run('/usr/bin/tar', ['-xzf', archive, '-C', scratch]);

    const versions = path.join(root, '.versions');
    const target = path.join(versions, `${provider}-${release.version}`);
    await rm(target, { recursive: true, force: true });
    await mkdir(target, { recursive: true });
    if (provider === 'claude') {
      await mkdir(path.join(target, 'bin'));
      await cp(path.join(scratch, 'package', 'claude'), path.join(target, 'bin', 'claude'));
    } else {
      const triple = darwinArch() === 'arm64' ? 'aarch64-apple-darwin' : 'x86_64-apple-darwin';
      await cp(path.join(scratch, 'package', 'vendor', triple), target, { recursive: true });
    }
    const binary = path.join(target, 'bin', provider);
    await chmod(binary, 0o755);
    await run(binary, ['--version'], 15_000);

    // Swap the provider link atomically so a run starting now sees either
    // the old runtime or the new one, never a half-written directory.
    const link = path.join(root, provider);
    const staged = `${link}.${process.pid}.tmp`;
    await rm(staged, { force: true });
    await symlink(target, staged);
    await rename(staged, link);
    await writeFile(path.join(root, `${provider}.json`),
      JSON.stringify({ version: release.version, appVersion } satisfies InstalledRuntime));

    for (const entry of await readdir(versions)) {
      if (entry.startsWith(`${provider}-`) && entry !== path.basename(target)) {
        await rm(path.join(versions, entry), { recursive: true, force: true });
      }
    }
    return { provider, version: release.version };
  } finally {
    await rm(scratch, { recursive: true, force: true });
  }
}
