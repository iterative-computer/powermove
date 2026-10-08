import { execFile } from 'node:child_process';
import path from 'node:path';
import { windowsScript } from './windows-system';

/** `started` is ps's lstart: with the pid, it names one process for its whole life. */
export type ProcessRow = { pid: number; ppid: number; pgid: number; uid: number; started: string };

export function parseProcessTable(stdout: string): ProcessRow[] {
  const rows: ProcessRow[] = [];
  for (const line of stdout.split('\n')) {
    const match = /^\s*(\d+)\s+(\d+)\s+(\d+)\s+(\d+)\s+(\S.*?)\s*$/u.exec(line);
    if (!match) continue;
    const [pid, ppid, pgid, uid] = match.slice(1, 5).map(Number) as [number, number, number, number];
    if (pid > 1) rows.push({ pid, ppid, pgid, uid, started: match[5]! });
  }
  return rows;
}

function processTable(): Promise<ProcessRow[]> {
  if (process.platform === 'win32') return windowsScript('ConvertTo-Json -Compress -InputObject @(Get-CimInstance Win32_Process | Where-Object { $_.ProcessId -gt 1 -and $_.CreationDate } | ForEach-Object { @{ pid = [int]$_.ProcessId; ppid = [int]$_.ParentProcessId; pgid = [int]$_.ProcessId; uid = 0; started = $_.CreationDate.ToUniversalTime().ToString("o") } })').then(source => {
    const rows: unknown = JSON.parse(source);
    if (!Array.isArray(rows)) throw new Error('Windows returned an invalid process table.');
    return rows.filter((row): row is ProcessRow => !!row && typeof row.pid === 'number' && typeof row.ppid === 'number' && typeof row.started === 'string');
  });
  return new Promise((resolve, reject) => {
    execFile('/bin/ps', ['-A', '-o', 'pid=,ppid=,pgid=,uid=,lstart='],
      { encoding: 'utf8', timeout: 5_000, maxBuffer: 16 * 1024 * 1024, env: { LC_ALL: 'C', ...(process.env.TZ ? { TZ: process.env.TZ } : {}) } },
      (error, stdout) => error ? reject(error) : resolve(parseProcessTable(stdout)));
  });
}

/** Working directories by pid; processes lsof cannot inspect are left out. */
function workingDirectories(pids: readonly number[]): Promise<Map<number, string>> {
  return new Promise(resolve => {
    // lsof exits 1 when any pid is gone; what it printed still holds.
    execFile('/usr/sbin/lsof', ['-a', '-d', 'cwd', '-Fn', '-w', '-p', pids.join(',')],
      { encoding: 'utf8', timeout: 5_000, maxBuffer: 4 * 1024 * 1024 }, (_error, stdout) => {
        const found = new Map<number, string>();
        let pid = 0;
        for (const line of (stdout ?? '').split('\n')) {
          if (line.startsWith('p')) pid = Number(line.slice(1));
          else if (line.startsWith('n') && pid) found.set(pid, line.slice(1));
        }
        resolve(found);
      });
  });
}

type NativeSandbox = typeof import('@powermove/macos-haptics');
let native: Promise<NativeSandbox | null> | undefined;
// The standalone Node host does not ship the desktop's native addon.
function nativeSandbox(): Promise<NativeSandbox | null> {
  return native ??= process.platform === 'darwin' ? import('@powermove/macos-haptics').then(module =>
    (module as NativeSandbox & { default?: NativeSandbox }).default ?? module, () => null) : Promise.resolve(null);
}

/**
 * Per pid, whether it runs in a sandbox that denies a lookup of one of
 * `marks` and allows its `.unmarked` twin, which other sandboxes deny too.
 * Null when that cannot be checked.
 */
async function sandboxMarked(pids: readonly number[], marks: readonly string[]): Promise<boolean[] | null> {
  const check = (await nativeSandbox())?.sandboxDeniesLookup;
  if (!check) return null;
  const found = pids.map(() => false);
  for (const mark of marks) {
    const denied = check(pids, mark), twin = check(pids, `${mark}.unmarked`);
    if (!denied || !twin) return null;
    denied.forEach((value, index) => { if (value && !twin[index]) found[index] = true; });
  }
  return found;
}

function signal(pid: number, name: NodeJS.Signals): boolean {
  try { process.kill(pid, name); return true; } catch { return false; }
}

const watched = new Set<ProcessFamily>();
let poll: NodeJS.Timeout | null = null, due = 0, polling = false;

/** One process table per tick serves every running command; fast while one is young. */
function schedulePoll(): void {
  if (polling || !watched.size) return;
  const young = [...watched].some(family => Date.now() - family.since < 5_000);
  const at = Date.now() + (young ? 100 : 500);
  if (poll && due <= at) return;
  if (poll) clearTimeout(poll);
  due = at;
  poll = setTimeout(() => {
    poll = null; polling = true;
    void processTable().then(rows => { for (const family of watched) family.record(rows); }, () => undefined)
      .finally(() => { polling = false; schedulePoll(); });
  }, at - Date.now());
  poll.unref();
}

export type FamilyOptions = { cwd: string; since: number; marks?: readonly string[] };

