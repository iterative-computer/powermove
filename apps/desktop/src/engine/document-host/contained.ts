/*
 * A contained process for code the document host does not trust: Store
 * extensions and generated scripts (docs/engine/document-host.md, "Contained
 * runtimes"). In the editor they run in an opaque-origin iframe (and a
 * worker) under a CSP; headlessly they run in a Node process of their own,
 * held three ways:
 *
 * 1. The operating system's sandbox (macOS seatbelt through sandbox-exec):
 *    deny by default; read only the runtime bundle, the code it runs, Node
 *    itself and the system libraries; write nothing; no network unless the
 *    manifest asks for it (then outbound TCP only); exec nothing but Node.
 * 2. Node's permission model (`--permission`): the same reads, no writes, no
 *    child processes, worker threads, native addons or WASI.
 * 3. Inside the runtime (sandbox-runtime.ts): Node's built-in modules cannot
 *    be imported by the code it runs, `process.binding`,
 *    `process.getBuiltinModule` and `process.dlopen` are gone, and `fetch` and
 *    `WebSocket` follow the CSP's `connect-src` (data: and blob:, plus https:
 *    and wss: with the network permission).
 *
 * The environment is scrubbed: nothing of the host's (tokens, paths) reaches
 * the process. Without an OS sandbox (not macOS, or sandbox-exec missing)
 * layers 2 and 3 hold; `containment()` says which, and the worker logs it.
 */
import { spawn, type ChildProcess } from 'node:child_process';
import { existsSync, realpathSync, statSync } from 'node:fs';
import path from 'node:path';

const SANDBOX_EXEC = '/usr/bin/sandbox-exec';

export interface ContainedOptions {
  /** The runtime entry (sandbox-runtime.mjs) and its arguments. */
  script: string;
  args: string[];
  /** Directories (recursive) and files the process may read, besides Node and the system. */
  read: string[];
  /** The manifest's `network` permission: outbound connections allowed. */
  network: boolean;
}

export type Containment = 'seatbelt+permission' | 'permission';

export function containment(): Containment {
  return process.platform === 'darwin' && existsSync(SANDBOX_EXEC) && process.env['POWERMOVE_CONTAINED_NO_SEATBELT'] !== '1' ? 'seatbelt+permission' : 'permission';
}

const real = (p: string): string => { try { return realpathSync(p); } catch { return path.resolve(p); } };
/** SBPL string literal. */
const lit = (s: string): string => JSON.stringify(s);

function ancestors(p: string): string[] {
  const out: string[] = [];
  for (let dir = path.dirname(p); ; dir = path.dirname(dir)) {
    out.push(dir);
    if (dir === path.dirname(dir)) break;
  }
  return out;
}

/** The seatbelt profile for one contained process. */
export function seatbeltProfile(node: string, read: Array<{ path: string; dir: boolean }>, network: boolean): string {
  const reads = read.map((r) => `(${r.dir ? 'subpath' : 'literal'} ${lit(r.path)})`).join(' ');
  const meta = [...new Set([node, ...read.map((r) => r.path)].flatMap(ancestors))].map((p) => `(literal ${lit(p)})`).join(' ');
  return [
    '(version 1)',
    '(deny default)',
    `(allow process-exec (literal ${lit(node)}))`,
    `(allow file-read* (literal ${lit(node)}) (literal "/") (subpath "/usr/lib") (subpath "/System") (subpath "/private/var/db/dyld") (literal "/dev/null") (literal "/dev/urandom") (literal "/dev/random") ${reads})`,
    `(allow file-read-metadata ${meta})`,
    '(allow file-ioctl (literal "/dev/null"))',
    '(allow file-write-data (literal "/dev/null"))',
    '(allow sysctl-read)',
    ...(network ? ['(allow network-outbound (remote tcp))', '(allow system-socket)', '(allow mach-lookup (global-name "com.apple.dnssd.service"))'] : []),
  ].join('\n');
}

/** Starts a contained process with an IPC channel (structured clone). */
export function spawnContained(options: ContainedOptions): ChildProcess {
  const node = real(process.execPath);
  // Each path as given and as its real path: a linked package is looked up by the link.
  const paths = [...new Set([options.script, ...options.read].flatMap((p) => [path.resolve(p), real(p)]))];
  const read = paths.map((p) => {
    let dir = false;
    try { dir = statSync(p).isDirectory(); } catch { dir = false; }
    return { path: p, dir };
  });
  const nodeArgs = [
    '--permission',
    ...read.map((r) => `--allow-fs-read=${r.dir ? `${r.path}${path.sep}*` : r.path}`),
    real(options.script),
    ...options.args,
  ];
  // Nothing of the host's environment: no tokens, no paths, no NODE_OPTIONS.
  const env: Record<string, string> = { PATH: '/usr/bin:/bin', LANG: 'en_US.UTF-8', TZ: process.env['TZ'] ?? 'UTC' };
  const [command, args] = containment() === 'seatbelt+permission'
    ? [SANDBOX_EXEC, ['-p', seatbeltProfile(node, read, options.network), node, ...nodeArgs]]
    : [node, nodeArgs];
  return spawn(command, args, {
    cwd: path.dirname(real(options.script)),
    env,
    stdio: ['ignore', 'ignore', 'pipe', 'ipc'],
    serialization: 'advanced',
  });
}
