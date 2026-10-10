/** Shared newline-delimited MCP transport. Diagnostics must go to stderr. */
export function startMcpStdio({ listTools, callTool, close = async () => {}, instructions = '', input = process.stdin, output = process.stdout }) {
  const versions = ['2025-11-25', '2025-06-18', '2025-03-26', '2024-11-05'];
  const write = message => output.write(`${JSON.stringify(message)}\n`);
  const error = (id, code, message) => write({ jsonrpc: '2.0', id: id ?? null, error: { code, message } });
  let initialized = false;
  let buffer = '';
  let chain = Promise.resolve();
  let stopping = false;
  async function handle(message) {
    if (!message || Array.isArray(message) || message.jsonrpc !== '2.0' || typeof message.method !== 'string') {
      error(message?.id, -32600, 'Invalid Request'); return;
    }
    if (message.id === undefined) return;
    const id = message.id;
    if (typeof id !== 'string' && typeof id !== 'number') { error(null, -32600, 'Invalid request id'); return; }
    if (message.method === 'initialize') {
      if (initialized) { error(id, -32600, 'Already initialized'); return; }
      initialized = true;
      write({ jsonrpc: '2.0', id, result: {
        protocolVersion: versions.includes(message.params?.protocolVersion) ? message.params.protocolVersion : versions[0],
        capabilities: { tools: { listChanged: false } }, serverInfo: { name: 'powermove', version: '1.0.0' },
        ...(instructions ? { instructions } : {})
      } }); return;
    }
    if (message.method === 'ping') { write({ jsonrpc: '2.0', id, result: {} }); return; }
    if (!initialized) { error(id, -32000, 'Initialize Powermove before using tools'); return; }
    if (message.method === 'tools/list') {
      write({ jsonrpc: '2.0', id, result: { tools: await listTools() } }); return;
    }
    if (message.method === 'tools/call') {
      const name = message.params?.name, args = message.params?.arguments ?? {};
      if (typeof name !== 'string' || !args || typeof args !== 'object' || Array.isArray(args)) {
        error(id, -32602, 'Invalid tool arguments'); return;
      }
      let result;
      try { result = await callTool(name, args, id); }
      catch (cause) { result = { isError: true, content: [{ type: 'text', text: cause instanceof Error ? cause.message : String(cause) }] }; }
      write({ jsonrpc: '2.0', id, result }); return;
    }
    error(id, -32601, 'Method not found');
  }
  const stopped = new Promise((resolve, reject) => {
    input.setEncoding('utf8');
    input.on('data', chunk => {
      if (stopping) return;
      buffer += chunk;
      if (Buffer.byteLength(buffer) > 2 * 1024 * 1024) {
        error(null, -32600, 'MCP request exceeds 2 MiB'); input.destroy(); return;
      }
      for (;;) {
        const newline = buffer.indexOf('\n'); if (newline < 0) break;
        const line = buffer.slice(0, newline).trim(); buffer = buffer.slice(newline + 1);
        if (!line) continue;
        chain = chain.then(async () => {
          let message;
          try { message = JSON.parse(line); } catch { error(null, -32700, 'Parse error'); return; }
          try { await handle(message); } catch (cause) { error(message.id, -32603, cause instanceof Error ? cause.message : String(cause)); }
        });
      }
    });
    const stop = () => {
      if (stopping) return; stopping = true;
      // Close immediately so a disconnected client cancels outstanding work.
      void close().then(() => chain).then(resolve, reject);
    };
    input.once('end', stop); input.once('close', stop); input.once('error', stop);
  });
  return stopped;
}
