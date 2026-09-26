/*
 * Extension values on disk: `<userData>/env/<envKey>.env`, one file per
 * extension (store plan §2.2b). The directory is 0700 and every file 0600.
 *
 * Format, one entry per line:
 *   # comments and blank lines are kept
 *   KEY=value
 *   KEY="json string"        values with newlines, `#`, surrounding spaces or a
 *                            leading quote are JSON-string encoded
 *   KEY=enc:<base64>         a secret, sealed with Electron's safeStorage
 *
 * A quoted value is never treated as sealed, so a plain value that happens to
 * start with `enc:` is written quoted. A secret key found in plaintext (hand
 * edited) is sealed on the next load.
 *
 * safeStorage is injected: it only works after `app.whenReady()`, and tests
 * pass a fake.
 */
import { chmod, lstat, mkdir, readFile, rename, rm, stat, writeFile } from 'node:fs/promises';
import { randomUUID } from 'node:crypto';
import path from 'node:path';

export interface SafeStorageLike {
  isEncryptionAvailable(): boolean;
  encryptString(plainText: string): Buffer;
  decryptString(encrypted: Buffer): string;
}

export interface EnvEntry {
  /** null when the value is sealed and this Mac can no longer open it. */
  value: string | null;
  secret: boolean;
}

export type EnvMap = Map<string, EnvEntry>;

export interface EnvStore {
  readonly dir: string;
  fileFor(envKey: string): string;
  exists(envKey: string): Promise<boolean>;
  /** `secretKeys` are the keys the manifest declares secret; plaintext ones are sealed on load. */
  readEnv(envKey: string, secretKeys?: ReadonlySet<string>): Promise<EnvMap>;
  writeVar(envKey: string, key: string, value: string, secret: boolean): Promise<void>;
  deleteVar(envKey: string, key: string): Promise<void>;
  deleteEnv(envKey: string): Promise<void>;
}

export class SecretStorageUnavailableError extends Error {
  constructor() {
    super('Secret values can’t be saved because secure storage is unavailable. Unlock your Mac and try again.');
    this.name = 'SecretStorageUnavailableError';
  }
}

const ENV_KEY = /^[A-Za-z_][A-Za-z0-9_]*$/;
const SEALED = 'enc:';

type Line =
  | { kind: 'text'; text: string }
  | { kind: 'var'; key: string; stored: string };

/** `local:<uuid>` → `local_<uuid>`; anything else that could escape the directory is refused. */
export function envFileName(envKey: string): string {
  if (typeof envKey !== 'string' || envKey.length === 0 || envKey.length > 200) throw new Error('Invalid env key.');
  const name = envKey.replace(/[:/]/g, '_');
  if (!/^[A-Za-z0-9_-][A-Za-z0-9._-]*$/.test(name)) throw new Error('Invalid env key.');
  return `${name}.env`;
}

/** Split a file into lines, keeping comments and unknown text in place. */
export function parseEnvText(text: string): Line[] {
  const lines: Line[] = [];
  for (const raw of text.split(/\r?\n/)) {
    const trimmed = raw.trim();
    const equals = raw.indexOf('=');
    if (trimmed === '' || trimmed.startsWith('#') || equals <= 0) {
      lines.push({ kind: 'text', text: raw });
      continue;
    }
    const key = raw.slice(0, equals).trim();
    if (!ENV_KEY.test(key)) {
      lines.push({ kind: 'text', text: raw });
      continue;
    }
    lines.push({ kind: 'var', key, stored: raw.slice(equals + 1) });
  }
  while (lines.length && lines.at(-1)?.kind === 'text' && (lines.at(-1) as { text: string }).text === '') lines.pop();
  return lines;
}

export function serializeEnvLines(lines: readonly Line[]): string {
  return `${lines.map((line) => (line.kind === 'text' ? line.text : `${line.key}=${line.stored}`)).join('\n')}\n`;
}

