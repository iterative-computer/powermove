import { createServer, type Server, type IncomingMessage, type ServerResponse } from 'node:http';
import { randomBytes, randomUUID, createHash, timingSafeEqual } from 'node:crypto';
import { mkdir, readFile, rename, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { MAX_PAYLOAD, validateFigmaDoc, type FigmaDoc } from '../shared/figma/doc';
import { FIGMA_PROTOCOL_VERSION, type FigmaSetupState } from '../shared/figma-link';
export const FIGMA_PORT = 47832;
export const FIGMA_APPROVAL_MS = 60000;
export const FIGMA_DENY_COOLDOWN_MS = 10000;
const digest = (token: string) => createHash('sha256').update(token).digest('hex');
type Options = {
  storagePath?: string;
  compositionOpen?: () => Promise<boolean>;
  changed?: (state: FigmaSetupState) => void;
  approvalMs?: number;
  cooldownMs?: number;
  now?: () => number;
};
type Pending = NonNullable<FigmaSetupState['pending']> & {
  resolve: (result: { status: number; body: unknown }) => void;
  timer: ReturnType<typeof setTimeout>;
};
/** Off by default. Only the open, trusted app sheet can approve a connection.
 * Token plaintext exists only long enough to return it to the approved plugin.
 * Disk, logs, status and project files never receive plaintext tokens. */
export class FigmaLink {
  private server?: Server;
  private starting?: Promise<{ port: number }>;
  private loading?: Promise<void>;
  private saving: Promise<void> = Promise.resolve();
  private hashes = new Set<string>();
  private owner?: string;
  private pending?: Pending;
  private cooldownUntil = 0;
  private deciding = false;
  private busy = false;
  private error?: string;
  private now: () => number;
  constructor(
    private send: (doc: FigmaDoc) => Promise<unknown>,
    private options: Options = {},
  ) {
    this.now = options.now ?? Date.now;
  }
  get paired(): boolean {
    return this.hashes.size > 0;
  }
  get state(): FigmaSetupState {
    const p = this.pending;
    return {
      running: !!this.server?.listening,
      paired: this.paired,
      pending: p ? { id: p.id, code: p.code, expiresAt: p.expiresAt } : null,
      ...(this.error ? { error: this.error } : {}),
    };
  }
  private changed(): void {
    this.options.changed?.(this.state);
  }
  async restore(): Promise<void> {
    this.loading ??= (async () => {
      if (!this.options.storagePath) return;
      try {
        const value = JSON.parse(await readFile(this.options.storagePath, 'utf8'));
        if (value.version === 1 && Array.isArray(value.tokenHashes))
          this.hashes = new Set(
            value.tokenHashes.filter((v: unknown) => typeof v === 'string' && /^[a-f0-9]{64}$/.test(v)).slice(0, 16),
          );
      } catch {
        /* Missing or invalid pairing data leaves the listener disabled. */
      }
    })();
    await this.loading;
  }
  private save(): Promise<void> {
    const storagePath = this.options.storagePath,
      hashes = [...this.hashes];
    const saved = this.saving
      .catch(() => {})
      .then(async () => {
        if (!storagePath) return;
        await mkdir(path.dirname(storagePath), { recursive: true });
        if (!hashes.length) {
          await rm(storagePath, { force: true });
          return;
        }
        const temp = storagePath + '.tmp';
        await writeFile(temp, JSON.stringify({ version: 1, tokenHashes: hashes }), { mode: 0o600 });
        await rename(temp, storagePath);
      });
    this.saving = saved;
    return saved;
  }
  async start(port = FIGMA_PORT): Promise<{ port: number }> {
    if (this.starting) return this.starting;
    this.starting = this.listen(port).finally(() => {
      this.starting = undefined;
    });
    return this.starting;
  }
  private async listen(port: number): Promise<{ port: number }> {
    await this.restore();
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
      } catch (error) {
        this.server = undefined;
        this.error =
          (error as NodeJS.ErrnoException).code === 'EADDRINUSE'
            ? 'Another copy of Powermove is using Figma. Close it and try again.'
            : 'Could not connect to Figma. Try again.';
        this.changed();
        throw error;
      }
    }
    this.error = undefined;
    this.changed();
    return { port: (this.server.address() as { port: number }).port };
  }
  openSetup(owner: string): void {
    if (this.owner && this.owner !== owner) this.finish(403, { error: 'Open File → Connect Figma in Powermove.' });
    this.owner = owner;
    this.changed();
  }
  closeSetup(owner: string): void {
    if (owner !== this.owner) return;
    this.owner = undefined;
    this.finish(403, { error: 'In Powermove, choose File → Connect Figma.' });
    this.changed();
  }
  private finish(status: number, body: unknown): void {
    const pending = this.pending;
    if (!pending) return;
    this.pending = undefined;
    clearTimeout(pending.timer);
    pending.resolve({ status, body });
    this.changed();
  }
  async approve(owner: string, id: string, allow: boolean): Promise<void> {
    if (this.deciding || owner !== this.owner || this.pending?.id !== id)
      throw new Error('This Figma connection request is no longer open.');
    if (this.now() >= this.pending.expiresAt) {
      this.finish(408, { error: 'Connection request expired. Click Connect again.' });
      return;
    }
    if (!allow) {
      this.cooldownUntil = this.now() + (this.options.cooldownMs ?? FIGMA_DENY_COOLDOWN_MS);
      this.finish(403, { error: 'Connection declined. Try again in a moment.' });
      return;
    }
    clearTimeout(this.pending.timer);
    this.deciding = true;
    const token = randomBytes(32).toString('hex'),
      hash = digest(token);
    try {
      this.hashes.add(hash);
      await this.save();
      this.finish(200, { token });
    } catch {
      this.hashes.delete(hash);
      this.finish(500, { error: 'Could not save this connection. Try again.' });
    } finally {
      this.deciding = false;
      this.changed();
    }
  }
  /** Revokes every token but keeps listening until quit, so an open plugin
   * can tell "disconnected" (Connect again) from "Powermove is closed".
   * Without a pairing the listener does not auto-start on the next launch. */
  async disconnect(): Promise<void> {
    await this.restore();
    this.hashes.clear();
    this.finish(403, { error: 'In Powermove, choose File → Connect Figma.' });
    await this.save();
    this.changed();
  }
  async close(): Promise<void> {
    await this.starting?.catch(() => {});
    this.owner = undefined;
    this.finish(503, { error: 'Open Powermove to continue.' });
    const server = this.server;
    this.server = undefined;
    if (server)
      await new Promise<void>((resolve) => {
        server.close(() => resolve());
        server.closeAllConnections();
      });
    this.changed();
  }
  private valid(token: string | undefined): boolean {
    if (!token || !/^[a-f0-9]{64}$/.test(token)) return false;
    const value = Buffer.from(digest(token), 'hex');
    return [...this.hashes].some((hash) => timingSafeEqual(value, Buffer.from(hash, 'hex')));
  }
  private async request(req: IncomingMessage, res: ServerResponse): Promise<void> {
    const reply = (status: number, body: unknown) => {
      if (res.destroyed || res.writableEnded) return;
      res.writeHead(status, { 'Content-Type': 'application/json', 'Cache-Control': 'no-store' });
      res.end(JSON.stringify(body));
    };
    if (!/^localhost:\d+$|^127\.0\.0\.1:\d+$/.test(req.headers.host ?? ''))
      return reply(403, { error: 'Invalid host' });
    const origin = req.headers.origin;
    if (origin && !['null', 'https://www.figma.com', 'https://figma.com'].includes(origin))
      return reply(403, { error: 'Invalid origin' });
    if (origin) {
      res.setHeader('Access-Control-Allow-Origin', origin);
      res.setHeader('Vary', 'Origin');
    }
    res.setHeader('Access-Control-Allow-Methods', 'POST, OPTIONS');
    res.setHeader('Access-Control-Allow-Headers', 'Authorization, Content-Type');
    res.setHeader('Access-Control-Allow-Private-Network', 'true');
    if (req.method === 'OPTIONS') return reply(204, {});
    if (req.method !== 'POST' || !['/pair', '/send', '/status'].includes(req.url ?? ''))
      return reply(404, { error: 'Unknown request' });
    const limit = req.url === '/send' ? MAX_PAYLOAD : 1024;
    if (Number(req.headers['content-length']) > limit)
      return reply(413, { error: 'Selection is too large. Select fewer layers.' });
    const token = req.headers.authorization?.replace(/^Bearer /, '');
    if (req.url === '/send' && !this.valid(token)) return reply(401, { error: 'Connect to Powermove first.' });
    if (!req.headers['content-type']?.startsWith('application/json')) return reply(415, { error: 'JSON required' });
    try {
      const chunks: Buffer[] = [];
      let size = 0;
      for await (const chunk of req) {
        size += chunk.length;
        if (size > limit) return reply(413, { error: 'Selection is too large. Select fewer layers.' });
        chunks.push(chunk);
      }
      const value = JSON.parse(Buffer.concat(chunks).toString('utf8'));
      if (req.url === '/status') {
        if (!value || typeof value !== 'object' || Array.isArray(value) || Object.keys(value).length)
          return reply(400, { error: 'Invalid status request' });
        const tokenValid = this.valid(token);
        return reply(200, {
          reachable: true,
          tokenValid,
          compositionOpen: tokenValid ? ((await this.options.compositionOpen?.()) ?? false) : false,
          protocolVersion: FIGMA_PROTOCOL_VERSION,
        });
      }
      if (req.url === '/pair') {
        if (
          !value ||
          Object.keys(value).join(',') !== 'code' ||
          typeof value.code !== 'string' ||
          !/^\d{6}$/.test(value.code)
        )
          return reply(400, { error: 'Invalid connection request' });
        if (!this.owner) return reply(403, { error: 'In Powermove, choose File → Connect Figma.' });
        if (this.now() < this.cooldownUntil)
          return reply(429, { error: 'Connection declined. Try again in a moment.' });
        if (this.pending || this.deciding || this.hashes.size >= 16)
          return reply(409, { error: 'Finish the current connection request first.' });
        const id = randomUUID(),
          expiresAt = this.now() + (this.options.approvalMs ?? FIGMA_APPROVAL_MS);
        const result = await new Promise<{ status: number; body: unknown }>((resolve) => {
          const timer = setTimeout(
            () => this.finish(408, { error: 'Connection request expired. Click Connect again.' }),
            this.options.approvalMs ?? FIGMA_APPROVAL_MS,
          );
          this.pending = { id, code: value.code, expiresAt, timer, resolve };
          this.changed();
          res.once('close', () => {
            if (this.pending?.id === id) this.finish(408, { error: 'Connection request cancelled.' });
          });
        });
        return reply(result.status, result.body);
      }
      const doc = validateFigmaDoc(value);
      if (this.busy) return reply(409, { error: 'Finish the current Figma import first.' });
      this.busy = true;
      try {
        reply(200, await this.send(doc));
      } finally {
        this.busy = false;
      }
    } catch {
      reply(400, { error: 'Could not complete the Figma request. Try again.' });
    }
  }
}
