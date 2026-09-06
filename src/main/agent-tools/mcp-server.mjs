import net from 'node:net';

const port = Number.parseInt(process.env.POWERMOVE_AGENT_TOOL_PORT || '', 10);
const token = process.env.POWERMOVE_AGENT_TOOL_TOKEN || '';
const runId = process.env.POWERMOVE_AGENT_RUN_ID || '';
const timeoutMs = Number.parseInt(process.env.POWERMOVE_AGENT_TOOL_TIMEOUT_MS || '120000', 10);

if (!Number.isInteger(port) || port < 1 || port > 65535 || !token || !runId) {
  process.stderr.write('Powermove MCP bridge environment is incomplete.\n');
  process.exit(1);
}

function write(message) {
  process.stdout.write(`${JSON.stringify(message)}\n`);
}

function bridgeCall(tool, args, id) {
  return new Promise((resolve, reject) => {
    const socket = net.createConnection({ host: '127.0.0.1', port });
    socket.setEncoding('utf8');
    socket.setTimeout(Number.isFinite(timeoutMs) ? timeoutMs + 5000 : 125000, () => {
      socket.destroy(new Error('Powermove tool call timed out.'));
    });
    let output = '';
    socket.once('connect', () => {
      socket.write(`${JSON.stringify({ token, runId, id, tool, arguments: args || {} })}\n`);
    });
    socket.on('data', (chunk) => {
      output += chunk;
      if (Buffer.byteLength(output, 'utf8') > 64 * 1024 * 1024) {
        socket.destroy(new Error('Powermove tool response was too large.'));
      }
    });
    socket.once('error', reject);
    socket.once('end', () => {
      try { resolve(JSON.parse(output.trim())); }
      catch { reject(new Error('Powermove returned an invalid tool response.')); }
    });
  });
}

function rpcError(id, code, message) {
  write({ jsonrpc: '2.0', id: id ?? null, error: { code, message } });
}

async function handle(message) {
  if (!message || message.jsonrpc !== '2.0' || typeof message.method !== 'string') {
    rpcError(message?.id, -32600, 'Invalid Request');
    return;
  }
  const id = message.id;
  if (message.method === 'notifications/initialized' || message.method.startsWith('notifications/')) return;
  if (id === undefined) return;
  if (message.method === 'initialize') {
    write({
      jsonrpc: '2.0',
      id,
      result: {
        protocolVersion: message.params?.protocolVersion || '2025-06-18',
        capabilities: { tools: { listChanged: false } },
        serverInfo: { name: 'powermove', version: '1.0.0' }
      }
    });
    return;
  }
  if (message.method === 'ping') {
    write({ jsonrpc: '2.0', id, result: {} });
    return;
  }
  if (message.method === 'tools/list') {
    const response = await bridgeCall('__list_tools', {}, id);
    if (!response.ok || !Array.isArray(response.tools)) throw new Error(response.error || 'Powermove tools are unavailable.');
    write({ jsonrpc: '2.0', id, result: { tools: response.tools } });
    return;
  }
  if (message.method === 'tools/call') {
    const name = message.params?.name;
    const args = message.params?.arguments ?? {};
    if (typeof name !== 'string' || typeof args !== 'object' || args === null || Array.isArray(args)) {
      rpcError(id, -32602, 'Invalid tool arguments');
      return;
    }
    const response = await bridgeCall(name, args, id);
    const content = Array.isArray(response.content) ? response.content : [];
    if (!response.ok && response.error && !content.some((item) => item?.type === 'text')) {
      content.push({ type: 'text', text: response.error });
    }
    write({
      jsonrpc: '2.0',
      id,
      result: { content, isError: response.ok !== true }
    });
    return;
  }
  rpcError(id, -32601, 'Method not found');
}

let input = '';
let chain = Promise.resolve();
process.stdin.setEncoding('utf8');
process.stdin.on('data', (chunk) => {
  input += chunk;
  for (;;) {
    const newline = input.indexOf('\n');
    if (newline < 0) break;
    const line = input.slice(0, newline).trim();
    input = input.slice(newline + 1);
    if (!line) continue;
    chain = chain.then(async () => {
      let message;
      try { message = JSON.parse(line); }
      catch { rpcError(null, -32700, 'Parse error'); return; }
      try { await handle(message); }
      catch (error) { rpcError(message?.id, -32603, error instanceof Error ? error.message : String(error)); }
    });
  }
});
