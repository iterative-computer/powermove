import { afterEach, expect, it } from 'vitest';
import { execFile, execFileSync } from 'node:child_process';
import net from 'node:net';
import { mkdir, mkdtemp, readFile, readdir, realpath, rm, stat, symlink, access, writeFile } from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import { COMPATIBLE_WORKSPACE_TOOLS, CompatibleWorkspace, runWorkspaceCommand } from './compatible-workspace';
import { prepareAgentWorkspace } from './codex/workspace';
import { agentResultSchema } from './codex/instructions';

const directories: string[] = [];
afterEach(async () => { await Promise.all(directories.splice(0).map(dir => rm(dir, { recursive: true, force: true }))); });
const signal = () => new AbortController().signal;
const escapeRegExp = (value: string) => value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
async function workspace() {
  const directory = await mkdtemp(path.join(os.tmpdir(), 'pm-api-workspace-')); directories.push(directory);
  const layout = await prepareAgentWorkspace({ projectId: 'proof', provider: 'compatible', projectJSON: '{}', images: [], attachments: [] } as any, directory, 'project', agentResultSchema(), { extensionsDir: path.join(directory, 'extensions'), apiPackFiles: [] });
  return new CompatibleWorkspace(layout, 'project');
}

it('creates, reads and lists files and rejects path and symbolic-link escapes', async () => {
  const ws = await workspace();
  await ws.call('write_file', { path: 'nested/file.txt', text: 'hello' }, signal());
  expect(await readFile(path.join(ws.layout.root, 'nested/file.txt'), 'utf8')).toBe('hello');
  expect(JSON.stringify(await ws.call('read_file', { path: 'nested/file.txt', offset: 1, limit: 2 }, signal()))).toContain('el');
  expect(JSON.stringify(await ws.call('list_files', { path: 'nested' }, signal()))).toContain('file.txt');
  await expect(ws.call('write_file', { path: '../escaped.txt', text: 'no' }, signal())).rejects.toThrow('outside');
  await symlink(path.dirname(ws.layout.root), path.join(ws.layout.root, 'escape'));
  await expect(ws.call('write_file', { path: 'escape/escaped.txt', text: 'no' }, signal())).rejects.toThrow('symbolic link');
});

it.runIf(process.platform === 'darwin')('writes files and compiles where a planted dangling link cannot lead outside', async () => {
  const ws = await workspace();
  const outside = await mkdtemp(path.join(os.tmpdir(), 'pm-planted-')); directories.push(outside);
  await symlink(path.join(outside, 'created.txt'), path.join(ws.layout.root, 'dangling.txt'));
  await expect(ws.call('write_file', { path: 'dangling.txt', text: 'escaped' }, signal())).rejects.toThrow();
  await symlink(outside, path.join(ws.layout.runDirectory, '.compiled'));
  const dir = path.join(ws.layout.stagingDirectory, 'linked-out');
  await ws.call('write_file', { path: path.join(dir, 'manifest.json'), text: JSON.stringify({ id: 'linked-out', name: 'Linked', version: '1.0.0', apiVersion: 1, contributes: ['effects'] }) }, signal());
  await ws.call('write_file', { path: path.join(dir, 'index.ts'), text: 'export default function activate() {}' }, signal());
  expect(JSON.parse(((await ws.call('compile_extension', { id: 'linked-out' }, signal()))[0] as any).text)).toMatchObject({ ok: true });
  expect(await readdir(outside)).toEqual([]);
});

