import { spawn } from 'node:child_process';
import os from 'node:os';
import path from 'node:path';

const SYSTEM_PATH = ['/usr/bin', '/bin', '/usr/sbin', '/sbin'];
const START = '__POWERMOVE_PATH_START__';
const END = '__POWERMOVE_PATH_END__';
const PROBE_TIMEOUT_MS = 5_000;
const RETRY_AFTER_MS = 60_000;

let probe: Promise<string[]> | null = null;
let retryAt = 0;

/** Absolute, unique PATH entries in order; relative entries resolve against a caller's cwd. */
export function absolutePathEntries(value: string | undefined, platform: NodeJS.Platform = process.platform): string[] {
  const paths = platform === 'win32' ? path.win32 : path.posix;
  const entries: string[] = [];
  for (const raw of (value ?? '').split(paths.delimiter)) {
    const entry = raw.replace(/^"(.*)"$/, '$1');
    if (paths.isAbsolute(entry) && !/[\0\r\n]/u.test(entry) && !entries.some(value => platform === 'win32' ? value.toLowerCase() === entry.toLowerCase() : value === entry)) entries.push(entry);
  }
  return entries;
}

/** The PATH printed between markers, ignoring anything a login profile echoes. */
export function markedPath(stdout: string): string | null {
  const end = stdout.lastIndexOf(END);
  const start = end < 0 ? -1 : stdout.lastIndexOf(START, end);
  return start < 0 ? null : stdout.slice(start + START.length, end);
}

/**
 * How to ask a login shell for its PATH. Directory services name the login
 * shell ($SHELL is only inherited); fish joins its PATH list itself, and a
 * shell whose syntax is unknown falls back to zsh, the macOS default.
 */
export function loginShellCommand(shell: string | undefined): [string, string[]] {
  const posix = `printf '${START}%s${END}' "$PATH"`;
  const name = shell && path.isAbsolute(shell) ? path.basename(shell) : '';
  if (name === 'fish') return [shell!, ['-l', '-i', '-c', `printf '${START}%s${END}' (string join : $PATH)`]];
  if (['zsh', 'bash', 'sh', 'ksh', 'dash'].includes(name)) return [shell!, ['-ilc', posix]];
  return ['/bin/zsh', ['-ilc', posix]];
}

function userShell(): string | undefined {
  try { return os.userInfo().shell || process.env.SHELL; } catch { return process.env.SHELL; }
}

function readLoginShellPath(): Promise<string[] | null> {
  return new Promise(resolve => {
    const [file, args] = loginShellCommand(userShell());
    let stdout = '', settled = false;
    const finish = (entries: string[] | null) => {
      if (settled) return;
      settled = true; clearTimeout(timer); child.stdout?.destroy();
      resolve(entries);
    };
    // Only PATH crosses over; the login environment can hold tokens. No
    // stdin, so a profile that reads input cannot hold the probe.
    const child = spawn(file, args, { stdio: ['ignore', 'pipe', 'ignore'] });
    const timer = setTimeout(() => { child.kill('SIGKILL'); finish(null); }, PROBE_TIMEOUT_MS);
    child.stdout!.setEncoding('utf8').on('data', (chunk: string) => {
      stdout += chunk;
      if (stdout.length > 1024 * 1024) { child.kill('SIGKILL'); finish(null); return; }
      // A profile may leave a daemon holding stdout open; the marker is enough.
      const marked = markedPath(stdout);
      if (marked !== null) finish(absolutePathEntries(marked));
    });
    child.on('error', () => finish(null));
    child.on('close', () => finish(null));
  });
}

/**
 * The user's login-shell PATH, so Dock and Finder launches (which get a
 * minimal PATH) still find Homebrew, bun and node tools. Homebrew and bun
 * folders are appended when the profile left them out. A successful probe is
 * kept for the app's lifetime; after a failed one, commands use the fallback
 * folders for a minute, so a slow profile does not delay each of them.
 */
export async function loginShellPath(home = process.env.HOME): Promise<string> {
  if (process.platform === 'win32') {
    const user = home ?? os.homedir();
    const inherited = process.env.PATH ?? process.env.Path;
    return absolutePathEntries([inherited, path.join(user, '.bun', 'bin'), path.join(user, '.local', 'bin'), process.env.APPDATA ? path.join(process.env.APPDATA, 'npm') : ''].join(';')).join(';');
  }
  let login: string[] = [];
  if (process.platform === 'darwin' && (probe || Date.now() >= retryAt)) {
    const current = probe ??= readLoginShellPath().then(entries => {
      if (!entries?.length) throw new Error('The login shell reported no PATH.');
      return entries;
    });
    login = await current.catch(() => {
      if (probe === current) { probe = null; retryAt = Date.now() + RETRY_AFTER_MS; }
      return [];
    });
  }
  const fallback = ['/opt/homebrew/bin', '/usr/local/bin', ...(home && path.isAbsolute(home) ? [path.join(home, '.bun', 'bin')] : [])];
  return absolutePathEntries([...login, ...fallback, ...absolutePathEntries(process.env.PATH), ...SYSTEM_PATH].join(':')).join(':');
}

export function resetLoginShellPathForTests(): void {
  probe = null;
  retryAt = 0;
}
