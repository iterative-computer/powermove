/* Powermove — model providers. Local is first-class; network providers share the tool harness. */
(() => {
const PM = window.PM, A = PM.Agent;
const P = { all: {} };
PM.Providers = P;

/* Native bridge to the locally installed Codex client. The Codex client owns
   ChatGPT authentication; no subscription cookie or account token enters the
   Powermove web interface. */
const pendingCodex = new Map();
PM.CodexBridge = {
  request(prompt, schema) {
    const bridge = window.webkit?.messageHandlers?.pmCodex;
    if (!bridge) return Promise.reject(new Error('ChatGPT subscription is available in the Powermove macOS app'));
    const id = PM.uid('codex');
    return new Promise((resolve, reject) => {
      pendingCodex.set(id, { resolve, reject });
      bridge.postMessage({ id, prompt, schema });
    });
  },
  resolve(id, result) {
    const pending = pendingCodex.get(id); if (!pending) return;
    pendingCodex.delete(id);
    let text = '';
    try {
      const bytes = Uint8Array.from(atob(result.dataBase64 || ''), char => char.charCodeAt(0));
      text = new TextDecoder().decode(bytes);
    } catch (error) { text = 'Could not decode the ChatGPT response'; }
    if (result.ok) pending.resolve(text); else pending.reject(new Error(text));
  },
};

P.register = (id, def) => { P.all[id] = { id, ...def }; };
P.get = (id) => P.all[id] || P.all.local;
P.config = () => PM.store.get('providerConfig', {});
P.saveConfig = (cfg) => PM.store.set('providerConfig', cfg);
P.label = (id) => (P.get(id) || {}).label || id;

function cfg(id) {
  const all = P.config();
  return all[id] || {};
}
function need(v, msg) { if (!v) throw new Error(msg); return v; }
function jsonText(v) { try { return JSON.stringify(v); } catch { return String(v); } }
function toolResult(out) { return jsonText(out && out.data !== undefined ? { ok: out.ok, message: out.message, data: out.data } : out); }
function freshAbort() { A.abort = new AbortController(); return A.abort.signal; }
function stateMessage() { return `# LIVE COMPOSITION STATE\n${A.digest()}`; }

P.register('local', {
  label: 'Powermove Local',
  models: ['Offline intent engine'],
  async run(text) {
    await new Promise(r => setTimeout(r, 90));
    if (!PM.LocalPlanner) throw new Error('Local planner is unavailable');
    return PM.LocalPlanner.run(text);
  },
});

P.register('chatgpt', {
  label: 'ChatGPT subscription',
  models: ['Codex account default'],
  async run(text) {
    const toolNames = Object.keys(PM.Tools);
    const schema = {
      type: 'object', additionalProperties: false, required: ['assistant', 'calls'],
      properties: {
        assistant: { type: 'string' },
        calls: { type: 'array', items: {
          type: 'object', additionalProperties: false, required: ['name', 'arguments'],
          properties: {
            name: { type: 'string', enum: toolNames },
            arguments: { type: 'string' },
          },
        } },
      },
    };
    const prompt = `${A.system()}\n\n# LIVE COMPOSITION STATE\n${A.digest()}\n\n# USER REQUEST\n${text}\n\n# PLANNING CONTRACT\nReturn a compact JSON action plan matching the supplied schema. Do not edit files and do not run shell commands. Put each Powermove tool call in calls, with arguments as a JSON-encoded object string. Use only the supplied tool schemas.\n${JSON.stringify(A.toolSchemas())}`;
    const raw = await PM.CodexBridge.request(prompt, schema);
    /* Stop remains meaningful even though Codex runs outside the web view: a
       stopped request is never allowed to apply a late action plan. */
    if (!A.running) return;
    let plan;
    try { plan = JSON.parse(raw); }
    catch (error) { throw new Error('ChatGPT returned an invalid action plan'); }
    const calls = Array.isArray(plan.calls) ? plan.calls.slice(0, A.maxSteps) : [];
    for (const call of calls) {
      if (!A.running) return;
      let args = {};
      try { args = JSON.parse(call.arguments || '{}'); }
      catch (error) { throw new Error(`ChatGPT returned invalid arguments for ${call.name}`); }
      const result = await A.callTool(call.name, args);
      if (result?.ok === false) throw new Error(result.message || `${call.name} failed`);
    }
    A.push({ role: 'assistant', content: plan.assistant || (calls.length ? 'The generated changes are live.' : 'I did not find a safe change to make.') });
  },
});

P.register('openai', {
  label: 'OpenAI',
  models: ['gpt-5', 'gpt-4.1', 'gpt-4.1-mini'],
  async run(text) {
    const c = cfg('openai');
    const key = need(c.key, 'Add an OpenAI API key in Assistant Settings');
    return openAICompatible({
      url: c.url || 'https://api.openai.com/v1/chat/completions', key,
      model: A.model || c.model || 'gpt-5', text,
      headers: {},
    });
  },
});

P.register('compatible', {
  label: 'OpenAI Compatible',
  models: [],
  async run(text) {
    const c = cfg('compatible');
    return openAICompatible({
      url: need(c.url, 'Add a chat-completions endpoint in Assistant Settings'),
      key: c.key || '', model: need(A.model || c.model, 'Add a model name in Assistant Settings'), text,
      headers: c.headers || {},
    });
  },
});

async function openAICompatible({ url, key, model, text, headers }) {
  const signal = freshAbort();
  const messages = [
    { role: 'system', content: A.system() },
    { role: 'system', content: stateMessage() },
    { role: 'user', content: text },
  ];
  for (let step = 0; step < A.maxSteps; step++) {
    const body = { model, messages, tools: A.toolSchemas(), tool_choice: 'auto' };
    const res = await fetch(url, {
      method: 'POST', signal,
      headers: { 'content-type': 'application/json', ...(key ? { authorization: 'Bearer ' + key } : {}), ...headers },
      body: JSON.stringify(body),
    });
    const data = await responseJSON(res);
    const msg = data.choices && data.choices[0] && data.choices[0].message;
    if (!msg) throw new Error(data.error && data.error.message || 'Provider returned no message');
    messages.push(msg);
    const calls = msg.tool_calls || [];
    if (!calls.length) {
      A.push({ role: 'assistant', content: msg.content || 'Done.' });
      return;
    }
    for (const tc of calls) {
      let args = {};
      try { args = typeof tc.function.arguments === 'string' ? JSON.parse(tc.function.arguments || '{}') : tc.function.arguments || {}; }
      catch (e) { args = {}; }
      const out = await A.callTool(tc.function.name, args);
      messages.push({ role: 'tool', tool_call_id: tc.id, content: toolResult(out) });
    }
    messages.push({ role: 'system', content: stateMessage() });
  }
  A.push({ role: 'assistant', content: 'I reached the action limit. The completed edits are live; continue the direction in a new prompt.' });
}

P.register('anthropic', {
  label: 'Anthropic',
  models: ['claude-sonnet-4-5', 'claude-opus-4-1'],
  async run(text) {
    const c = cfg('anthropic');
    const key = need(c.key, 'Add an Anthropic API key in Assistant Settings');
    const model = A.model || c.model || 'claude-sonnet-4-5';
    const signal = freshAbort();
    const messages = [{ role: 'user', content: `${stateMessage()}\n\n# USER REQUEST\n${text}` }];
    for (let step = 0; step < A.maxSteps; step++) {
      const res = await fetch(c.url || 'https://api.anthropic.com/v1/messages', {
        method: 'POST', signal,
        headers: {
          'content-type': 'application/json', 'x-api-key': key,
          'anthropic-version': '2023-06-01', 'anthropic-dangerous-direct-browser-access': 'true',
        },
        body: JSON.stringify({ model, max_tokens: 4096, system: A.system(), messages, tools: A.anthropicTools() }),
      });
      const data = await responseJSON(res);
      if (!Array.isArray(data.content)) throw new Error(data.error && data.error.message || 'Provider returned no content');
      messages.push({ role: 'assistant', content: data.content });
      const uses = data.content.filter(x => x.type === 'tool_use');
      const textOut = data.content.filter(x => x.type === 'text').map(x => x.text).join('\n').trim();
      if (!uses.length) { A.push({ role: 'assistant', content: textOut || 'Done.' }); return; }
      const results = [];
      for (const use of uses) {
        const out = await A.callTool(use.name, use.input || {});
        results.push({ type: 'tool_result', tool_use_id: use.id, content: toolResult(out), is_error: out && out.ok === false });
      }
      results.push({ type: 'text', text: stateMessage() });
      messages.push({ role: 'user', content: results });
    }
    A.push({ role: 'assistant', content: 'I reached the action limit. The completed edits are live; continue the direction in a new prompt.' });
  },
});

async function responseJSON(res) {
  let data;
  try { data = await res.json(); }
  catch { throw new Error(`Provider request failed (${res.status})`); }
  if (!res.ok) throw new Error(data.error && (data.error.message || data.error.type) || `Provider request failed (${res.status})`);
  return data;
}
})();
