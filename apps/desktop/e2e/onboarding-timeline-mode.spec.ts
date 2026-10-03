import { mkdtemp, rm, readFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { _electron, type Page } from 'playwright';
import { expect, test, repoRoot } from './helpers/app';

test('onboarding lets a new user choose the track timeline before opening the editor', async () => {
  test.setTimeout(90_000);
  const userData = await mkdtemp(path.join(os.tmpdir(), 'pm-onb-'));
  const env: Record<string, string> = { ...process.env as any, POWERMOVE_USER_DATA: userData, POWERMOVE_DEVTOOLS: '0', POWERMOVE_BACKGROUND_TEST: '1', POWERMOVE_TEST_ONBOARDING: '1',
    CODEX_BINARY: path.join(repoRoot, 'src/main/codex/__fixtures__/fake-codex-app-server.sh'), POWERMOVE_FAKE_CHATGPT_STATUS: 'connected' };
  delete env.ELECTRON_RUN_AS_NODE;
  const app = await _electron.launch({ args: ['.'], cwd: repoRoot, env });
  const errors: string[] = [];
  try {
    const find = async (pred: (url: string) => boolean): Promise<Page> => {
      for (let i = 0; i < 100; i++) {
        const w = app.windows().find(p => pred(p.url()));
        if (w) return w;
        await new Promise(r => setTimeout(r, 200));
      }
      throw new Error('window not found: ' + app.windows().map(w => w.url()).join(','));
    };
    const animation = await find(url => /onboarding/.test(url) && !/welcome/.test(url));
    await animation.waitForLoadState('domcontentloaded');
    await animation.evaluate(() => (window as any).onboarding.animationComplete());
    const welcome = await find(url => /welcome/.test(url));
    welcome.on('pageerror', e => errors.push(String(e)));
    await welcome.waitForSelector('#begin');
    await welcome.click('#begin');
    await expect(welcome.locator('#timeline-title')).toBeFocused();
    await welcome.getByText('Tracks', { exact: true }).click();
    await welcome.getByRole('button', { name: 'Continue' }).click();
    await expect(welcome.locator('#agent-title')).toBeFocused();
    await welcome.getByText('Claude', { exact: true }).click();
    await welcome.getByLabel('Claude model').selectOption('claude-sonnet-5-5');
    await welcome.locator('#agent-step').getByRole('button', { name: 'Continue' }).click();
    await expect(welcome.locator('#workspace-title')).toBeFocused();
    await welcome.getByRole('button', { name: 'Back to agent choice' }).click();
    await expect(welcome.locator('input[value="claude"]')).toBeChecked();
    await welcome.getByRole('button', { name: 'Back to timeline choice' }).click();
    await expect(welcome.locator('input[value="tracks"]')).toBeChecked();
    await welcome.locator('#timeline-step').getByRole('button', { name: 'Continue' }).click();
    await welcome.locator('#agent-step').getByRole('button', { name: 'Continue' }).click();
    await welcome.getByRole('button', { name: 'Start fresh' }).click();
    const editor = await find(url => !/onboarding/.test(url) && url.startsWith('app:'));
    await editor.waitForFunction(() => Boolean((window as any).PM?.Kernel?.services?.get?.('timeline')), undefined, { timeout: 30000 });
    const mode = await editor.evaluate(() => (window as any).PM.Kernel.services.get('timeline').mode);
    const saved = JSON.parse(await readFile(path.join(userData, 'onboarding-v1.json'), 'utf8'));
    expect(mode).toBe('tracks');
    expect(saved.timelineMode).toBe('tracks');
    expect(saved.agent).toEqual({ provider: 'claude', model: 'claude-sonnet-5-5' });
    const agent = await editor.evaluate(() => { const { provider, model } = (window as any).PM.AgentUI.state; return { provider, model }; });
    expect(agent).toEqual({ provider: 'claude', model: 'claude-sonnet-5-5' });
    expect(errors).toEqual([]);
  } finally {
    await app.close().catch(() => undefined);
    await rm(userData, { recursive: true, force: true });
  }
});