it.runIf(process.platform === 'darwin')('reads text, pages and images and lists odd names', async () => {
  const ws = await workspace();
  const json = async (name: string, args: Record<string, unknown>) => JSON.parse(((await ws.call(name, args, signal()))[0] as any).text);
  const png = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0xff, 0x80, 0x0a]);
  await writeFile(path.join(ws.layout.root, 'shot.png'), png);
  const [image] = await ws.call('read_file', { path: 'shot.png' }, signal());
  expect(image).toMatchObject({ type: 'image', mimeType: 'image/png' });
  expect(Buffer.from((image as any).data)).toEqual(png);
  await writeFile(path.join(ws.layout.root, 'notes.txt'), 'héllo wörld');
  expect(await json('read_file', { path: 'notes.txt', offset: 1, limit: 4 })).toEqual({ text: 'éllo', offset: 1, totalChars: 11 });
  await writeFile(path.join(ws.layout.root, 'limit.txt'), Buffer.alloc(8 * 1024 * 1024, 97));
  expect((await json('read_file', { path: 'limit.txt', limit: 1 })).totalChars).toBe(8 * 1024 * 1024);
  await writeFile(path.join(ws.layout.root, 'large.txt'), Buffer.alloc(8 * 1024 * 1024 + 1, 97));
  await expect(ws.call('read_file', { path: 'large.txt' }, signal())).rejects.toThrow('no larger than 8 MB');
  await expect(ws.call('read_file', { path: 'missing.txt' }, signal())).rejects.toThrow(/no such file/i);
  const odd = path.join(ws.layout.root, 'odd');
  await mkdir(path.join(odd, 'sub dir'), { recursive: true });
  for (const name of ['line\nbreak', "%s\\0 -x '", '.hidden']) await writeFile(path.join(odd, name), '');
  await symlink('sub dir', path.join(odd, 'linked'));
  execFileSync('/usr/bin/mkfifo', [path.join(odd, 'pipe')]);
  const listed = await json('list_files', { path: 'odd' });
  expect(listed.total).toBe(6);
  expect(listed.entries.sort((a: any, b: any) => a.name.localeCompare(b.name))).toEqual([
    { name: "%s\\0 -x '", directory: false }, { name: '.hidden', directory: false }, { name: 'line\nbreak', directory: false },
    { name: 'linked', directory: false }, { name: 'pipe', directory: false }, { name: 'sub dir', directory: true }
  ].sort((a, b) => a.name.localeCompare(b.name)));
  await writeFile(path.join(odd, 'sub dir', 'inner.txt'), 'inner');
  expect(await json('list_files', { path: 'odd/linked' })).toEqual({ entries: [{ name: 'inner.txt', directory: false }], total: 1 });
  expect((await json('read_file', { path: 'odd/linked/inner.txt' })).text).toBe('inner');
  // Neither waits on a FIFO nor reads a folder.
  for (const file of ['odd/pipe', 'odd']) await expect(ws.call('read_file', { path: file }, signal())).rejects.toThrow('regular file');
  await expect(ws.call('list_files', { path: 'notes.txt' }, signal())).rejects.toThrow(/not a directory/i);
  await expect(ws.call('list_files', { path: 'missing' }, signal())).rejects.toThrow(/no such file/i);
}, 30_000);

it('reports real compile errors before publishing and exports created artifacts', async () => {
  const ws = await workspace();
  const dir = path.join(ws.layout.stagingDirectory, 'broken-effect');
  await ws.call('write_file', { path: path.join(dir, 'manifest.json'), text: JSON.stringify({ id: 'broken-effect', name: 'Broken', version: '1.0.0', apiVersion: 1, contributes: ['effects'] }) }, signal());
  await ws.call('write_file', { path: path.join(dir, 'index.ts'), text: 'export const = ;' }, signal());
  const result = { summary: 'Done', commands: [], artifacts: [], extensions: [{ id: 'broken-effect', action: 'created' }], notes: [], externalActions: [] };
  await expect(ws.finish(result, signal())).rejects.toThrow('failed compilation');
  await expect(access(path.join(ws.layout.liveDirectory, 'broken-effect'))).rejects.toThrow();
  await ws.call('write_file', { path: path.join(dir, 'index.ts'), text: 'export default function activate() {}' }, signal());
  await ws.call('write_file', { path: path.join(ws.layout.runDirectory, 'note.txt'), text: 'proof' }, signal());
  const finished = await ws.finish(result, signal());
  expect(finished.ok).toBe(true);
  if (finished.ok) expect(JSON.parse(finished.text).artifacts).toEqual([expect.objectContaining({ name: 'note.txt', size: 5 })]);
});

