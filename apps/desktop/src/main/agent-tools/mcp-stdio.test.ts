import { spawn } from 'node:child_process';
import { PassThrough } from 'node:stream';
import { pathToFileURL } from 'node:url';
import { describe, expect, it, vi } from 'vitest';
import { startMcpStdio } from './mcp-stdio.mjs';

describe('public MCP stdio protocol', () => {
  it.each(['async', 'sync'])('closes safely when an output write fails (%s)', async failure => {
    // Isolate an unhandled stream error so the regression cannot crash Vitest.
    const source = `
      import { PassThrough, Writable } from 'node:stream';
      import { startMcpStdio } from ${JSON.stringify(pathToFileURL(`${__dirname}/mcp-stdio.mjs`).href)};
      const input = new PassThrough();
      const output = new Writable({ write(chunk, encoding, callback) {
        const error = Object.assign(new Error('write EPIPE'), { code: 'EPIPE' });
        ${failure === 'sync' ? 'throw error;' : 'process.nextTick(() => callback(error));'}
      } });
      let closed = 0, calls = 0;
      const stopped = startMcpStdio({ input, output,
        listTools: async () => [], callTool: async () => { calls++; return {}; },
        close: async () => { closed++; }
      });
      input.write([
        { jsonrpc: '2.0', id: 1, method: 'initialize' },
        { jsonrpc: '2.0', id: 2, method: 'tools/call', params: { name: 'test' } }
      ].map(message => JSON.stringify(message)).join('\\n') + '\\n');
      await stopped;
      input.destroy();
      await new Promise(resolve => setImmediate(resolve));
      console.log(JSON.stringify({ closed, calls }));
    `;
    const child = spawn(process.execPath, ['--input-type=module', '-e', source], { stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '', stderr = '';
    child.stdout.on('data', chunk => { stdout += chunk; });
    child.stderr.on('data', chunk => { stderr += chunk; });
    const code = await new Promise<number | null>((resolve, reject) => {
      child.once('error', reject);
      child.once('close', resolve);
    });
    expect({ code, stderr }).toEqual({ code: 0, stderr: '' });
    expect(JSON.parse(stdout)).toEqual({ closed: 1, calls: 0 });
  });

  it('exits cleanly when the client closes the real stdout pipe', async () => {
    const source = `
      import { startMcpStdio } from ${JSON.stringify(pathToFileURL(`${__dirname}/mcp-stdio.mjs`).href)};
      let closed = 0, calls = 0;
      const stopped = startMcpStdio({
        listTools: async () => [], callTool: async () => { calls++; return {}; },
        close: async () => { closed++; }
      });
      process.stderr.write('ready\\n');
      await stopped;
      process.stderr.write(JSON.stringify({ closed, calls }) + '\\n');
      process.exit(0);
    `;
    const child = spawn(process.execPath, ['--input-type=module', '-e', source], { stdio: ['pipe', 'pipe', 'pipe'] });
    let stderr = '', sent = false;
    child.stderr.on('data', chunk => {
      stderr += chunk;
      if (!sent && stderr.includes('ready\n')) {
        sent = true;
        child.stdout.destroy();
        child.stdin.write([
          { jsonrpc: '2.0', id: 1, method: 'initialize' },
          { jsonrpc: '2.0', id: 2, method: 'tools/call', params: { name: 'test' } }
        ].map(message => JSON.stringify(message)).join('\n') + '\n');
      }
    });
    const code = await new Promise<number | null>((resolve, reject) => {
      child.once('error', reject);
      child.once('close', resolve);
    });
    expect({ code, stderr }).toEqual({ code: 0, stderr: 'ready\n{"closed":1,"calls":0}\n' });
  });

  it('cancels pending work on output close and skips queued edits and late replies', async () => {
    const input = new PassThrough(), output = new PassThrough();
    let written = ''; output.on('data', chunk => { written += chunk.toString(); });
    let finish!: (result: object) => void, started!: () => void;
    const pending = new Promise<object>(resolve => { finish = resolve; });
    const ready = new Promise<void>(resolve => { started = resolve; });
    const callTool = vi.fn(() => { started(); return pending; });
    const close = vi.fn(async () => { finish({ cancelled: true }); });
    const stopped = startMcpStdio({ input, output, listTools: async () => [], callTool, close });
    input.write([
      { jsonrpc: '2.0', id: 1, method: 'initialize' },
      { jsonrpc: '2.0', id: 2, method: 'tools/call', params: { name: 'pending' } },
      { jsonrpc: '2.0', id: 3, method: 'tools/call', params: { name: 'queued-edit' } }
    ].map(message => JSON.stringify(message)).join('\n') + '\n');
    await ready;
    output.destroy();
    await stopped;
    input.destroy();
    expect(close).toHaveBeenCalledTimes(1);
    expect(callTool).toHaveBeenCalledTimes(1);
    expect(written.trim().split('\n').map(line => JSON.parse(line).id)).toEqual([1]);
    expect(output.listenerCount('error')).toBe(0);
  });

  it('negotiates supported versions, rejects malformed requests and returns tool failures as tool results', async () => {
    const input = new PassThrough(), output = new PassThrough();
    let written = ''; output.on('data', chunk => { written += chunk.toString(); });
    const stopped = startMcpStdio({ input, output, listTools: async () => [{ name: 'test' }], callTool: async () => { throw new Error('Tool failed'); } });
    input.end([
      '{broken', JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'tools/list' }),
      JSON.stringify({ jsonrpc: '2.0', id: 2, method: 'initialize', params: { protocolVersion: 'not-supported' } }),
      JSON.stringify({ jsonrpc: '2.0', method: 'notifications/initialized' }),
      JSON.stringify({ jsonrpc: '2.0', id: 3, method: 'tools/list' }),
      JSON.stringify({ jsonrpc: '2.0', id: 4, method: 'tools/call', params: { name: 'test', arguments: {} } }),
      JSON.stringify({ jsonrpc: '2.0', id: 5, method: 'unknown' })
    ].join('\n') + '\n');
    await stopped;
    const messages = written.trim().split('\n').map(line => JSON.parse(line));
    expect(messages.map(message => message.id)).toEqual([null, 1, 2, 3, 4, 5]);
    expect(messages[0].error.code).toBe(-32700);
    expect(messages[1].error.message).toContain('Initialize');
    expect(messages[2].result.protocolVersion).toBe('2025-11-25');
    expect(messages[4].result).toEqual({ isError: true, content: [{ type: 'text', text: 'Tool failed' }] });
    expect(messages[5].error.code).toBe(-32601);
  });
});
