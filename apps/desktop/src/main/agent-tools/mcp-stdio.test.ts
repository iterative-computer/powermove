import { PassThrough } from 'node:stream';
import { describe, expect, it } from 'vitest';
import { startMcpStdio } from './mcp-stdio.mjs';

describe('public MCP stdio protocol', () => {
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