it('runs the credential scan before publishing, as Codex and Claude runs do', async () => {
  const ws = await workspace();
  const dir = path.join(ws.layout.stagingDirectory, 'keyed-effect');
  await mkdir(dir, { recursive: true });
  await writeFile(path.join(dir, 'manifest.json'), JSON.stringify({ id: 'keyed-effect', name: 'Keyed', version: '1.0.0', apiVersion: 1, contributes: ['effects'] }));
  // Assembled so this test file does not itself look like it holds a key.
  const key = ['sk', 'proj', 'A1b2C3d4E5f6G7h8I9j0K1l2'].join('-');
  await writeFile(path.join(dir, 'index.ts'), `const key = '${key}';\nexport default function activate() { return key; }\n`);
  const result = { summary: 'Done', commands: [], artifacts: [], extensions: [{ id: 'keyed-effect', action: 'created' }], notes: [], externalActions: [] };
  await expect(ws.finish(result, signal())).rejects.toThrow('keyed-effect/index.ts:1 looks like an OpenAI API key');
  await expect(access(path.join(ws.layout.liveDirectory, 'keyed-effect'))).rejects.toThrow();
  await writeFile(path.join(dir, 'index.ts'), 'export default function activate() {}\n');
  expect((await ws.finish(result, signal())).ok).toBe(true);
});

it.runIf(process.platform === 'darwin')('runs commands with real Project write isolation and propagates cancellation', async () => {
  const ws = await workspace();
  const outside = path.join(path.dirname(ws.layout.root), 'outside.txt');
  const quote = (value: string) => "'" + value.replaceAll("'", "'\\''") + "'";
  const success = await runWorkspaceCommand(ws.layout.root, 'project', 'printf proof > proof.txt && cat proof.txt', 5000, signal());
  expect(success).toMatchObject({ output: 'proof', exitCode: 0 });
  const denied = await runWorkspaceCommand(ws.layout.root, 'project', `printf denied > ${quote(outside)}`, 5000, signal());
  expect(denied.exitCode).not.toBe(0);
  await expect(access(outside)).rejects.toThrow();
  await expect(runWorkspaceCommand(ws.layout.root, 'project', 'sleep 10', 25, signal())).rejects.toThrow('timed out');
  const controller = new AbortController();
  const pending = runWorkspaceCommand(ws.layout.root, 'project', 'sleep 10', 5000, controller.signal);
  setTimeout(() => controller.abort(new Error('Stopped by test')), 30);
  await expect(pending).rejects.toThrow('Stopped by test');
});

it.runIf(process.platform === 'darwin')('keeps Project temp files, heredocs and mktemp inside the workspace scratch folder', async () => {
  const ws = await workspace();
  const scratch = path.join(await realpath(ws.layout.root), '.powermove', 'tmp');
  const heredoc = await runWorkspaceCommand(ws.layout.root, 'project', 'cat <<EOF\nheredoc works\nEOF', 5000, signal());
  expect(heredoc).toMatchObject({ output: 'heredoc works\n', exitCode: 0 });
  const script = [
    'a=$(mktemp) && b=$(mktemp -d) && c=$(mktemp -t render) && d=$(mktemp -dq -t clip)',
    'e=$(mktemp -p "$TMPDIR/.." inside.XXXXXX) && f=$(mktemp local.XXXXXX)',
    'printf data > "$a" && printf "%s\\n" "$a" "$b" "$c" "$d" "$e" "$f" && [ -d "$b" ] && [ -d "$d" ] && cat "$a"'
  ].join(' && ');
  const made = await runWorkspaceCommand(ws.layout.root, 'project', script, 5000, signal());
  expect(made.exitCode, made.output).toBe(0);
  const [a = '', b = '', c = '', d = '', e = '', f = '', data] = made.output.split('\n');
  for (const file of [a, b, c, d]) expect(path.dirname(file)).toBe(scratch);
  expect(path.basename(c)).toMatch(/^render\.[A-Za-z0-9]{8}$/);
  expect(path.basename(d)).toMatch(/^clip\./);
  expect(path.normalize(e)).toMatch(new RegExp(`^${escapeRegExp(path.dirname(scratch))}/inside\\.[A-Za-z0-9]{6}$`));
  expect(f).toMatch(/^local\.[A-Za-z0-9]{6}$/);
  expect(data).toBe('data');
  expect((await runWorkspaceCommand(ws.layout.root, 'project', 'printf "%s|%s|%s" "$TMP" "$TEMP" "$TMPPREFIX"', 5000, signal())).output)
    .toBe(`${scratch}|${scratch}|${path.join(scratch, 'zsh')}`);
  // An explicit template outside the workspace is still refused.
  expect((await runWorkspaceCommand(ws.layout.root, 'project', 'mktemp /private/tmp/pm-escape.XXXXXX', 5000, signal())).exitCode).not.toBe(0);
});

