import { createApp } from './app';
import { localData, neonData } from './db/client';
import { gc } from './gc';
const app = createApp({ data: env => env.LOCAL_POSTGRES === '1' ? localData(env.DATABASE_URL) : neonData(env.DATABASE_URL) });
export type { AppType } from './app';
export default { fetch: app.fetch, scheduled: gc, request: app.request.bind(app) };
