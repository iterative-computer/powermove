/* Ported from js/core/scripting.js — behavior-preserving. */
import type { PMRegistry } from '../registry';

export function install(PM: PMRegistry): void {
const LIMITS = Object.freeze({ source: 40_000, commands: 240, result: 600_000, runtime: 900 });
const DENIED_OUTPUTS = new Set(['set_expression']);
const CHANNEL = 'powermove-isolated-script';
const RESULT_CHANNEL = 'powermove-isolated-script-result';
const clone = (value: any) => value === undefined ? undefined : JSON.parse(JSON.stringify(value));
const text = (value: any, fallback = '', max = 100) => typeof value === 'string' && value.trim()
  ? value.trim().slice(0, max) : fallback;

function sourceCode(value: any) {
  if (typeof value !== 'string') return '';
  const code = value.trim();
  return code && code.length <= LIMITS.source ? code : '';
}

function snapshot() {
  const state = PM.AgentHarness?.projectState?.();
  if (!state) throw new Error('The project snapshot service is unavailable');
  return clone(state);
}

function selectedLayers(state: any) {
  const ids = new Set(state?.selection?.layers || []);
  return (state?.layers || []).filter((layer: any) => ids.has(layer.id));
}

function validateLocks(commands: any, state: any) {
  const locked = new Set((state.layers || []).filter((layer: any) => layer.locked).map((layer: any) => layer.id));
  for (const command of commands) {
    const targets = command.type === 'delete_layers'
      ? (Array.isArray(command.targets) ? command.targets : [command.target]).filter(Boolean)
      : [command.target].filter(Boolean);
    if (targets.some((target: any) => locked.has(target))) throw new Error('Scripts cannot change locked layers');
  }
}

function normalizeCommands(raw: any, state: any) {
  let source = raw && typeof raw === 'object' && !Array.isArray(raw) && Array.isArray(raw.commands)
    ? raw.commands : raw;
  if (source && typeof source === 'object' && !Array.isArray(source)) source = [source];
  if (!Array.isArray(source) || !source.length) throw new Error('The script did not return any source edits');
  if (source.length > LIMITS.commands) throw new Error(`The script returned more than ${LIMITS.commands} edits`);
  let serialized;
  try { serialized = JSON.stringify(source); } catch { throw new Error('The script result is not serializable'); }
  if (!serialized || serialized.length > LIMITS.result) throw new Error('The script result is too large');
  const commands = source.map((command: any) => PM.AgentHarness?.cleanCommand?.(command));
  if (commands.some((command: any) => !command)) throw new Error('The script returned an unsupported or malformed source edit');
  if (commands.some((command: any) => DENIED_OUTPUTS.has(command.type)
    || (command.type === 'replace_keyframes' && command.expression))) {
    throw new Error('Sandboxed scripts cannot install executable project expressions');
  }
  validateLocks(commands, state);
  return commands;
}

/* This worker owns the dynamic Function constructor. The app page never evals
   generated source. Its enclosing sandbox has an opaque origin and a CSP that
   blocks connections; the parent accepts only validated structured data. */
const WORKER_SOURCE = `
const publish = self.postMessage.bind(self);
const deepFreeze = value => {
  if (!value || typeof value !== 'object' || Object.isFrozen(value)) return value;
  Object.freeze(value); Object.values(value).forEach(deepFreeze); return value;
};
const copy = value => value === undefined ? undefined : JSON.parse(JSON.stringify(value));
const cleanPrefix = value => String(value || 'script').replace(/[^a-z0-9_-]/gi, '').slice(0, 28) || 'script';
self.onmessage = async event => {
  const message = event.data || {};
  let sequence = 0, emitted = [];
  try {
    const project = deepFreeze(copy(message.project || {}));
    const input = deepFreeze(copy(message.input || {}));
    const selectedIds = new Set(project.selection?.layers || []);
    const layers = deepFreeze(project.layers || []);
    const selectedLayers = deepFreeze(layers.filter(layer => selectedIds.has(layer.id)));
    const command = (type, value = {}) => ({ type, ...copy(value) });
    const sdk = {
      project, composition: project.composition, input, layers, selectedLayers,
      uid(prefix = 'script') { return cleanPrefix(prefix) + '-' + message.runId + '-' + (++sequence); },
      clone: copy,
      assert(condition, reason = 'Script requirement was not met') { if (!condition) throw new Error(reason); },
      emit(value) { emitted.push(copy(value)); return value; },
      addLayer(value) { return command('add_layer', value); },
      setLayer(target, patch) { return command('set_layer', { target, patch }); },
      setContent(target, patch) { return command('set_content', { target, patch }); },
      setProperty(target, path, value, options = {}) { return command('set_property', { target, path, value, ...copy(options) }); },
      replaceKeyframes(target, path, keyframes, options = {}) { return command('replace_keyframes', { target, path, keyframes, ...copy(options) }); },
      deleteLayers(targets) { return command('delete_layers', { targets: [].concat(targets || []) }); },
      groupLayers(targets, name) { return command('group_layers', { targets, name }); },
      ungroupLayers(targets) { return command('ungroup_layers', { targets }); },
      moveToGroup(targets, group) { return command('move_to_group', { targets, group }); },
      reorderLayer(target, index) { return command('reorder_layer', { target, index }); },
      setComposition(patch) { return command('set_composition', { patch }); },
      addEffect(target, effect, parameters = {}) { return command('add_effect', { target, effect, parameters }); },
      transformLayers(transform, state = input) { return command('transform_layers', { transform, state }); },
    };
    Object.freeze(sdk);
    const AsyncFunction = Object.getPrototypeOf(async function(){}).constructor;
    const prologue = '"use strict"; const window=undefined,document=undefined,self=undefined,globalThis=undefined,parent=undefined,top=undefined,opener=undefined,frames=undefined,navigator=undefined,location=undefined,fetch=undefined,XMLHttpRequest=undefined,WebSocket=undefined,EventSource=undefined,indexedDB=undefined,caches=undefined,localStorage=undefined,sessionStorage=undefined,importScripts=undefined,postMessage=undefined,Worker=undefined,SharedWorker=undefined,BroadcastChannel=undefined,MessageChannel=undefined,Function=undefined;\\n';
    const run = new AsyncFunction('PM', prologue + String(message.code || ''));
    const returned = await run(sdk);
    publish({ ok: true, result: returned === undefined ? emitted : returned });
  } catch (error) {
    publish({ ok: false, error: String(error?.message || error).slice(0, 500) });
  }
};`;

function frameDocument() {
  const worker = JSON.stringify(WORKER_SOURCE).replace(/</g, '\\u003c');
  return `<!doctype html><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline' 'unsafe-eval'; worker-src blob:; connect-src 'none'; img-src 'none'; media-src 'none'; frame-src 'none'"><script>
  const SOURCE=${worker};
  addEventListener('message', event => {
    const m=event.data||{}; if(m.channel!==${JSON.stringify(CHANNEL)}) return;
    let settled=false;
    const url=URL.createObjectURL(new Blob([SOURCE],{type:'text/javascript'}));
    const worker=new Worker(url); URL.revokeObjectURL(url);
    const finish=data=>{ if(settled)return; settled=true; clearTimeout(timer); worker.terminate(); parent.postMessage({channel:${JSON.stringify(RESULT_CHANNEL)},token:m.token,...data},'*'); };
    const timer=setTimeout(()=>finish({ok:false,error:'Script exceeded its time limit'}),m.runtime||900);
    worker.onmessage=e=>finish(e.data||{ok:false,error:'Script returned no result'});
    worker.onerror=event=>finish({ok:false,error:String(event?.message||'Script failed inside the isolated runtime').slice(0,500)});
    worker.postMessage({code:m.code,project:m.project,input:m.input,runId:m.runId});
  });
  <\/script>`;
}

function runIsolated(code: any, project: any, input: any = {}) {
  return new Promise((resolve: any, reject: any) => {
    if (!window.document?.body) { reject(new Error('The isolated runtime needs a browser document')); return; }
    const token = PM.uid('script-token'), runId = PM.uid('run').replace(/[^a-z0-9_-]/gi, '');
    const frame = window.document.createElement('iframe');
    frame.hidden = true;
    frame.setAttribute('sandbox', 'allow-scripts');
    frame.setAttribute('aria-hidden', 'true');
    let settled = false;
    const cleanup = () => { window.removeEventListener('message', receive); frame.remove(); };
    const finish = (error: any, value?: any) => {
      if (settled) return; settled = true; window.clearTimeout(timer); cleanup();
      if (error) reject(error); else resolve(value);
    };
    const receive = (event: any) => {
      const message = event.data || {};
      if (event.source !== frame.contentWindow || message.channel !== RESULT_CHANNEL || message.token !== token) return;
      finish(message.ok ? null : new Error(message.error || 'Script failed'), message.result);
    };
    const timer = window.setTimeout(() => finish(new Error('The isolated runtime did not respond')), LIMITS.runtime + 900);
    window.addEventListener('message', receive);
    frame.addEventListener('load', () => (frame.contentWindow as any).postMessage({
      channel: CHANNEL, token, code, project, input: clone(input), runId, runtime: LIMITS.runtime,
    }, '*'), { once: true });
    if ((window as any).powermove) frame.src = 'host/sandbox.html';
    else frame.srcdoc = frameDocument();
    window.document.body.appendChild(frame);
  });
}

function describe(commands: any) {
  return commands.map((command: any) => PM.AgentHarness?.describeCommand?.(command) || command.type);
}

async function prepare(code: any, input: any = {}, meta: any = {}) {
  const clean = sourceCode(code);
  if (!clean) return { ok: false, message: `Script source must be between 1 and ${LIMITS.source.toLocaleString()} characters`, commands: [], changes: [] };
  try {
    const state = snapshot();
    const raw = await runIsolated(clean, state, input);
    const commands = normalizeCommands(raw, state);
    return {
      ok: true,
      message: `${commands.length} validated source edit${commands.length === 1 ? '' : 's'} ready`,
      label: text(meta.label, 'Run generated tool', 80), commands,
      changes: describe(commands).map((description: any) => ({ description })),
      baseRevision: Number(state.composition?.revision) || 0,
    };
  } catch (error: any) {
    return { ok: false, message: String(error.message || error), commands: [], changes: [] };
  }
}

async function preview(code: any, input: any, meta: any) { return prepare(code, input, meta); }

async function apply(code: any, input: any = {}, meta: any = {}) {
  const prepared = await prepare(code, input, meta);
  if (!prepared.ok) return prepared;
  const applied = PM.Edit.apply(prepared.commands, {
    label: prepared.label, origin: meta.origin || 'generated-script', baseRevision: prepared.baseRevision,
  });
  if (applied.ok) {
    const added = prepared.commands.filter((command: any) => command.type === 'add_layer' && command.id).map((command: any) => command.id);
    if (added.length) PM.selectLayers(added);
  }
  return { ...prepared, ...applied, preview: prepared, changes: prepared.changes };
}

function canRun(action: any) {
  const required = new Set(action?.requiredTypes || []);
  if (!required.size) return true;
  const selected = PM.selLayers?.() || [];
  return selected.some((layer: any) => required.has(layer.type) && !layer.lock);
}

PM.Script = {
  LIMITS, sourceCode, snapshot, preview, apply, canRun,
  catalog: () => ({
    language: 'sandboxed-javascript', maxSourceCharacters: LIMITS.source,
    maxCommands: LIMITS.commands, timeoutMs: LIMITS.runtime,
    sdk: ['project', 'composition', 'input', 'layers', 'selectedLayers', 'uid', 'clone', 'assert', 'emit', 'addLayer', 'setLayer', 'setContent', 'setProperty', 'replaceKeyframes', 'deleteLayers', 'reorderLayer', 'groupLayers', 'ungroupLayers', 'moveToGroup', 'setComposition', 'addEffect', 'transformLayers'],
    guarantees: ['opaque origin', 'network blocked', 'timeout worker', 'read-only snapshot', 'validated commands', 'atomic apply', 'one-step undo'],
  }),
  test: { normalizeCommands, validateLocks, selectedLayers, frameDocument, workerSource: WORKER_SOURCE },
};
}