it.runIf(process.platform === 'darwin')('never writes tool shims or scratch folders through links a command planted', async () => {
  const ws = await workspace();
  const outside = await mkdtemp(path.join(os.tmpdir(), 'pm-planted-')); directories.push(outside);
  const quote = (value: string) => "'" + value.replaceAll("'", "'\\''") + "'";
  const target = path.join(outside, 'zshrc');
  await writeFile(target, 'keep', { mode: 0o600 });
  const plant = (script: string) => runWorkspaceCommand(ws.layout.root, 'project', script, 5000, signal());
  expect((await plant(`mkdir -p .powermove/bin && ln -sf ${quote(target)} .powermove/bin/mktemp`)).exitCode).toBe(0);
  expect((await plant('mktemp')).exitCode).toBe(0);
  expect(await readFile(target, 'utf8')).toBe('keep');
  expect((await stat(target)).mode & 0o777).toBe(0o600);
  // A linked bin/ or scratch folder must not let main create files or folders elsewhere.
  expect((await plant(`rm -rf .powermove/bin .powermove/tmp && ln -s ${quote(outside)} .powermove/bin && ln -s ${quote(path.join(outside, 'scratch'))} .powermove/tmp`)).exitCode).toBe(0);
  await plant('true');
  expect(await readdir(outside)).toEqual(['zshrc']);
});

it.runIf(process.platform === 'darwin')('gives commands the login PATH without the rest of the host environment', async () => {
  const ws = await workspace();
  process.env.PM_TEST_PROVIDER_TOKEN = 'must-not-leak';
  try {
    const result = await runWorkspaceCommand(ws.layout.root, 'project', 'env', 5000, signal());
    expect(result.output).not.toContain('must-not-leak');
    const PATH = /^PATH=(.*)$/m.exec(result.output)?.[1]?.split(':') ?? [];
    // ~/.zshenv may still prepend its own entries inside the shell.
    const bin = path.join(path.dirname(path.dirname(await realpath(ws.layout.root))), 'Agent Tools', 'bin');
    expect(PATH).toEqual(expect.arrayContaining([bin, '/usr/bin', '/bin']));
    expect(PATH.indexOf(bin)).toBeLessThan(PATH.indexOf('/usr/bin'));
    expect(PATH.every(entry => path.isAbsolute(entry))).toBe(true);
  } finally { delete process.env.PM_TEST_PROVIDER_TOKEN; }
});

it.runIf(process.platform === 'darwin')('points bun and npm caches into the workspace while HOME stays unwritable', async () => {
  const ws = await workspace();
  const root = await realpath(ws.layout.root);
  const probe = path.join(os.homedir(), `.powermove-sandbox-probe-${process.pid}`);
  const home = await runWorkspaceCommand(ws.layout.root, 'project', `printf x > ${JSON.stringify(probe)}`, 5000, signal());
  expect(home.exitCode).not.toBe(0);
  await expect(access(probe)).rejects.toThrow();
  const cache = (tool: string) => path.join(root, '.powermove', 'cache', tool);
  const env = await runWorkspaceCommand(ws.layout.root, 'project', 'printf "%s|%s|%s|%s" "$BUN_INSTALL_CACHE_DIR" "$npm_config_cache" "$XDG_CACHE_HOME" "$PIP_CACHE_DIR"', 5000, signal());
  expect(env.output).toBe([cache('bun'), cache('npm'), cache('xdg'), cache('pip')].join('|'));
});

