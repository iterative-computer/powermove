import { ensureBuilt } from './helpers/app';

export default async function globalSetup(): Promise<void> {
  // Callers that built the app before acquiring the Electron lock can skip
  // only this redundant build; the Playwright command still owns the lock.
  if (process.env.PM_E2E_SKIP_BUILD === '1') return;
  await ensureBuilt(true);
}
