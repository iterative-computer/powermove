// Exercise the actual packaged executable with its production fuses intact.
import assert from 'node:assert/strict';
import { spawn, execFile } from 'node:child_process';
import { once } from 'node:events';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { createServer } from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';
import { chromium } from 'playwright';

assert.equal(process.platform, 'win32', 'Run packaged Windows validation on Windows.');
const desktop = fileURLToPath(new URL('../', import.meta.url));
const executable = path.join(desktop, 'dist/win-unpacked/Powermove.exe');
const run = promisify(execFile);
const delay = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));

async function availablePort() {
  const server = createServer();
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const port = server.address().port;
  await new Promise(resolve => server.close(resolve));
  return port;
}

async function waitFor(predicate, message, timeout = 30_000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (await predicate()) return;
    await delay(100);
  }
  throw new Error(message);
}

for (const action of ['Start fresh', 'Bring my workspace']) {
  const userData = await mkdtemp(path.join(os.tmpdir(), 'powermove-packaged-startup-'));
  const port = await availablePort();
  const env = { ...process.env, POWERMOVE_USER_DATA: userData, POWERMOVE_BACKGROUND_TEST: '0', POWERMOVE_DEVTOOLS: '0' };
  delete env.ELECTRON_RUN_AS_NODE;
  const child = spawn(executable, [`--remote-debugging-port=${port}`, '--remote-debugging-address=127.0.0.1'], {
    env, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe']
  });
  const exited = once(child, 'exit');
  let stderr = '', browser;
  child.stderr.on('data', data => { stderr = (stderr + data).slice(-12_000); });
  child.stdout.resume();
  try {
    await waitFor(async () => {
      if (child.exitCode !== null) throw new Error(`The packaged app exited during startup (${child.exitCode}).`);
      return fetch(`http://127.0.0.1:${port}/json/version`, { signal: AbortSignal.timeout(1000) }).then(response => response.ok, () => false);
    }, 'The packaged app did not start its renderer.');
    browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
    const context = browser.contexts()[0];
    const pageAt = async url => {
      await waitFor(() => context.pages().some(page => page.url() === url), `The packaged app did not open ${url}.`);
      return context.pages().find(page => page.url() === url);
    };
    const animation = await pageAt('app://powermove/onboarding/animation.html');
    await animation.waitForFunction(() => !!window.onboarding);
    await animation.evaluate(() => window.onboarding.animationComplete());
    const welcome = await pageAt('app://powermove/onboarding/welcome.html');
    await welcome.getByRole('button', { name: 'Begin', exact: true }).click();
    await welcome.locator('#timeline-step').getByRole('button', { name: 'Continue', exact: true }).click();
    // Keep the workspace handoff at the local provider's connection gate.
    // This packaged smoke test must not launch an authenticated agent run.
    await welcome.getByText('API or local model', { exact: true }).click();
    await welcome.locator('#agent-step').getByRole('button', { name: 'Continue', exact: true }).click();
    await welcome.getByRole('button', { name: action, exact: true }).click();
    const editor = await pageAt('app://powermove/');
    await editor.waitForFunction(() => Boolean(window.PM?.AgentUI?.state && document.querySelector('#titlebar')), undefined, { timeout: 25_000 });
    await waitFor(() => welcome.isClosed(), 'Setup did not release the packaged editor.');
    assert.equal(await editor.evaluate(() => document.visibilityState), 'visible');
    const saved = JSON.parse(await readFile(path.join(userData, 'onboarding-v1.json'), 'utf8'));
    assert.equal(saved.workspaceImport, action === 'Start fresh' ? null : 'after-effects');
    await editor.evaluate(() => window.close()).catch(error => { if (!editor.isClosed()) throw error; });
    await waitFor(() => child.exitCode !== null, 'The packaged editor could not close normally.', 10_000);
    const [code] = await exited;
    assert.equal(code, 0);
    console.log(`Packaged Windows first launch passed: ${action}, visible editor, normal close.`);
  } catch (error) {
    // The test profiles hold no user account data. Still redact links from
    // Chromium/agent diagnostics before printing public validation logs.
    console.error(stderr.replace(/(?:https?|wss?):\/\/\S+/g, '[URL]').slice(-4000));
    throw error;
  } finally {
    if (child.exitCode === null) {
      await run('taskkill.exe', ['/PID', String(child.pid), '/T', '/F'], { windowsHide: true }).catch(() => undefined);
      await exited.catch(() => undefined);
    }
    await browser?.close().catch(() => undefined);
    await rm(userData, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
  }
}
