import { createApp } from './app';
import { neonData } from './db/client';
import { gc } from './gc';
const app = createApp({ data: env => neonData(env.DATABASE_URL) });
export type { AppType } from './app';
export default { fetch: app.fetch, scheduled: gc, request: app.request.bind(app) };