/**
 * Everything a command started, by pid and start time. A group kill misses
 * children that left the group, and the parent walk loses them once their
 * parent exits (launchd adopts them), so descendants are recorded while the
 * command runs. At kill time, processes that started since then and carry
 * one of the command's sandbox marks are claimed too: a process cannot leave
 * its sandbox, whatever its parent, group or folder. Without marks, adopted
 * processes in the command's folder that no other running command recorded
 * are claimed instead.
 */
export class ProcessFamily {
  private readonly members = new Map<number, string>();
  private readonly checked = new Set<string>();
  private killing: Promise<void> = Promise.resolve();

  /** `group` is the command's process group, or null to claim strays only. */
  constructor(readonly group: number | null, readonly options: FamilyOptions) {}

  get since(): number { return this.options.since; }

  watch(): void { watched.add(this); schedulePoll(); }

  /** Add group members and children of recorded members; forget pids now naming another process. */
  record(rows: readonly ProcessRow[]): void {
    const current = new Map(rows.map(row => [row.pid, row.started]));
    for (const [pid, started] of this.members) if (current.get(pid) !== started) this.members.delete(pid);
    // A group id outlives its leader only while members remain; later reuse starts later.
    const floor = this.options.since - 1_000;
    for (const row of rows) {
      if (row.pid !== process.pid && row.pgid === this.group && (process.platform === 'win32' || !(Date.parse(row.started) < floor))) this.members.set(row.pid, row.started);
    }
    for (let grew = true; grew;) {
      grew = false;
      for (const row of rows) {
        if (this.members.has(row.pid) || row.pid === process.pid || !this.members.has(row.ppid)) continue;
        this.members.set(row.pid, row.started); grew = true;
      }
    }
  }

  private async claimStrays(rows: readonly ProcessRow[]): Promise<void> {
    const uid = process.getuid?.();
    const floor = this.options.since - 1_000;
    const candidates = rows.filter(row => row.uid === uid && row.pid !== process.pid
      && !this.members.has(row.pid) && !this.checked.has(`${row.pid}:${row.started}`) && Date.parse(row.started) >= floor);
    if (!candidates.length) return;
    const marked = this.options.marks?.length ? await sandboxMarked(candidates.map(row => row.pid), this.options.marks) : null;
    if (marked) {
      candidates.forEach((row, index) => {
        this.checked.add(`${row.pid}:${row.started}`);
        if (marked[index]) this.members.set(row.pid, row.started);
      });
      return;
    }
    const others = [...watched].filter(family => family !== this);
    const adopted = candidates.filter(row => row.ppid === 1
      && !others.some(family => family.members.get(row.pid) === row.started));
    if (!adopted.length) return;
    const cwd = await workingDirectories(adopted.map(row => row.pid));
    const inside = (dir: string) => dir === this.options.cwd || dir.startsWith(this.options.cwd + path.sep);
    for (const row of adopted) {
      this.checked.add(`${row.pid}:${row.started}`);
      const dir = cwd.get(row.pid);
      if (dir !== undefined && inside(dir)) this.members.set(row.pid, row.started);
    }
  }

  /**
   * SIGKILL every member still alive. Members are frozen until no new one
   * appears (a stopped process cannot fork), then killed.
   */
  kill(): Promise<void> {
    watched.delete(this);
    this.killing = this.killing.then(() => this.killOnce(), () => this.killOnce());
    return this.killing;
  }

  private async killOnce(): Promise<void> {
    if (process.platform === 'win32') {
      // Windows has no POSIX groups or SIGSTOP. Kill verified process trees,
      // including descendants recorded before their parent exited.
      for (let round = 0; round < 3; round++) {
        const rows = await processTable();
        this.record(rows);
        const alive = rows.filter(row => this.members.get(row.pid) === row.started);
        if (!alive.length) break;
        await Promise.all(alive.map(row => new Promise<void>(resolve => {
          execFile('taskkill.exe', ['/PID', String(row.pid), '/T', '/F'], { windowsHide: true, timeout: 5_000 }, () => resolve());
        })));
      }
      this.members.clear();
      return;
    }
    const frozen = new Set<number>();
    // The whole group stops at once, but only while it is still this command's.
    let group = false;
    try {
      for (let round = 0; round < 16; round++) {
        const rows = await processTable();
        this.record(rows);
        await this.claimStrays(rows);
        group = rows.some(row => row.pgid === this.group && this.members.get(row.pid) === row.started);
        if (group) signal(-this.group!, 'SIGSTOP');
        let fresh = false;
        for (const pid of this.members.keys()) {
          if (frozen.has(pid)) continue;
          frozen.add(pid); fresh = true;
          signal(pid, 'SIGSTOP');
        }
        if (!fresh) break;
      }
    } catch { /* Kill what is known when ps fails. */ }
    finally {
      if (group) signal(-this.group!, 'SIGKILL');
      for (const pid of new Set([...frozen, ...this.members.keys()])) signal(pid, 'SIGKILL');
      this.members.clear();
    }
  }
}

/** SIGKILL a command's process group and every descendant recorded or found now. */
export function killProcessFamily(group: number, options: FamilyOptions): Promise<void> {
  return new ProcessFamily(group, options).kill();
}

/** SIGKILL whatever escaped every command since `since`: marked processes, or adopted ones in `cwd`, with their descendants. */
export function killStrays(options: FamilyOptions): Promise<void> {
  return new ProcessFamily(null, options).kill();
}