it.runIf(process.platform === 'darwin')('gives Project commands direct network with no proxy, listening sockets and LaunchServices', async () => {
  const ws = await workspace();
  const env = await runWorkspaceCommand(ws.layout.root, 'project', 'env', 5000, signal());
  expect(env.output).not.toMatch(/^(?:HTTPS?_PROXY|ALL_PROXY|NO_PROXY|NODE_USE_ENV_PROXY)=/im);
  // Any port, not only a proxy's: a local server stands in for a remote host.
  const server = net.createServer(socket => socket.end('reached'));
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve));
  try {
    const port = (server.address() as net.AddressInfo).port;
    const script = `const net = require('node:net');
      const listen = host => new Promise(done => { const server = net.createServer();
        server.once('error', error => done(error.code)); server.listen(0, host, () => server.close(() => done('listening'))); });
      const reach = () => new Promise(done => { const socket = net.connect(${port}, '127.0.0.1');
        let text = ''; socket.on('data', chunk => text += chunk); socket.on('end', () => done(text)); socket.on('error', error => done(error.code)); });
      (async () => console.log(JSON.stringify({ any: await listen('0.0.0.0'), loopback: await listen('127.0.0.1'), reach: await reach() })))();`;
    const quote = (value: string) => "'" + value.replaceAll("'", "'\\''") + "'";
    const result = await runWorkspaceCommand(ws.layout.root, 'project', `${quote(process.execPath)} -e ${quote(script)}`, 10_000, signal());
    expect(JSON.parse(result.output)).toEqual({ any: 'listening', loopback: 'listening', reach: 'reached' });
  } finally { await new Promise(resolve => server.close(resolve)); }
  // lsappinfo only reads LaunchServices state, unlike `open`, which would launch an app.
  const front = await new Promise<string>(resolve => execFile('/usr/bin/lsappinfo', ['front'], (_error, stdout) => resolve(stdout)));
  if (front.includes('ASN:')) expect((await runWorkspaceCommand(ws.layout.root, 'project', '/usr/bin/lsappinfo front', 5000, signal())).output).toContain('ASN:');
});

it.runIf(process.platform === 'darwin')('downloads in Project access from any public host', async () => {
  const ws = await workspace();
  for (const url of ['https://example.com', 'https://registry.npmjs.org/is-number']) {
    const result = await runWorkspaceCommand(ws.layout.root, 'project', `curl -sS --max-time 20 -o /dev/null -w "%{http_code}" ${url}`, 30_000, signal());
    expect(result.exitCode, result.output).toBe(0);
    expect(result.output).toBe('200');
  }
}, 60_000);

it.runIf(process.platform === 'darwin')('lets Project commands read account keys and provider logins, but still write only the workspace', async () => {
  const ws = await workspace();
  const home = await mkdtemp(path.join(os.tmpdir(), 'pm-api-home-')); directories.push(home);
  const userData = path.dirname(path.dirname(ws.layout.root));
  const files = [path.join(home, '.ssh', 'id_ed25519'), path.join(home, '.codex', 'auth.json'),
    path.join(userData, 'codex-runtime', 'auth.json'), path.join(userData, 'claude-runtime', '.credentials.json')];
  for (const file of files) { await mkdir(path.dirname(file), { recursive: true }); await writeFile(file, 'readable'); }
  const previous = process.env.HOME;
  process.env.HOME = home;
  try {
    for (const file of files) {
      const read = await runWorkspaceCommand(ws.layout.root, 'project', `cat ${JSON.stringify(file)}`, 5000, signal());
      expect(read, file).toMatchObject({ output: 'readable', exitCode: 0 });
      const write = await runWorkspaceCommand(ws.layout.root, 'project', `printf changed > ${JSON.stringify(file)}`, 5000, signal());
      expect(write.exitCode, file).not.toBe(0);
    }
  } finally { process.env.HOME = previous; }
});

