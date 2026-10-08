import { execFile } from 'node:child_process';
import { createHash, randomUUID } from 'node:crypto';
import { createWriteStream, readFileSync } from 'node:fs';
import { chmod, cp, mkdir, mkdtemp, rename, rm, writeFile } from 'node:fs/promises';
import { Readable } from 'node:stream';
import { pipeline } from 'node:stream/promises';
import os from 'node:os';
import { agentPlatform, executableName } from './platform';
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
  directory?: string;
}

interface Release {
  version: string;
  tarball: string;
  integrity: string;
}

const REGISTRY = 'https://registry.npmjs.org';
const RELATIVE_BINARY: Record<RuntimeProvider, string> = {
  claude: path.join('claude', 'bin', executableName('claude')),
  codex: path.join('codex', 'bin', executableName('codex'))
};

let root: string | null = null;
let appVersion = '';

export function configureRuntimeUpdates(options: { root: string; appVersion: string }): void {
  root = options.root;
  appVersion = options.appVersion;
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
  const value = installed(provider);
  if (root === null || !value) return [];
  if (value.directory && /^\.versions[\\/][A-Za-z0-9._-]+$/.test(value.directory)) return [path.join(root, value.directory, 'bin', executableName(provider))];
  return [path.join(root, RELATIVE_BINARY[provider])];
}

async function registry(name: string, tag: string): Promise<Release> {
  const response = await fetch(`${REGISTRY}/${name.replace('/', '%2F')}/${encodeURIComponent(tag)}`);
  if (!response.ok) throw new Error(`Couldn’t check for the latest ${name} (${response.status}).`);
  const body = await response.json() as { version?: unknown; dist?: { tarball?: unknown; integrity?: unknown } };
  const { version, dist } = body;
  if (typeof version !== 'string' || !/^\d+\.\d+\.\d+(?:[-+][A-Za-z0-9.-]+)?$/.test(version) || typeof dist?.tarball !== 'string' || typeof dist.integrity !== 'string'
    || !dist.tarball.startsWith(`${REGISTRY}/`)) {
    throw new Error(`The registry returned an unexpected release for ${name}.`);
  }
  return { version, tarball: dist.tarball, integrity: dist.integrity };
}

async function latestRelease(provider: RuntimeProvider): Promise<Release> {
  const { suffix } = agentPlatform();
  if (provider === 'claude') return registry(`@anthropic-ai/claude-code-${suffix}`, 'latest');
  // Codex publishes each platform build as a prerelease tag of the main package.
  const { version } = await registry('@openai/codex', 'latest');
  const release = await registry('@openai/codex', `${version}-${suffix}`);
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
  if (root === null || !['darwin', 'win32', 'linux'].includes(process.platform)) return null;
  const current = installed(provider)?.version
    ?? (await run(currentBinary, ['--version'], 15_000)).match(/\d+\.\d+\.\d+\S*/)?.[0];
  const release = await latestRelease(provider);
  if (current && !isNewerVersion(release.version, current)) return null;
  return installRelease(provider, release);
}

/** Download, verify, and activate the newest runtime for a provider. */
export async function installLatestRuntime(provider: RuntimeProvider): Promise<RuntimeUpdateResult> {
  if (root === null) throw new Error('Runtime updates aren’t available in this build.');
  agentPlatform();
  return installRelease(provider, await latestRelease(provider));
}

async function installRelease(provider: RuntimeProvider, release: Release): Promise<RuntimeUpdateResult> {
  if (root === null) throw new Error('Runtime updates aren’t available in this build.');
  const scratch = await mkdtemp(path.join(os.tmpdir(), `powermove-${provider}-`));
  try {
    const archive = path.join(scratch, 'package.tgz');
    await download(release, archive);
    await run(process.platform === 'win32' ? path.win32.join(process.env.SystemRoot ?? 'C:\\Windows', 'System32', 'tar.exe') : 'tar', ['-xzf', archive, '-C', scratch]);

    const versions = path.join(root, '.versions');
    const target = path.join(versions, `${provider}-${release.version}-${randomUUID()}`);
    await rm(target, { recursive: true, force: true });
    await mkdir(target, { recursive: true });
    if (provider === 'claude') {
      await mkdir(path.join(target, 'bin'));
      await cp(path.join(scratch, 'package', executableName('claude')), path.join(target, 'bin', executableName('claude')));
    } else {
      const { triple } = agentPlatform();
      await cp(path.join(scratch, 'package', 'vendor', triple), target, { recursive: true });
    }
    const binary = path.join(target, 'bin', executableName(provider));
    await chmod(binary, 0o755);
    await run(binary, ['--version'], 15_000);

    // Activate with a small atomic manifest, avoiding administrator-only
    // symlinks and replacement of executables Windows may still have open.
    const manifest = path.join(root, `${provider}.json`);
    const staged = `${manifest}.${randomUUID()}.tmp`;
    await writeFile(staged, JSON.stringify({ version: release.version, appVersion, directory: path.relative(root, target) } satisfies InstalledRuntime));
    await rename(staged, manifest);
    // Keep older directories: an active run may still be using one of them.
    return { provider, version: release.version };
  } finally {
    await rm(scratch, { recursive: true, force: true });
  }
}
