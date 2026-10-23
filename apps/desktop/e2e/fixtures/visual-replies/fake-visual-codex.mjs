#!/usr/bin/env node
// Deterministic provider: use the actual run-scoped MCP configuration supplied
// by main. No model call, renderer override, or manually inserted chat message.
import { spawn } from 'node:child_process';
import { readFile, writeFile } from 'node:fs/promises';
import { createInterface } from 'node:readline';

const args = process.argv.slice(2);
if (args.includes('--help')) {
  console.log('--json --output-schema --output-last-message --sandbox --skip-git-repo-check --image --model --config --add-dir');
} else if (args.includes('app-server')) {
  for await (const line of createInterface({ input: process.stdin })) {
    const message = JSON.parse(line);
    if (!message.id) continue;
    const result = message.method === 'initialize'
      ? { userAgent: 'visual-reply-fixture', codexHome: '/tmp/visual-reply-fixture', platformFamily: 'unix', platformOs: 'macos' }
      : message.method === 'account/read'
        ? { account: { type: 'chatgpt', email: 'visual@example.com', planType: 'pro' }, requiresOpenaiAuth: true }
        : {};
    console.log(JSON.stringify({ id: message.id, result }));
  }
} else {
  const config = {};
  for (let i = 0; i < args.length; i++) {
    if (args[i] !== '--config') continue;
    const entry = args[++i];
    const equal = entry.indexOf('=');
    if (entry.startsWith('mcp_servers.powermove.')) {
      try { config[entry.slice('mcp_servers.powermove.'.length, equal)] = JSON.parse(entry.slice(equal + 1)); } catch {}
    }
  }
  const env = { ...process.env };
  for (const [key, value] of Object.entries(config)) if (key.startsWith('env.')) env[key.slice(4)] = value;
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
    await writeFile(args[args.indexOf('--output-last-message') + 1], JSON.stringify({ summary: 'Published an interactive timing chart through the native agent tool.', commands: [], artifacts: [], externalActions: [], notes: [] }));
  } finally {
    child.stdin.end();
    child.kill();
  }
}
