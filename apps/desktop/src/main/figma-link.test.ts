import { it, expect } from 'vitest';
import { FigmaLink } from './figma-link';
import { MAX_PAYLOAD } from '../shared/figma/doc';
it('requires single-use pairing, validates sends, bounds payloads and revokes tokens', async () => {
  const received: any[] = [];
  const link = new FigmaLink(async (d) => {
    received.push(d);
    return { layers: 1, flattened: 0 };
  });
  const { port, code } = await link.start(0);
  const url = `http://127.0.0.1:${port}`;
  const post = (path: string, value: any, token?: string, extra: Record<string, string> = {}) =>
    fetch(url + path, {
      method: 'POST',
      headers: { 'content-type': 'application/json', ...(token ? { authorization: `Bearer ${token}` } : {}), ...extra },
      body: JSON.stringify(value),
    });
  try {
    expect((await post('/send', {})).status).toBe(401);
    expect((await post('/pair', { code: '000000' })).status).toBe(401);
    const paired = await post('/pair', { code });
    expect(paired.status).toBe(200);
    const { token } = (await paired.json()) as any;
    expect((await post('/pair', { code })).status).toBe(401);
    expect((await post('/send', { bad: true }, token)).status).toBe(400);
    expect((await post('/send', {}, token, { origin: 'https://evil.example' })).status).toBe(403);
    expect((await post('/send', { large: 'x'.repeat(MAX_PAYLOAD) }, token)).status).toBe(413);
    const doc = {
      format: 'powermove-figma',
      version: 1,
      source: 'plugin',
      nodes: [
        {
          id: '1',
          fileKey: 'synthetic',
          type: 'RECTANGLE',
          name: 'Card',
          w: 100,
          h: 60,
          absoluteTransform: [1, 0, 0, 1, 0, 0],
          fills: [],
          strokes: [],
          effects: [],
          children: [],
        },
      ],
      images: {},
    };
    expect((await post('/send', doc, token)).status).toBe(200);
    expect(received).toHaveLength(1);
    await link.close();
    await link.start(port);
    expect((await post('/send', doc, token)).status).toBe(401);
  } finally {
    await link.close();
  }
});
