import { ensureBuilt } from './helpers/app';

export default async function globalSetup(): Promise<void> {
  await ensureBuilt(true);
}
