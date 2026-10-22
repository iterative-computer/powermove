import base from './playwright.config';

/** Run only the sustained Phase 4a app measurements, with enough suite time
 * for two paired 60-second sources and the uninterrupted one-hour memory row. */
export default {
  ...base,
  testMatch: '**/engine-proxy-performance.spec.ts',
  // Build before taking the Electron lease; the ordinary setup always rebuilds.
  globalSetup: undefined,
  globalTimeout: 4_800_000,
  timeout: 3_800_000,
  workers: 1,
};
