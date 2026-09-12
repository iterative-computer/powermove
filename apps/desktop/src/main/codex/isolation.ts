import { chmod, copyFile, lstat, mkdir, readFile, readdir, realpath, stat, unlink, writeFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import path from 'node:path';

export const ISOLATED_CODEX_HOME_NAME = 'codex-runtime';
export const POWERMOVE_AUTH_OWNER_FILE = '.powermove-auth-owned';
export const POWERMOVE_AUTH_STORE_CONFIG = 'cli_auth_credentials_store = "file"';
const MAX_USER_SKILL_FILES = 2_000;
const MAX_USER_SKILL_DEPTH = 6;
const preparingHomes = new Map<string, Promise<string>>();

/** The real Codex home is used only to bootstrap Powermove's private login once. */
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

async function exists(file: string): Promise<boolean> {
  try {
    await lstat(file);
    return true;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') return false;
    throw error;
  }
}

async function ensurePrivateCredentialStore(runtimeHome: string): Promise<void> {
  const configFile = path.join(runtimeHome, 'config.toml');
  let current = '';
  try {
    current = await readFile(configFile, 'utf8');
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error;
  }
  const setting = /^[ \t]*cli_auth_credentials_store[ \t]*=.*$/mu;
  const next = setting.test(current)
    ? current.replace(setting, POWERMOVE_AUTH_STORE_CONFIG)
    : `${POWERMOVE_AUTH_STORE_CONFIG}\n${current}`;
  if (next !== current) await writeFile(configFile, next, { mode: 0o600 });
  await chmod(configFile, 0o600);
}

async function prepareHome(runtimeHome: string, sourceHome: string): Promise<string> {
  await mkdir(runtimeHome, { recursive: true, mode: 0o700 });
  await chmod(runtimeHome, 0o700);
  // `auto` can find the user's shared macOS Keychain entry. File-only storage
  // makes account/read and account/logout act solely on Powermove's auth.json.
  await ensurePrivateCredentialStore(runtimeHome);

  const authSource = path.join(sourceHome, 'auth.json');
  const authFile = path.join(runtimeHome, 'auth.json');
  const ownerFile = path.join(runtimeHome, POWERMOVE_AUTH_OWNER_FILE);
  const wasInitialized = await exists(ownerFile);

  try {
    const metadata = await lstat(authFile);
    if (metadata.isSymbolicLink()) {
      // Migrate older Powermove builds away from a shared credential symlink.
      // The copy makes later logout/reconnect operations local to Powermove.
      await unlink(authFile);
      if (await exists(authSource)) {
        await copyFile(authSource, authFile);
        await chmod(authFile, 0o600);
      }
    } else if (metadata.isFile()) {
      await chmod(authFile, 0o600);
    } else {
      throw new Error('Powermove Codex authentication path is not a file.');
    }
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error;
    // Import an existing Codex login only on the first run. Once the owner
    // marker exists, a missing auth file means the user disconnected Powermove.
    if (!wasInitialized && await exists(authSource)) {
      await copyFile(authSource, authFile);
      await chmod(authFile, 0o600);
    }
  }

  if (!wasInitialized) {
    await writeFile(ownerFile, 'powermove\n', { mode: 0o600, flag: 'wx' }).catch((error) => {
      if ((error as NodeJS.ErrnoException).code !== 'EEXIST') throw error;
      // Another process completed initialization first.
    });
  }
  return runtimeHome;
}

/**
 * Build an app-owned Codex home with no user config, hooks, or MCP state.
 * Authentication is copied on first use, then owned and refreshed by Codex
 * inside Powermove's private app data so disconnect never signs other apps out.
 */
export async function prepareIsolatedCodexHome(
  userData: string,
  sourceHome = userCodexHome()
): Promise<string> {
  const runtimeHome = isolatedCodexHome(userData, sourceHome);
  const active = preparingHomes.get(runtimeHome);
  if (active) return active;
  const preparation = prepareHome(runtimeHome, sourceHome).finally(() => {
    if (preparingHomes.get(runtimeHome) === preparation) preparingHomes.delete(runtimeHome);
  });
  preparingHomes.set(runtimeHome, preparation);
  return preparation;
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
