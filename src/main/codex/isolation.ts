import { chmod, lstat, mkdir, readdir, readlink, realpath, stat, symlink, unlink } from 'node:fs/promises';
import { homedir } from 'node:os';
import path from 'node:path';

export const ISOLATED_CODEX_HOME_NAME = 'codex-runtime';
const MAX_USER_SKILL_FILES = 2_000;
const MAX_USER_SKILL_DEPTH = 6;

/** The real Codex home is used only as the source of ChatGPT authentication. */
export function userCodexHome(environment: NodeJS.ProcessEnv = process.env): string {
  const configured = environment.CODEX_HOME?.trim();
  return configured ? path.resolve(configured) : path.join(homedir(), '.codex');
}

export function isolatedCodexHome(userData: string, sourceHome = userCodexHome()): string {
  const preferred = path.join(userData, ISOLATED_CODEX_HOME_NAME);
  return path.resolve(preferred) === path.resolve(sourceHome)
    ? path.join(userData, `${ISOLATED_CODEX_HOME_NAME}-isolated`)
    : preferred;
}

function resolvedLinkTarget(linkPath: string, target: string): string {
  return path.resolve(path.dirname(linkPath), target);
}

/**
 * Build an app-owned Codex home with no config, plugins, skills, hooks, or MCP
 * state. The auth file remains at its normal location and is exposed through a
 * symlink so Codex token refreshes continue to work in both applications.
 */
export async function prepareIsolatedCodexHome(
  userData: string,
  sourceHome = userCodexHome()
): Promise<string> {
  const runtimeHome = isolatedCodexHome(userData, sourceHome);
  await mkdir(runtimeHome, { recursive: true, mode: 0o700 });
  await chmod(runtimeHome, 0o700);

  const authSource = path.join(sourceHome, 'auth.json');
  const authLink = path.join(runtimeHome, 'auth.json');
  try {
    const metadata = await lstat(authLink);
    if (metadata.isSymbolicLink()) {
      const target = await readlink(authLink);
      if (resolvedLinkTarget(authLink, target) === path.resolve(authSource)) return runtimeHome;
    }
    await unlink(authLink);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error;
  }

  // A dangling link is intentional when Codex is not signed in yet: signing in
  // through the normal CLI makes the next Powermove run authenticated.
  await symlink(authSource, authLink);
  return runtimeHome;
}

export function isolatedCodexEnvironment(runtimeHome: string): NodeJS.ProcessEnv {
  return { ...process.env, CODEX_HOME: runtimeHome };
}

/**
 * Find every user-installed skill Codex can discover from $HOME/.agents/skills.
 * Symlinked skill directories are followed without allowing directory loops.
 */
export async function discoverUserSkillFiles(userHome = homedir()): Promise<string[]> {
  const root = path.join(userHome, '.agents', 'skills');
  const files: string[] = [];
  const visitedDirectories = new Set<string>();

  const visit = async (directory: string, depth: number): Promise<void> => {
    if (depth > MAX_USER_SKILL_DEPTH || files.length >= MAX_USER_SKILL_FILES) return;
    let canonicalDirectory: string;
    try {
      canonicalDirectory = await realpath(directory);
    } catch {
      return;
    }
    if (visitedDirectories.has(canonicalDirectory)) return;
    visitedDirectories.add(canonicalDirectory);

    let names: string[];
    try {
      names = await readdir(directory);
    } catch {
      return;
    }
    names.sort();
    for (const name of names) {
      if (files.length >= MAX_USER_SKILL_FILES) break;
      const candidate = path.join(directory, name);
      let metadata;
      try {
        metadata = await stat(candidate);
      } catch {
        continue;
      }
      if (metadata.isFile() && name === 'SKILL.md') files.push(candidate);
      else if (metadata.isDirectory()) await visit(candidate, depth + 1);
    }
  };

  await visit(root, 0);
  return files;
}
