import { Hono } from 'hono';
import type { Env } from '../env';

export const health = new Hono<Env>().get('/', (c) =>
  c.json({ ok: true as const, service: 'powermove-cloud' as const }),
);
