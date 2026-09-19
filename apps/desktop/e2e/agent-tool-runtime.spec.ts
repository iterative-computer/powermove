import { spawn } from 'node:child_process';
import net from 'node:net';
import electron from 'electron';
import { test, expect } from '@playwright/test';
import { repoRoot } from './helpers/app';

test('the app-owned MCP entrypoint lists live tools without Electron Node mode', async () => {
  const server = net.createServer(socket => {
    let data = '';
    socket.on('data', chunk => {
      data += chunk;
      if (!data.includes('\n')) return;
      const request = JSON.parse(data);
      const ok = request.token === 'test-token' && request.runId === 'test-run' && request.tool === '__list_tools';
      socket.end(JSON.stringify({ ok, tools: ok ? [{ name: 'get_project_state', inputSchema: { type: 'object' } }] : [] }) + '\n');
    });
  });
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve));
  const env = { ...process.env, POWERMOVE_AGENT_TOOL_PORT: String((server.address() as net.AddressInfo).port), POWERMOVE_AGENT_TOOL_TOKEN: 'test-token', POWERMOVE_AGENT_RUN_ID: 'test-run' };
  delete env.ELECTRON_RUN_AS_NODE;
  const child = spawn(electron as unknown as string, [repoRoot, '--powermove-agent-tools'], { env, stdio: ['pipe', 'pipe', 'pipe'] });
  let output = '', errors = '';
  child.stdout.on('data', chunk => output += chunk);
  child.stderr.on('data', chunk => errors += chunk);
  try {
    const exited = new Promise<number | null>((resolve, reject) => { child.on('exit', resolve); child.on('error', reject); });
    child.stdin.end(JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'initialize' }) + '\n' + JSON.stringify({ jsonrpc: '2.0', id: 2, method: 'tools/list' }) + '\n');
    expect(await exited, errors).toBe(0);
    const messages = output.trim().split('\n').map(line => JSON.parse(line));
    expect(messages[0].result.serverInfo.name).toBe('powermove');
    expect(messages[1].result.tools[0].name).toBe('get_project_state');
  } finally { child.kill(); await new Promise<void>(resolve => server.close(() => resolve())); }
});
