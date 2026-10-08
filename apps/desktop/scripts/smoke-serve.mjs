import assert from 'node:assert/strict';
import { execFile, spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdtemp, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';
import { get } from 'node:https';

const entry = fileURLToPath(new URL('../../../packages/cli/bin/powermove.mjs', import.meta.url));
const root = await mkdtemp(path.join(os.tmpdir(), 'powermove-host-smoke-'));
const child = spawn(process.execPath, [entry, 'serve', '--host', '127.0.0.1', '--port', '0', '--user-data', root, '--exports', path.join(root, 'exports')], { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
const closed = once(child, 'close');
let output = '';
child.stdout.on('data', data => { output += data; });
child.stderr.on('data', data => { output += data; });
const request = (url, headers = {}) => new Promise((resolve, reject) => {
  // This disposable local host uses a self-signed certificate, just like a
  // browser's explicit first-visit acceptance. TLS remains enabled.
  get(url, { rejectUnauthorized: false, headers }, response => {
    let body = ''; response.on('data', data => { body += data; });
    response.on('end', () => resolve({ status: response.statusCode, headers: response.headers, body }));
  }).on('error', reject);
});
try {
  const deadline = Date.now() + 60_000;
  while (!/\[serve\] engine \d+ connected/.test(output)) {
    if (child.exitCode !== null || Date.now() > deadline) {
      throw new Error(`Host did not start its document engine: ${output.replace(/token=[^\s]+/g, 'token=[redacted]')}`);
    }
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  const address = /https:\/\/127\.0\.0\.1:\d+\/\?token=[^\s]+/.exec(output)?.[0];
  assert.ok(address, 'Host should print a local browser address');
  const login = await request(address);
  assert.equal(login.status, 302);
  const cookie = login.headers['set-cookie']?.[0]?.split(';')[0];
  assert.ok(cookie, 'Login should establish a session');
  const response = await request(new URL('/', address), { Cookie: cookie });
  assert.equal(response.status, 200);
  assert.match(response.body, /<script[^>]+type="module"/);
  assert.equal((await request(new URL('/', address))).status, 401);
  console.log('Host smoke passed: HTTPS, document engine, browser page, and session protection.');
} finally {
  if (child.exitCode === null) {
    if (process.platform === 'win32') await promisify(execFile)('taskkill.exe', ['/PID', String(child.pid), '/T', '/F'], { windowsHide: true }).catch(() => undefined);
    else child.kill('SIGTERM');
  }
  await Promise.race([closed, new Promise(resolve => setTimeout(resolve, 6_000))]);
  if (child.exitCode === null) child.kill('SIGKILL');
  await rm(root, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
}
