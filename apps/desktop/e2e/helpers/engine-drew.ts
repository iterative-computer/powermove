/*
 * With "Use New Engine" on (POWERMOVE_NEW_ENGINE=1), a text spec passing is
 * evidence only if the engine drew its text: the editor's viewer session
 * routes to the engine (never to the legacy renderer), and the presenter
 * reports no layer it failed to draw. With the flag off this checks
 * nothing. A missing status API fails. Specs call it after each test (`test.afterEach`).
 */
import { expect } from '@playwright/test';

import type { LaunchedApp } from './app';

export async function expectEngineDrewText(session: Pick<LaunchedApp, 'app' | 'page'>): Promise<void> {
  if (process.env.POWERMOVE_NEW_ENGINE !== '1') return;
  // A missing or throwing status API fails the check: with the flag on the
  // connection is always installed, so null means the evidence is gone.
  const read = () => session.page.evaluate(() => (window as any).PM?.EngineConnection?.status?.() ?? null);
  // A test that just replaced the project leaves its session `pending`
  // until the engine has answered; wait for the answer.
  await expect.poll(async () => (await read())?.route ?? null, { message: 'the viewer session settles', timeout: 15_000 }).not.toBe('pending');
  const status = await read();
  expect(status, 'PM.EngineConnection.status() with the new engine on').not.toBeNull();
  // A test that never opened the editor has no viewer session to judge.
  if (status.route === 'none') return;
  expect(status.route, `the viewer routed: ${status.routeReason}`).toBe('engine');
  expect(status.failures, 'layers the engine refused').toEqual([]);
  const report = await session.app.evaluate(async () => (globalThis as any).__powermoveNativeEngine?.engine?.report?.('interactive') ?? null) as { presenter?: { layerFailures?: number; renderErrors?: number } } | null;
  expect(report?.presenter, 'main-process presenter report').toBeDefined();
  expect(report?.presenter?.layerFailures, 'layers the presenter failed to draw').toBe(0);
  expect(report?.presenter?.renderErrors, 'presenter render errors').toBe(0);
}
