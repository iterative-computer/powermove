import net, { type Socket } from 'node:net';
import { randomBytes, randomUUID, timingSafeEqual } from 'node:crypto';
import { mkdir, writeFile, readFile, rename, rm } from 'node:fs/promises';
import path from 'node:path';
import { ExternalAgentSession, mcpToolResult } from './external-session';

/** Local-only discovery; its credential never belongs in MCP client config. */
export async function startExternalMcp(options: { userData: string; createSession(): ExternalAgentSession }) {
  const token = randomBytes(32).toString('hex'), instanceId = randomUUID();
  const sockets = new Set<Socket>(), cleanups = new Set<Promise<void>>();
  const server = net.createServer(socket => {
    sockets.add(socket); socket.setEncoding('utf8'); socket.on('error', () => socket.destroy());
    const session = options.createSession();
    let input = '', authorized = false, queued = 0, chain = Promise.resolve();
    socket.setTimeout(10000, () => socket.destroy());
    socket.on('data', chunk => {
      input += chunk;
      if (Buffer.byteLength(input) > 2 * 1024 * 1024) { socket.destroy(); return; }
      for (;;) {
        const end = input.indexOf('\n'); if (end < 0) break;
        const line = input.slice(0, end); input = input.slice(end + 1);
        if (++queued > 32) { socket.destroy(); return; }
        chain = chain.then(async () => {
          let id: string | number | null = null;
          try {
            const message = JSON.parse(line); id = message.id ?? null;
            if (!authorized) {
              if (message.method !== 'connect' || typeof message.token !== 'string' || Buffer.byteLength(message.token) !== Buffer.byteLength(token)
                || !timingSafeEqual(Buffer.from(message.token), Buffer.from(token))) throw new Error('Unauthorized Powermove MCP connection.');
              authorized = true; socket.setTimeout(0);
              socket.write(`${JSON.stringify({ id, result: { connected: true } })}\n`); return;
            }
            if (message.method === 'tools/list') socket.write(`${JSON.stringify({ id, result: session.tools() })}\n`);
            else if (message.method === 'tools/call') {
              if (typeof message.name !== 'string' || !message.arguments || typeof message.arguments !== 'object' || Array.isArray(message.arguments)) throw new Error('Invalid tool arguments.');
              const result = mcpToolResult(await session.call(message.name, message.arguments));
              if (!socket.destroyed) socket.write(`${JSON.stringify({ id, result })}\n`);
            } else throw new Error('Unknown MCP bridge method.');
          } catch (error) {
            if (!socket.destroyed) socket.write(`${JSON.stringify({ id, error: error instanceof Error ? error.message : String(error) })}\n`);
            if (!authorized) socket.destroy();
          } finally { queued--; }
        });
      }
    });
    socket.once('close', () => {
      sockets.delete(socket);
      const cleanup = session.close().catch(() => {}).finally(() => cleanups.delete(cleanup));
      cleanups.add(cleanup);
    });
  });
  await new Promise<void>((resolve, reject) => {
    server.once('error', reject); server.listen(0, '127.0.0.1', () => { server.off('error', reject); resolve(); });
  });
  const address = server.address();
  if (!address || typeof address === 'string') throw new Error('MCP received no local port.');
  const file = path.join(options.userData, 'mcp', 'connection.json');
  const descriptor = { version: 1, instanceId, pid: process.pid, port: address.port, token };
  try {
    await mkdir(path.dirname(file), { recursive: true, mode: 0o700 });
    await writeFile(`${file}.${instanceId}.tmp`, JSON.stringify(descriptor), { mode: 0o600, flag: 'wx' });
    await rename(`${file}.${instanceId}.tmp`, file);
  } catch (error) { server.close(); await rm(`${file}.${instanceId}.tmp`, { force: true }); throw error; }
  let closing: Promise<void> | undefined;
  return { file, async close() {
    return closing ??= (async () => {
      for (const socket of sockets) socket.destroy();
      await new Promise<void>(resolve => server.close(() => resolve()));
      await Promise.all([...cleanups]);
      try { if (JSON.parse(await readFile(file, 'utf8')).instanceId === instanceId) await rm(file, { force: true }); }
      catch (error) { if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error; }
    })();
  } };
}
