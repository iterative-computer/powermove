import net from 'node:net';
import { startMcpStdio } from './mcp-stdio.mjs';

const port = Number.parseInt(process.env.POWERMOVE_AGENT_TOOL_PORT || '', 10);
const token = process.env.POWERMOVE_AGENT_TOOL_TOKEN || '';
const runId = process.env.POWERMOVE_AGENT_RUN_ID || '';
const timeoutMs = Number.parseInt(process.env.POWERMOVE_AGENT_TOOL_TIMEOUT_MS || '120000', 10);
// run_outside_sandbox waits for the person's approval (spec.ts OUTSIDE_SANDBOX_CALL_MS).
const LONG_CALLS = new Map([['run_outside_sandbox', 4_200_000], ['save_project', 3_605_000], ['export_video', 3_605_000]]);

if (!Number.isInteger(port) || port < 1 || port > 65535 || !token || !runId) {
  process.stderr.write('Powermove MCP bridge environment is incomplete.\n');
  process.exit(1);
}

function bridgeCall(tool, args, id) {
  return new Promise((resolve, reject) => {
    const socket = net.createConnection({ host: '127.0.0.1', port });
    socket.setEncoding('utf8');
    socket.setTimeout(LONG_CALLS.get(tool) ?? (Number.isFinite(timeoutMs) ? timeoutMs + 5000 : 125000), () => {
      socket.destroy(new Error('Powermove tool call timed out.'));
    });
    let output = '';
    socket.once('connect', () => {
      socket.write(`${JSON.stringify({ token, runId, id, tool, arguments: args || {}, workspace: process.cwd() })}\n`);
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

void startMcpStdio({
  listTools: async () => {
    const response = await bridgeCall('__list_tools', {}, null);
    if (!response.ok || !Array.isArray(response.tools)) throw new Error(response.error || 'Powermove tools are unavailable.');
    return response.tools;
  },
  callTool: async (name, args, id) => {
    const response = await bridgeCall(name, args, id);
    const content = Array.isArray(response.content) ? response.content : [];
    if (!response.ok && response.error && !content.some(item => item?.type === 'text')) content.push({ type: 'text', text: response.error });
    return { content, isError: response.ok !== true };
  }
}).then(() => process.exit(0), error => { process.stderr.write(`${String(error)}\n`); process.exit(1); });
