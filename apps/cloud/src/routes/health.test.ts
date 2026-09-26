import { describe, expect, test } from 'bun:test';
import app from '../index';

// `Response.json()` is typed by both bun and workerd here; read as unknown.
const json = async (res: Response): Promise<unknown> => res.json();

describe('health', () => {
  test('reports the service', async () => {
    const res = await app.request('/health');
    expect(res.status).toBe(200);
    expect(await json(res)).toEqual({ ok: true, service: 'powermove-cloud' });
  });

  test('unknown routes are 404 json', async () => {
    const res = await app.request('/nope');
    expect(res.status).toBe(404);
    expect(await json(res)).toEqual({ error: 'not_found' });
  });
});