it.runIf(process.platform === 'darwin')('allows new sessions and still stops what a command leaves running in them', async () => {
  const ws = await workspace();
  const alive = (pid: number) => { try { process.kill(pid, 0); return true; } catch { return false; } };
  const leftover = await runWorkspaceCommand(ws.layout.root, 'project', 'sleep 30 >/dev/null 2>&1 & printf %s $!', 5000, signal());
  await expect.poll(() => alive(Number(leftover.output))).toBe(false);
  // Piped, so perl is not already a session leader (which would refuse anyway).
  const setsid = await runWorkspaceCommand(ws.layout.root, 'project', `perl -MPOSIX -e 'POSIX::setsid() < 0 and die "setsid: $!\\n"; print "new session"' 2>&1 | cat`, 5000, signal());
  expect(setsid.output).toBe('new session');
  // As Python's start_new_session does: the child starts a session, its parent exits.
  const session = `perl -MPOSIX -e 'if (fork) { select(undef, undef, undef, 0.05) until -s "session.pid"; exit } POSIX::setsid() < 0 and die; open my $f, ">", "session.pid"; print $f $$; close $f; sleep 30'`;
  expect((await runWorkspaceCommand(ws.layout.root, 'project', session, 5000, signal())).exitCode).toBe(0);
  const leader = Number(await readFile(path.join(ws.layout.root, 'session.pid'), 'utf8'));
  expect(leader).toBeGreaterThan(1);
  await expect.poll(() => alive(leader)).toBe(false);
  // A posix_spawn new session with a live parent is found through that parent.
  const script = `const c = require('node:child_process').spawn('/bin/sleep', ['30'], { detached: true, stdio: 'ignore' }); console.log(c.pid); setInterval(() => {}, 1000);`;
  const node = JSON.stringify(process.execPath);
  const timedOut = await runWorkspaceCommand(ws.layout.root, 'project', `${node} -e ${JSON.stringify(script)}`, 1500, signal()).catch((error: Error) => error.message);
  const escaped = Number(/(\d+)/.exec(String(timedOut))?.[1]);
  expect(escaped).toBeGreaterThan(1);
  await expect.poll(() => alive(escaped)).toBe(false);
});

it.runIf(process.platform === 'darwin')('stops detached children after their parent exits, at once or after they moved away', async () => {
  const ws = await workspace();
  const alive = (pid: number) => { try { process.kill(pid, 0); return true; } catch { return false; } };
  const quote = (value: string) => "'" + value.replaceAll("'", "'\\''") + "'";
  const node = (script: string) => `${quote(process.execPath)} -e ${quote(script)}`;
  // The parent exits immediately, so no parent link leads to the child.
  const late = `require('node:child_process').spawn('/bin/sh', ['-c', 'echo $$ > late.pid; sleep 1; echo late > late.txt'], { detached: true, stdio: 'ignore' }).unref()`;
  expect((await runWorkspaceCommand(ws.layout.root, 'project', node(late), 5000, signal())).exitCode).toBe(0);
  // This one leaves the workspace folder while its parent still lives.
  const moved = `const c = require('node:child_process').spawn('/bin/sleep', ['30'], { detached: true, stdio: 'ignore', cwd: '/' }); c.unref(); console.log(c.pid); setTimeout(() => {}, 800)`;
  const escaped = Number((await runWorkspaceCommand(ws.layout.root, 'project', node(moved), 5000, signal())).output);
  expect(escaped).toBeGreaterThan(1);
  await expect.poll(() => alive(escaped)).toBe(false);
  await new Promise(resolve => setTimeout(resolve, 1500));
  await expect(access(path.join(ws.layout.root, 'late.txt'))).rejects.toThrow();
  const latePid = Number(await readFile(path.join(ws.layout.root, 'late.pid'), 'utf8').catch(() => '0'));
  if (latePid) expect(alive(latePid)).toBe(false);
});

it.runIf(process.platform === 'darwin')('stops a child that left the folder and lost its parent before any poll', async () => {
  const ws = await workspace();
  const alive = (pid: number) => { try { process.kill(pid, 0); return true; } catch { return false; } };
  const quote = (value: string) => "'" + value.replaceAll("'", "'\\''") + "'";
  const late = path.join(ws.layout.root, 'late.txt');
  const script = `const c = require('node:child_process').spawn('/bin/sh', ['-c', ${JSON.stringify(`sleep 2; echo late > ${quote(late)}`)}], { detached: true, stdio: 'ignore', cwd: '/' }); console.log(c.pid); c.unref()`;
  const escaped = Number((await runWorkspaceCommand(ws.layout.root, 'project', `${quote(process.execPath)} -e ${quote(script)}`, 5000, signal())).output);
  expect(escaped).toBeGreaterThan(1);
  await expect.poll(() => alive(escaped)).toBe(false);
  await new Promise(resolve => setTimeout(resolve, 2500));
  await expect(access(late)).rejects.toThrow();
});

