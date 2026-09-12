import { execFile } from 'node:child_process';
import { constants } from 'node:fs';
import { access, open, stat } from 'node:fs/promises';
import path from 'node:path';

export const PACKAGED_CODEX_RELATIVE_PATH = path.join('codex', 'bin', 'codex');
export const DEVELOPMENT_CODEX_RELATIVE_PATH = path.join(
  'node_modules',
  '@openai',
  'codex-darwin-arm64',
  'vendor',
  'aarch64-apple-darwin',
  'bin',
  'codex'
);

export const KNOWN_CODEX_PATHS = [
  '/opt/homebrew/lib/node_modules/@openai/codex/node_modules/@openai/codex-darwin-arm64/vendor/aarch64-apple-darwin/bin/codex',
  '/usr/local/lib/node_modules/@openai/codex/node_modules/@openai/codex-darwin-arm64/vendor/aarch64-apple-darwin/bin/codex'
] as const;

export const CODEX_NOT_FOUND_MESSAGE =
  "Powermove's built-in ChatGPT runtime is missing or unavailable. Reinstall Powermove, set CODEX_BINARY, or choose a Codex binary in Settings.";

export interface CodexDiscoveryOptions {
  /** Overrides the built-in candidates so discovery order can be tested without the installed dependency. */
  bundledCandidates?: readonly string[];
}

let loginShellProbe: Promise<string | null> | null = null;
let description: Promise<CodexDescription> | null = null;

function execFileText(file: string, args: readonly string[]): Promise<string> {
  return new Promise((resolve, reject) => {
    execFile(file, [...args], { encoding: 'utf8', timeout: 5_000, maxBuffer: 1024 * 1024 }, (error, stdout) => {
      if (error) reject(error);
      else resolve(stdout);
    });
  });
}

async function executable(candidate: string | null | undefined): Promise<string | null> {
  const path = candidate?.trim();
  if (!path) return null;
  try {
    if (!(await stat(path)).isFile()) return null;
    await access(path, constants.X_OK);
    return path;
  } catch {
    return null;
  }
}

/**
 * Codex delegates shell and MCP work to a companion executable beside the
 * main binary. iCloud can leave that large companion as a dataless placeholder
 * while keeping the small launcher executable, which otherwise makes runtime
 * discovery succeed and every tool call fail during its handshake.
 */
async function executableRuntime(candidate: string | null | undefined): Promise<string | null> {
  const binary = await executable(candidate);
  if (binary === null) return null;
  const host = path.join(path.dirname(binary), 'codex-code-mode-host');
  const companion = await executable(host);
  if (companion === null) return null;
  let file;
  try {
    file = await open(companion, 'r');
    const probe = Buffer.allocUnsafe(1);
    const { bytesRead } = await file.read(probe, 0, 1, 0);
    return bytesRead === 1 ? binary : null;
  } catch {
    return null;
  } finally {
    await file?.close().catch(() => undefined);
  }
}

async function probeLoginShell(): Promise<string | null> {
  if (loginShellProbe === null) {
    loginShellProbe = (async () => {
      try {
        const stdout = await execFileText('/bin/zsh', ['-ilc', 'command -v codex']);
        const lines = stdout
          .split(/\r?\n/u)
          .map((line) => line.trim())
          .filter(Boolean);
        return executable(lines.at(-1));
      } catch {
        return null;
      }
    })();
  }
  return loginShellProbe;
}

export function bundledCodexCandidates(
  appRoot = process.cwd(),
  resourcesPath = (process as NodeJS.Process & { resourcesPath?: string }).resourcesPath
): string[] {
  const candidates = [path.join(appRoot, DEVELOPMENT_CODEX_RELATIVE_PATH)];
  if (resourcesPath) candidates.unshift(path.join(resourcesPath, PACKAGED_CODEX_RELATIVE_PATH));
  return candidates;
}

export async function discoverCodex(
  codexBinary: string | null,
  options: CodexDiscoveryOptions = {}
): Promise<string> {
  // Explicit binaries may be npm/shell launchers whose companion lives in the
  // package they resolve internally rather than beside the launcher itself.
  const environmentBinary = await executable(process.env.CODEX_BINARY);
  if (environmentBinary !== null) return environmentBinary;

  const preferredBinary = await executable(codexBinary);
  if (preferredBinary !== null) return preferredBinary;

  for (const candidate of options.bundledCandidates ?? bundledCodexCandidates()) {
    const found = await executableRuntime(candidate);
    if (found !== null) return found;
  }

  const shellBinary = await probeLoginShell();
  if (shellBinary !== null) return shellBinary;

  for (const candidate of KNOWN_CODEX_PATHS) {
    const found = await executableRuntime(candidate);
    if (found !== null) return found;
  }
  throw new Error(CODEX_NOT_FOUND_MESSAGE);
}

/** Runner-facing name; kept explicit so discovery remains easy to identify at call sites. */
export const discoverCodexBinary = discoverCodex;

export interface CodexDescription {
  binary: string;
  version: string;
}

export function describeCodex(codexBinary: string | null = null): Promise<CodexDescription> {
  if (description === null) {
    description = (async () => {
      const binary = await discoverCodex(codexBinary);
      const stdout = await execFileText(binary, ['--version']);
      return { binary, version: stdout.trim() };
    })();
  }
  return description;
}

/** Test-only reset for the two deliberately process-wide discovery caches. */
export function resetCodexEnvironmentCacheForTests(): void {
  loginShellProbe = null;
  description = null;
}
