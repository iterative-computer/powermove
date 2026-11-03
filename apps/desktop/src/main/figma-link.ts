import { createServer, type Server } from 'node:http';
import { randomBytes, randomInt, timingSafeEqual } from 'node:crypto';
import { MAX_PAYLOAD, validateFigmaDoc, type FigmaDoc } from '../shared/figma/doc';
export const FIGMA_PORT = 47832;
/** Explicitly enabled, loopback-only, ephemeral pairing. Tokens/code never go
 * into project files, preferences, logs or URLs. Restart/disable revokes them. */
export class FigmaLink {
  private server: Server | undefined;
  private code = '';
  private expires = 0;
  private attempts = 0;
  private tokens = new Set<string>();
  private busy = false;
  constructor(private send: (doc: FigmaDoc) => Promise<unknown>) {}
  async start(port = FIGMA_PORT): Promise<{ code: string; port: number }> {
    if (!this.server) {
      const server = createServer((req, res) => {
        void this.request(req, res);
      });
      this.server = server;
      server.maxConnections = 8;
      server.requestTimeout = 15000;
      server.headersTimeout = 10000;
      try {
        await new Promise<void>((resolve, reject) => {
          server.once('error', reject);
          server.listen(port, '127.0.0.1', () => {
            server.off('error', reject);
            resolve();
          });
        });
      } catch (e) {
        this.server = undefined;
        throw e;
      }
    }
    this.code = String(randomInt(100000, 1000000));
    this.expires = Date.now() + 5 * 60 * 1000;
    this.attempts = 0;
    return { code: this.code, port: (this.server.address() as { port: number }).port };
  }
  async close(): Promise<void> {
    this.tokens.clear();
    this.code = '';
    const server = this.server;
    this.server = undefined;
    if (server)
      await new Promise<void>((resolve) => {
        server.close(() => resolve());
        server.closeAllConnections();
      });
  }
  private async request(req: import('node:http').IncomingMessage, res: import('node:http').ServerResponse) {
    const reply = (status: number, data: unknown) => {
      res.writeHead(status, { 'Content-Type': 'application/json', 'Cache-Control': 'no-store' });
      res.end(JSON.stringify(data));
    };
    const host = req.headers.host ?? '';
    if (!/^localhost:\d+$|^127\.0\.0\.1:\d+$/.test(host)) return reply(403, { error: 'Invalid host' });
    const origin = req.headers.origin;
    if (origin && origin !== 'null' && origin !== 'https://www.figma.com' && origin !== 'https://figma.com')
      return reply(403, { error: 'Invalid origin' });
    if (origin) {
      res.setHeader('Access-Control-Allow-Origin', origin);
      res.setHeader('Vary', 'Origin');
    }
    res.setHeader('Access-Control-Allow-Methods', 'POST, OPTIONS');
    res.setHeader('Access-Control-Allow-Headers', 'Authorization, Content-Type');
    res.setHeader('Access-Control-Allow-Private-Network', 'true');
    if (req.method === 'OPTIONS') return reply(204, {});
    if (req.method !== 'POST' || !['/pair', '/send'].includes(req.url ?? ''))
      return reply(404, { error: 'Unknown request' });
    if (Number(req.headers['content-length']) > MAX_PAYLOAD) return reply(413, { error: 'Selection is too large' });
    const token = req.headers.authorization?.replace(/^Bearer /, '');
    if (req.url === '/send' && (!token || !this.tokens.has(token)))
      return reply(401, { error: 'Pair with Powermove first' });
    if (!req.headers['content-type']?.startsWith('application/json')) return reply(415, { error: 'JSON required' });
    try {
      const limit = req.url === '/pair' ? 1024 : MAX_PAYLOAD;
      const chunks: Buffer[] = [];
      let size = 0;
      for await (const chunk of req) {
        size += chunk.length;
        if (size > limit) {
          reply(413, { error: 'Selection is too large' });
          return;
        }
        chunks.push(chunk);
      }
      const value = JSON.parse(Buffer.concat(chunks).toString('utf8'));
      if (req.url === '/pair') {
        if (++this.attempts > 10) return reply(429, { error: 'Reopen Connect Figma in Powermove for a new code' });
        if (
          !value ||
          Object.keys(value).join(',') !== 'code' ||
          typeof value.code !== 'string' ||
          !/^\d{6}$/.test(value.code) ||
          Date.now() > this.expires ||
          !this.code ||
          !timingSafeEqual(Buffer.from(value.code), Buffer.from(this.code))
        )
          return reply(401, { error: 'Incorrect or expired pairing code' });
        const paired = randomBytes(32).toString('hex');
        this.tokens.add(paired);
        this.code = '';
        return reply(200, { token: paired });
      }
      const doc = validateFigmaDoc(value);
      if (this.busy) return reply(409, { error: 'Finish the current Figma import first' });
      this.busy = true;
      try {
        const result = await this.send(doc);
        reply(200, result);
      } finally {
        this.busy = false;
      }
    } catch (e) {
      reply(400, { error: e instanceof Error ? e.message.slice(0, 300) : 'Invalid Figma document' });
    }
  }
}