it.runIf(process.platform === 'darwin')('a foreground command exiting leaves a running background job’s daemon alone', async () => {
  const ws = await workspace();
  const alive = (pid: number) => { try { process.kill(pid, 0); return true; } catch { return false; } };
  const quote = (value: string) => "'" + value.replaceAll("'", "'\\''") + "'";
  const daemon = `require('node:child_process').spawn('/bin/sh', ['-c', 'echo $$ > daemon.pid; exec sleep 30'], { detached: true, stdio: 'ignore' }).unref()`;
  await ws.call('run_command', { command: `sleep 0.3; ${quote(process.execPath)} -e ${quote(daemon)}; exec sleep 30`, background: true }, signal());
  await ws.call('run_command', { command: 'sleep 1.5' }, signal());
  const pid = Number(await readFile(path.join(ws.layout.root, 'daemon.pid'), 'utf8'));
  expect(pid).toBeGreaterThan(1);
  await new Promise(resolve => setTimeout(resolve, 500));
  expect(alive(pid)).toBe(true);
  await ws.stopCommands();
  await expect.poll(() => alive(pid)).toBe(false);
});

it('allows up to ten minutes per command and keeps the thirty second default', async () => {
  const ws = await workspace();
  await expect(ws.call('run_command', { command: 'true', timeoutMs: 600_001 }, signal())).rejects.toThrow('between 1 and 600000');
  await expect(ws.call('run_command', { command: 'true', background: 'yes' }, signal())).rejects.toThrow('background');
  const spec = COMPATIBLE_WORKSPACE_TOOLS.find(tool => tool.name === 'run_command')!;
  expect((spec.inputSchema as any).properties.timeoutMs.maximum).toBe(600_000);
  expect(spec.description).toContain('30 seconds');
});

it.runIf(process.platform === 'darwin')('runs background jobs, reports them and stops them with everything they started', async () => {
  const ws = await workspace();
  const alive = (pid: number) => { try { process.kill(pid, 0); return true; } catch { return false; } };
  const json = async (name: string, args: Record<string, unknown>) => JSON.parse(((await ws.call(name, args, signal()))[0] as any).text);
  expect(await json('run_command', { command: 'sleep 0.2; printf done', background: true })).toEqual({ jobId: 'job-1', state: 'running' });
  expect(await json('command_status', { jobId: 'job-1', waitMs: 10_000 })).toMatchObject({ jobId: 'job-1', state: 'exited', exitCode: 0, output: 'done' });
  await ws.call('run_command', { command: 'printf "%s " $$; sleep 30 & printf "%s" $!; wait', background: true }, signal());
  await expect.poll(async () => (await json('command_status', { jobId: 'job-2' })).output).toMatch(/^\d+ \d+$/);
  const pids = (await json('command_status', { jobId: 'job-2' })).output.split(' ').map(Number) as number[];
  expect(pids.every(alive)).toBe(true);
  expect(await json('command_status', { jobId: 'job-2', stop: true })).toMatchObject({ state: 'stopped' });
  for (const pid of pids) await expect.poll(() => alive(pid)).toBe(false);
  await expect(ws.call('command_status', { jobId: 'job-9' }, signal())).rejects.toThrow('Unknown background job');
  for (let index = 0; index < 4; index++) await ws.call('run_command', { command: 'sleep 30', background: true }, signal());
  await expect(ws.call('run_command', { command: 'sleep 30', background: true }, signal())).rejects.toThrow('At most 4');
  await ws.stopCommands();
  expect((await json('command_status', { jobId: 'job-6' })).state).toBe('stopped');
});

