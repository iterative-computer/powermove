/* A local stand-in for the OpenAI Responses API and the Anthropic Messages API, so Aero runs with no provider keys.
   It streams a canned reply in each provider's own SSE format: summarized reasoning (with a signature or an encrypted
   item), text, and a call to the first tool when the request offers tools and the newest message is not a tool result.
   Usage is reported like the real APIs, cache included: the first request of a thread writes the cache (Anthropic) and
   later ones read it (both). A user message containing "[slow]" streams slowly, for abort tests.

   bun scripts/ai-simulator.ts [--port 4010]
   AERO_OPENAI_BASE_URL=http://localhost:4010/openai/v1  AERO_ANTHROPIC_BASE_URL=http://localhost:4010/anthropic/v1 */

export interface SimulatorRequest { provider: 'openai' | 'anthropic'; body: Record<string, any>; headers: Record<string, string> }
export interface Simulator { url: string; requests: SimulatorRequest[]; /** Streams the client cut off before the end. */ aborted: number; stop(): Promise<void> }

const encoder = new TextEncoder();
const sleep = (ms: number) => new Promise(resolve => setTimeout(resolve, ms));
const tokens = (value: unknown) => Math.max(1, Math.ceil(JSON.stringify(value ?? '').length / 4));

function lastUserText(messages: any[]): string {
  for (let i = messages.length - 1; i >= 0; i--) {
    const m = messages[i];
    if (m.role === 'user' || m.type === 'message' && m.role === 'user') {
      const content = m.content;
      if (typeof content === 'string') return content;
      if (Array.isArray(content)) return content.map((part: any) => part.text ?? '').join(' ');
    }
  }
  return '';
}

