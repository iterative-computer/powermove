import { Hono } from 'hono';
import { HTTPException } from 'hono/http-exception';
import type { Env } from './env';
import { health } from './routes/health';

const app = new Hono<Env>();

app.notFound((c) => c.json({ error: 'not_found' }, 404));

app.onError((err, c) => {
  if (err instanceof HTTPException) return err.getResponse();
  console.error(err);
  return c.json({ error: 'internal' }, 500);
});

// Routes are chained so `AppType` carries every route for the typed client.
const routes = app.route('/health', health);

export type AppType = typeof routes;

export default app;