it.runIf(process.platform === 'darwin')('stops background jobs before complete_task validates and when the run is stopped', async () => {
  const ws = await workspace();
  const alive = (pid: number) => { try { process.kill(pid, 0); return true; } catch { return false; } };
  const pidOf = async (file: string) => {
    await expect.poll(() => readFile(path.join(ws.layout.root, file), 'utf8').then(text => /^\d+\n$/.test(text), () => false)).toBe(true);
    return Number(await readFile(path.join(ws.layout.root, file), 'utf8'));
  };
  await ws.call('run_command', { command: 'echo $$ > a.pid; exec sleep 30', background: true }, signal());
  const first = await pidOf('a.pid');
  // Validation fails, but the job is already gone.
  await expect(ws.finish({ summary: 'x' }, signal())).rejects.toThrow('complete_task requires');
  expect(alive(first)).toBe(false);
  const controller = new AbortController();
  await ws.call('run_command', { command: 'echo $$ > b.pid; exec sleep 30', background: true }, controller.signal);
  const second = await pidOf('b.pid');
  controller.abort(new Error('Stopped by test'));
  await expect.poll(() => alive(second)).toBe(false);
});

it.runIf(process.platform === 'darwin')('lets Project commands write to their inherited stdio but not other devices', async () => {
  const ws = await workspace();
  const result = await runWorkspaceCommand(ws.layout.root, 'project', 'printf out > /dev/stdout && printf fd > /dev/fd/1 && printf err > /dev/stderr', 5000, signal());
  expect(result.exitCode).toBe(0);
  // stdout and stderr are separate pipes, so only their own order is fixed.
  expect(result.output.replace('err', '')).toBe('outfd');
  expect(result.output).toContain('err');
  const device = await runWorkspaceCommand(ws.layout.root, 'project', 'printf x > /dev/zero', 5000, signal());
  expect(device.exitCode).not.toBe(0);
  expect(device.output).toContain('not permitted');
});

it('returns deprecated compatibility API uses on write, read, compile and completion without refusing any', async () => {
  const ws = await workspace();
  const dir = path.join(ws.layout.stagingDirectory, 'old-snapshot');
  const json = async (name: string, args: Record<string, unknown>) => JSON.parse(((await ws.call(name, args, signal()))[0] as any).text);
  await ws.call('write_file', { path: path.join(dir, 'manifest.json'), text: JSON.stringify({ id: 'old-snapshot', name: 'Old', version: '1.0.0', apiVersion: 2, contributes: ['commands'] }) }, signal());
  const source = 'export default function activate(api) {\n  api.render.snapshot(0, 480);\n}\n';
  const written = await json('write_file', { path: path.join(dir, 'index.ts'), text: source });
  expect(written.deprecations).toEqual([expect.objectContaining({ extension: 'old-snapshot', file: 'index.ts', line: 2, member: 'render.snapshot', replacement: 'await api.render.snapshotAsync(time, maxWidth)' })]);
  expect(written.guidance).toMatch(/suggest the migration/);
  expect((await json('read_file', { path: path.join(dir, 'index.ts') })).deprecations).toHaveLength(1);
  expect((await json('read_file', { path: path.join(dir, 'manifest.json') })).deprecations).toBeUndefined();
  const compiled = await json('compile_extension', { id: 'old-snapshot' });
  expect(compiled).toMatchObject({ ok: true, deprecations: [expect.objectContaining({ line: 2, member: 'render.snapshot' })] });
  const finished = await ws.finish({ summary: 'Done', commands: [], artifacts: [], extensions: [{ id: 'old-snapshot', action: 'created' }], notes: [], externalActions: [] }, signal());
  expect(finished.ok).toBe(true);
  if (!finished.ok) return;
  expect(finished.extensionWarnings).toEqual([expect.objectContaining({ extension: 'old-snapshot', line: 2 })]);
  expect(JSON.parse(finished.text).extensionWarnings).toHaveLength(1);
  // Published unchanged: the deprecated call stays.
  expect(await readFile(path.join(ws.layout.liveDirectory, 'old-snapshot', 'index.ts'), 'utf8')).toBe(source);
}, 30_000);
