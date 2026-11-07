import { test, expect } from '@playwright/test';
import { readFile } from 'node:fs/promises';
const html = () => readFile('../../packages/figma-plugin/ui.html', 'utf8');
test('plugin recovers from offline, approves, tracks selection, keeps results and drops rejected tokens', async ({
  page,
}) => {
  await page.clock.install();
  let reachable = false,
    valid = false,
    composition = true;
  let pair: any;
  await page.route('http://localhost:47832/**', async (route) => {
    const routeName = new URL(route.request().url()).pathname;
    if (!reachable) return route.abort();
    if (routeName === '/status')
      return route.fulfill({
        json: { reachable: true, tokenValid: valid, compositionOpen: composition, protocolVersion: 2 },
      });
    if (routeName === '/pair') {
      pair = route;
      return;
    }
    return route.fulfill({ json: { layers: 14, flattened: 2 } });
  });
  await page.setContent(await html());
  const host = (m: any) => page.evaluate((m) => window.postMessage({ pluginMessage: m }, '*'), m);
  await expect(page.locator('body')).toHaveAttribute('data-state', 'offline');
  await expect(page.getByText('Open Powermove to continue.')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Copy for Powermove' })).toBeVisible();
  reachable = true;
  await page.clock.runFor(2100);
  await expect(page.getByRole('button', { name: 'Connect', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Connect', exact: true }).click();
  await expect(page.getByLabel('Connection code')).toHaveText(/^[0-9]{6}$/);
  await expect.poll(() => !!pair).toBe(true);
  valid = true;
  await pair.fulfill({ json: { token: 'a'.repeat(64) } });
  await host({ type: 'count', count: 3 });
  await expect(page.getByRole('button', { name: 'Send 3 Layers' })).toBeEnabled();
  await page.getByRole('button', { name: 'Send 3 Layers' }).click();
  await host({ type: 'selection', doc: { format: 'synthetic' } });
  await expect(page.getByText('Sent 14 layers · 2 flattened')).toBeVisible();
  await page.clock.runFor(4100);
  await expect(page.getByText('Sent 14 layers · 2 flattened')).toBeVisible();
  await host({ type: 'count', count: 0 });
  await expect(page.getByRole('button', { name: 'Select layers in Figma' })).toBeDisabled();
  composition = false;
  await page.clock.runFor(2100);
  await expect(page.getByText('Open a composition in Powermove.')).toBeVisible();
  valid = false;
  await page.clock.runFor(2100);
  await expect(page.getByRole('button', { name: 'Connect', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: /Send [0-9]/ })).toHaveCount(0);
});
test('plugin explains a closed setup sheet and stops polling while hidden', async ({ page }) => {
  await page.clock.install();
  let count = 0;
  await page.route('http://localhost:47832/**', (route) => {
    if (route.request().url().endsWith('/pair'))
      return route.fulfill({ status: 403, json: { error: 'In Powermove, choose File → Connect Figma.' } });
    count++;
    return route.fulfill({ json: { reachable: true, tokenValid: false, compositionOpen: false, protocolVersion: 2 } });
  });
  await page.setContent(await html());
  await expect(page.getByRole('button', { name: 'Connect', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Connect', exact: true }).click();
  await expect(page.getByText('In Powermove, choose File → Connect Figma.')).toBeVisible();
  await page.evaluate(() => {
    Object.defineProperty(document, 'hidden', { configurable: true, value: true });
    document.dispatchEvent(new Event('visibilitychange'));
  });
  const before = count;
  await page.clock.runFor(5000);
  expect(count).toBe(before);
  await page.evaluate(() => {
    Object.defineProperty(document, 'hidden', { configurable: true, value: false });
    document.dispatchEvent(new Event('visibilitychange'));
  });
  await expect.poll(() => count).toBeGreaterThan(before);
});

test('accepts desktop Figma messages through its top frame and rejects other ancestors', async ({ page }) => {
  await page.route('http://localhost:47832/status', route => route.fulfill({
    json: { reachable: true, tokenValid: true, compositionOpen: true, protocolVersion: 2 },
  }));
  await page.setContent('<iframe></iframe>');
  const wrapper = page.frames()[1]!;
  await wrapper.setContent('<iframe></iframe>');
  const ui = wrapper.childFrames()[0]!;
  await ui.setContent(await html());
  expect(await ui.evaluate(() => parent !== top)).toBe(true);
  const host = (source: 'top' | 'parent' | 'self', origin: string, message: unknown) => ui.evaluate(
    ({ source, origin, message }) => window.dispatchEvent(new MessageEvent('message', {
      source: source === 'top' ? top : source === 'parent' ? parent : window,
      origin,
      data: { pluginMessage: message },
    })), { source, origin, message },
  );
  await host('top', 'https://www.figma.com', { type: 'count', count: 3 });
  await host('top', 'https://www.figma.com', { type: 'token', token: 'a'.repeat(64) });
  await expect(ui.getByRole('button', { name: 'Send 3 Layers', exact: true })).toBeEnabled();
  await host('top', 'https://untrusted.example', { type: 'count', count: 99 });
  await host('self', 'https://www.figma.com', { type: 'count', count: 98 });
  await expect(ui.getByRole('button', { name: 'Send 3 Layers', exact: true })).toBeEnabled();
  await host('parent', 'null', { type: 'count', count: 4 });
  await expect(ui.getByRole('button', { name: 'Send 4 Layers', exact: true })).toBeEnabled();
});