/** What the right-hand side of a line means. */
export function decodeStored(stored: string): { sealed: true; payload: string } | { sealed: false; value: string } {
  const value = stored.trim();
  if (value.startsWith('"')) {
    try {
      const parsed: unknown = JSON.parse(value);
      if (typeof parsed === 'string') return { sealed: false, value: parsed };
    } catch {
      // Fall through: an unterminated quote is read as the raw text.
    }
    return { sealed: false, value };
  }
  if (value.startsWith(SEALED)) return { sealed: true, payload: value.slice(SEALED.length) };
  // Unquoted values end at an inline ` # comment`, as in dotenv.
  const comment = value.search(/\s#/);
  return { sealed: false, value: comment >= 0 ? value.slice(0, comment).trimEnd() : value };
}

export function encodePlain(value: string): string {
  const needsQuotes = /[\r\n#"]/.test(value) || value !== value.trim() || value.startsWith(SEALED);
  return needsQuotes ? JSON.stringify(value) : value;
}

export function createEnvStore(options: {
  dir: string;
  safeStorage: SafeStorageLike | (() => SafeStorageLike);
}): EnvStore {
  const { dir } = options;
  const storage = (): SafeStorageLike =>
    typeof options.safeStorage === 'function' ? options.safeStorage() : options.safeStorage;
  const queues = new Map<string, Promise<unknown>>();
  const serial = <T>(envKey: string, task: () => Promise<T>): Promise<T> => {
    const previous = queues.get(envKey) ?? Promise.resolve();
    const run = previous.then(task);
    const settled = run.catch(() => undefined);
    queues.set(envKey, settled);
    void settled.then(() => { if (queues.get(envKey) === settled) queues.delete(envKey); });
    return run;
  };

  const fileFor = (envKey: string): string => path.join(dir, envFileName(envKey));

  async function ensureDir(): Promise<void> {
    await mkdir(dir, { recursive: true, mode: 0o700 });
    await chmod(dir, 0o700);
  }

  async function load(envKey: string): Promise<Line[]> {
    const file = fileFor(envKey);
    try {
      // Files written by hand or copied from another profile may be too open;
      // tighten them before reading so the 0700/0600 guarantee holds on load.
      const info = await lstat(file);
      if (!info.isFile()) throw Object.assign(new Error(`${file} is not a regular file`), { code: 'ENOTREG' });
      await chmod(dir, 0o700).catch(() => undefined);
      if ((info.mode & 0o077) !== 0) await chmod(file, 0o600);
      return parseEnvText(await readFile(file, 'utf8'));
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === 'ENOENT') return [];
      throw error;
    }
  }

  async function save(envKey: string, lines: readonly Line[]): Promise<void> {
    await ensureDir();
    const file = fileFor(envKey);
    const temporary = path.join(dir, `.${path.basename(file)}.${randomUUID()}.tmp`);
    try {
      await writeFile(temporary, serializeEnvLines(lines), { mode: 0o600 });
      await chmod(temporary, 0o600);
      await rename(temporary, file);
    } catch (error) {
      await rm(temporary, { force: true }).catch(() => undefined);
      throw error;
    }
  }

  function seal(value: string): string {
    const safe = storage();
    if (!safe.isEncryptionAvailable()) throw new SecretStorageUnavailableError();
    return `${SEALED}${safe.encryptString(value).toString('base64')}`;
  }

  function open(payload: string): string | null {
    try {
      const safe = storage();
      if (!safe.isEncryptionAvailable() || payload.length === 0) return null;
      return safe.decryptString(Buffer.from(payload, 'base64'));
    } catch {
      return null;
    }
  }

  return {
    dir,
    fileFor,

    async exists(envKey) {
      try {
        return (await stat(fileFor(envKey))).isFile();
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code === 'ENOENT') return false;
        throw error;
      }
    },

    readEnv: (envKey, secretKeys = new Set()) => serial(envKey, async () => {
      const lines = await load(envKey);
      const result: EnvMap = new Map();
      let reseal = false;
      for (const line of lines) {
        if (line.kind !== 'var') continue;
        const decoded = decodeStored(line.stored);
        if (decoded.sealed) {
          result.set(line.key, { value: open(decoded.payload), secret: true });
          continue;
        }
        const secret = secretKeys.has(line.key);
        if (secret && decoded.value.length > 0) {
          // A declared secret sitting in plaintext is sealed now, or withheld
          // until it can be: it is never delivered unsealed.
          if (!storage().isEncryptionAvailable()) { result.set(line.key, { value: null, secret }); continue; }
          line.stored = seal(decoded.value);
          reseal = true;
        }
        result.set(line.key, { value: decoded.value, secret });
      }
      if (reseal) await save(envKey, lines);
      return result;
    }),

    writeVar: (envKey, key, value, secret) => serial(envKey, async () => {
      if (!ENV_KEY.test(key)) throw new Error('Invalid key.');
      if (typeof value !== 'string' || value.includes('\0')) throw new Error('Invalid value.');
      const stored = secret ? seal(value) : encodePlain(value);
      const lines = await load(envKey);
      const existing = lines.filter((line): line is Extract<Line, { kind: 'var' }> => line.kind === 'var' && line.key === key);
      if (existing.length) {
        existing[0]!.stored = stored;
        // Later duplicates would shadow the new value; drop them.
        for (const duplicate of existing.slice(1)) lines.splice(lines.indexOf(duplicate), 1);
      } else {
        lines.push({ kind: 'var', key, stored });
      }
      await save(envKey, lines);
    }),

    deleteVar: (envKey, key) => serial(envKey, async () => {
      const lines = await load(envKey);
      const kept = lines.filter((line) => line.kind !== 'var' || line.key !== key);
      if (kept.length === lines.length) return;
      await save(envKey, kept);
    }),

    deleteEnv: (envKey) => serial(envKey, async () => {
      await rm(fileFor(envKey), { force: true });
    })
  };
}
