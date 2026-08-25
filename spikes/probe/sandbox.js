'use strict';

addEventListener('message', (event) => {
  if (event.source !== parent) return;
  const message = event.data || {};
  if (!message.token || typeof message.code !== 'string') return;
  let parentReachable = true;
  try { void parent.document; } catch (_) { parentReachable = false; }
  const source = "onmessage=e=>{const code=e.data.code;const AF=(async()=>{}).constructor;new AF(code)().then(v=>postMessage({v})).catch(error=>postMessage({error:String(error&&error.message||error)}))}";
  const url = URL.createObjectURL(new Blob([source], { type: 'text/javascript' }));
  const workerResult = new Promise((resolve) => {
    const worker = new Worker(url);
    URL.revokeObjectURL(url);
    const timer = setTimeout(() => { worker.terminate(); resolve({ error: 'worker timed out' }); }, 4000);
    worker.onmessage = (workerEvent) => { clearTimeout(timer); worker.terminate(); resolve(workerEvent.data || {}); };
    worker.onerror = (workerEvent) => { clearTimeout(timer); worker.terminate(); resolve({ error: String(workerEvent.message || workerEvent) }); };
    worker.postMessage({ code: message.code });
  });
  const fetchResult = fetch('https://example.com').then(
    () => ({ fetchBlocked: false }),
    (error) => ({ fetchBlocked: true, fetchError: String(error && error.message || error) }),
  );
  Promise.all([workerResult, fetchResult]).then(([worker, fetchInfo]) => {
    parent.postMessage({ token: message.token, kind: 'final', worker, parentReachable, ...fetchInfo }, '*');
  });
}, false);

parent.postMessage({ kind: 'sandbox-ready', variant: 'V1' }, '*');
