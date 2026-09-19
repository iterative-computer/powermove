/*
 * A stand-in for an OpenAI-compatible model, for exercising agent runs on the
 * remote host without a real provider. Scripted: the first turn calls
 * apply_commands to add a layer named after the prompt, the next turn calls
 * complete_task. Anything else gets a plain "OK".
 *
 *   node scripts/fake-model.mjs [port]
 */
import { createServer } from 'node:http';

const port = Number(process.argv[2] || 4899);

function sse(response, chunks) {
  response.writeHead(200, { 'Content-Type': 'text/event-stream', 'Cache-Control': 'no-cache' });
  for (const chunk of chunks) response.write(`data: ${JSON.stringify(chunk)}\n\n`);
  response.write('data: [DONE]\n\n');
  response.end();
}
const toolCall = (id, name, args) => ({
  choices: [{ index: 0, delta: { tool_calls: [{ index: 0, id, type: 'function', function: { name, arguments: JSON.stringify(args) } }] }, finish_reason: null }]
});
const finish = (reason) => ({ choices: [{ index: 0, delta: {}, finish_reason: reason }] });

createServer((request, response) => {
  let body = '';
  request.on('data', (chunk) => { body += chunk; });
  request.on('end', () => {
    if (!request.url.endsWith('/chat/completions')) { response.writeHead(404); response.end(); return; }
    const payload = JSON.parse(body || '{}');
    const messages = payload.messages || [];
    const tools = (payload.tools || []).map((tool) => tool.function.name);
    if (!payload.stream) {
      response.writeHead(200, { 'Content-Type': 'application/json' });
      response.end(JSON.stringify({ choices: [{ message: { role: 'assistant', content: 'OK' } }] }));
      return;
    }
    const userText = messages.filter((m) => m.role === 'user').map((m) => typeof m.content === 'string' ? m.content : '').join(' ');
    const name = (/layer named "([^"]+)"/i.exec(userText)?.[1]) || 'Agent layer';
    const applied = messages.some((m) => m.role === 'tool');
    const seenComplete = messages.some((m) => m.role === 'assistant' && m.tool_calls?.some((t) => t.function?.name === 'complete_task'));
    console.log(`[fake-model] turn: tools=${tools.length} applied=${applied}`);
    if (tools.includes('apply_commands') && !applied) {
      sse(response, [toolCall('call-apply-1', 'apply_commands', { commands: [{ type: 'add_layer', layerType: 'shape', name }], label: `Add ${name}` }), finish('tool_calls')]);
    } else if (tools.includes('complete_task') && !seenComplete) {
      sse(response, [toolCall('call-done-1', 'complete_task', { summary: `Added ${name}.`, commands: [], artifacts: [], extensions: [], notes: [], externalActions: [] }), finish('tool_calls')]);
    } else {
      sse(response, [{ choices: [{ index: 0, delta: { content: 'OK' }, finish_reason: null }] }, finish('stop')]);
    }
  });
}).listen(port, '127.0.0.1', () => console.log(`[fake-model] listening on http://127.0.0.1:${port}/v1`));
