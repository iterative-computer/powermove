import { lstat, mkdir, mkdtemp, readFile, readlink, rename, symlink, unlink, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { randomUUID } from 'node:crypto';
import { parse, stringify } from 'smol-toml';

const CODEX_RESOURCE_KEYS = [
  'features', 'skills', 'plugins', 'marketplaces', 'apps', 'mcp_servers',
  'tools', 'web_search', 'hooks', 'agents', 'windows', 'developer_instructions',
  'model_instructions_file', 'project_doc_fallback_filenames', 'project_doc_max_bytes'
] as const;

async function exists(file: string): Promise<boolean> {
  try { await lstat(file); return true; }
  catch (error) { if ((error as NodeJS.ErrnoException).code === 'ENOENT') return false; throw error; }
}

function sameResourcePath(a: string, b: string): boolean {
  const normalize = (value: string) => process.platform === 'win32'
    ? path.resolve(value.replace(/^\\\\\?\\UNC\\/i, '\\\\').replace(/^\\\\\?\\/, '')).toLowerCase()
    : path.resolve(value);
  return normalize(a) === normalize(b);
}

async function linkResource(source: string, target: string): Promise<void> {
  if (!await exists(source)) {
    if (await exists(target) && (await lstat(target)).isSymbolicLink() && sameResourcePath(await readlink(target), source)) {
      await unlink(target);
    }
    return;
  }
  if (await exists(target)) {
    if (process.platform === 'win32' && (await lstat(source)).isFile() && (await lstat(target)).isFile()
      && await readFile(source, 'utf8') === await readFile(target, 'utf8')) return;
    if ((await lstat(target)).isSymbolicLink() && sameResourcePath(await readlink(target), source)) return;
    // Preserve resources created by an older private runtime before linking the
    // normal user installation. Never move or replace account/session files.
    const backupRoot = path.join(path.dirname(target), '.powermove-resource-backups');
    await mkdir(backupRoot, { recursive: true, mode: 0o700 });
    const backup = await mkdtemp(path.join(backupRoot, `${path.basename(target)}-`));
    await rename(target, path.join(backup, path.basename(target)));
  }
  const info = await lstat(source);
  if (process.platform === 'win32' && info.isFile()) await writeConfig(target, await readFile(source, 'utf8'));
  else await symlink(source, target, process.platform === 'win32' ? 'junction' : undefined);
}

async function readConfig(file: string, toml: boolean): Promise<Record<string, unknown>> {
  let source: string;
  try { source = await readFile(file, 'utf8'); }
  catch (error) { if ((error as NodeJS.ErrnoException).code === 'ENOENT') return {}; throw error; }
  try {
    const value = toml ? parse(source) : JSON.parse(source);
    if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error();
    return value;
  } catch { throw new Error(`Invalid agent resource configuration: ${file}`); }
}

/** Share skills, plugins, hooks and instructions, while keeping authentication,
 * session history and login files inside Powermove. */
async function syncUserResources(
  runtimeHome: string,
  sourceHome: string,
  provider: 'chatgpt' | 'claude'
): Promise<void> {
  if (path.resolve(runtimeHome) === path.resolve(sourceHome)) return;
  await mkdir(runtimeHome, { recursive: true, mode: 0o700 });
  const names = provider === 'chatgpt'
    ? ['skills', 'plugins', 'rules', 'hooks', 'AGENTS.md', 'AGENTS.override.md']
    : ['skills', 'plugins', 'commands', 'agents', 'rules', 'hooks', 'CLAUDE.md'];
  for (const name of names) await linkResource(path.join(sourceHome, name), path.join(runtimeHome, name));

  if (provider === 'claude') {
    // Copy settings rather than linking: provider settings updates must not
    // rewrite the user's terminal configuration. Credentials are separate.
    // They are shared whole, sandbox, permissions and env included: the
    // user's own configuration is theirs to widen.
    const settings = await readConfig(path.join(sourceHome, 'settings.json'), false);
    await writeConfig(path.join(runtimeHome, 'settings.json'), `${JSON.stringify(settings, null, 2)}\n`);
    return;
  }

  const source = await readConfig(path.join(sourceHome, 'config.toml'), true);
  const configFile = path.join(runtimeHome, 'config.toml');
  const runtime = await readConfig(configFile, true);
  for (const key of CODEX_RESOURCE_KEYS) {
    delete runtime[key];
    if (Object.hasOwn(source, key)) runtime[key] = source[key];
    if (key === 'mcp_servers' && runtime[key] && typeof runtime[key] === 'object') {
      runtime[key] = Object.fromEntries(Object.entries(runtime[key]).filter(([name]) => name !== 'powermove'));
    }
  }
  // The native engine, skills, app connectors and plugins are no longer
  // disabled by Powermove. Explicit user choices retain their native meaning.
  runtime.cli_auth_credentials_store = 'file';
  await writeConfig(configFile, stringify(runtime));
}

async function writeConfig(file: string, contents: string): Promise<void> {
  if (await readFile(file, 'utf8').catch(() => null) === contents) return;
  const temporary = `${file}.${randomUUID()}.tmp`;
  try {
    await writeFile(temporary, contents, { mode: 0o600 });
    await rename(temporary, file);
  } finally {
    await unlink(temporary).catch(error => {
      if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error;
    });
  }
}

const preparing = new Map<string, Promise<void>>();
export async function prepareUserResources(
  runtimeHome: string,
  sourceHome: string,
  provider: 'chatgpt' | 'claude'
): Promise<void> {
  const active = preparing.get(runtimeHome);
  if (active) return active;
  const work = syncUserResources(runtimeHome, sourceHome, provider);
  preparing.set(runtimeHome, work);
  try { await work; }
  finally { if (preparing.get(runtimeHome) === work) preparing.delete(runtimeHome); }
}
