import { createHash } from 'node:crypto';
import { createReadStream, existsSync } from 'node:fs';
import { mkdtemp, rm, stat, writeFile } from 'node:fs/promises';
import { createServer, type Server } from 'node:http';
import type { AddressInfo } from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { _electron, type Page } from 'playwright';

import { expect, launchApp, repoRoot, test, type LaunchedApp } from './helpers/app';

/*
 * On-device transcription in the real app: the download sheet, Settings ›
 * Transcription and the utility-process engine. Models come from a local
 * server through POWERMOVE_TEST_TRANSCRIPTION_CATALOG (honoured only in
 * background tests), so nothing here touches the network.
 *
 * PM_E2E_SHOTS=/some/dir saves screenshots there (never inside the repo).
 * POWERMOVE_E2E_PARAKEET_DIR=<folder with the real Parakeet V3 sherpa files>
 * also runs a real transcription through the sheet-downloaded model.
 */

const shots = process.env.PM_E2E_SHOTS;
const parakeetDir = process.env.POWERMOVE_E2E_PARAKEET_DIR;

interface Hosted { body?: Buffer; file?: string; size: number; sha256: string }

async function serve(files: Record<string, Hosted>, gate?: Promise<void>): Promise<{ server: Server; origin: string }> {
  const server = createServer((request, response) => {
    const hosted = files[request.url ?? ''];
    if (!hosted) { response.writeHead(404).end(); return; }
    const range = /bytes=(\d+)-/.exec(request.headers.range ?? '');
    const from = range ? Number(range[1]) : 0;
    response.writeHead(range ? 206 : 200, { 'Content-Length': String(hosted.size - from) });
    if (hosted.file) { createReadStream(hosted.file, { start: from }).pipe(response); return; }
    const body = hosted.body!.subarray(from);
    const half = Math.floor(body.length / 2);
    response.write(body.subarray(0, half));
    void (gate ?? Promise.resolve()).then(() => response.end(body.subarray(half)));
  });
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  return { server, origin: `http://127.0.0.1:${(server.address() as AddressInfo).port}` };
}

const blob = (size: number, seed: number) => Buffer.from(Array.from({ length: size }, (_, index) => (index * 7 + seed) % 256));
const sha = (data: Buffer) => createHash('sha256').update(data).digest('hex');

async function fileSha(file: string): Promise<string> {
  const hash = createHash('sha256');
  for await (const chunk of createReadStream(file)) hash.update(chunk as Buffer);
  return hash.digest('hex');
}

async function catalog(dir: string, origin: string, files: Record<string, Hosted>, real: boolean): Promise<string> {
  const entry = (id: string, name: string, description: string, recommended: boolean, languages: 'en' | 'multi', names: string[]) => ({
    id, name, description, recommended, languages,
    languageCodes: languages === 'en' ? ['en'] : ['en', 'de', 'fr', 'es', 'it'],
    speed: 0.85, accuracy: languages === 'en' ? 0.85 : 0.8, wordTimestamps: true, license: 'CC BY 4.0 · NVIDIA',
    files: names.map((file) => ({ name: file, url: `${origin}/${id}/${file}`, size: files[`/${id}/${file}`]!.size, sha256: files[`/${id}/${file}`]!.sha256 }))
  });
  const names = ['encoder.int8.onnx', 'decoder.int8.onnx', 'joiner.int8.onnx', 'tokens.txt'];
  if (!real) {
    names.forEach((name, index) => {
      for (const id of ['parakeet-tdt-0.6b-v3', 'parakeet-tdt-0.6b-v2']) {
        const body = blob(index === 0 ? 3_000_000 : 40_000, index + id.length);
        files[`/${id}/${name}`] = { body, size: body.length, sha256: sha(body) };
      }
    });
  } else {
    for (const name of names) {
      const file = path.join(parakeetDir!, name);
      files[`/parakeet-tdt-0.6b-v3/${name}`] = { file, size: (await stat(file)).size, sha256: await fileSha(file) };
      const body = blob(1000, name.length);
      files[`/parakeet-tdt-0.6b-v2/${name}`] = { body, size: body.length, sha256: sha(body) };
    }
  }
  const out = path.join(dir, 'catalog.json');
  await writeFile(out, JSON.stringify([
    entry('parakeet-tdt-0.6b-v3', 'Parakeet V3', 'Fast and accurate in 25 European languages.', true, 'multi', names),
    entry('parakeet-tdt-0.6b-v2', 'Parakeet V2', 'English only. The most accurate for English.', false, 'en', names)
  ]));
  return out;
}

