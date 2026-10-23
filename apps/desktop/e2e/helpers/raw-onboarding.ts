import { rm } from 'node:fs/promises';
import type { Server } from 'node:http';
import type { ElectronApplication } from 'playwright';
import { test as base } from './app';
import { closeTestProcess, SHUTDOWN_MS } from './close';

interface RawOnboardingResources {
  root?: string;
  app?: ElectronApplication;
  server?: Server;
  releaseDownload?: () => void;
}

// Raw onboarding launches cannot use the editor-ready session fixture. Give
// their owned resources the same independent native-shutdown cleanup budget.
export const test = base.extend<{ onboardingResources: RawOnboardingResources }>({
  onboardingResources: [async ({}, use, testInfo) => {
    const owned: RawOnboardingResources = {};
    try {
      await use(owned);
    } finally {
      const failures: unknown[] = [];
      const started = performance.now();
      let closedAt = started;
      let forced: boolean | null = null;
      try {
        if (owned.app) {
          forced = await closeTestProcess(owned.app);
          if (forced) failures.push(new Error(`Electron shutdown exceeded ${SHUTDOWN_MS} ms; terminated the test process for cleanup.`));
        }
      } catch (error) {
        failures.push(error);
      } finally {
        closedAt = performance.now();
        try {
          owned.releaseDownload?.();
          if (owned.server) {
            const server = owned.server;
            let timer: ReturnType<typeof setTimeout> | undefined;
            try {
              const closing = new Promise<void>((resolve, reject) => server.close(error => error ? reject(error) : resolve()));
              // Stop accepting new connections before aborting existing ones.
              server.closeAllConnections();
              await Promise.race([
                closing,
                new Promise<never>((_, reject) => { timer = setTimeout(() => reject(new Error('Test download server shutdown exceeded 5000 ms.')), 5_000); }),
              ]);
            } finally { clearTimeout(timer); }
          }
        } catch (error) {
          failures.push(error);
        } finally {
          try {
            if (owned.root) await rm(owned.root, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
          } catch (error) {
            failures.push(error);
          }
        }
      }
      try {
        await testInfo.attach('session-teardown', {
          body: Buffer.from(JSON.stringify({ appCloseMs: Math.round(closedAt - started), cleanupMs: Math.round(performance.now() - closedAt), forced })),
          contentType: 'application/json',
        });
      } catch (error) {
        failures.push(error);
      }
      if (failures.length === 1) throw failures[0];
      if (failures.length) throw new AggregateError(failures, failures.map(error => error instanceof Error ? error.stack ?? error.message : String(error)).join('\n\n'));
    }
  }, { timeout: 60_000 }],
});
