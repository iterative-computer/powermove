#!/usr/bin/env node
// Deterministic provider: use the actual run-scoped MCP configuration supplied
// by main. No model call, renderer override, or manually inserted chat message.
import { spawn } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import { createInterface } from 'node:readline';

import { serveCodexFixture } from '../codex-app-server.mjs';

await serveCodexFixture(async thread => {
  const config = thread.config.mcp_servers.powermove;
  if (!config?.command) throw new Error('Missing authenticated run-scoped MCP configuration');
  const env = { ...process.env, ...config.env };
  const child = spawn(config.command, config.args, { env, stdio: ['pipe', 'pipe', 'ignore'] });
  const replies = createInterface({ input: child.stdout });
  const pending = new Map();
  replies.on('line', line => {
    const reply = JSON.parse(line);
    pending.get(reply.id)?.(reply);
    pending.delete(reply.id);
  });
  let nextId = 0;
  const rpc = (method, params) => new Promise((resolve, reject) => {
    const id = ++nextId;
    const timer = setTimeout(() => reject(new Error('Fixture MCP timeout')), 10000);
    pending.set(id, reply => { clearTimeout(timer); resolve(reply); });
    child.stdin.write(JSON.stringify({ jsonrpc: '2.0', id, method, params }) + '\n');
  });
  try {
    await rpc('initialize', { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'visual-provider-fixture', version: '1' } });
    const listed = await rpc('tools/list', {});
    if (!listed.result.tools.some(tool => tool.name === 'publish_visual_reply')) throw new Error('Visual tool missing from actual provider session');
    for (const name of ['easing', 'storyboard', 'palette']) {
      const input = JSON.parse(await readFile(new URL(`./${name}.json`, import.meta.url), 'utf8'));
      const reply = await rpc('tools/call', { name: 'publish_visual_reply', arguments: input });
      if (reply.error || reply.result?.isError) throw new Error('Actual provider publication failed');
    }
    return { summary: 'Published an interactive timing chart through the native agent tool.', commands: [], artifacts: [], externalActions: [], notes: [] };
  } finally {
    child.stdin.end();
    child.kill();
  }
});
