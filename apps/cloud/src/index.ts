import { createApp } from './app';
import { dataFor } from './db/client';
import { gc } from './gc';
const app = createApp({ data: dataFor });
export { AeroCall } from './aero/call';
export type { AppType } from './app';
export default { fetch: app.fetch, scheduled: gc, request: app.request.bind(app) };
