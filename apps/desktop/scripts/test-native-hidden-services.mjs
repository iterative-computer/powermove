/** Run the native MCP smoke outside an Aqua launch domain or over a temporary,
 * loopback-only SSH server. Invoke under pm-electron-lock. No system service,
 * login setting, authorized_keys or existing host key is changed. */
import { spawn, execFileSync } from 'node:child_process';
import { createServer } from 'node:net';
import { mkdtemp, readFile, writeFile, rm } from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import { fileURLToPath } from 'node:url';
import assert from 'node:assert/strict';

const mode = process.argv[2];
if (!['--launchd', '--ssh'].includes(mode)) throw new Error('Use --launchd or --ssh');
const root = await mkdtemp(path.join(os.tmpdir(), 'pm-native-service-'));
const script = fileURLToPath(new URL('./test-mcp.mjs', import.meta.url));
const env = Object.fromEntries(['PATH', 'POWERMOVE_TEST_CLI', 'POWERMOVE_ENGINE_ADDON', 'POWERMOVE_PM_BRIDGE', 'POWERMOVE_CHROMIUM', 'PM_FFMPEG_DIR', 'POWERMOVE_MCP_EVIDENCE'].filter(k => process.env[k]).map(k => [k, process.env[k]]));
if (env.POWERMOVE_MCP_EVIDENCE) env.POWERMOVE_MCP_EVIDENCE = path.join(env.POWERMOVE_MCP_EVIDENCE, mode.slice(2));
const xml = value => String(value).replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&apos;' })[c]);
const quote = value => `'${String(value).replaceAll("'", "'\\''")}'`;
const run = (file, args) => execFileSync(file, args, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] });
let sshd, loaded = false;
const label = `com.powermove.native-mcp-test.${process.pid}`, domain = `user/${process.getuid()}`, plist = path.join(root, 'service.plist');
try {
  if (mode === '--launchd') {
    const output = path.join(root, 'stdout'), error = path.join(root, 'stderr');
    await writeFile(plist, `<?xml version="1.0"?><!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd"><plist version="1.0"><dict>
      <key>Label</key><string>${label}</string><key>RunAtLoad</key><true/>
      <key>ProgramArguments</key><array>${[process.execPath, script, '--native'].map(a => `<string>${xml(a)}</string>`).join('')}</array>
      <key>EnvironmentVariables</key><dict>${Object.entries(env).map(([k,v]) => `<key>${xml(k)}</key><string>${xml(v)}</string>`).join('')}</dict>
      <key>StandardOutPath</key><string>${xml(output)}</string><key>StandardErrorPath</key><string>${xml(error)}</string>
      <key>ProcessType</key><string>Background</string></dict></plist>`, { mode: 0o600 });
    run('/bin/launchctl', ['bootstrap', domain, plist]); loaded = true;
    const until = Date.now() + 240_000;
    for (;;) {
      const state = run('/bin/launchctl', ['print', `${domain}/${label}`]);
      const out = await readFile(output, 'utf8').catch(() => ''), err = await readFile(error, 'utf8').catch(() => '');
      if (!/\n\s*pid = \d+/.test(state) && /last exit code =/.test(state)) {
        process.stdout.write(out); process.stderr.write(err);
        assert(/last exit code = 0\b/.test(state) && /MCP native hidden smoke passed/.test(out), `launchd smoke failed: ${state}\n${err}`); break;
      }
      if (Date.now() > until) throw new Error(`launchd smoke timed out: ${err}`);
      await new Promise(resolve => setTimeout(resolve, 500));
    }
    console.log(`Native MCP launchd smoke passed in ${domain} (Background, no Aqua domain).`);
  } else {
    for (const name of ['host', 'client']) run('/usr/bin/ssh-keygen', ['-q', '-t', 'ed25519', '-N', '', '-f', path.join(root, name)]);
    const clientKey = await readFile(path.join(root, 'client.pub'), 'utf8'), hostKey = (await readFile(path.join(root, 'host.pub'), 'utf8')).trim();
    await writeFile(path.join(root, 'authorized_keys'), clientKey, { mode: 0o600 });
    const port = await new Promise((resolve, reject) => { const server = createServer(); server.once('error', reject); server.listen(0, '127.0.0.1', () => { const port = server.address().port; server.close(() => resolve(port)); }); });
    const config = path.join(root, 'sshd_config');
    await writeFile(config, `Port ${port}\nListenAddress 127.0.0.1\nHostKey ${root}/host\nPidFile ${root}/pid\nAuthorizedKeysFile ${root}/authorized_keys\nStrictModes no\nUsePAM no\nPasswordAuthentication no\nKbdInteractiveAuthentication no\nPubkeyAuthentication yes\nAuthenticationMethods publickey\nAllowUsers ${os.userInfo().username}\nAllowTcpForwarding no\nX11Forwarding no\nPermitTTY no\nPermitTunnel no\nLogLevel ERROR\n`, { mode: 0o600 });
    const sshLog = path.join(root, 'sshd.log');
    await writeFile(plist, `<?xml version="1.0"?><plist version="1.0"><dict><key>Label</key><string>${label}</string><key>RunAtLoad</key><true/><key>ProgramArguments</key><array><string>/usr/sbin/sshd</string><string>-D</string><string>-e</string><string>-f</string><string>${xml(config)}</string></array><key>StandardErrorPath</key><string>${xml(sshLog)}</string><key>ProcessType</key><string>Background</string></dict></plist>`, { mode: 0o600 });
    run('/bin/launchctl', ['bootstrap', domain, plist]); loaded = true;
    await new Promise(resolve => setTimeout(resolve, 500));
    const known = path.join(root, 'known_hosts'); await writeFile(known, `[localhost]:${port} ${hostKey}\n`, { mode: 0o600 });
    const command = `manager=$(/bin/launchctl managername); echo "SSH launch domain: $manager"; test "$manager" != Aqua || exit 90; exec env ${Object.entries(env).map(([k,v]) => `${k}=${quote(v)}`).join(' ')} ${quote(process.execPath)} ${quote(script)} --native`;
    const child = spawn('/usr/bin/ssh', ['-T', '-p', String(port), '-i', path.join(root, 'client'), '-o', 'IdentitiesOnly=yes', '-o', 'BatchMode=yes', '-o', 'ConnectTimeout=5', '-o', 'StrictHostKeyChecking=yes', '-o', `UserKnownHostsFile=${known}`, 'localhost', command], { stdio: ['ignore', 'pipe', 'pipe'] });
    let output = '', error = ''; child.stdout.on('data', b => { output += b; process.stdout.write(b); }); child.stderr.on('data', b => { error += b; process.stderr.write(b); });
    const code = await new Promise(resolve => child.once('exit', resolve));
    assert(code === 0 && /MCP native hidden smoke passed/.test(output), `SSH smoke failed (${code}): ${error}\n${await readFile(sshLog, 'utf8').catch(() => '')}`);
    console.log('Native MCP SSH localhost smoke passed with no TTY, forwarding or GUI session.');
  }
} finally {
  if (loaded) { try { run('/bin/launchctl', ['bootout', `${domain}/${label}`]); } catch {} }
  if (sshd && sshd.exitCode === null && sshd.signalCode === null) { const exit = new Promise(resolve => sshd.once('exit', resolve)); sshd.kill('SIGTERM'); await exit; }
  await rm(root, { recursive: true, force: true });
}
