import { execFile } from 'node:child_process';
import { constants } from 'node:fs';
import { access, stat } from 'node:fs/promises';
import path from 'node:path';

export const PACKAGED_CLAUDE_RELATIVE_PATH = path.join('claude', 'bin', 'claude');
export const DEVELOPMENT_CLAUDE_RELATIVE_PATH = path.join(
  'node_modules',
  '@anthropic-ai',
  'claude-code-darwin-arm64',
  'claude'
);

export const CLAUDE_NOT_FOUND_MESSAGE =
  "Powermove's built-in Claude runtime is missing or unavailable. Reinstall Powermove or set CLAUDE_BINARY.";

let loginShellProbe: Promise<string | null> | null = null;

async function executable(candidate: string | null | undefined): Promise<string | null> {
  const value = candidate?.trim();
  if (!value) return null;
  try {
    if (!(await stat(value)).isFile()) return null;
    await access(value, constants.X_OK);
    return value;
  } catch {
    return null;
  }
}

function execFileText(file: string, args: readonly string[]): Promise<string> {
  return new Promise((resolve, reject) => {
    execFile(file, [...args], { encoding: 'utf8', timeout: 5_000, maxBuffer: 1024 * 1024 }, (error, stdout) => {
      if (error) reject(error);
      else resolve(stdout);
    });
  });
}

async function probeLoginShell(): Promise<string | null> {
  if (loginShellProbe === null) {
    loginShellProbe = (async () => {
      try {
        const stdout = await execFileText('/bin/zsh', ['-ilc', 'command -v claude']);
        return executable(stdout.split(/\r?\n/u).map((line) => line.trim()).filter(Boolean).at(-1));
      } catch {
        return null;
      }
    })();
  }
  return loginShellProbe;
}

export function bundledClaudeCandidates(
  appRoot = process.cwd(),
  resourcesPath = (process as NodeJS.Process & { resourcesPath?: string }).resourcesPath
): string[] {
  const candidates = [path.join(appRoot, DEVELOPMENT_CLAUDE_RELATIVE_PATH)];
  if (resourcesPath) candidates.unshift(path.join(resourcesPath, PACKAGED_CLAUDE_RELATIVE_PATH));
  return candidates;
}

export async function discoverClaudeBinary(preference: string | null = null): Promise<string> {
  for (const candidate of [
    process.env.CLAUDE_BINARY,
    preference,
    ...bundledClaudeCandidates(),
    await probeLoginShell()
  ]) {
    const found = await executable(candidate);
    if (found) return found;
  }
  throw new Error(CLAUDE_NOT_FOUND_MESSAGE);
}

export function resetClaudeEnvironmentCacheForTests(): void {
  loginShellProbe = null;
}