async function shot(page: Page, name: string): Promise<void> {
  if (!shots) return;
  await page.waitForTimeout(350);
  await page.screenshot({ path: path.join(shots, `${name}.png`) });
}

async function theme(page: Page, mode: 'light' | 'dark'): Promise<void> {
  await page.evaluate((value) => (window as any).PM.theme.apply(value), mode);
  await page.waitForFunction((value) => (document.documentElement.dataset.theme ?? 'light') === value, mode);
}

test('the download sheet downloads a model, closes itself, and Settings manages it', async () => {
  test.setTimeout(120_000);
  const home = await mkdtemp(path.join(os.tmpdir(), 'pm-transcription-'));
  let release!: () => void;
  const gate = new Promise<void>((resolve) => { release = resolve; });
  const files: Record<string, Hosted> = {};
  const { server, origin } = await serve(files, gate);
  const catalogFile = await catalog(home, origin, files, false);
  let session: LaunchedApp | null = null;
  try {
    session = await launchApp({ userData: path.join(home, 'profile'), env: { POWERMOVE_TEST_TRANSCRIPTION_CATALOG: catalogFile } });
    await session.openEditor();
    const page = session.page;
    for (const mode of ['light', 'dark'] as const) {
      await theme(page, mode);
      await page.evaluate(() => { void (window as any).PM.Transcription.ensureModel('Captions need a speech model to transcribe “Interview.mov”.').then((ready: boolean) => { (window as any).__sheetResult = ready; }); });
      const sheet = page.locator('.transcription-modal');
      await expect(sheet).toBeVisible();
      await expect(sheet.getByText('Captions need a speech model to transcribe “Interview.mov”.')).toBeVisible();
      await expect(sheet.getByRole('radio', { name: /Parakeet V3/ })).toBeChecked();
      await expect(sheet.getByRole('meter', { name: 'Speed' }).first()).toBeVisible();
      await shot(page, `sheet-${mode}`);
      if (mode === 'light') {
        // Dismissing resolves false.
        await sheet.getByRole('button', { name: 'Not Now' }).click();
        await expect(sheet).toBeHidden();
        expect(await page.evaluate(() => (window as any).__sheetResult)).toBe(false);
      }
    }
    // A second request while the sheet is up joins it and updates the reason.
    await page.evaluate(() => { void (window as any).PM.Transcription.ensureModel('The agent needs a speech model to find the best take.'); });
    await expect(page.locator('.transcription-modal').getByText('The agent needs a speech model to find the best take.')).toBeVisible();
    await expect(page.locator('.transcription-modal')).toHaveCount(1);

    const sheet = page.locator('.transcription-modal');
    await sheet.getByRole('button', { name: 'Download' }).click();
    await expect(sheet.getByRole('progressbar')).toBeVisible();
    await expect(sheet.getByRole('button', { name: 'Cancel Download' })).toBeVisible();
    await shot(page, 'sheet-downloading-dark');
    release();
    await expect(sheet).toBeHidden({ timeout: 20_000 });
    expect(await page.evaluate(() => (window as any).__sheetResult)).toBe(true);
    const status = await page.evaluate(() => (window as any).PM.Transcription.status());
    expect(status.activeModelId).toBe('parakeet-tdt-0.6b-v3');
    // Ready: no sheet the next time.
    expect(await page.evaluate(() => (window as any).PM.Transcription.ensureModel('again'))).toBe(true);

    // Settings › Transcription.
    for (const mode of ['dark', 'light'] as const) {
      await theme(page, mode);
      await page.evaluate(() => (window as any).PM.SettingsUI.open('transcription'));
      const pageRegion = page.locator('#settings-page-transcription');
      await expect(pageRegion.getByRole('heading', { name: 'Transcription' })).toBeVisible();
      await expect(pageRegion.locator('[data-model="parakeet-tdt-0.6b-v3"]').getByText('In use')).toBeVisible();
      await expect(pageRegion.locator('[data-model="parakeet-tdt-0.6b-v2"]').getByRole('button', { name: 'Download' })).toBeVisible();
      await pageRegion.scrollIntoViewIfNeeded();
      await page.locator('#settings-tab-transcription').click();
      await shot(page, `settings-${mode}`);
      await page.evaluate(() => (window as any).PM.SettingsUI.close());
    }
    await page.evaluate(() => (window as any).PM.SettingsUI.open('transcription'));
    const region = page.locator('#settings-page-transcription');
    await expect(region.getByText(/MB in Powermove’s data folder/)).toBeVisible();
    // Language: the active model's languages.
    const language = region.getByRole('combobox', { name: 'Spoken language' });
    await region.locator('select[aria-label="Spoken language"]').selectOption('de', { force: true });
    await expect(language).toHaveText(/German/);
    await expect.poll(() => page.evaluate(async () => (await (window as any).PM.Transcription.status()).language)).toBe('de');
    // A second model downloads in place and can be put to use.
    await region.locator('[data-model="parakeet-tdt-0.6b-v2"]').getByRole('button', { name: 'Download' }).click();
    await expect(region.locator('[data-model="parakeet-tdt-0.6b-v2"]').getByRole('button', { name: 'Use' })).toBeVisible({ timeout: 20_000 });
    await region.locator('[data-model="parakeet-tdt-0.6b-v2"]').getByRole('button', { name: 'Use' }).click();
    await expect(region.locator('[data-model="parakeet-tdt-0.6b-v2"]').getByText('In use')).toBeVisible();
    await expect(language).toBeDisabled();
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally {
    await session?.close();
    server.closeAllConnections();
    await new Promise<void>((resolve) => server.close(() => resolve()));
    await rm(home, { recursive: true, force: true });
  }
});

test('main asks the focused editor for a model with a reason', async ({ session }) => {
  await session.openEditor();
  await session.app.evaluate(({ BrowserWindow }) => {
    for (const window of BrowserWindow.getAllWindows()) window.webContents.send('transcription:model-requested', { reason: 'Transcribe needs a speech model.' });
  });
  const sheet = session.page.locator('.transcription-modal');
  await expect(sheet.getByText('Transcribe needs a speech model.')).toBeVisible();
  await session.page.keyboard.press('Escape');
  await expect(sheet).toBeHidden();
});

test('a real Parakeet model transcribes speech in the utility process', async () => {
  test.skip(!parakeetDir || !existsSync(path.join(parakeetDir, 'encoder.int8.onnx')), 'POWERMOVE_E2E_PARAKEET_DIR is not set');
  test.setTimeout(180_000);
  const home = await mkdtemp(path.join(os.tmpdir(), 'pm-transcription-real-'));
  const files: Record<string, Hosted> = {};
  const { server, origin } = await serve(files);
  const catalogFile = await catalog(home, origin, files, true);
  let session: LaunchedApp | null = null;
  try {
    session = await launchApp({ userData: path.join(home, 'profile'), env: { POWERMOVE_TEST_TRANSCRIPTION_CATALOG: catalogFile } });
    await session.openEditor();
    const page = session.page;
    await page.evaluate(() => { void (window as any).PM.Transcription.ensureModel('Transcribe the test clip.'); });
    await page.locator('.transcription-modal').getByRole('button', { name: 'Download' }).click();
    await expect(page.locator('.transcription-modal')).toBeHidden({ timeout: 120_000 });
    const clip = path.join(home, 'speech.aiff');
    const { execFileSync } = await import('node:child_process');
    execFileSync('/usr/bin/say', ['-o', clip, 'The quick brown fox jumps over the lazy dog.']);
    const result = await page.evaluate(async (file) => {
      const progress: number[] = [];
      const started = performance.now();
      const transcript = await (window as any).PM.Transcription.transcribe({ path: file }, (value: number) => progress.push(value));
      return { transcript, progress, ms: performance.now() - started };
    }, clip);
    const words = result.transcript.segments.flatMap((segment: any) => segment.words);
    expect(result.transcript.segments.map((segment: any) => segment.text).join(' ')).toMatch(/quick brown fox jumps over the lazy dog/i);
    expect(words.length).toBeGreaterThanOrEqual(9);
    expect(words[0].start).toBeLessThan(0.6);
    for (let index = 1; index < words.length; index++) expect(words[index].start).toBeGreaterThanOrEqual(words[index - 1].end);
    expect(result.progress.at(-1)).toBe(1);
    // Asking again answers from the cache.
    const again = await page.evaluate(async (file) => {
      const started = performance.now();
      await (window as any).PM.Transcription.transcribe({ path: file });
      return performance.now() - started;
    }, clip);
    expect(again).toBeLessThan(500);
    console.log(`[transcription e2e] ${result.transcript.duration}s in ${Math.round(result.ms)} ms (cold, includes model load); cached ${Math.round(again)} ms`);
  } finally {
    await session?.close();
    server.closeAllConnections();
    await new Promise<void>((resolve) => server.close(() => resolve()));
    await rm(home, { recursive: true, force: true });
  }
});

test('onboarding starts the chosen model download and it finishes after onboarding', async () => {
  test.setTimeout(120_000);
  const home = await mkdtemp(path.join(os.tmpdir(), 'pm-transcription-onb-'));
  let release!: () => void;
  const gate = new Promise<void>((resolve) => { release = resolve; });
  const files: Record<string, Hosted> = {};
  const { server, origin } = await serve(files, gate);
  const catalogFile = await catalog(home, origin, files, false);
  const env: Record<string, string> = { ...process.env as Record<string, string>, POWERMOVE_USER_DATA: path.join(home, 'profile'), POWERMOVE_DEVTOOLS: '0',
    POWERMOVE_BACKGROUND_TEST: '1', POWERMOVE_TEST_ONBOARDING: '1', POWERMOVE_TEST_TRANSCRIPTION_CATALOG: catalogFile,
    CODEX_BINARY: path.join(repoRoot, 'src/main/codex/__fixtures__/fake-codex-app-server.sh'), POWERMOVE_FAKE_CHATGPT_STATUS: 'connected' };
  delete env.ELECTRON_RUN_AS_NODE;
  const app = await _electron.launch({ args: ['.'], cwd: repoRoot, env });
  try {
    const find = async (pred: (url: string) => boolean): Promise<Page> => {
      for (let i = 0; i < 100; i++) {
        const w = app.windows().find(p => pred(p.url()));
        if (w) return w;
        await new Promise(r => setTimeout(r, 200));
      }
      throw new Error('window not found');
    };
    const animation = await find(url => /onboarding/.test(url) && !/welcome/.test(url));
    await animation.waitForLoadState('domcontentloaded');
    await animation.evaluate(() => (window as any).onboarding.animationComplete());
    const welcome = await find(url => /welcome/.test(url));
    const errors: string[] = [];
    welcome.on('pageerror', e => errors.push(String(e)));
    await welcome.waitForSelector('#begin');
    await welcome.click('#begin');
    await welcome.locator('#timeline-step').getByRole('button', { name: 'Continue' }).click();
    await welcome.locator('#agent-step').getByRole('button', { name: 'Continue' }).click();
    const step = welcome.locator('#transcription-step');
    await expect(welcome.locator('#transcription-title')).toBeFocused();
    await expect(step.getByText('Parakeet V2')).toBeVisible();
    for (const mode of ['light', 'dark']) {
      await welcome.evaluate((value) => { document.documentElement.dataset.theme = value; }, mode);
      await welcome.waitForTimeout(900);
      if (shots) await welcome.screenshot({ path: path.join(shots, `onboarding-${mode}.png`) });
    }
    // Download, go back, then decline: nothing of that download stays on disk.
    const partial = path.join(home, 'profile', 'models', 'transcription', '.partial', 'parakeet-tdt-0.6b-v3');
    const v3State = () => welcome.evaluate(async () => (await (window as any).onboarding.transcription.status()).models[0].state);
    await step.getByRole('button', { name: 'Download' }).click();
    await expect(welcome.locator('#workspace-title')).toBeFocused();
    await expect.poll(v3State).toBe('downloading');
    await expect.poll(() => existsSync(partial)).toBe(true);
    await welcome.locator('#workspace-step').getByRole('button', { name: /Back/ }).click();
    await expect(welcome.locator('#transcription-title')).toBeFocused();
    await step.getByRole('button', { name: 'Not now' }).click();
    await expect(welcome.locator('#workspace-title')).toBeFocused();
    await expect.poll(v3State).toBe('absent');
    await expect.poll(() => existsSync(partial)).toBe(false);
    // Changing course again: download after all, and it outlives onboarding.
    await welcome.locator('#workspace-step').getByRole('button', { name: /Back/ }).click();
    await step.getByRole('button', { name: 'Download' }).click();
    await expect(welcome.locator('#workspace-title')).toBeFocused();
    await welcome.getByRole('button', { name: 'Start fresh' }).click();
    const editor = await find(url => !/onboarding/.test(url) && url.startsWith('app:'));
    await editor.waitForFunction(() => Boolean((window as any).PM?.Transcription));
    // Still downloading after onboarding has closed; finishing it makes the model active.
    expect((await editor.evaluate(() => (window as any).PM.Transcription.status())).models[0].state).toBe('downloading');
    release();
    await expect.poll(() => editor.evaluate(async () => (await (window as any).PM.Transcription.status()).activeModelId), { timeout: 20_000 }).toBe('parakeet-tdt-0.6b-v3');
    expect(errors).toEqual([]);
  } finally {
    await app.close().catch(() => undefined);
    server.closeAllConnections();
    await new Promise<void>((resolve) => server.close(() => resolve()));
    await rm(home, { recursive: true, force: true });
  }
});
