import { execFile } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { chmod, cp, mkdir, mkdtemp, readdir, rename, rm, symlink, writeFile } from 'node:fs/promises';
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
  if (!response.ok) throw new Error(`The download failed (${response.status}).`);
  const bytes = Buffer.from(await response.arrayBuffer());
  const [algorithm, expected] = release.integrity.split('-', 2);
  if (algorithm !== 'sha512' || createHash('sha512').update(bytes).digest('base64') !== expected) {
    throw new Error('The downloaded runtime didn’t match its published checksum.');
  }
  await writeFile(file, bytes);
}

function run(file: string, args: readonly string[], timeout = 60_000): Promise<string> {
  return new Promise((resolve, reject) => {
    execFile(file, [...args], { encoding: 'utf8', timeout, maxBuffer: 4 * 1024 * 1024 }, (error, stdout) => {
      if (error) reject(error);
      else resolve(stdout);
    });
  });
}

/** Download, verify, and activate the newest runtime for a provider. */
export async function installLatestRuntime(provider: RuntimeProvider): Promise<RuntimeUpdateResult> {
  if (root === null) throw new Error('Runtime updates aren’t available in this build.');
  if (process.platform !== 'darwin') throw new Error('Runtime updates are only available on macOS.');
  const release = await latestRelease(provider);
  const scratch = await mkdtemp(path.join(os.tmpdir(), `powermove-${provider}-`));
  try {
    const archive = path.join(scratch, 'package.tgz');
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