export function startAiSimulator(options: { port?: number } = {}): Simulator {
  const requests: SimulatorRequest[] = [];
  const seenThreads = new Set<string>();

  const server = Bun.serve({
    port: options.port ?? 0,
    idleTimeout: 60,
    async fetch(request) {
      const url = new URL(request.url);
      if (request.method === 'GET' && url.pathname === '/health') return new Response('ok');
      if (request.method !== 'POST') return new Response('not found', { status: 404 });
      const body = await request.json().catch(() => null) as Record<string, any> | null;
      if (!body) return Response.json({ error: { message: 'bad json' } }, { status: 400 });
      const headers: Record<string, string> = {};
      request.headers.forEach((value, key) => { headers[key] = value; });
      if (url.pathname === '/openai/v1/responses') {
        requests.push({ provider: 'openai', body, headers });
        return openai(body, request.signal);
      }
      if (url.pathname === '/anthropic/v1/messages') {
        requests.push({ provider: 'anthropic', body, headers });
        return anthropic(body, request.signal);
      }
      return Response.json({ error: { message: 'not found' } }, { status: 404 });
    },
  });

  /** One reply plan for both providers. */
  function plan(messages: any[], tools: any[] | undefined, lastIsToolResult: boolean) {
    const user = lastUserText(messages);
    const slow = user.includes('[slow]');
    const toolName: string | undefined = !lastIsToolResult && tools?.length ? (tools[0].name ?? tools[0].function?.name) : undefined;
    const text = lastIsToolResult ? 'The tool finished. Done.' : toolName ? `I'll use ${toolName}.` : `You said: ${user.slice(0, 80)}`;
    const pieces = slow ? Array.from({ length: 60 }, (_, i) => `word${i} `) : text.match(/.{1,8}/g) ?? [text];
    return { slow, toolName, pieces, reasoning: ['Thinking about ', 'the request.'] };
  }

  function stream(signal: AbortSignal, run: (send: (event: string, data: unknown) => void) => Promise<void>): Response {
    const body = new ReadableStream<Uint8Array>({
      async start(controller) {
        const send = (event: string, data: unknown) => {
          if (signal.aborted) throw new Error('aborted');
          controller.enqueue(encoder.encode(`event: ${event}\ndata: ${JSON.stringify(data)}\n\n`));
        };
        try { await run(send); controller.close(); } catch { simulator.aborted++; try { controller.close(); } catch { /* cancelled */ } }
      },
    });
    return new Response(body, { headers: { 'content-type': 'text/event-stream', 'cache-control': 'no-cache' } });
  }

  function openai(body: Record<string, any>, signal: AbortSignal): Response {
    const input: any[] = Array.isArray(body.input) ? body.input : [];
    const lastIsToolResult = input.at(-1)?.type === 'function_call_output';
    const { slow, toolName, pieces, reasoning } = plan(input, body.tools, lastIsToolResult);
    const inputTokens = tokens([body.instructions, body.input, body.tools]);
    const key = body.prompt_cache_key ? `openai:${body.prompt_cache_key}` : null;
    const cached = key && seenThreads.has(key) ? Math.floor(inputTokens * 0.8 / 64) * 64 : 0;
    if (key) seenThreads.add(key);
    const id = `resp_${crypto.randomUUID().replaceAll('-', '')}`;
    const created = Math.floor(Date.now() / 1000);
    return stream(signal, async (send) => {
      let seq = 0;
      const out = (type: string, data: Record<string, unknown>) => send(type, { type, sequence_number: seq++, ...data });
      out('response.created', { response: { id, object: 'response', created_at: created, model: body.model, status: 'in_progress', output: [], usage: null } });
      let index = 0;
      let outputTokens = 0;
      if (body.reasoning?.effort && body.reasoning.effort !== 'none') {
        const rid = `rs_${index}`;
        out('response.output_item.added', { output_index: index, item: { type: 'reasoning', id: rid, summary: [], encrypted_content: null } });
        out('response.reasoning_summary_part.added', { item_id: rid, output_index: index, summary_index: 0, part: { type: 'summary_text', text: '' } });
        for (const delta of reasoning) out('response.reasoning_summary_text.delta', { item_id: rid, output_index: index, summary_index: 0, delta });
        out('response.reasoning_summary_text.done', { item_id: rid, output_index: index, summary_index: 0, text: reasoning.join('') });
        out('response.reasoning_summary_part.done', { item_id: rid, output_index: index, summary_index: 0, part: { type: 'summary_text', text: reasoning.join('') } });
        out('response.output_item.done', { output_index: index, item: { type: 'reasoning', id: rid, summary: [{ type: 'summary_text', text: reasoning.join('') }], encrypted_content: 'sim-encrypted-reasoning' } });
        outputTokens += 12;
        index++;
      }
      const mid = `msg_${index}`;
      out('response.output_item.added', { output_index: index, item: { type: 'message', id: mid, role: 'assistant', status: 'in_progress', content: [] } });
      out('response.content_part.added', { item_id: mid, output_index: index, content_index: 0, part: { type: 'output_text', text: '', annotations: [] } });
      for (const delta of pieces) {
        if (slow) await sleep(25);
        out('response.output_text.delta', { item_id: mid, output_index: index, content_index: 0, delta, logprobs: [] });
        outputTokens += 2;
      }
      const text = pieces.join('');
      out('response.output_text.done', { item_id: mid, output_index: index, content_index: 0, text, logprobs: [] });
      out('response.content_part.done', { item_id: mid, output_index: index, content_index: 0, part: { type: 'output_text', text, annotations: [] } });
      out('response.output_item.done', { output_index: index, item: { type: 'message', id: mid, role: 'assistant', status: 'completed', content: [{ type: 'output_text', text, annotations: [] }] } });
      index++;
      if (toolName) {
        const fid = `fc_${index}`, callId = `call_${crypto.randomUUID().slice(0, 8)}`;
        const args = JSON.stringify({ note: 'from the simulator' });
        out('response.output_item.added', { output_index: index, item: { type: 'function_call', id: fid, call_id: callId, name: toolName, arguments: '', status: 'in_progress' } });
        for (const delta of [args.slice(0, 10), args.slice(10)]) out('response.function_call_arguments.delta', { item_id: fid, output_index: index, delta });
        out('response.function_call_arguments.done', { item_id: fid, output_index: index, arguments: args });
        out('response.output_item.done', { output_index: index, item: { type: 'function_call', id: fid, call_id: callId, name: toolName, arguments: args, status: 'completed' } });
        outputTokens += 10;
      }
      out('response.completed', { response: { id, object: 'response', created_at: created, model: body.model, status: 'completed', usage: { input_tokens: inputTokens, input_tokens_details: { cached_tokens: cached }, output_tokens: outputTokens, output_tokens_details: { reasoning_tokens: body.reasoning?.effort && body.reasoning.effort !== 'none' ? 12 : 0 }, total_tokens: inputTokens + outputTokens } } });
    });
  }

  function anthropic(body: Record<string, any>, signal: AbortSignal): Response {
    const messages: any[] = Array.isArray(body.messages) ? body.messages : [];
    const last = messages.at(-1);
    const lastIsToolResult = Array.isArray(last?.content) && last.content.some((part: any) => part.type === 'tool_result');
    const { slow, toolName, pieces, reasoning } = plan(messages, body.tools, lastIsToolResult);
    const inputTokens = tokens([body.system, body.messages, body.tools]);
    const key = `anthropic:${JSON.stringify([body.system, body.tools]).length}:${tokens(body.system)}`;
    const seen = seenThreads.has(key);
    seenThreads.add(key);
    const cacheRead = seen ? Math.floor(inputTokens * 0.8) : 0;
    const cacheWrite = seen ? Math.floor(inputTokens * 0.1) : Math.floor(inputTokens * 0.8);
    const fresh = inputTokens - cacheRead - cacheWrite;
    const id = `msg_${crypto.randomUUID().replaceAll('-', '')}`;
    return stream(signal, async (send) => {
      const out = (type: string, data: Record<string, unknown>) => send(type, { type, ...data });
      out('message_start', { message: { id, type: 'message', role: 'assistant', model: body.model, content: [], stop_reason: null, stop_sequence: null, usage: { input_tokens: fresh, cache_creation_input_tokens: cacheWrite, cache_read_input_tokens: cacheRead, output_tokens: 1 } } });
      let index = 0;
      let outputTokens = 0;
      if (body.thinking && body.thinking.type !== 'disabled') {
        out('content_block_start', { index, content_block: { type: 'thinking', thinking: '', signature: '' } });
        for (const thinking of reasoning) out('content_block_delta', { index, delta: { type: 'thinking_delta', thinking } });
        out('content_block_delta', { index, delta: { type: 'signature_delta', signature: 'sim-signature' } });
        out('content_block_stop', { index });
        outputTokens += 12;
        index++;
      }
      out('content_block_start', { index, content_block: { type: 'text', text: '' } });
      for (const text of pieces) {
        if (slow) await sleep(25);
        out('content_block_delta', { index, delta: { type: 'text_delta', text } });
        outputTokens += 2;
      }
      out('content_block_stop', { index });
      index++;
      if (toolName) {
        const args = JSON.stringify({ note: 'from the simulator' });
        out('content_block_start', { index, content_block: { type: 'tool_use', id: `toolu_${crypto.randomUUID().slice(0, 8)}`, name: toolName, input: {} } });
        for (const partial_json of [args.slice(0, 10), args.slice(10)]) out('content_block_delta', { index, delta: { type: 'input_json_delta', partial_json } });
        out('content_block_stop', { index });
        outputTokens += 10;
      }
      out('message_delta', { delta: { stop_reason: toolName ? 'tool_use' : 'end_turn', stop_sequence: null }, usage: { output_tokens: outputTokens } });
      out('message_stop', {});
    });
  }

  const simulator: Simulator = { url: `http://localhost:${server.port}`, requests, aborted: 0, stop: async () => { await server.stop(true); } };
  return simulator;
}

if (import.meta.main) {
  const portFlag = process.argv.indexOf('--port');
  const simulator = startAiSimulator({ port: portFlag > 0 ? Number(process.argv[portFlag + 1]) : 4010 });
  console.log(`AI simulator on ${simulator.url}`);
  console.log(`AERO_OPENAI_BASE_URL=${simulator.url}/openai/v1`);
  console.log(`AERO_ANTHROPIC_BASE_URL=${simulator.url}/anthropic/v1`);
}
