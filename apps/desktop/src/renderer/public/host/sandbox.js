/* Powermove — generated-script sandbox host. */
(() => {
  'use strict';

  const CHANNEL = 'powermove-isolated-script';
  const RESULT_CHANNEL = 'powermove-isolated-script-result';
  const SOURCE = `
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

  addEventListener('message', event => {
    const message = event.data || {};
    if (message.channel !== CHANNEL) return;
    let settled = false;
    const url = URL.createObjectURL(new Blob([SOURCE], { type: 'text/javascript' }));
    const worker = new Worker(url);
    URL.revokeObjectURL(url);
    const finish = data => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      worker.terminate();
      parent.postMessage({ channel: RESULT_CHANNEL, token: message.token, ...data }, '*');
    };
    const timer = setTimeout(
      () => finish({ ok: false, error: 'Script exceeded its time limit' }),
      message.runtime || 900
    );
    worker.onmessage = event => finish(event.data || { ok: false, error: 'Script returned no result' });
    worker.onerror = event => finish({
      ok: false,
      error: String(event?.message || 'Script failed inside the isolated runtime').slice(0, 500)
    });
    worker.postMessage({
      code: message.code,
      project: message.project,
      input: message.input,
      runId: message.runId
    });
  });
})();
