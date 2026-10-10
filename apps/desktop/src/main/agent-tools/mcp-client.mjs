import net from 'node:net';
import { readFile } from 'node:fs/promises';
import path from 'node:path';

export async function connectMcpHost(userData) {
  let descriptor;
  try { descriptor = JSON.parse(await readFile(path.join(userData, 'mcp', 'connection.json'), 'utf8')); }
  catch { throw new Error('No Powermove MCP host is running in this profile. Open Powermove, or use powermove mcp to start a hidden host.'); }
  if (descriptor.version !== 1 || !Number.isInteger(descriptor.port) || descriptor.port < 1 || descriptor.port > 65535 || !/^[a-f0-9]{64}$/.test(descriptor.token)) throw new Error('Invalid Powermove MCP connection file. Restart the host.');
  const socket = net.createConnection({ host: '127.0.0.1', port: descriptor.port });
  socket.setEncoding('utf8');
  let nextId = 0, buffer = '';
  const pending = new Map();
  const fail = () => { for (const call of pending.values()) call.reject(new Error('Powermove MCP host disconnected. Inspect the project before repeating edits.')); pending.clear(); };
  socket.on('error', fail); socket.on('close', fail);
  socket.on('data', chunk => {
    buffer += chunk;
    if (Buffer.byteLength(buffer) > 64 * 1024 * 1024) { socket.destroy(); return; }
    for (;;) {
      const end = buffer.indexOf('\n'); if (end < 0) break;
      const line = buffer.slice(0, end); buffer = buffer.slice(end + 1);
      try {
        const message = JSON.parse(line), call = pending.get(message.id);
        if (!call) continue;
        pending.delete(message.id);
        if (message.error) call.reject(new Error(message.error)); else call.resolve(message.result);
      } catch { socket.destroy(); return; }
    }
  });
  const call = (method, args = {}) => new Promise((resolve, reject) => {
    if (socket.destroyed) { reject(new Error('Powermove MCP host is closed.')); return; }
    const id = ++nextId; pending.set(id, { resolve, reject });
    socket.write(`${JSON.stringify({ id, method, ...args })}\n`);
  });
  try {
    await new Promise((resolve, reject) => { socket.once('connect', resolve); socket.once('error', reject); });
    await call('connect', { token: descriptor.token });
  } catch (error) { socket.destroy(); throw error; }
  return { listTools: () => call('tools/list'), callTool: (name, args) => call('tools/call', { name, arguments: args }), close: async () => { socket.destroy(); } };
}
