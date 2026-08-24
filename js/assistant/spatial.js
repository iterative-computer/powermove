/* Powermove — spatial coding assistant.
   Shake the pointer to summon a full-window shader, drag over interface, describe
   the change, preview the generated section manifest, then apply it safely. */
(() => {
const PM = window.PM, h = PM.h;

/* The native Codex client owns ChatGPT authentication. This bridge only moves a
   prompt and a strict JSON schema across WKWebView; no account secret enters JS. */
const pending = new Map();
function codexAbortError(message = 'The previous agent run was replaced') {
  const error = new Error(message); error.name = 'AbortError'; return error;
}
PM.CodexBridge = {
  request(prompt, schema, images = [], options = {}) {
    const bridge = window.webkit?.messageHandlers?.pmCodex;
    if (!bridge) return Promise.reject(new Error('The coding agent is available in the Powermove macOS app'));
    const id = PM.uid('spatial-codex-');
    return new Promise((resolve, reject) => {
      const signal = options.signal;
      const stopNative = () => window.webkit?.messageHandlers?.pmCodexCancel?.postMessage({ id });
      const settle = (error) => {
        const job = pending.get(id); if (!job) return;
        pending.delete(id); clearTimeout(job.timer);
        job.signal?.removeEventListener('abort', job.abort);
        stopNative(); reject(error);
      };
      const abort = () => settle(codexAbortError());
      const timer = setTimeout(() => settle(new Error('The coding agent took too long to respond')), 120000);
      pending.set(id, { resolve, reject, timer, signal, abort, onProgress: options.onProgress });
      if (signal?.aborted) { abort(); return; }
      signal?.addEventListener('abort', abort, { once: true });
      bridge.postMessage({
        id, prompt, schema, images: images.slice(0, 6),
        model: options.model || '', reasoningEffort: options.reasoningEffort || '',
      });
    });
  },
  resolve(id, result) {
    const job = pending.get(id); if (!job) return;
    pending.delete(id); clearTimeout(job.timer);
    job.signal?.removeEventListener('abort', job.abort);
    let text = '';
    try {
      const bytes = Uint8Array.from(atob(result.dataBase64 || ''), c => c.charCodeAt(0));
      text = new TextDecoder().decode(bytes);
    } catch { text = 'The coding-agent response could not be decoded'; }
    if (result.ok) job.resolve(text); else job.reject(new Error(text));
  },
  progress(id, result) {
    const job = pending.get(id); if (!job || typeof job.onProgress !== 'function') return;
    try {
      const bytes = Uint8Array.from(atob(result.dataBase64 || ''), c => c.charCodeAt(0));
      const summary = new TextDecoder().decode(bytes).replace(/\s+/g, ' ').trim().slice(0, 320);
      if (summary) job.onProgress(summary);
    } catch { /* Ignore malformed progress without interrupting the real run. */ }
  },
};

/* WebGPU can only displace pixels it can sample. The native shell captures the
   WKWebView before the overlay mounts, then this bridge decodes that frame into
   an ImageBitmap suitable for copyExternalImageToTexture(). */
const pendingCaptures = new Map();
PM.WindowCapture = {
  request() {
    const bridge = window.webkit?.messageHandlers?.pmCaptureWindow;
    if (!bridge) return Promise.resolve(null);
    const id = PM.uid('window-capture-');
    return new Promise(resolve => {
      const timer = setTimeout(() => { pendingCaptures.delete(id); resolve(null); }, 700);
      pendingCaptures.set(id, { resolve, timer });
      bridge.postMessage({ id });
    });
  },
  async resolve(id, result) {
    const job = pendingCaptures.get(id); if (!job) return;
    pendingCaptures.delete(id); clearTimeout(job.timer);
    if (!result?.ok || !result.dataBase64) { job.resolve(null); return; }
    try {
      const bytes = Uint8Array.from(atob(result.dataBase64), c => c.charCodeAt(0));
      job.resolve(await createImageBitmap(new Blob([bytes], { type: 'image/png' })));
    } catch (error) { console.warn('Could not decode the interface snapshot', error); job.resolve(null); }
  },
};

const S = {
  initialized: false, active: false, pressed: false, phase: 'idle',
  samples: [], points: [], lastTrigger: 0, origin: { x: 0, y: 0 },
  root: null, ink: null, path: null, shadePath: null, hint: null, card: null, outline: null,
  region: null, context: null, plan: null, renderStop: null, requestToken: 0,
  requestText: '', run: null, conversation: [], activity: '', activeRequest: null, composerDraft: '',
  rippleWarmup: null, sceneCache: null, sceneCacheAt: 0, cachePending: null,
  sceneFrame: null, regionImage: null,
  hintFrame: 0, hintPoint: null,
  panelBody: null, attachments: [], requestAttachments: [], steps: [], stepsExpanded: false,
  pendingEntering: false,
  panelRun: null, scope: PM.store?.get?.('agentScope', 'workspace') || 'workspace',
  autoApplyPanels: PM.store?.get?.('agentAutoApplyPanels', true) !== false,
  model: PM.store?.get?.('agentModel', 'gpt-5.6-sol') || 'gpt-5.6-sol',
  reasoningEffort: PM.store?.get?.('agentReasoningEffort', 'high') || 'high',
};

const AGENT_MODELS = [
  { id: 'gpt-5.6-sol', label: '5.6 Sol' },
  { id: 'gpt-5.6-terra', label: '5.6 Terra' },
  { id: 'gpt-5.6-luna', label: '5.6 Luna' },
];
const REASONING_EFFORTS = ['low', 'medium', 'high', 'xhigh', 'max'];
const SEND_TRANSITION_MS = 240;

const Spatial = {
  init,
  activate,
  open: openAgentPanel,
  cancel,
  get active() { return S.active; },
  /* Small pure seams are exposed for deterministic regression tests. */
  math: { motionProfile, shakeReady, shakeIntent, selectionRect, bitmapCropRect, isClickGesture, overlayPointerAction, pointInPolygon, sanitizePlan, sanitizePanelEdit, applyPanelEdit, applyChromeEdit, hintPosition, clampFloatingPosition, textareaLayout, composerMode },
  lifecycle: { requestAdapter: requestRippleAdapter },
};
PM.SpatialAssistant = Spatial;

PM.registerPanel('agent', {
  title: 'Powermove agent', size: 350, min: 240, persist: true, noscroll: true,
  build(body) {
    S.panelBody = body;
    body.classList.add('agent-panel-body');
    renderConversation();
  },
});

function init() {
  if (S.initialized) return;
  S.initialized = true;
  addEventListener('pointerdown', () => { S.pressed = true; }, true);
  addEventListener('pointerup', () => { S.pressed = false; }, true);
  addEventListener('pointercancel', () => { S.pressed = false; }, true);
  addEventListener('pointermove', watchShake, true);
  refreshSceneCache();
}

function openAgentPanel() {
  if (PM.ProjectsScreen?.isOpen) PM.ProjectsScreen.hide();
  if (PM.LibraryUI?.isOpen) PM.LibraryUI.close?.();
  const workspace = PM.WS?.current;
  if (!workspace) return;
  const visible = PM.Layout.hasPanel(workspace, 'agent');
  const hidden = (workspace.hiddenPanels || []).some(item => item.id === 'agent');
  if (!visible) {
    PM.WS.mutate(draft => {
      if (hidden) PM.Layout.restorePanel(draft, 'agent');
      else PM.Layout.addPanel(draft, 'agent', 'right');
    });
  }
  requestAnimationFrame(() => {
    renderConversation(true);
    PM.panelInst.agent?.el?.scrollIntoView?.({ block: 'nearest', inline: 'nearest' });
  });
}

function requestRippleAdapter() {
  return navigator.gpu?.requestAdapter?.({ powerPreference: 'high-performance' }) || null;
}

function watchShake(event) {
  /* Once summoned, keep the short-lived distortion centered on the live
     pointer. S.origin is the same object read by the WebGPU render loop. */
  if (S.active) {
    const live = event.getCoalescedEvents?.().at(-1) || event;
    S.origin.x = live.clientX; S.origin.y = live.clientY;
    scheduleHint(live.clientX, live.clientY);
    return;
  }
  if (!isEditorPointer(event)) { S.samples = []; return; }
  if (S.pressed || event.buttons || performance.now() - S.lastTrigger < 1600) return;
  if (!S.cachePending && performance.now() - S.sceneCacheAt > 1100) refreshSceneCache();
  const events = event.getCoalescedEvents?.().length ? event.getCoalescedEvents() : [event];
  for (const e of events) {
    /* event.timeStamp preserves the real spacing of coalesced samples. Using
       performance.now() for every point made fast mice look like zero-time
       teleports and slow event streams look artificially weak. */
    const eventTime = Number.isFinite(e.timeStamp) ? e.timeStamp : performance.now();
    const sample = { x: e.clientX, y: e.clientY, t: eventTime };
    const last = S.samples[S.samples.length - 1];
    if (!last || sample.t > last.t && Math.hypot(sample.x - last.x, sample.y - last.y) >= 1.5) S.samples.push(sample);
  }
  const newest = S.samples.at(-1)?.t ?? performance.now();
  const cutoff = newest - 900;
  S.samples = S.samples.filter(p => p.t >= cutoff).slice(-160);
  if (shakeIntent(S.samples) && !S.rippleWarmup) warmRipple();
  if (shakeReady(S.samples)) {
    const p = S.samples[S.samples.length - 1];
    S.samples = []; S.lastTrigger = performance.now();
    const warmup = S.rippleWarmup;
    if (warmup) warmup.claimed = true;
    S.rippleWarmup = null;
    activate(p.x, p.y, warmup);
  }
}

function isEditorPointer(event) {
  const target = event?.target;
  return window.opener == null
    && !(PM.ProjectsScreen && PM.ProjectsScreen.isOpen)
    && !(PM.LibraryUI && PM.LibraryUI.isOpen)
    && !document.querySelector('#scrim.on,.modal')
    && !!target?.closest?.('#body');
}

function motionProfile(points) {
  if (!Array.isArray(points) || points.length < 3) return { duration: 0, path: 0, span: 0, net: 0, reversals: 0, oscillation: 0, peakSpeed: 0, energy: 0 };
  let path = 0, reversals = 0, oscillation = 0, peakSpeed = 0, energy = 0;
  let minX = points[0].x, maxX = minX, minY = points[0].y, maxY = minY;
  let priorVelocity = null, distanceSinceTurn = 0;
  for (let i = 1; i < points.length; i++) {
    const dt = points[i].t - points[i - 1].t;
    if (!(dt > 0) || dt > 140) { priorVelocity = null; distanceSinceTurn = 0; continue; }
    const dx = points[i].x - points[i - 1].x;
    const dy = points[i].y - points[i - 1].y;
    const distance = Math.hypot(dx, dy);
    if (distance < .5) continue;
    const seconds = Math.max(dt, 4) / 1000;
    const velocity = { x: dx / seconds, y: dy / seconds };
    const speed = Math.hypot(velocity.x, velocity.y);
    path += distance; distanceSinceTurn += distance;
    peakSpeed = Math.max(peakSpeed, speed);
    energy += speed * speed * seconds;
    if (priorVelocity) {
      const priorSpeed = Math.hypot(priorVelocity.x, priorVelocity.y);
      const alignment = (velocity.x * priorVelocity.x + velocity.y * priorVelocity.y) / (speed * priorSpeed);
      /* A reversal needs real momentum and travel on both sides. This rejects
         hand tremor/high-frequency sensor jitter without penalizing event rate. */
      if (speed >= 260 && priorSpeed >= 260 && alignment < -.35 && distanceSinceTurn >= 18) {
        reversals++; oscillation += distanceSinceTurn; distanceSinceTurn = 0;
      }
    }
    if (speed >= 120) priorVelocity = velocity;
    minX = Math.min(minX, points[i].x); maxX = Math.max(maxX, points[i].x);
    minY = Math.min(minY, points[i].y); maxY = Math.max(maxY, points[i].y);
  }
  const duration = Math.max(0, points.at(-1).t - points[0].t);
  const net = Math.hypot(points.at(-1).x - points[0].x, points.at(-1).y - points[0].y);
  return { duration, path, span: Math.max(maxX - minX, maxY - minY), net, reversals, oscillation, peakSpeed, energy: duration ? energy / (duration / 1000) : 0 };
}

function shakeIntent(points) {
  const m = motionProfile(points);
  return m.duration <= 900 && m.path >= 72 && m.span >= 32 && m.peakSpeed >= 420 && m.reversals >= 1;
}

function warmRipple() {
  const warmup = {
    claimed: false,
    /* A selected-region attachment must represent this gesture, not an older
       idle cache. Start a fresh native snapshot as soon as shake intent is clear. */
    capture: PM.WindowCapture.request(),
    /* A GPUAdapter is deliberately warmed per gesture. WebKit can invalidate a
       long-lived adapter after a device is destroyed or lost. Reusing one from
       app boot made every later Ripple device arrive already lost. */
    adapter: requestRippleAdapter(),
  };
  S.rippleWarmup = warmup;
  setTimeout(() => {
    if (warmup.claimed || S.rippleWarmup !== warmup) return;
    S.rippleWarmup = null;
    warmup.capture?.then(bitmap => bitmap?.close?.());
  }, 1000);
}

function refreshSceneCache() {
  if (S.active || S.cachePending) return;
  const request = PM.WindowCapture.request();
  S.cachePending = request;
  request.then(bitmap => {
    if (S.cachePending !== request) { bitmap?.close?.(); return; }
    S.cachePending = null;
    if (!bitmap || S.active) { bitmap?.close?.(); return; }
    S.sceneCache?.close?.();
    S.sceneCache = bitmap; S.sceneCacheAt = performance.now();
  });
}

function shakeReady(points) {
  const m = motionProfile(points);
  /* Three momentum reversals over about 180 CSS pixels is a short intentional
     shake. CSS pixels make the gesture consistent across Retina scale factors;
     timestamp-normalized speed makes it consistent across mouse event rates. */
  return m.duration >= 120 && m.duration <= 900
    && m.path >= 180 && m.span >= 44 && m.oscillation >= 108
    && m.peakSpeed >= 430 && m.reversals >= 3 && m.net < m.path * .62;
}

function hintPosition(x, y, width, height, viewportWidth, viewportHeight, offset = 18) {
  const pad = 12;
  const left = PM.clamp(x + offset, pad, Math.max(pad, viewportWidth - width - pad));
  const below = y + offset;
  const top = below + height + pad <= viewportHeight
    ? below
    : PM.clamp(y - height - offset, pad, Math.max(pad, viewportHeight - height - pad));
  return { x: Math.round(left), y: Math.round(top) };
}

function scheduleHint(x, y) {
  S.hintPoint = { x, y };
  if (!S.hint || S.hintFrame) return;
  S.hintFrame = requestAnimationFrame(() => {
    S.hintFrame = 0;
    if (!S.hint || !S.hintPoint) return;
    /* The prompt replaces the instruction pill while the assistant is waiting
       for a gesture. Keep the editable surface attached to the live cursor. */
    if (S.card && !S.context?.targetPanelId && ['arming', 'selecting', 'composing'].includes(S.phase)) {
      S.hint.style.display = 'none';
      const p = hintPosition(
        S.hintPoint.x, S.hintPoint.y,
        S.card.offsetWidth || 390, S.card.offsetHeight || 52,
        innerWidth, innerHeight,
      );
      Object.assign(S.card.style, { left: p.x + 'px', top: p.y + 'px' });
      return;
    }
    const p = hintPosition(
      S.hintPoint.x, S.hintPoint.y,
      S.hint.offsetWidth || 250, S.hint.offsetHeight || 38,
      innerWidth, innerHeight,
    );
    S.hint.style.transform = `translate3d(${p.x}px,${p.y}px,0)`;
  });
}

function activate(x, y, warmup = null) {
  if (S.active) return;
  S.active = true; S.phase = 'arming'; S.origin = { x, y }; S.points = [];
  const canvas = h('canvas', { 'aria-hidden': 'true' });
  S.shadePath = document.createElementNS('http://www.w3.org/2000/svg', 'path');
  S.shadePath.classList.add('spatial-shade');
  S.path = document.createElementNS('http://www.w3.org/2000/svg', 'path');
  S.ink = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  S.ink.classList.add('spatial-ink'); S.ink.append(S.shadePath, S.path);
  S.hint = h('div.spatial-hint', h('span', 'Type a prompt or drag to select an area'));
  S.root = h('div#spatial-assistant', { role: 'dialog', 'aria-label': 'Spatial coding assistant' },
    canvas, h('div.spatial-wash'), S.ink, S.hint);
  addEventListener('keydown', onKey, true);
  /* Capture first, while the overlay is not in the DOM. This is the texture the
     WGSL pass genuinely displaces instead of merely painting over the UI. */
  const cachedScene = S.sceneCache;
  if (cachedScene) { S.sceneCache = null; S.sceneCacheAt = 0; }
  const sceneRequest = warmup?.capture
    ? warmup.capture.then(fresh => {
      if (fresh) { cachedScene?.close?.(); return fresh; }
      return cachedScene;
    })
    : cachedScene ? Promise.resolve(cachedScene) : PM.WindowCapture.request();
  sceneRequest.then(sceneBitmap => {
    if (!S.active) { sceneBitmap?.close?.(); return; }
    /* Preserve clean pre-overlay pixels for the eventual selected-region
       attachment before WebGPU uploads and closes the ImageBitmap. */
    S.sceneFrame = snapshotScene(sceneBitmap);
    S.root.addEventListener('pointerdown', onOverlayPointerDown);
    document.body.appendChild(S.root);
    S.region = { x, y, width: 1, height: 1 };
    S.context = {
      targetPanelId: '', targetTitle: 'full composition and workspace',
      rect: { x: Math.round(x), y: Math.round(y), width: 1, height: 1 }, panels: [], elements: [],
    };
    showComposer();
    scheduleHint(x, y);
    S.renderStop = startRipple(canvas, S.origin, sceneBitmap, warmup?.adapter || null);
    setTimeout(() => { if (S.active && S.phase === 'arming') S.phase = 'selecting'; }, 340);
  });
}

function overlayPointerAction(phase, button, insideComposer) {
  if (button !== 0 || insideComposer) return 'ignore';
  if (phase === 'selecting') return 'select';
  if (phase === 'arming' || phase === 'composing') return 'cancel';
  return 'ignore';
}

function onOverlayPointerDown(event) {
  const action = overlayPointerAction(S.phase, event.button, !!event.target?.closest?.('.spatial-compose'));
  if (action === 'cancel') {
    event.preventDefault();
    cancel();
    return;
  }
  if (action === 'select') beginSelection(event);
}

function beginSelection(event) {
  event.preventDefault();
  S.phase = 'drawing'; S.points = [{ x: event.clientX, y: event.clientY }];
  showOutline({ x: event.clientX, y: event.clientY, width: 1, height: 1 }, true);
  S.root.setPointerCapture?.(event.pointerId);
  const move = e => {
    const p = { x: e.clientX, y: e.clientY };
    S.points[1] = p;
    updateOutline(selectionRect(S.points[0], p));
  };
  const finish = e => {
    S.root.removeEventListener('pointermove', move);
    S.root.removeEventListener('pointerup', finish);
    S.root.removeEventListener('pointercancel', abort);
    S.root.releasePointerCapture?.(event.pointerId);
    finishSelection(e);
  };
  const abort = () => {
    S.root.removeEventListener('pointermove', move);
    S.root.removeEventListener('pointerup', finish);
    S.root.removeEventListener('pointercancel', abort);
    S.outline?.remove(); S.outline = null;
    resetSelection('Drag across any part of the interface');
  };
  S.root.addEventListener('pointermove', move);
  S.root.addEventListener('pointerup', finish);
  S.root.addEventListener('pointercancel', abort);
}

function finishSelection(event) {
  if (event) S.points[1] = { x: event.clientX, y: event.clientY };
  if (isClickGesture(S.points)) { cancel(); return; }
  const rect = selectionRect(S.points[0], S.points[1]);
  if (rect.width < 34 || rect.height < 34) {
    S.outline?.remove(); S.outline = null;
    resetSelection('Drag across a larger interface area');
    return;
  }
  S.points = rectanglePolygon(rect);
  S.shadePath.setAttribute('fill-rule', 'evenodd');
  S.shadePath.setAttribute('d', `M 0 0 H ${innerWidth} V ${innerHeight} H 0 Z M ${rect.x} ${rect.y} H ${rect.x + rect.width} V ${rect.y + rect.height} H ${rect.x} Z`);
  S.root.classList.add('target-selected');
  const draft = S.card?.querySelector('textarea')?.value || '';
  S.region = rect;
  S.context = inspectRegion(S.points, S.region);
  S.scope = S.context.targetPanelId ? `panel:${S.context.targetPanelId}` : 'workspace';
  PM.store.set('agentScope', S.scope);
  const regionCapture = captureRegionImage(S.sceneFrame, S.region);
  S.regionImage = regionCapture?.dataUrl || null;
  S.context.visualReference = regionCapture ? {
    attachment: 'image-1', width: regionCapture.width, height: regionCapture.height,
    sourceRect: regionCapture.sourceRect,
  } : { attachment: '', unavailable: true };
  S.phase = 'composing';
  S.hint.style.display = 'none';
  S.outline?.classList.remove('selecting'); S.outline?.classList.add('confirmed');
  showComposer(draft);
}

function isClickGesture(points) {
  if (!Array.isArray(points) || !points.length) return false;
  const start = points[0];
  return points.every(p => Math.hypot(p.x - start.x, p.y - start.y) <= 7);
}

function selectionRect(start, end) {
  if (!start || !end) return { x: 0, y: 0, width: 0, height: 0 };
  return {
    x: Math.round(Math.min(start.x, end.x)), y: Math.round(Math.min(start.y, end.y)),
    width: Math.round(Math.abs(end.x - start.x)), height: Math.round(Math.abs(end.y - start.y)),
  };
}

function bitmapCropRect(rect, bitmapWidth, bitmapHeight, viewportWidth, viewportHeight, maxEdge = 1280) {
  const scaleX = bitmapWidth / Math.max(1, viewportWidth);
  const scaleY = bitmapHeight / Math.max(1, viewportHeight);
  const sx = Math.max(0, Math.min(bitmapWidth - 1, Math.round(rect.x * scaleX)));
  const sy = Math.max(0, Math.min(bitmapHeight - 1, Math.round(rect.y * scaleY)));
  const sw = Math.max(1, Math.min(bitmapWidth - sx, Math.round(rect.width * scaleX)));
  const sh = Math.max(1, Math.min(bitmapHeight - sy, Math.round(rect.height * scaleY)));
  const outputScale = Math.min(1, maxEdge / Math.max(sw, sh));
  return {
    sx, sy, sw, sh,
    width: Math.max(1, Math.round(sw * outputScale)),
    height: Math.max(1, Math.round(sh * outputScale)),
  };
}

function snapshotScene(bitmap) {
  if (!bitmap?.width || !bitmap?.height) return null;
  try {
    const scale = Math.min(1, 2400 / Math.max(bitmap.width, bitmap.height));
    const canvas = document.createElement('canvas');
    canvas.width = Math.max(1, Math.round(bitmap.width * scale));
    canvas.height = Math.max(1, Math.round(bitmap.height * scale));
    const context = canvas.getContext('2d', { alpha: false });
    if (!context) return null;
    context.drawImage(bitmap, 0, 0, canvas.width, canvas.height);
    return canvas;
  } catch (error) {
    console.warn('Could not retain the interface snapshot for selected-region context', error);
    return null;
  }
}

function captureRegionImage(sceneFrame, rect) {
  if (!sceneFrame?.width || !sceneFrame?.height || !rect?.width || !rect?.height) return null;
  try {
    const crop = bitmapCropRect(rect, sceneFrame.width, sceneFrame.height, innerWidth, innerHeight);
    const canvas = document.createElement('canvas');
    canvas.width = crop.width; canvas.height = crop.height;
    const context = canvas.getContext('2d', { alpha: false });
    if (!context) return null;
    context.drawImage(
      sceneFrame, crop.sx, crop.sy, crop.sw, crop.sh, 0, 0, crop.width, crop.height,
    );
    return {
      dataUrl: canvas.toDataURL('image/jpeg', .92), width: crop.width, height: crop.height,
      sourceRect: { x: Math.round(rect.x), y: Math.round(rect.y), width: Math.round(rect.width), height: Math.round(rect.height) },
    };
  } catch (error) {
    console.warn('Could not crop the selected interface region', error);
    return null;
  }
}

function rectanglePolygon(rect) {
  return [
    { x: rect.x, y: rect.y }, { x: rect.x + rect.width, y: rect.y },
    { x: rect.x + rect.width, y: rect.y + rect.height }, { x: rect.x, y: rect.y + rect.height },
  ];
}

function resetSelection(message) {
  S.phase = 'selecting'; S.points = []; S.path.setAttribute('d', '');
  S.regionImage = null;
  S.shadePath?.setAttribute('d', ''); S.root?.classList.remove('target-selected');
  if (S.card && !S.context?.targetPanelId) {
    const input = S.card.querySelector('textarea');
    if (input) {
      const original = 'Full composition';
      input.placeholder = message;
      setTimeout(() => { if (S.active && S.phase === 'selecting' && input) input.placeholder = original; }, 1400);
    }
    S.hint.style.display = 'none';
    scheduleHint(S.hintPoint?.x ?? S.origin.x, S.hintPoint?.y ?? S.origin.y);
    return;
  }
  S.hint.style.display = ''; S.hint.querySelector('span').textContent = message;
  setTimeout(() => {
    if (S.active && S.phase === 'selecting') S.hint.querySelector('span').textContent = 'Type a prompt or drag to select an area';
  }, 1400);
}

function pointInPolygon(point, polygon) {
  let inside = false;
  for (let i = 0, j = polygon.length - 1; i < polygon.length; j = i++) {
    const a = polygon[i], b = polygon[j];
    const hit = ((a.y > point.y) !== (b.y > point.y))
      && point.x < (b.x - a.x) * (point.y - a.y) / ((b.y - a.y) || .00001) + a.x;
    if (hit) inside = !inside;
  }
  return inside;
}

function inspectRegion(polygon, rect) {
  const seen = new Map(), panels = new Map();
  S.root.style.pointerEvents = 'none';
  for (let gy = 0; gy < 5; gy++) for (let gx = 0; gx < 5; gx++) {
    const p = { x: rect.x + rect.width * (gx + .5) / 5, y: rect.y + rect.height * (gy + .5) / 5 };
    if (!pointInPolygon(p, polygon)) continue;
    document.elementsFromPoint(p.x, p.y).forEach(el => {
      if (el === document.body || el === document.documentElement || el.closest('#spatial-assistant')) return;
      const panel = el.closest('.panel');
      if (panel?.dataset.panel) panels.set(panel.dataset.panel, (panels.get(panel.dataset.panel) || 0) + 1);
      const key = el.id || `${el.tagName}.${[...el.classList].slice(0, 3).join('.')}`;
      if (!seen.has(key) && seen.size < 18) seen.set(key, describeElement(el));
    });
  }
  S.root.style.pointerEvents = '';
  const ranked = [...panels].sort((a, b) => b[1] - a[1]);
  const targetPanelId = ranked[0]?.[0] || '';
  const target = targetPanelId ? PM.$(`#panel-${CSS.escape(targetPanelId)}`) : null;
  return {
    targetPanelId,
    targetTitle: target?.querySelector('.ptitle')?.textContent?.trim() || (targetPanelId ? PM.PANELS[targetPanelId]?.title : '') || 'interface area',
    rect: { x: Math.round(rect.x), y: Math.round(rect.y), width: Math.round(rect.width), height: Math.round(rect.height) },
    panels: ranked.map(([id]) => ({ id, title: PM.PANELS[id]?.title || id })).slice(0, 5),
    elements: [...seen.values()],
  };
}

function describeElement(el) {
  return {
    tag: el.tagName.toLowerCase(), id: el.id || '',
    classes: [...el.classList].slice(0, 4),
    label: (el.getAttribute('aria-label') || el.getAttribute('title') || '').slice(0, 100),
    text: String(el.innerText || el.textContent || '').replace(/\s+/g, ' ').trim().slice(0, 180),
  };
}

function updateOutline(rect) {
  if (!S.outline) return;
  Object.assign(S.outline.style, {
    left: Math.max(4, rect.x) + 'px', top: Math.max(4, rect.y) + 'px',
    width: Math.max(1, Math.min(innerWidth - Math.max(4, rect.x) - 4, rect.width)) + 'px',
    height: Math.max(1, Math.min(innerHeight - Math.max(4, rect.y) - 4, rect.height)) + 'px',
  });
}

function showOutline(rect, selecting = false) {
  S.outline?.remove();
  S.outline = h('div.spatial-outline', { style: {
    left: Math.max(4, rect.x) + 'px', top: Math.max(4, rect.y) + 'px',
    width: Math.max(1, rect.width) + 'px', height: Math.max(1, rect.height) + 'px',
  } });
  if (selecting) S.outline.classList.add('selecting');
  S.root.appendChild(S.outline);
}

function cardPosition(card, rect) {
  const left = PM.clamp(rect.x + rect.width + 14, 14, innerWidth - Math.min(420, innerWidth - 28) - 14);
  let top = PM.clamp(rect.y, 60, innerHeight - card.offsetHeight - 14);
  if (left < rect.x + rect.width && rect.y + rect.height + 14 + card.offsetHeight < innerHeight) top = rect.y + rect.height + 14;
  Object.assign(card.style, { left: Math.round(left) + 'px', top: Math.round(top) + 'px' });
}

function composerPosition(card) {
  /* Full-composition prompts belong to the visible canvas, not the pointer's
     activation point. Selected-area edits remain spatially anchored to their region. */
  if (S.context?.targetPanelId) { cardPosition(card, S.region); return; }
  const canvas = document.querySelector('#stage-inner')?.getBoundingClientRect();
  if (!canvas || !canvas.width || !canvas.height) { cardPosition(card, S.region); return; }
  const width = card.offsetWidth || 390;
  const p = clampFloatingPosition(canvas.left + 22, canvas.top + 18, width, card.offsetHeight || 92, innerWidth, innerHeight, 14);
  Object.assign(card.style, { left: p.x + 'px', top: p.y + 'px' });
}

function clampFloatingPosition(x, y, width, height, viewportWidth, viewportHeight, pad = 12) {
  return {
    x: Math.round(PM.clamp(x, pad, Math.max(pad, viewportWidth - width - pad))),
    y: Math.round(PM.clamp(y, pad, Math.max(pad, viewportHeight - height - pad))),
  };
}

function moveCardTo(card, x, y) {
  const p = clampFloatingPosition(x, y, card.offsetWidth || 420, card.offsetHeight || 180, innerWidth, innerHeight);
  Object.assign(card.style, { left: p.x + 'px', top: p.y + 'px' });
  return p;
}

function textareaLayout(scrollHeight, minHeight, maxHeight) {
  const contentHeight = Math.max(0, Math.ceil(Number(scrollHeight) || 0));
  const min = Number.isFinite(minHeight) ? Math.max(0, minHeight) : 0;
  const max = Number.isFinite(maxHeight) ? Math.max(min, maxHeight) : Number.POSITIVE_INFINITY;
  const height = Math.min(max, Math.max(min, contentHeight));
  return { height, overflowY: contentHeight > height + 1 ? 'auto' : 'hidden' };
}

function composerMode(phase) {
  const working = phase === 'working';
  return {
    working,
    disabled: phase === 'applying',
    placeholder: working ? 'Add direction while the agent works…' : 'Describe what you want changed…',
    sendLabel: working ? 'Steer current run' : 'Send message',
  };
}

function autosizeTextarea(input, onResize) {
  const resize = () => {
    /* A detached panel keeps its authoritative controls in a hidden source host.
       Measuring that narrow/offscreen copy reports an enormous scrollHeight and
       used to make the visible prompt jump to its maximum height on every key. */
    if (input.closest('[data-popout-source]')) return;
    input.style.height = 'auto';
    const style = getComputedStyle(input);
    const layout = textareaLayout(input.scrollHeight, Number.parseFloat(style.minHeight), Number.parseFloat(style.maxHeight));
    input.style.height = `${layout.height}px`;
    input.style.overflowY = layout.overflowY;
    onResize?.();
  };
  input.addEventListener('input', resize);
  resize();
}

function cardCoordinates(card) {
  const left = Number.parseFloat(card.style.left), top = Number.parseFloat(card.style.top);
  if (Number.isFinite(left) && Number.isFinite(top)) return { left, top };
  return card.getBoundingClientRect();
}

function makeCardMovable(card, handle) {
  handle.tabIndex = 0; handle.setAttribute('role', 'button');
  handle.setAttribute('aria-label', 'Move assistant result');
  handle.addEventListener('pointerdown', event => {
    if (event.button !== 0) return;
    event.stopPropagation();
    const rect = cardCoordinates(card);
    /* The conversation's parent is deliberately click-through. Use the shared
       window-level drag helper so movement continues after leaving the header. */
    PM.drag(event, {
      cursor: 'grabbing',
      move: (dx, dy) => moveCardTo(card, rect.left + dx, rect.top + dy),
    });
  });
  handle.addEventListener('keydown', event => {
    if (!['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(event.key)) return;
    event.preventDefault(); event.stopPropagation();
    const rect = card.getBoundingClientRect(), step = event.shiftKey ? 24 : 8;
    const dx = event.key === 'ArrowLeft' ? -step : event.key === 'ArrowRight' ? step : 0;
    const dy = event.key === 'ArrowUp' ? -step : event.key === 'ArrowDown' ? step : 0;
    moveCardTo(card, rect.left + dx, rect.top + dy);
  });
}

function showComposer(draft = '') {
  S.card?.remove();
  const selectedContext = !!S.context.targetPanelId;
  const input = h('textarea', {
    placeholder: selectedContext ? 'How should this area change?' : 'Full composition',
    rows: '1', 'data-autosize': 'true',
  });
  input.value = draft;
  const status = h('span.spatial-status', 'Enter to send');
  const cancelBtn = h('button.spatial-action', { onclick: cancel }, 'Cancel');
  const sendBtn = h('button.spatial-action.pri.spatial-send', { 'aria-label': 'Press Enter to send', title: 'Press Enter to send', onclick: () => sendRequest(input) }, PM.icon('return'));
  input.addEventListener('keydown', e => {
    e.stopPropagation();
    if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); sendRequest(input); }
  });
  const handle = h('div.spatial-target', selectedContext ? `Selected · ${S.context.targetTitle}` : 'Powermove agent · full composition');
  const contextLabel = selectedContext ? h('span.spatial-context-label', `Selected ${S.context.targetTitle}`) : null;
  S.card = h('div.spatial-compose.compact.full',
    handle,
    h('div.spatial-input-row', contextLabel, input, sendBtn),
    h('div.spatial-actions', status, cancelBtn));
  S.root.appendChild(S.card);
  autosizeTextarea(input, () => {
    if (!S.card || !input.isConnected) return;
    const position = cardCoordinates(S.card);
    moveCardTo(S.card, position.left, position.top);
  });
  /* Place and focus immediately as well as on the next frame. A cached window
     capture can make repeat activations complete inside the click dispatch;
     waiting only for rAF left the card at an unpositioned static location. */
  composerPosition(S.card);
  input.focus();
  requestAnimationFrame(() => {
    if (!S.active || !S.card) return;
    composerPosition(S.card);
    input.focus();
  });
}

function conversationReply(plan) {
  if (plan.kind === 'scene') return plan.sceneEdit.summary || plan.message || 'I prepared an editable scene change.';
  if (plan.kind === 'panels') return plan.message || `I prepared ${plan.panelEdit.actions.length} panel changes.`;
  if (plan.kind === 'workspace') return plan.message || `I prepared the ${plan.workspaceEdit.name} workspace.`;
  if (plan.kind === 'interface') return plan.message || 'I prepared a structured Timeline redesign.';
  if (plan.kind === 'chrome') return plan.message || 'I prepared an interface change.';
  return plan.message || `I prepared the ${plan.section.title} section.`;
}

function promoteToConversation() {
  dismissOverlay(true);
  openAgentPanel();
}

function planPreview() {
  const plan = S.plan;
  if (!plan) return null;
  const chrome = plan.kind === 'chrome';
  const interfaceChange = plan.kind === 'interface';
  const scene = plan.kind === 'scene';
  const workspace = plan.kind === 'workspace';
  const panels = plan.kind === 'panels';
  const title = panels ? 'Panel arrangement' : interfaceChange ? 'Timeline redesign' : chrome ? 'Interface edit' : scene ? plan.sceneEdit.label : workspace ? plan.workspaceEdit.name : plan.section.title;
  const preview = h('div.spatial-proposal',
    h('div.spatial-proposal-kicker', 'Proposed change'),
    h('h3', title));
  if (panels) plan.panelEdit.actions.forEach(action => preview.appendChild(h('div.spatial-preview-control.panel-action', PM.icon('panel'), h('span', describePanelAction(action)))));
  else if (interfaceChange) Object.entries(plan.interfaceEdit.patch).forEach(([key, value]) => preview.appendChild(h('div.spatial-preview-control', h('span', key), h('span', String(value)))));
  else if (chrome) preview.appendChild(h('div.spatial-preview-control',
    h('span', plan.chromeEdit.target === 'timeline.surfaceOrder' ? 'Timeline surfaces' : 'Corner style'),
    h('span', plan.chromeEdit.value)));
  else if (scene) plan.sceneEdit.commands.forEach(command => preview.appendChild(h('div.spatial-preview-control', h('span', PM.AgentHarness.describeCommand(command)))));
  else if (workspace) {
    plan.workspaceEdit.docks.forEach(dock => preview.appendChild(h('div.spatial-preview-control', h('span', dock.id), h('span', dock.panels.map(panel => panel.id).join(' · ')))));
    plan.workspaceEdit.sections.forEach(section => preview.appendChild(h('div.spatial-preview-control', h('span', section.title), h('span', `${section.controls.length} connected controls`))));
  } else {
    plan.section.controls.forEach(control => preview.appendChild(h('div.spatial-preview-control', h('span', control.label), h('span', control.connection || control.type), ['slider', 'curve'].includes(control.type) ? h('i', { class: control.type }) : null)));
  }
  preview.appendChild(h('div.spatial-proposal-actions',
    h('button.spatial-action', { onclick: () => { S.plan = null; renderConversation(true); } }, 'Dismiss'),
    h('button.spatial-action.pri', { onclick: applyPlan }, panels ? 'Apply panel changes' : (chrome || interfaceChange) ? 'Apply interface edit' : scene ? 'Apply scene edit' : workspace ? 'Create workspace' : 'Apply section')));
  return preview;
}

function resultPreview() {
  if (S.panelRun && S.phase === 'result') {
    const preview = h('div.spatial-proposal', h('div.spatial-proposal-kicker', 'Reversible agent change'), h('h3', S.panelRun.summary));
    S.panelRun.actions.forEach(action => preview.appendChild(h('div.spatial-preview-control.panel-action', PM.icon('panel'), h('span', describePanelAction(action)))));
    preview.appendChild(h('p', 'This is also in the normal Command-Z Undo history.'));
    preview.appendChild(h('div.spatial-proposal-actions', h('button.spatial-action', { onclick: undoPanelRun }, 'Undo change'), h('button.spatial-action.pri', { onclick: keepPanelRun }, 'Keep change')));
    return preview;
  }
  const run = S.run;
  if (!run || S.phase !== 'result') return null;
  const preview = h('div.spatial-proposal', h('div.spatial-proposal-kicker', 'Rendered result'), h('h3', 'Review the actual result'));
  const message = run.review?.message || 'The rendered change is ready.';
  preview.appendChild(h('p', message));
  if (run.review?.critique) preview.appendChild(h('p', run.review.critique));
  preview.appendChild(h('p', 'The complete agent run is one Command-Z Undo step.'));
  if (run.reviewError) preview.appendChild(h('p.spatial-review-warning', `Visual review stopped: ${run.reviewError.slice(0, 130)}. You can still inspect and undo the rendered change.`));
  const frames = h('div.spatial-frame-grid');
  (run.frames?.images || []).forEach((src, index) => frames.appendChild(h('figure',
    h('img', { src, alt: `Rendered composition at ${run.frames.times[index]} seconds` }),
    h('figcaption', `${run.frames.times[index]}s`))));
  if (frames.childElementCount) preview.appendChild(frames);
  preview.appendChild(h('div.spatial-proposal-actions',
    h('button.spatial-action', { onclick: undoSceneRun }, 'Undo change'),
    h('button.spatial-action.pri', { onclick: keepSceneRun }, 'Keep change')));
  return preview;
}

function describePanelAction(action) {
  const title = PM.PANELS[action.panelId]?.title || action.panelId || action.dockId || 'Dock';
  if (action.type === 'add') return `Open ${title}${action.dockId ? ` in ${action.dockId}` : ''}`;
  if (action.type === 'restore') return `Restore ${title}`;
  if (action.type === 'hide') return `Hide ${title}`;
  if (action.type === 'move') return `Move ${title} to ${action.dockId}${Number.isInteger(action.position) ? ` · position ${action.position + 1}` : ''}`;
  if (action.type === 'reorder') return `Place ${title} at position ${(action.position ?? 0) + 1}`;
  if (action.type === 'resize') return `Resize ${title} to ${Math.round(action.size || 0)} px`;
  if (action.type === 'resizeDock') return `Resize ${action.dockId} dock to ${Math.round(action.size || 0)} px`;
  if (action.type === 'rename') return `Rename ${title} to ${action.title}`;
  if (action.type === 'collapse') return `Collapse ${title}`;
  if (action.type === 'expand') return `Expand ${title}`;
  if (action.type === 'popout') return `Pop out ${title}`;
  if (action.type === 'dock') return `Dock ${title}`;
  return `${action.type} ${title}`;
}

function stepProgress() {
  const complete = S.steps.filter(step => step.status === 'complete').length;
  const active = S.steps.findIndex(step => step.status === 'active');
  return { complete, current: active >= 0 ? active + 1 : Math.min(complete + 1, S.steps.length), total: S.steps.length };
}

function renderSteps() {
  const needed = S.phase === 'working' || S.phase === 'applying' || !!S.plan;
  if (!needed || !S.steps.length) return null;
  const progress = stepProgress();
  const toggle = h('button.agent-steps-toggle', {
    type: 'button', 'aria-expanded': String(S.stepsExpanded),
    onclick: () => { S.stepsExpanded = !S.stepsExpanded; renderConversation(); },
  }, h('i'), h('span', progress.complete === progress.total ? `Done · ${progress.total} steps` : `Step ${progress.current} / ${progress.total}`), PM.icon('chev'));
  const block = h('div.agent-steps', toggle);
  if (S.stepsExpanded) {
    const list = h('ol.agent-todo');
    S.steps.forEach(step => list.appendChild(h(`li.${step.status || 'pending'}`, h('i'), h('span', step.title))));
    block.appendChild(list);
  }
  return block;
}

function modelLabel() {
  const model = AGENT_MODELS.find(item => item.id === S.model) || AGENT_MODELS[0];
  const effort = S.reasoningEffort.replace(/^./, value => value.toUpperCase());
  return `${model.label} · ${effort}`;
}

function openModelPicker(event) {
  const setModel = id => {
    S.model = id; PM.store.set('agentModel', id); renderConversation(true);
  };
  const setEffort = effort => {
    S.reasoningEffort = effort; PM.store.set('agentReasoningEffort', effort); renderConversation(true);
  };
  const items = [
    { header: 'Model' },
    ...AGENT_MODELS.map(model => ({ label: model.label, on: model.id === S.model, run: () => setModel(model.id) })),
    '-', { header: 'Reasoning' },
    ...REASONING_EFFORTS.map(effort => ({ label: effort.replace(/^./, value => value.toUpperCase()), on: effort === S.reasoningEffort, run: () => setEffort(effort) })),
  ];
  const rect = event.currentTarget.getBoundingClientRect();
  PM.menu(document.body, items, { x: rect.right, y: rect.top });
}

function modelPickerControl() {
  const select = h('select', { 'aria-label': 'Model and reasoning effort' });
  for (const model of AGENT_MODELS) for (const effort of REASONING_EFFORTS) {
    const option = h('option', { value: `${model.id}|${effort}` }, `${model.label} · ${effort.replace(/^./, value => value.toUpperCase())}`);
    if (model.id === S.model && effort === S.reasoningEffort) option.selected = true;
    select.appendChild(option);
  }
  select.onchange = () => {
    [S.model, S.reasoningEffort] = select.value.split('|');
    PM.store.set('agentModel', S.model); PM.store.set('agentReasoningEffort', S.reasoningEffort);
    renderConversation(true);
  };
  return h('label.agent-model', { title: 'Choose model and reasoning' }, select, PM.icon('chev'));
}

function scopeLabel() {
  if (S.scope === 'composition') return 'Composition';
  if (S.scope?.startsWith('panel:')) {
    const id = S.scope.slice(6);
    return PM.PANELS[id]?.title || id;
  }
  return 'Entire workspace';
}

function openScopePicker(event) {
  const workspace = PM.WS.current;
  const visible = new Set((workspace.layout?.docks || []).flatMap(dock => (dock.panels || []).map(panel => panel.id)));
  const hidden = new Set((workspace.hiddenPanels || []).map(item => item.id));
  const choose = scope => {
    S.scope = scope; PM.store.set('agentScope', scope); renderConversation(true);
  };
  const panels = Object.values(PM.PANELS || {}).filter(panel => panel.id !== 'toolbar' && (visible.has(panel.id) || hidden.has(panel.id)));
  const items = [
    { header: 'Agent scope' },
    { label: 'Entire workspace', on: S.scope === 'workspace', run: () => choose('workspace') },
    { label: 'Composition', on: S.scope === 'composition', run: () => choose('composition') },
    '-', { header: 'Panel' },
    ...panels.map(panel => ({
      label: panel.title + (hidden.has(panel.id) ? ' · hidden' : ''),
      on: S.scope === `panel:${panel.id}`, run: () => choose(`panel:${panel.id}`),
    })),
  ];
  const rect = event.currentTarget.getBoundingClientRect();
  PM.menu(document.body, items, { x: rect.left, y: rect.bottom });
}

function scopePickerControl() {
  const workspace = PM.WS.current;
  const visible = new Set((workspace.layout?.docks || []).flatMap(dock => (dock.panels || []).map(panel => panel.id)));
  const hidden = new Set((workspace.hiddenPanels || []).map(item => item.id));
  const select = h('select', { 'aria-label': 'Agent scope' },
    h('option', { value: 'workspace' }, 'Entire workspace'),
    h('option', { value: 'composition' }, 'Composition'));
  const group = h('optgroup', { label: 'Panel' });
  Object.entries(PM.PANELS || {}).filter(([id]) => id !== 'toolbar' && (visible.has(id) || hidden.has(id))).forEach(([id, panel]) => {
    const title = id === 'viewer' ? 'Composition panel' : panel.title;
    group.appendChild(h('option', { value: `panel:${id}` }, title + (hidden.has(id) ? ' · hidden' : '')));
  });
  select.appendChild(group); select.value = S.scope;
  if (!select.value) { S.scope = 'workspace'; select.value = S.scope; }
  select.onchange = () => {
    S.scope = select.value; PM.store.set('agentScope', S.scope); renderConversation(true);
  };
  return h('label.agent-scope', { title: 'Choose what the agent should work on' }, PM.icon('panel'), select, PM.icon('chev'));
}

const TEXT_ATTACHMENT_TYPES = new Set([
  'application/json', 'application/javascript', 'application/xml', 'image/svg+xml',
]);

function filesFromTransfer(transfer) {
  const direct = [...(transfer?.files || [])].filter(file => file instanceof File);
  if (direct.length) return direct;
  return [...(transfer?.items || [])]
    .filter(item => item.kind === 'file').map(item => item.getAsFile()).filter(Boolean);
}

function isTextAttachment(file) {
  return file.type.startsWith('text/') || TEXT_ATTACHMENT_TYPES.has(file.type)
    || /\.(?:txt|md|json|js|mjs|cjs|ts|tsx|jsx|css|html?|svg|xml|wgsl|glsl|csv|log)$/i.test(file.name);
}

async function addAttachmentFiles(files) {
  const available = Math.max(0, 6 - S.attachments.length);
  for (const file of [...files].slice(0, available)) {
    if (file.size > 4_000_000) { PM.toast(`${file.name} is larger than 4 MB`); continue; }
    const item = { id: PM.uid('attachment-'), name: file.name || 'Pasted attachment', type: file.type || 'application/octet-stream', size: file.size };
    if (/^image\/(?:png|jpeg|webp|gif)$/i.test(file.type)) {
      item.dataUrl = await new Promise((resolve, reject) => {
        const reader = new FileReader(); reader.onload = () => resolve(reader.result); reader.onerror = reject; reader.readAsDataURL(file);
      });
    } else if (isTextAttachment(file)) item.content = (await file.text()).slice(0, 100_000);
    S.attachments.push(item);
  }
  renderConversation(true);
}

function attachmentView(item, removable = false) {
  const preview = item.dataUrl
    ? h('img', { src: item.dataUrl, alt: '' })
    : h('span.agent-attachment-type', (item.name.split('.').pop() || 'file').slice(0, 5).toUpperCase());
  const card = h('span.agent-attachment', preview, h('span.agent-attachment-name', item.name));
  if (removable) card.appendChild(h('button', { type: 'button', title: `Remove ${item.name}`, onclick: () => removeAttachment(item.id) }, PM.icon('x')));
  return card;
}

function activityWords(value) {
  return String(value || '').split(/\s+/).filter(Boolean).map((word, index) => h('span.agent-thinking-word', {
    style: `--word-index:${Math.min(index, 28)}`,
  }, word + (index < String(value || '').trim().split(/\s+/).length - 1 ? ' ' : '')));
}

function chooseAttachments() {
  if (S.phase === 'applying') return;
  const input = h('input', {
    type: 'file', multiple: true,
  });
  input.onchange = () => addAttachmentFiles(input.files);
  input.click();
}

function removeAttachment(id) {
  S.attachments = S.attachments.filter(item => item.id !== id);
  renderConversation(true);
}

function renderConversation(focusInput = false) {
  const body = S.panelBody;
  if (!body) return;
  body.textContent = '';
  const shell = h('div.agent-shell');
  const steps = renderSteps(); if (steps) shell.appendChild(steps);
  const messages = h('div.spatial-conversation-log', { role: 'log', 'aria-live': 'polite' });
  let input;
  if (!S.conversation.length && !S.activity) {
    const suggestions = h('div.agent-suggestions');
    [
      ['Organize for animation', 'Organize my panels into a focused animation workspace'],
      ['Move Timeline right', 'Move the Timeline to the right dock and give it more room'],
      ['Open Inspector + Effects', 'Open the Inspector and Effects panels beside the composition'],
      ['Focus the canvas', 'Focus the composition by hiding panels I do not need right now'],
    ].forEach(([label, prompt]) => suggestions.appendChild(h('button', { type: 'button', onclick: () => { S.composerDraft = prompt; input.value = prompt; input.focus(); } }, label)));
    messages.appendChild(h('div.agent-welcome', h('div.agent-welcome-icon', PM.icon('sparkle')), h('b', 'Build or rearrange anything'), h('span', 'Edit the composition, build controls, or tell me exactly how to arrange your panels.'), suggestions));
  }
  S.conversation.slice(-30).forEach(message => {
    const bubble = h(`div.spatial-message.${message.role}${message.entering ? '.is-entering' : ''}`, message.text);
    if (message.attachments?.length) {
      const rail = h('div.agent-message-files');
      message.attachments.forEach(item => rail.appendChild(attachmentView(typeof item === 'string' ? { name: item } : item)));
      bubble.appendChild(rail);
    }
    messages.appendChild(bubble);
    message.entering = false;
  });
  if (S.activity) {
    messages.appendChild(h(`div.spatial-message.assistant.pending${S.pendingEntering ? '.is-entering' : ''}`,
      h('i'), h('span.agent-thinking-copy', ...activityWords(S.activity))));
    S.pendingEntering = false;
  }
  const proposal = resultPreview() || planPreview();
  if (proposal) messages.appendChild(proposal);
  shell.appendChild(messages);

  const mode = composerMode(S.phase);
  input = h('textarea.spatial-followup', {
    rows: '1', placeholder: mode.placeholder, 'aria-label': 'Message Powermove agent',
    'data-autosize': 'true',
    disabled: mode.disabled,
  });
  input.value = S.composerDraft;
  input.addEventListener('input', () => { S.composerDraft = input.value; });
  input.addEventListener('paste', event => {
    const files = filesFromTransfer(event.clipboardData);
    if (!files.length) return;
    event.preventDefault();
    addAttachmentFiles(files).catch(error => PM.toast(String(error.message || error)));
  });
  input.addEventListener('keydown', event => {
    event.stopPropagation();
    if (event.key === 'Enter' && !event.shiftKey) { event.preventDefault(); sendRequest(input); }
  });
  const attachmentRail = h('div.agent-attachment-rail');
  S.attachments.forEach(item => attachmentRail.appendChild(attachmentView(item, true)));
  const attach = h('button.agent-attach', { type: 'button', title: 'Attach images or text files', 'aria-label': 'Add attachments', onclick: chooseAttachments }, PM.icon('plus'));
  const scope = scopePickerControl();
  const approval = h('button.agent-approval', {
    type: 'button', 'aria-pressed': String(S.autoApplyPanels),
    title: S.autoApplyPanels ? 'Safe panel changes apply automatically' : 'Review panel changes before applying',
    onclick: () => {
      S.autoApplyPanels = !S.autoApplyPanels;
      PM.store.set('agentAutoApplyPanels', S.autoApplyPanels);
      renderConversation(true);
    },
  }, h('i'), S.autoApplyPanels ? 'Auto-apply panels' : 'Review panel edits');
  const model = modelPickerControl();
  const stop = mode.working ? h('button.agent-stop', {
    type: 'button', 'aria-label': 'Stop current run', title: 'Stop current run', onclick: stopActiveRequest,
  }, PM.icon('x')) : null;
  const send = h('button.spatial-action.pri.agent-send', {
    type: 'button', 'aria-label': mode.sendLabel, title: mode.sendLabel, disabled: mode.disabled,
    onclick: () => sendRequest(input),
  }, mode.disabled ? h('i') : PM.icon('return'));
  const composer = h('div.agent-composer', h('div.agent-composer-head', scope, approval), input, attachmentRail,
    h('div.agent-composer-tools', attach, h('span.sp'), model, stop, send));
  shell.appendChild(composer);
  body.appendChild(shell);
  autosizeTextarea(input);
  requestAnimationFrame(() => {
    messages.scrollTop = S.conversation.length || S.activity || proposal ? messages.scrollHeight : 0;
    if (focusInput && !input.disabled) input.focus();
  });
}

function stopActiveRequest() {
  if (S.phase !== 'working') return;
  const active = S.activeRequest;
  S.activeRequest = null;
  ++S.requestToken;
  active?.abort();
  S.steps = []; S.activity = ''; S.plan = null; S.phase = 'conversation';
  S.conversation.push({ role: 'assistant', text: 'Stopped. Add direction whenever you are ready.' });
  renderConversation(true);
}

async function sendRequest(input) {
  const typedRequest = input.value.trim();
  if ((!typedRequest && !S.attachments.length) || S.phase === 'applying') return;
  const request = typedRequest || 'Review the attached files and make the relevant editable change.';
  const steering = S.phase === 'working';
  const previousRequest = S.activeRequest;
  const token = ++S.requestToken;
  const controller = new AbortController();
  S.activeRequest = controller;
  previousRequest?.abort();
  S.requestText = request;
  S.requestAttachments = S.attachments.splice(0);
  S.composerDraft = '';
  S.conversation.push({
    role: 'user', text: typedRequest || `Attached ${S.requestAttachments.length} file${S.requestAttachments.length === 1 ? '' : 's'}`,
    attachments: S.requestAttachments.map(item => ({ name: item.name, type: item.type, dataUrl: item.dataUrl })), entering: true,
  });
  updateSteps(steering ? [
    'Review the new direction',
    'Revise the editable change',
    'Prepare the updated source edits',
    'Review the visible result',
  ] : [
    'Understand the request and context',
    'Design an editable change',
    'Prepare the source edits',
    'Review the visible result',
  ], 0);
  S.stepsExpanded = false;
  S.plan = null; S.panelRun = null; S.phase = 'working';
  S.activity = steering ? 'Updating the run with your direction…' : 'Looking at the composition and workspace…';
  S.pendingEntering = true;
  promoteToConversation(); renderConversation(true);
  try {
    /* Let the send handoff finish before the next progress render replaces the
       message DOM. Observation still runs immediately, so the beat adds only
       the portion of the 240 ms transition that useful work did not consume. */
    const observationPromise = PM.AgentHarness ? PM.AgentHarness.observe() : Promise.resolve({ state: {}, times: [], images: [] });
    const [observation] = await Promise.all([
      observationPromise,
      new Promise(resolve => setTimeout(resolve, SEND_TRANSITION_MS)),
    ]);
    if (token !== S.requestToken) return;
    S.steps[0].status = 'complete'; S.steps[1].status = 'active';
    S.activity = steering ? 'Reworking the editable change…' : 'Designing an editable change…'; renderConversation(true);
    const userImages = S.requestAttachments.filter(item => item.dataUrl).map(item => item.dataUrl);
    const attachedImages = [...userImages, ...(S.regionImage ? [S.regionImage] : []), ...observation.images].slice(0, 6);
    const raw = await PM.CodexBridge.request(
      agentPrompt(request, observation, steering), responseSchema(), attachedImages,
      {
        model: S.model, reasoningEffort: S.reasoningEffort, signal: controller.signal,
        onProgress: summary => {
          if (token !== S.requestToken || !summary) return;
          S.activity = summary; renderConversation();
        },
      },
    );
    if (token !== S.requestToken) return;
    let decoded;
    try { decoded = JSON.parse(raw); } catch { throw new Error('The coding agent returned an invalid section'); }
    const plan = sanitizePlan(decoded, S.context, request);
    if (plan.operation === 'noop') throw new Error(plan.message || 'I could not turn that into an editable change yet');
    if (plan.kind === 'scene' && !plan.sceneEdit.commands.length) throw new Error(plan.message || 'I could not prepare the composition edit');
    if (plan.kind === 'section' && !plan.section.controls.length) throw new Error('The generated section had no controls connected to editable source');
    if (plan.kind === 'workspace' && !plan.workspaceEdit) throw new Error('The generated workspace was not safe or complete enough to preview');
    if (plan.kind === 'panels' && !plan.panelEdit.actions.length) throw new Error('I could not find a valid panel action to perform');
    updateSteps(plan.steps);
    S.stepsExpanded = S.steps.length > 1;
    S.conversation.push({ role: 'assistant', text: conversationReply(plan) });
    S.activity = ''; S.plan = plan; S.phase = 'conversation';
    if (plan.kind === 'panels' && S.autoApplyPanels) await applyPlan();
    else showPreview();
  } catch (error) {
    if (token !== S.requestToken) return;
    if (error?.name === 'AbortError') return;
    const current = S.steps.find(step => step.status === 'active'); if (current) current.status = 'error';
    S.activity = ''; S.phase = 'conversation';
    S.conversation.push({ role: 'assistant', text: String(error.message || error).slice(0, 220) });
    renderConversation(true);
  } finally {
    if (token === S.requestToken && S.activeRequest === controller) S.activeRequest = null;
  }
}

function responseSchema() {
  return {
    type: 'object', additionalProperties: false,
    required: ['kind', 'operation', 'targetPanelId', 'dockId', 'placement', 'message', 'steps', 'chromeEdit', 'interfaceEdit', 'section', 'sceneEdit', 'workspaceEdit', 'panelEdit'],
    properties: {
      kind: { type: 'string', enum: ['section', 'chrome', 'interface', 'scene', 'workspace', 'panels'] },
      operation: { type: 'string', enum: ['create', 'modify', 'noop'] },
      targetPanelId: { type: 'string' }, dockId: { type: 'string' },
      placement: { type: 'string', enum: ['before', 'after', 'replace'] },
      message: { type: 'string' },
      steps: { type: 'array', minItems: 1, maxItems: 6, items: { type: 'string' } },
      chromeEdit: {
        type: 'object', additionalProperties: false, required: ['target', 'value'],
        properties: {
          target: { type: 'string', enum: ['preview.cornerRadius', 'timeline.surfaceOrder'] },
          value: { type: 'string', enum: ['square', 'rounded', 'normal', 'reversed'] },
        },
      },
      interfaceEdit: { type: 'string' },
      section: {
        type: 'object', additionalProperties: false, required: ['id', 'title', 'size', 'note', 'tool', 'controls'],
        properties: {
          id: { type: 'string' }, title: { type: 'string' }, size: { type: 'number' }, note: { type: 'string' }, tool: { type: 'string' },
          controls: { type: 'array', maxItems: 64, items: {
            type: 'object', additionalProperties: false,
            required: ['type', 'label', 'parameter', 'defaultValue', 'min', 'max', 'step', 'options', 'target', 'path', 'command', 'stateKey', 'source', 'action', 'primary'],
            properties: {
              type: { type: 'string', enum: ['slider', 'text', 'color', 'fill', 'toggle', 'select', 'button', 'readout', 'curve'] }, label: { type: 'string' },
              parameter: { type: 'string' }, defaultValue: { anyOf: [{ type: 'string' }, { type: 'number' }, { type: 'boolean' }, { type: 'null' }] }, min: { type: 'number' }, max: { type: 'number' }, step: { type: 'number' },
              options: { type: 'array', items: { type: 'string' } }, target: { type: 'string' }, path: { type: 'string' }, command: { type: 'string' },
              stateKey: { type: 'string' }, source: { type: 'string' }, action: { type: 'string' }, primary: { type: 'boolean' },
            },
          } },
        },
      },
      sceneEdit: PM.AgentHarness.sceneSchema(),
      workspaceEdit: { type: 'string' },
      panelEdit: { type: 'string' },
    },
  };
}

function agentPrompt(request, observation, steering = false) {
  const workspace = PM.WS.current;
  const commands = Object.values(PM.commands || {}).map(c => ({ id: c.id, label: c.label }));
  const attachedFiles = S.requestAttachments.slice(0, 6).map(item => ({
    name: item.name, type: item.type, content: item.content ? item.content.slice(0, 30_000) : undefined,
    image: !!item.dataUrl,
  }));
  const editableSource = PM.Edit?.sourceCatalog?.() || observation?.state?.editableSource || {};
  const capabilities = PM.Capabilities?.catalog?.() || {};
  const conversation = S.conversation.slice(0, -1).slice(-12).map(message => ({
    role: message.role, text: message.text,
    attachments: (message.attachments || []).map(item => typeof item === 'string' ? item : item.name),
  }));
  return `You are the action-oriented visual editing agent inside Powermove. Turn the user's request into the strongest editable change supported by the available source operations. Return only the requested JSON object.

RULES
- ${steering ? 'This is steering for an active run. Replace the unfinished plan with one updated plan that honors the earlier request and the newest direction.' : 'This is a new run. Build one complete editable plan for the latest request.'}
- Act on clear change requests. Prefer a useful executable interpretation over explaining limitations. Use noop only when none of the available scene, section, workspace, or chrome operations can produce a meaningful result.
- Do not answer with a limitation when the requested target appears in EDITABLE SOURCE CATALOG, AVAILABLE PANELS, AVAILABLE COMMANDS, availableOperations, or the supported chrome targets. Build the executable change.
- Return 2–6 short steps that describe the actual work you will carry out. Each step must start with a verb and be specific enough to show in the interface as a to-do item.
- While working, emit concise user-visible reasoning summaries about what you are inspecting, deciding, or validating. Do not expose private chain-of-thought.
- kind=scene for changes to layers, content, motion, timing, effects, or composition settings. Each sceneEdit.commands item must be one JSON-encoded source-edit object using only availableOperations. Use stable explicit ids for new layers that later commands target. Never output JavaScript, shell commands, or whole-project JSON.
- For scene requests, inspect the live source and attached rendered frames. Preserve locked layers, hand-edited channels, and unrelated work. Set reviewTimes to the most revealing moments. Return neutral section and chromeEdit fields.
- kind=panels for direct changes to the current panel layout. panelEdit must be one JSON-encoded object shaped like {"actions":[{"type":"add|restore|hide|move|reorder|resize|resizeDock|rename|collapse|expand|popout|dock","panelId":"PANEL_ID","dockId":"left|center|right|EXISTING_DOCK","position":0,"size":300,"title":"New title"}]}. You may return up to 16 ordered actions. Use add for an available panel that is not present, restore for a hidden panel, move for a different dock, reorder for an exact zero-based position, resize for panel height, resizeDock for dock width, rename for its visible title, and popout/dock/collapse/expand for its window state. Never hide or collapse viewer. Never pop out viewer or timeline. Prefer a direct panels plan over rebuilding the whole workspace when the user asks to rearrange existing panels.
- kind=workspace when the user asks for a complete workspace, layout, editing environment, or a coordinated group of panels. workspaceEdit must be one JSON-encoded manifest shaped like {"name":"...","density":"compact|normal|comfy","accent":"#RRGGBB","docks":[{"id":"left|center|right","size":number,"flex":boolean,"panels":[{"id":"viewer|timeline|inspector|assets|fxbrowser|takes|notes|CUSTOM_ID","size":number,"flex":boolean}]}],"sections":[SECTION_OBJECTS]}. Include viewer, keep all panels reachable, and make every generated section control source-connected under the same rules below.
- For non-panel requests return panelEdit="{\"actions\":[]}". For non-workspace requests return workspaceEdit="{}". For non-interface requests return interfaceEdit="{}". For non-scene requests return an empty neutral sceneEdit. For non-section requests return a neutral empty section with tool="".
- kind=chrome for a supported app-interface style change. Supported targets are preview.cornerRadius with square|rounded and timeline.surfaceOrder with normal|reversed. Use operation=modify, and return a neutral empty section object.
- kind=interface for a structured Timeline redesign. interfaceEdit must be one JSON-encoded manifest shaped like {"target":"timeline","patch":{"rowHeight":22..48,"gutterWidth":160..360,"rulerHeight":20..42,"clipRadius":0..12,"keyframeSize":4..12,"showLayerNumbers":boolean,"showTypeBadges":boolean,"toolbarDensity":"compact|normal","surfaceOrder":"normal|reversed"}}. Include only fields requested or clearly useful. This edits the Timeline view over the existing source; never rewrite layers or keyframes for an interface request.
- kind=section for editable panels/controls. For a known reusable generated tool, set section.tool to an id from GENERATED TOOL CAPABILITIES and leave controls empty. The tool recipe supplies validated selection state, settings, Preview, Apply, and Undo controls. Return a neutral chromeEdit of {"target":"preview.cornerRadius","value":"square"}.
- operation=create when adding a section; operation=modify when replacing or changing an existing source surface.
- A section is a compact native Powermove panel made from slider, text, color, fill, toggle, select, button, readout, and visual curve controls.
- Every non-button control that edits project data must bind to real editable source. Use target="$selection" for the selected layer, an exact layer id from EDITABLE SOURCE CATALOG, or target="$composition" for composition paths. A visual tool control may instead use stateKey only when a validated source-action button consumes that state.
- The EDITABLE SOURCE CATALOG below is authoritative and complete for the current project. If a requested field is listed, create the working control; never claim it is unavailable. Choose each control type and range from its catalog entry.
- Do not create decorative or disconnected scene parameters. If a requested control has no source yet, prefer a scene or workspace action that creates useful editable source rather than refusing the whole request.
- Buttons may use only one of the listed command ids. Never invent commands.
- Advanced generated tools may use local settings with stateKey plus buttons whose action is a JSON-encoded safe transform action. A transform action is {"type":"transform","mode":"preview|apply","transform":{"label":"...","selector":{"scope":"selection|all|visible","types":[]},"order":"stack|reverseStack|selection|reverseSelection|start|reverseStart|name|random","edits":[{"path":"layer.from|layer.duration|layer.*|properties.*|content.*","value":EXPRESSION}]}}. Expressions are constants or objects using state, ref, aggregate, and bounded math ops from GENERATED TOOL CAPABILITIES. Prefer this declarative form when it can express the tool.
- For a Flow-style easing tool, prefer section.tool="easing-flow". To author a custom version, use a curve control with stateKey="curve", defaultValue="[0.62,0.05,0,1]", min=-1, max=2, and preset names in options. Pair it with a JSON-encoded easing button action shaped like {"type":"easing","mode":"preview|apply","scope":"selected-keyframes","curveState":"curve","defaultCurve":[0.62,0.05,0,1]}. Curve handles are visual, draggable, keyboard-accessible tool state; the action applies them to real selected keyframes through one undoable source transaction.
- When a generated tool genuinely needs loops, branching, computed layer counts, or create-many behavior, a button may instead use a JSON-encoded sandboxed script action: {"type":"script","mode":"preview|apply","label":"...","requiredTypes":["text"],"code":"JAVASCRIPT_FUNCTION_BODY"}. The function body receives a frozen PM SDK with project, composition, input, layers, selectedLayers, uid, clone, assert, emit, and typed command builders. It must return one command or an array of commands. It has no app DOM, storage, network, native bridge, or direct mutation access; its output is validated and applied atomically. Use PM.input for stateKey values. Pair an apply button with a preview button using the same code. Never attempt to escape the sandbox or access unavailable globals.
- Include every control the user explicitly requests. For open-ended requests, prefer a focused set unless the user asks for all or everything, in which case include the complete relevant catalog. Use a short title and useful one-sentence note; avoid decorative filler.
- For unused control fields, still return schema-safe neutral values: empty string/array, 0, or false.
- App chrome is a valid editable source target when it is listed above. In particular, requests to reverse the Timeline's grey surface order map to timeline.surfaceOrder=reversed.
- Keep the current interface reachable. Do not remove unrelated docks or panels. Never alter rendered composition shapes or export geometry for a chrome request.
- The active scope is a strong hint, not a restriction. Carry out dependent panel actions elsewhere when needed to satisfy the request.

VISUAL REFERENCES
${attachedFiles.some(file => file.image) ? '- User-attached images come first and are direct visual references.' : '- No user image attachment was provided.'}
${S.regionImage ? '- After user images, the next attached image is an exact screenshot of the selected editor region before the Ripple overlay appeared.' : '- No editor region was selected.'}
- Remaining attached images are rendered composition frames at the times listed below.

ATTACHED FILES
${JSON.stringify(attachedFiles)}

SELECTED REGION SEMANTICS
${JSON.stringify(S.context)}

ACTIVE PROMPT SCOPE
${JSON.stringify({ scope: S.scope, label: scopeLabel() })}

CONVERSATION SO FAR
${JSON.stringify(conversation)}

SEMANTIC WORKSPACE MAP
${JSON.stringify(workspaceSemanticContext(workspace))}

CURRENT WORKSPACE MANIFEST
${JSON.stringify(workspace)}

AVAILABLE PANELS
${JSON.stringify(panelCatalog(workspace))}

AVAILABLE COMMANDS
${JSON.stringify(commands)}

EDITABLE SOURCE CATALOG
${JSON.stringify(editableSource)}

GENERATED TOOL CAPABILITIES
${JSON.stringify(capabilities)}

${PM.AgentHarness.promptContext(observation)}

USER REQUEST
${request}`;
}

function workspaceSemanticContext(workspace) {
  const custom = new Map((workspace?.custom || []).map(section => [section.id, section]));
  return {
    id: workspace?.id || '', name: workspace?.name || '', density: workspace?.density || '',
    theme: workspace?.theme || {}, chrome: workspace?.chrome || {},
    docks: (workspace?.layout?.docks || []).map(dock => ({
      id: dock.id, size: dock.size, flex: !!dock.flex,
      panels: (dock.panels || []).map(spec => {
        const section = custom.get(spec.id);
        return {
          id: spec.id, title: section?.title || PM.PANELS?.[spec.id]?.title || spec.id,
          role: section ? 'generated editable section' : 'native editor panel',
          size: spec.size, flex: !!spec.flex,
          bindings: (section?.controls || []).map(control => ({
            label: control.label, type: control.type, target: control.target || '',
            path: control.path || '', command: control.cmd || control.command || '',
          })),
        };
      }),
    })),
  };
}

function panelCatalog(workspace) {
  const visible = new Map((workspace?.layout?.docks || []).flatMap(dock => (dock.panels || []).map((spec, index) => [spec.id, {
    state: 'visible', dockId: dock.id, position: index, size: spec.size || null,
    collapsed: !!spec.collapsed, poppedOut: !!PM.Popout?.isOpen?.(spec.id),
  }])));
  const hidden = new Map((workspace?.hiddenPanels || []).map(item => [item.id, {
    state: 'hidden', dockId: item.dockId || '', position: item.index ?? 0, size: item.spec?.size || null,
  }]));
  return Object.values(PM.PANELS || {}).filter(panel => panel.id !== 'toolbar').map(panel => ({
    id: panel.id, title: panel.title, ...(visible.get(panel.id) || hidden.get(panel.id) || { state: 'available' }),
    canPopOut: !['viewer', 'timeline'].includes(panel.id), canHide: panel.id !== 'viewer',
  }));
}

function controlConnection(target, path, controlType) {
  const catalog = PM.Edit?.sourceCatalog?.();
  if (target === '$composition' || target === 'composition') {
    const fallback = {
      'composition.name': 'text', 'composition.width': 'slider', 'composition.height': 'slider',
      'composition.fps': 'slider', 'composition.duration': 'slider', 'composition.shutter': 'slider',
      'composition.workArea.start': 'slider', 'composition.workArea.end': 'slider',
      'composition.backgroundFill': 'fill', 'composition.background': 'color',
      'composition.background.type': 'select', 'composition.background.startColor': 'color',
      'composition.background.endColor': 'color', 'composition.background.angle': 'slider',
      'composition.background.midpoint': 'slider',
    };
    const spec = catalog?.composition?.find(item => item.path === path);
    const control = spec?.control || fallback[path]; if (!control) return null;
    return { ...spec, target: '$composition', path, control, connection: path.startsWith('composition.background.') ? 'Composition background' : 'Composition' };
  }
  const layer = target === '$selection' || target === 'selection' ? PM.firstSel() : (PM.L(target) || PM.byName(target));
  if (!layer) return null;
  const spec = catalog?.layers?.find(item => item.id === layer.id)?.controls?.find(item => item.path === path);
  if (spec) return { ...spec, target: target === '$selection' || target === 'selection' ? '$selection' : layer.id, path, connection: `Layer · ${layer.name}` };
  let control = controlType;
  if (path.startsWith('properties.')) {
    const channel = path.slice('properties.'.length);
    const prop = PM.findProp(layer, channel); if (!prop) return null;
    const value = PM.evP ? PM.evP(layer, prop, PM.time, channel) : prop.v;
    control = typeof value === 'number' ? 'slider' : typeof value === 'boolean' ? 'toggle' : /^#[0-9a-f]{6}$/i.test(value) ? 'color' : 'text';
  } else if (path.startsWith('content.')) {
    const key = path.slice('content.'.length);
    const current = layer.d?.[key];
    if (!['string', 'number', 'boolean'].includes(typeof current)) return null;
    control = typeof current === 'number' ? 'slider' : typeof current === 'boolean' ? 'toggle'
      : /^#[0-9a-f]{6}$/i.test(current) ? 'color' : 'text';
  } else if (path.startsWith('layer.')) {
    const key = path.slice('layer.'.length);
    control = ['visible', 'locked', 'solo', 'shy', 'motionBlur', 'collapsed'].includes(key) ? 'toggle'
      : ['duration', 'from'].includes(key) ? 'slider'
        : ['blend', 'parent'].includes(key) ? 'select'
          : key === 'color' ? 'color' : key === 'name' ? 'text' : '';
    if (!control) return null;
  } else return null;
  return { target: target === '$selection' || target === 'selection' ? '$selection' : layer.id, path, control, connection: `Layer · ${layer.name}` };
}

const PANEL_ACTION_TYPES = new Set(['add', 'restore', 'hide', 'move', 'reorder', 'resize', 'resizeDock', 'rename', 'collapse', 'expand', 'popout', 'dock']);

function sanitizePanelEdit(encoded) {
  let raw;
  try {
    if (typeof encoded !== 'string' || encoded.length > 80_000) return { actions: [] };
    raw = JSON.parse(encoded);
  } catch { return { actions: [] }; }
  const known = new Set(Object.keys(PM.PANELS || {}).filter(id => id !== 'toolbar'));
  const text = (value, max = 100) => typeof value === 'string' ? value.trim().slice(0, max) : '';
  const actions = (Array.isArray(raw?.actions) ? raw.actions : []).slice(0, 16).map(value => {
    const action = value && typeof value === 'object' ? value : {};
    const type = text(action.type, 20), panelId = text(action.panelId, 100), dockId = text(action.dockId, 80);
    if (!PANEL_ACTION_TYPES.has(type)) return null;
    if (type !== 'resizeDock' && (!known.has(panelId) || panelId === 'toolbar')) return null;
    if (['hide', 'collapse'].includes(type) && panelId === 'viewer') return null;
    if (type === 'popout' && ['viewer', 'timeline'].includes(panelId)) return null;
    const out = { type, panelId, dockId };
    if (Number.isFinite(action.position)) out.position = PM.clamp(Math.floor(action.position), 0, 20);
    if (Number.isFinite(action.size)) out.size = PM.clamp(action.size, type === 'resizeDock' ? 200 : 56, type === 'resizeDock' ? 760 : 1600);
    const title = text(action.title, 70); if (title) out.title = title;
    return out;
  }).filter(Boolean);
  return { actions };
}

function applyPanelEdit(workspace, edit) {
  const applied = [], runtime = [];
  const visible = id => PM.Layout.findPanel(workspace, id);
  const show = (id, dockId = 'right') => {
    if (visible(id)) return visible(id);
    const hidden = (workspace.hiddenPanels || []).some(item => item.id === id);
    if (hidden) PM.Layout.restorePanel(workspace, id);
    else PM.Layout.addPanel(workspace, id, dockId || 'right');
    return visible(id);
  };
  const place = (id, position) => {
    const found = visible(id); if (!found || !Number.isInteger(position)) return false;
    const from = found.dock.panels.indexOf(found.spec);
    const to = Math.max(0, Math.min(position, found.dock.panels.length - 1));
    if (from === to) return false;
    found.dock.panels.splice(from, 1); found.dock.panels.splice(to, 0, found.spec); return true;
  };
  for (const action of edit?.actions || []) {
    let changed = false;
    if (action.type === 'add') {
      const existed = !!visible(action.panelId);
      const found = show(action.panelId, action.dockId || 'right');
      if (action.dockId && found?.dock.id !== action.dockId) PM.Layout.movePanel(workspace, action.panelId, action.dockId);
      changed = !existed || !!action.dockId;
      if (Number.isInteger(action.position)) changed = place(action.panelId, action.position) || changed;
    } else if (action.type === 'restore') {
      changed = !!show(action.panelId, action.dockId || 'right');
      if (action.dockId && visible(action.panelId)?.dock.id !== action.dockId) changed = PM.Layout.movePanel(workspace, action.panelId, action.dockId) || changed;
      if (Number.isInteger(action.position)) changed = place(action.panelId, action.position) || changed;
    } else if (action.type === 'hide') changed = PM.Layout.hidePanel(workspace, action.panelId);
    else if (action.type === 'move') {
      show(action.panelId, action.dockId || 'right');
      changed = action.dockId ? PM.Layout.movePanel(workspace, action.panelId, action.dockId) : false;
      if (Number.isInteger(action.position)) changed = place(action.panelId, action.position) || changed;
    } else if (action.type === 'reorder') changed = place(action.panelId, action.position);
    else if (action.type === 'resize') {
      const found = show(action.panelId, action.dockId || 'right');
      if (found && Number.isFinite(action.size)) { found.spec.size = action.size; delete found.spec.flex; changed = true; }
    } else if (action.type === 'resizeDock') {
      const dock = (workspace.layout?.docks || []).find(item => item.id === action.dockId);
      if (dock && Number.isFinite(action.size)) { dock.size = action.size; if (dock.id !== 'center') delete dock.flex; changed = true; }
    } else if (action.type === 'rename') {
      const found = show(action.panelId, action.dockId || 'right');
      if (found && action.title) { found.spec.title = action.title; changed = true; }
    } else runtime.push(action);
    if (changed) applied.push(action);
  }
  return { applied, runtime };
}

function sanitizePlan(raw, context, request = '') {
  const operation = ['create', 'modify', 'noop'].includes(raw?.operation) ? raw.operation : 'noop';
  const chromeTarget = ['preview.cornerRadius', 'timeline.surfaceOrder'].includes(raw?.chromeEdit?.target) ? raw.chromeEdit.target : '';
  const allowedChromeValues = chromeTarget === 'preview.cornerRadius' ? ['square', 'rounded']
    : chromeTarget === 'timeline.surfaceOrder' ? ['normal', 'reversed'] : [];
  const chromeValue = allowedChromeValues.includes(raw?.chromeEdit?.value) ? raw.chromeEdit.value : '';
  const interfaceEdit = PM.WS?.sanitizeInterfaceEdit?.(raw?.interfaceEdit) || null;
  const requestedKind = raw?.kind;
  const panelEdit = sanitizePanelEdit(raw?.panelEdit);
  const kind = requestedKind === 'scene' ? 'scene'
    : requestedKind === 'workspace' ? 'workspace'
    : requestedKind === 'panels' ? 'panels'
    : requestedKind === 'interface' && interfaceEdit ? 'interface'
    : requestedKind === 'chrome' && chromeTarget && chromeValue ? 'chrome' : 'section';
  const cleanText = (v, fallback = '', max = 100) => typeof v === 'string' && v.trim() ? v.trim().slice(0, max) : fallback;
  const source = raw?.section && typeof raw.section === 'object' ? raw.section : {};
  const recipe = PM.Capabilities?.panelRecipe?.(cleanText(source.tool, '', 80)) || null;
  const sectionSource = recipe ? {
    ...recipe,
    id: cleanText(source.id, recipe.id, 80), title: cleanText(source.title, recipe.title, 70),
    size: Number(source.size) || recipe.size, note: cleanText(source.note, recipe.note, 240),
  } : source;
  const steps = (Array.isArray(raw?.steps) ? raw.steps : []).filter(step => typeof step === 'string' && step.trim())
    .map(step => step.trim().slice(0, 100)).slice(0, 6);
  const baseId = slug(cleanText(sectionSource.id, cleanText(sectionSource.title, 'Generated section')));
  const controls = (Array.isArray(sectionSource.controls) ? sectionSource.controls : []).slice(0, 64).map((control, index) => {
    const c = control && typeof control === 'object' ? control : {};
    let type = ['slider', 'text', 'color', 'fill', 'toggle', 'select', 'button', 'readout', 'curve'].includes(c.type) ? c.type : 'slider';
    const label = cleanText(c.label, `Control ${index + 1}`, 60);
    const out = { type, label };
    if (type === 'readout') {
      out.source = ['selection.summary', 'selection.count', 'keyframes.summary', 'keyframes.count'].includes(c.source) ? c.source : 'selection.summary';
      out.connection = out.source.startsWith('keyframes.') ? 'Live keyframes' : 'Live selection';
      return out;
    }
    if (type === 'button') {
      if (c.command && PM.commands?.[c.command]) out.cmd = c.command;
      else if (c.cmd && PM.commands?.[c.cmd]) out.cmd = c.cmd;
      const action = PM.Capabilities?.sanitizeControlAction?.(c.action);
      if (action) out.action = action;
      out.primary = c.primary === true;
      out.connection = action ? 'Source action' : 'Command';
      return out;
    }
    const localKey = cleanText(c.stateKey, '', 80);
    if (localKey && /^[a-z][a-z0-9_.-]{0,79}$/i.test(localKey)) {
      out.stateKey = localKey; out.connection = 'Tool setting';
      const sourceValue = c.def !== undefined ? c.def : c.defaultValue;
      if (type === 'curve') {
        out.def = PM.Capabilities?.sanitizeCurve?.(sourceValue, [.62, .05, 0, 1]) || [.62, .05, 0, 1];
        out.minY = PM.clamp(Number.isFinite(c.minY) ? c.minY : Number.isFinite(c.min) ? c.min : -1, -4, 0);
        out.maxY = PM.clamp(Number.isFinite(c.maxY) ? c.maxY : Number.isFinite(c.max) ? c.max : 2, 1, 4);
        out.presets = (Array.isArray(c.presets) ? c.presets : Array.isArray(c.options) ? c.options : [])
          .filter(name => typeof name === 'string' && PM.Ease?.PRESETS?.[name]).slice(0, 16);
      } else if (type === 'text') out.def = sourceValue == null ? '' : String(sourceValue).slice(0, 500);
      else if (type === 'color') out.def = /^#[0-9a-f]{6}$/i.test(sourceValue) ? sourceValue.toUpperCase() : '#FF6B1A';
      else if (type === 'toggle') out.def = !!sourceValue;
      else if (type === 'select') {
        out.options = (Array.isArray(c.options) ? c.options : [])
          .filter(v => typeof v === 'string' || (v && typeof v === 'object' && ['string', 'number', 'boolean'].includes(typeof v.v) && typeof v.label === 'string'))
          .map(v => typeof v === 'string' ? v : ({ v: v.v, label: v.label.slice(0, 80) })).slice(0, 80);
        const values = out.options.map(option => typeof option === 'string' ? option : option.v);
        out.def = values.includes(sourceValue) ? sourceValue : (values[0] ?? 'Default');
      } else {
        out.min = Number.isFinite(c.min) ? c.min : 0; out.max = Number.isFinite(c.max) ? c.max : 100;
        if (out.max < out.min) [out.min, out.max] = [out.max, out.min];
        out.step = Number.isFinite(c.step) && c.step > 0 ? c.step : Math.max((out.max - out.min) / 100, .01);
        out.unit = cleanText(c.unit, '', 12);
        out.def = Number.isFinite(sourceValue) ? PM.clamp(sourceValue, out.min, out.max) : out.min;
      }
      return out;
    }
    const target = cleanText(c.target, '', 120), path = cleanText(c.path, '', 120);
    const connection = controlConnection(target, path, type);
    if (!connection) return null;
    type = connection.control || type; out.type = type;
    Object.assign(out, connection);
    const sourceValue = connection.value !== undefined ? connection.value : c.defaultValue;
    if (type === 'text') out.def = sourceValue == null ? '' : String(sourceValue);
    else if (type === 'color') out.def = /^#[0-9a-f]{6}$/i.test(sourceValue) ? sourceValue : '#FF6B1A';
    else if (type === 'fill') out.def = sourceValue && typeof sourceValue === 'object' ? sourceValue : null;
    else if (type === 'toggle') out.def = !!sourceValue;
    else if (type === 'select') {
      out.options = (Array.isArray(connection.options) ? connection.options : Array.isArray(c.options) ? c.options : [])
        .filter(v => typeof v === 'string' || (v && typeof v === 'object' && 'v' in v && typeof v.label === 'string')).slice(0, 160);
      const values = out.options.map(option => typeof option === 'string' ? option : option.v);
      out.def = values.includes(sourceValue) ? sourceValue : (values[0] ?? 'Default');
      if (!out.options.length) out.options = [out.def];
    } else {
      out.min = Number.isFinite(connection.min) ? connection.min : Number.isFinite(c.min) ? c.min : 0;
      out.max = Number.isFinite(connection.max) ? connection.max : Number.isFinite(c.max) ? c.max : 100;
      if (out.max < out.min) [out.min, out.max] = [out.max, out.min];
      out.step = Number.isFinite(connection.step) && connection.step > 0 ? connection.step : Number.isFinite(c.step) && c.step > 0 ? c.step : Math.max((out.max - out.min) / 100, .01);
      out.unit = connection.unit || '';
      out.def = Number.isFinite(sourceValue) ? PM.clamp(sourceValue, out.min, out.max) : out.min;
    }
    return out;
  }).filter(c => c && (c.type !== 'button' || c.cmd || c.action));
  const state = { ...(recipe?.state || {}) };
  controls.filter(control => control.stateKey).forEach(control => {
    if (state[control.stateKey] === undefined) state[control.stateKey] = control.def;
  });
  return {
    kind,
    operation,
    targetPanelId: cleanText(raw?.targetPanelId, context?.targetPanelId || '', 100),
    dockId: cleanText(raw?.dockId, '', 100),
    placement: ['before', 'after', 'replace'].includes(raw?.placement) ? raw.placement : (operation === 'modify' ? 'replace' : 'after'),
    message: cleanText(raw?.message, operation === 'modify' ? 'The redesigned section is ready.' : 'The new section is ready.', 220),
    steps: steps.length ? steps : ['Prepare the editable change', 'Review the visible result'],
    chromeEdit: kind === 'chrome' ? { target: chromeTarget, value: chromeValue } : null,
    interfaceEdit: kind === 'interface' ? interfaceEdit : null,
    section: {
      id: baseId, title: cleanText(sectionSource.title, 'Generated section', 70),
      size: PM.clamp(Number(sectionSource.size) || 220, 120, 700), note: cleanText(sectionSource.note, '', 240),
      tool: recipe?.id || '', state, controls,
    },
    sceneEdit: kind === 'scene' ? PM.AgentHarness.sanitizeProposal(raw?.sceneEdit, request) : null,
    workspaceEdit: kind === 'workspace' ? sanitizeWorkspaceEdit(raw?.workspaceEdit, context) : null,
    panelEdit: kind === 'panels' ? panelEdit : { actions: [] },
  };
}

function sanitizeWorkspaceEdit(encoded, context) {
  let raw;
  try {
    if (typeof encoded !== 'string' || encoded.length > 120_000) return null;
    raw = JSON.parse(encoded);
  } catch { return null; }
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return null;
  const clean = (value, fallback, max = 80) => typeof value === 'string' && value.trim() ? value.trim().slice(0, max) : fallback;
  const sections = (Array.isArray(raw.sections) ? raw.sections : []).slice(0, 6).map(section => {
    const plan = sanitizePlan({
      kind: 'section', operation: 'create', targetPanelId: '', dockId: '', placement: 'after', message: '',
      chromeEdit: { target: 'preview.cornerRadius', value: 'square' }, section,
      sceneEdit: { label: '', summary: '', commands: [], reviewTimes: [] }, workspaceEdit: '{}',
    }, context);
    return plan.section.controls.length ? plan.section : null;
  }).filter(Boolean);
  const customIds = new Set(sections.map(section => section.id));
  const known = new Set([...Object.keys(PM.PANELS || {}), ...customIds]);
  const usedDockIds = new Set();
  const docks = (Array.isArray(raw.docks) ? raw.docks : []).slice(0, 4).map((dock, index) => {
    const baseId = slug(clean(dock?.id, index === 0 ? 'center' : `dock-${index + 1}`));
    let id = baseId, suffix = 2; while (usedDockIds.has(id)) id = `${baseId}-${suffix++}`;
    usedDockIds.add(id);
    const panels = (Array.isArray(dock?.panels) ? dock.panels : []).slice(0, 10).map(panel => {
      const spec = typeof panel === 'string' ? { id: panel } : panel;
      const panelId = clean(spec?.id, '', 80);
      if (!known.has(panelId)) return null;
      const out = { id: panelId };
      if (Number.isFinite(spec?.size)) out.size = PM.clamp(spec.size, 100, 900);
      if (spec?.flex === true) out.flex = true;
      return out;
    }).filter(Boolean);
    return { id, size: Number.isFinite(dock?.size) ? PM.clamp(dock.size, 180, 700) : undefined, flex: dock?.flex === true, panels };
  }).filter(dock => dock.panels.length);
  if (!docks.some(dock => dock.panels.some(panel => panel.id === 'viewer'))) {
    let center = docks.find(dock => dock.id === 'center');
    if (!center) {
      center = { id: 'center', flex: true, panels: [] };
      if (docks.length >= 4) docks[docks.length - 1] = center;
      else docks.push(center);
    }
    center.panels.unshift({ id: 'viewer', flex: true });
  }
  const placed = new Set(docks.flatMap(dock => dock.panels.map(panel => panel.id)));
  const unplaced = sections.filter(section => !placed.has(section.id));
  if (unplaced.length) {
    let side = docks.find(dock => dock.id === 'left' || dock.id === 'right');
    if (!side) side = docks.find(dock => dock.id !== 'center');
    if (!side) {
      side = { id: 'right', size: 300, flex: false, panels: [] };
      if (docks.length >= 4) docks[docks.length - 1] = side;
      else docks.push(side);
    }
    unplaced.forEach(section => side.panels.push({ id: section.id, size: section.size }));
  }
  return {
    name: clean(raw.name, 'Generated workspace'),
    density: ['compact', 'normal', 'comfy'].includes(raw.density) ? raw.density : 'normal',
    accent: /^#[0-9a-f]{6}$/i.test(raw.accent) ? raw.accent.toUpperCase() : '#FF6B1A',
    docks: docks.slice(0, 4), sections,
  };
}

/* Apply only explicitly supported app-chrome edits to the versioned workspace
   manifest. This is a pure mutation seam used inside WS.mutate, never a CSS or
   project-canvas write from model output. */
function applyChromeEdit(workspace, edit) {
  if (!workspace || !edit) return false;
  workspace.chrome = workspace.chrome && typeof workspace.chrome === 'object' ? workspace.chrome : {};
  if (edit.target === 'preview.cornerRadius' && ['square', 'rounded'].includes(edit.value)) {
    workspace.chrome.previewCornerRadius = edit.value; return true;
  }
  if (edit.target === 'timeline.surfaceOrder' && ['normal', 'reversed'].includes(edit.value)) {
    workspace.chrome.timelineSurfaceOrder = edit.value; return true;
  }
  return false;
}

function slug(value) {
  const out = String(value || '').toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '').slice(0, 48);
  return out || 'generated-section';
}

function showPreview() {
  S.phase = 'conversation';
  renderConversation(true);
}

function setStepProgress(index) {
  if (!S.steps.length) return;
  const active = Math.max(0, Math.min(index, S.steps.length - 1));
  S.steps.forEach((step, i) => { step.status = i < active ? 'complete' : i === active ? 'active' : 'pending'; });
}

function updateSteps(titles, active = -1) {
  S.steps = (Array.isArray(titles) ? titles : []).map((title, index) => ({
    id: `${S.requestToken}-${index}`, title: String(title || '').trim(),
    status: index < active ? 'complete' : index === active ? 'active' : 'pending',
  })).filter(step => step.title);
}

function finishSteps() {
  S.steps.forEach(step => { step.status = 'complete'; });
}

function locatePanel(workspace, panelId) {
  for (const dock of workspace.layout?.docks || []) {
    const index = (dock.panels || []).findIndex(p => p.id === panelId);
    if (index >= 0) return { dock, index };
  }
  return null;
}

function uniqueSectionId(workspace, requested, keepId = '') {
  const occupied = new Set([
    ...Object.keys(PM.PANELS || {}),
    ...(workspace.custom || []).map(p => p.id),
  ]);
  if (requested === keepId || !occupied.has(requested)) return requested;
  let n = 2; while (occupied.has(`${requested}-${n}`)) n++;
  return `${requested}-${n}`;
}

function finishWorkspaceRun(checkpoint, summary, actions = []) {
  const after = PM.WS.historySnapshot();
  const historyId = PM.hist.external(
    `Agent · ${summary}`,
    () => PM.WS.restoreHistorySnapshot(checkpoint),
    () => PM.WS.restoreHistorySnapshot(after),
  );
  finishSteps();
  S.activity = ''; S.plan = null;
  S.panelRun = { checkpoint, after, historyId, actions, summary };
  S.conversation.push({ role: 'assistant', text: `${summary}. This change is in Undo history.` });
  S.phase = 'result'; renderConversation();
}

async function applyPlan() {
  if (!S.plan || !['conversation', 'preview'].includes(S.phase)) return;
  const plan = S.plan;
  if (plan.kind === 'scene') {
    await applyScenePlan(plan);
    return;
  }
  if (plan.kind === 'panels') {
    await applyPanelPlan(plan);
    return;
  }
  S.phase = 'applying'; S.activity = 'Applying the editable change…'; setStepProgress(0); renderConversation();
  await new Promise(resolve => requestAnimationFrame(resolve));
  const checkpoint = PM.WS.historySnapshot();
  if (plan.kind === 'workspace') {
    const manifest = plan.workspaceEdit;
    const created = PM.WS.create({
      name: manifest.name, base: PM.WS.current.id, density: manifest.density,
      theme: { ...(PM.WS.current.theme || {}), accent: manifest.accent },
      custom: manifest.sections,
      layout: { docks: manifest.docks },
    });
    PM.toast(`Created workspace · ${created.name}`);
    finishWorkspaceRun(checkpoint, `Created ${created.name}`);
    return;
  }
  if (plan.kind === 'chrome') {
    let changed = false;
    PM.WS.mutate(workspace => { changed = applyChromeEdit(workspace, plan.chromeEdit); });
    if (changed) PM.toast('Updated preview corner style');
    if (changed) finishWorkspaceRun(checkpoint, 'Applied the interface edit');
    else {
      finishSteps(); S.plan = null; S.activity = ''; S.phase = 'conversation';
      S.conversation.push({ role: 'assistant', text: 'That interface setting was already in place.' }); renderConversation(true);
    }
    return;
  }
  if (plan.kind === 'interface') {
    let changed = false;
    PM.WS.mutate(workspace => { changed = PM.WS.applyInterfaceEdit(workspace, plan.interfaceEdit); });
    if (changed) PM.toast('Updated Timeline design');
    if (changed) finishWorkspaceRun(checkpoint, 'Applied the Timeline redesign');
    else {
      finishSteps(); S.plan = null; S.activity = ''; S.phase = 'conversation';
      S.conversation.push({ role: 'assistant', text: 'Those Timeline settings were already in place.' }); renderConversation(true);
    }
    return;
  }
  const current = PM.WS.current;
  const target = locatePanel(current, plan.targetPanelId || S.context.targetPanelId);
  const targetIsCustom = (current.custom || []).some(p => p.id === target?.dock?.panels?.[target.index]?.id);
  const replacing = plan.operation === 'modify' && !!target;
  const desiredId = replacing && targetIsCustom ? target.dock.panels[target.index].id : plan.section.id;
  const sectionId = uniqueSectionId(current, desiredId, replacing && targetIsCustom ? desiredId : '');
  /* A rebuilt custom panel must not retain the old definition/cache. Remove its
     live instance before WS.activate() reconstructs the edited workspace. */
  if (replacing && targetIsCustom) delete PM.panelInst[desiredId];
  PM.WS.mutate(workspace => {
    workspace.custom = Array.isArray(workspace.custom) ? workspace.custom : [];
    const oldId = target && target.dock.panels[target.index].id;
    if (replacing && targetIsCustom) workspace.custom = workspace.custom.filter(p => p.id !== oldId);
    workspace.custom.push({ ...plan.section, id: sectionId });

    const liveTarget = locatePanel(workspace, oldId || '');
    let dock = liveTarget?.dock || (workspace.layout.docks || []).find(d => d.id === plan.dockId)
      || (workspace.layout.docks || []).find(d => d.id === 'center') || workspace.layout.docks[0];
    if (!dock) {
      dock = { id: 'center', flex: true, panels: [] };
      workspace.layout = workspace.layout || {}; workspace.layout.docks = [dock];
    }
    const spec = { id: sectionId, size: plan.section.size };
    if (replacing && liveTarget) liveTarget.dock.panels.splice(liveTarget.index, 1, spec);
    else {
      const index = liveTarget ? liveTarget.index + (plan.placement === 'before' ? 0 : 1) : dock.panels.length;
      dock.panels.splice(index, 0, spec);
    }
  });
  PM.toast((replacing ? 'Redesigned ' : 'Added ') + plan.section.title);
  finishWorkspaceRun(checkpoint, `${replacing ? 'Redesigned' : 'Added'} ${plan.section.title}`);
}

async function applyPanelPlan(plan) {
  const checkpoint = PM.WS.historySnapshot();
  S.phase = 'applying'; S.activity = 'Applying panel changes…'; setStepProgress(0); renderConversation();
  let result = { applied: [], runtime: [] };
  try {
    PM.WS.mutate(workspace => { result = applyPanelEdit(workspace, plan.panelEdit); });
    await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    for (const action of result.runtime) {
      let changed = false;
      if (action.type === 'collapse') changed = PM.Layout.setCollapsed(action.panelId, true);
      else if (action.type === 'expand') changed = PM.Layout.setCollapsed(action.panelId, false);
      else if (action.type === 'popout') changed = PM.Popout.open(action.panelId);
      else if (action.type === 'dock') changed = PM.Popout.dock(action.panelId);
      if (changed) result.applied.push(action);
    }
    if (!result.applied.length) throw new Error('Those panels were already arranged that way');
    const summary = `${result.applied.length} panel change${result.applied.length === 1 ? '' : 's'} applied`;
    finishWorkspaceRun(checkpoint, summary, result.applied);
    PM.toast(summary);
  } catch (error) {
    PM.WS.restoreHistorySnapshot(checkpoint);
    const current = S.steps.find(step => step.status === 'active'); if (current) current.status = 'error';
    S.activity = ''; S.plan = null; S.phase = 'conversation';
    S.conversation.push({ role: 'assistant', text: `${String(error.message || error).slice(0, 180)}. Nothing was changed.` });
    renderConversation(true);
  }
}

function undoPanelRun() {
  if (!S.panelRun) return;
  const checkpoint = S.panelRun.checkpoint;
  S.panelRun.actions.filter(action => action.type === 'popout').forEach(action => PM.Popout.dock(action.panelId));
  if (!PM.hist.undoIfTop(S.panelRun.historyId)) {
    const current = PM.WS.historySnapshot();
    PM.WS.restoreHistorySnapshot(checkpoint);
    PM.hist.external('Restore before agent change',
      () => PM.WS.restoreHistorySnapshot(current),
      () => PM.WS.restoreHistorySnapshot(checkpoint));
  }
  S.panelRun = null; S.phase = 'conversation';
  S.conversation.push({ role: 'assistant', text: 'I restored the previous panel layout.' });
  renderConversation(true); PM.toast('Panel changes undone');
}

function keepPanelRun() {
  if (!S.panelRun) return;
  S.panelRun = null; S.phase = 'conversation';
  S.conversation.push({ role: 'assistant', text: 'Kept the agent change. Command-Z can still reverse it.' });
  renderConversation(true);
}

async function applyScenePlan(plan) {
  S.phase = 'applying'; S.plan = null; S.activity = 'Applying structured source edit…';
  setStepProgress(0);
  renderConversation();
  try {
    let progressIndex = 0;
    const run = await PM.AgentHarness.execute(S.requestText, plan.sceneEdit, value => {
      setStepProgress(Math.min(progressIndex++, S.steps.length - 1));
      S.activity = value; renderConversation();
    });
    finishSteps();
    S.activity = ''; S.run = run;
    showSceneResult(run);
  } catch (error) {
    const current = S.steps.find(step => step.status === 'active'); if (current) current.status = 'error';
    S.activity = ''; S.phase = 'conversation';
    S.conversation.push({ role: 'assistant', text: `${String(error.message || error).slice(0, 180)} Nothing was applied.` });
    renderConversation(true);
  }
}

function showSceneResult(run) {
  S.phase = 'result'; S.run = run;
  renderConversation();
}

function keepSceneRun() {
  if (!S.run) return;
  PM.toast('Kept agent change');
  S.conversation.push({ role: 'assistant', text: `Kept ${S.run.applied.length} editable source changes. Command-Z can still reverse the complete run.` });
  S.run = null; S.phase = 'conversation'; renderConversation(true);
}

function undoSceneRun() {
  if (!S.run) return;
  const restored = PM.AgentHarness.rollback(S.run.checkpoint);
  PM.toast(restored ? 'Agent change undone' : 'Could not restore the agent checkpoint');
  S.conversation.push({ role: 'assistant', text: restored ? 'I restored the checkpoint. Tell me what to try differently.' : 'I could not restore that checkpoint.' });
  S.run = null; S.phase = 'conversation'; renderConversation(true);
}

function onKey(event) {
  if (!S.active) return;
  if (event.key === 'Escape') {
    event.preventDefault(); event.stopPropagation();
    if (S.phase === 'applying') return;
    if (S.phase === 'result') return undoSceneRun();
    cancel();
  }
}

function dismissOverlay(preserveContext) {
  if (!S.active) return;
  S.active = false;
  removeEventListener('keydown', onKey, true);
  cancelAnimationFrame(S.hintFrame); S.hintFrame = 0; S.hintPoint = null;
  if (S.renderStop) S.renderStop();
  S.root?.remove();
  Object.assign(S, {
    root: null, ink: null, path: null, shadePath: null, hint: null, card: null,
    outline: null, renderStop: null, points: [], sceneFrame: null,
  });
  if (!preserveContext) {
    S.region = null; S.context = null; S.regionImage = null;
    S.phase = S.conversation.length || S.plan || S.run ? 'conversation' : 'idle';
  }
  setTimeout(refreshSceneCache, 80);
}

function cancel() {
  if (!S.active) return;
  dismissOverlay(false);
  renderConversation();
}

function startRipple(canvas, origin, sceneBitmap, adapterPromise = null) {
  if (window.matchMedia?.('(prefers-reduced-motion: reduce)').matches) {
    sceneBitmap?.close?.();
    canvas.dataset.renderer = 'reduced-motion';
    return () => {};
  }
  let stopped = false, failed = false, frame = 0, fallbackTimer = 0, device = null;
  let context = null, uniformBuffer = null, sceneTexture = null;
  const fallback = reason => {
    if (stopped || failed) return;
    failed = true; cancelAnimationFrame(frame);
    /* A lost device can leave an opaque swapchain frame over the fallback. */
    try { context?.unconfigure?.(); } catch {}
    canvas.width = 1; canvas.height = 1;
    canvas.dataset.renderer = 'css-fallback';
    canvas.style.background = `radial-gradient(circle at ${origin.x}px ${origin.y}px,rgba(255,107,26,.34),rgba(76,35,88,.16) 30%,rgba(8,8,12,.05) 64%,transparent 78%)`;
    canvas.style.opacity = '1';
    canvas.style.transition = 'opacity .34s ease';
    /* The selection workflow remains visibly active even without WebGPU. The
       old fallback faded to zero, which made a recoverable renderer failure
       look exactly like a dead feature. */
    fallbackTimer = setTimeout(() => { if (!stopped) canvas.style.opacity = '.24'; }, 900);
    if (reason) console.warn('WebGPU ripple unavailable; using visual fallback', reason);
  };

  /* Device and adapter acquisition are async, but activation needs a synchronous
     cleanup handle. The closure below can cancel safely at every await boundary. */
  (async () => {
    if (!navigator.gpu) throw new Error('WebGPU is not supported by this web view');
    canvas.dataset.renderer = 'webgpu-initializing';
    const adapter = await (adapterPromise || requestRippleAdapter());
    if (!adapter) throw new Error('No WebGPU adapter is available');
    device = await adapter.requestDevice();
    if (stopped) { device.destroy(); return; }

    context = canvas.getContext('webgpu');
    if (!context) throw new Error('Could not create a WebGPU canvas context');
    let format = navigator.gpu.getPreferredCanvasFormat();
    let hdr = false;
    /* Prefer a floating-point extended-range swapchain. Modern WebGPU engines
       expose extended tone mapping here; older ones safely fall back to SDR. */
    try {
      format = 'rgba16float';
      context.configure({
        device, format, alphaMode: 'premultiplied', colorSpace: 'display-p3',
        toneMapping: { mode: 'extended' },
      });
      hdr = true;
    } catch {
      format = navigator.gpu.getPreferredCanvasFormat();
      context.configure({ device, format, alphaMode: 'premultiplied' });
    }
    canvas.dataset.dynamicRange = hdr ? 'hdr' : 'sdr';

    const wgsl = `
      struct Uniforms {
        resolution: vec2f,
        origin: vec2f,
        time: f32,
        intensity: f32,
        hasScene: f32,
        padding: f32,
      }
      @group(0) @binding(0) var<uniform> uniforms: Uniforms;
      @group(0) @binding(1) var sceneSampler: sampler;
      @group(0) @binding(2) var sceneTexture: texture_2d<f32>;

      @vertex
      fn vertexMain(@builtin(vertex_index) vertexIndex: u32) -> @builtin(position) vec4f {
        var positions = array<vec2f, 3>(
          vec2f(-1.0, -1.0),
          vec2f( 3.0, -1.0),
          vec2f(-1.0,  3.0)
        );
        return vec4f(positions[vertexIndex], 0.0, 1.0);
      }

      @fragment
      fn fragmentMain(@builtin(position) pixel: vec4f) -> @location(0) vec4f {
        let uv = pixel.xy / uniforms.resolution;
        let source = uniforms.origin / uniforms.resolution;
        let aspect = uniforms.resolution.x / uniforms.resolution.y;
        let delta = (uv - source) * vec2f(aspect, 1.0);
        let distanceFromSource = length(delta);
        // The cursor remains the emitter, while the wavefront is free to carry
        // beyond it and across the full window before the animation settles.
        let entrance = smoothstep(0.0, 0.11, uniforms.time);
        let propagationFade = exp(-uniforms.time * 0.38);
        let front = uniforms.time * 1.32;

        // Wide, low-energy feedback bands make the deliberate shake legible
        // across the editor instead of reading as tiny lines near the cursor.
        // Their alpha remains restrained and the full-screen canvas clips them
        // to the viewport without ever intercepting pointer input.
        let crest = exp(-pow((distanceFromSource - front) * 4.4, 2.0)) * propagationFade;
        let echo = exp(-pow((distanceFromSource - front + 0.28) * 6.2, 2.0)) * propagationFade;
        let wakeMask = smoothstep(front + 0.48, front - 0.2, distanceFromSource);
        let wake = (0.5 + 0.5 * sin(distanceFromSource * 34.0 - uniforms.time * 12.0))
          * wakeMask * exp(-distanceFromSource * 1.2);
        let core = exp(-distanceFromSource * 5.4) * exp(-uniforms.time * 0.74);
        let cursorLens = exp(-pow(distanceFromSource * 3.4, 2.0));
        let cursorRipple = sin(distanceFromSource * 38.0 - uniforms.time * 17.0)
          * cursorLens * 0.006;
        let shimmer = 0.5 + 0.5 * cos(atan2(delta.y, delta.x) * 3.0 - uniforms.time * 2.0);

        let ringAlpha = clamp((crest * 0.24 + echo * 0.08 + wake * 0.04 + core * 0.13)
          * uniforms.intensity * entrance, 0.0, 0.42);
        let violet = vec3f(0.34, 0.18, 0.55);
        let hot = clamp(crest + core + shimmer * echo * 0.35, 0.0, 1.0);
        // Keep the defined displacement edge neutral; orange belongs only to
        // the very broad HDR haze below, never to a crisp ring.
        let ringColor = mix(violet, vec3f(0.92, 0.86, 0.82), hot * 0.24);

        // True radial displacement: the interface texture itself is sampled at
        // offset coordinates around the wave crest, with a restrained RGB split.
        let radialDirection = delta / max(distanceFromSource, 0.0001);
        let displacementStrength = (crest * 0.018 - echo * 0.006 + wake * 0.0015 + cursorRipple)
          * uniforms.intensity * entrance;
        let displacement = radialDirection * displacementStrength;
        let displacedUV = clamp(uv - displacement, vec2f(0.001), vec2f(0.999));
        let red = textureSample(sceneTexture, sceneSampler, clamp(displacedUV - displacement * 0.08, vec2f(0.001), vec2f(0.999))).r;
        let green = textureSample(sceneTexture, sceneSampler, displacedUV).g;
        let blue = textureSample(sceneTexture, sceneSampler, clamp(displacedUV + displacement * 0.08, vec2f(0.001), vec2f(0.999))).b;
        let displacedScene = vec3f(red, green, blue);
        let sceneColor = displacedScene * (1.0 + crest * 0.012 * uniforms.intensity);
        // Orange is intentionally defocused into two enormous Gaussian lobes.
        // Their low energy still exceeds SDR white after scene compositing,
        // but there is no sharp orange crest left anywhere in the effect.
        let hdrBloomNear = exp(-pow((distanceFromSource - front) * 1.15, 2.0));
        let hdrBloomFar = exp(-pow((distanceFromSource - front) * 0.52, 2.0));
        let hdrCrest = (hdrBloomNear * 0.18 + hdrBloomFar * 0.045)
          * propagationFade * uniforms.intensity * entrance;
        let hdrColor = vec3f(1.0, 0.72, 0.52);
        let composited = sceneColor + ringColor * ringAlpha + hdrColor * hdrCrest;
        let outputAlpha = mix(ringAlpha, 1.0, uniforms.hasScene);
        let outputColor = mix(ringColor * ringAlpha, composited, uniforms.hasScene);
        return vec4f(outputColor, outputAlpha);
      }`;
    const module = device.createShaderModule({ label: 'Spatial ripple WGSL', code: wgsl });
    const compilation = await module.getCompilationInfo();
    const errors = compilation.messages.filter(message => message.type === 'error');
    if (errors.length) throw new Error(errors.map(message => message.message).join('\n'));

    const pipeline = device.createRenderPipeline({
      label: 'Spatial ripple pipeline', layout: 'auto',
      vertex: { module, entryPoint: 'vertexMain' },
      fragment: {
        module, entryPoint: 'fragmentMain', targets: [{
          format,
          blend: {
            color: { srcFactor: 'one', dstFactor: 'one-minus-src-alpha', operation: 'add' },
            alpha: { srcFactor: 'one', dstFactor: 'one-minus-src-alpha', operation: 'add' },
          },
        }],
      },
      primitive: { topology: 'triangle-list' },
    });
    uniformBuffer = device.createBuffer({
      label: 'Spatial ripple uniforms', size: 32,
      usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });
    sceneTexture = device.createTexture({
      label: 'Spatial interface snapshot', size: [1, 1], format: 'rgba8unorm',
      usage: GPUTextureUsage.TEXTURE_BINDING | GPUTextureUsage.COPY_DST | GPUTextureUsage.RENDER_ATTACHMENT,
    });
    let hasScene = 0;
    if (sceneBitmap) {
      sceneTexture.destroy();
      sceneTexture = device.createTexture({
        label: 'Spatial interface snapshot', size: [sceneBitmap.width, sceneBitmap.height], format: 'rgba8unorm',
        usage: GPUTextureUsage.TEXTURE_BINDING | GPUTextureUsage.COPY_DST | GPUTextureUsage.RENDER_ATTACHMENT,
      });
      device.queue.copyExternalImageToTexture(
        { source: sceneBitmap }, { texture: sceneTexture }, [sceneBitmap.width, sceneBitmap.height],
      );
      sceneBitmap.close?.(); hasScene = 1;
    }
    const sceneSampler = device.createSampler({ magFilter: 'linear', minFilter: 'linear', addressModeU: 'clamp-to-edge', addressModeV: 'clamp-to-edge' });
    const bindGroup = device.createBindGroup({
      layout: pipeline.getBindGroupLayout(0),
      entries: [
        { binding: 0, resource: { buffer: uniformBuffer } },
        { binding: 1, resource: sceneSampler },
        { binding: 2, resource: sceneTexture.createView() },
      ],
    });
    const startedAt = performance.now();
    const fadeStartsAt = 0.48;
    const settlesAt = 1.45;
    const trackedOrigin = { x: origin.x, y: origin.y };
    let priorFrameAt = startedAt;
    canvas.dataset.renderer = 'webgpu';

    device.addEventListener?.('uncapturederror', event => {
      console.warn('WebGPU ripple uncaptured error', event.error || event);
    });
    device.lost.then(info => {
      if (stopped) return;
      const reason = info?.reason || 'unknown';
      const message = info?.message || 'The WebGPU device was lost';
      fallback(new Error(`${message} (reason: ${reason})`));
    });
    const draw = now => {
      if (stopped || failed) return;
      try {
        const scale = Math.min(devicePixelRatio || 1, 2);
        const width = Math.max(1, Math.round(innerWidth * scale));
        const height = Math.max(1, Math.round(innerHeight * scale));
        if (canvas.width !== width || canvas.height !== height) { canvas.width = width; canvas.height = height; }
        const elapsed = (now - startedAt) / 1000;
        const fadeProgress = Math.max(0, Math.min(1, (elapsed - fadeStartsAt) / (settlesAt - fadeStartsAt)));
        const intensity = 1 - fadeProgress * fadeProgress * (3 - 2 * fadeProgress);
        const deltaSeconds = Math.min(.05, Math.max(0, (now - priorFrameAt) / 1000));
        const follow = 1 - Math.exp(-deltaSeconds * 18);
        trackedOrigin.x += (origin.x - trackedOrigin.x) * follow;
        trackedOrigin.y += (origin.y - trackedOrigin.y) * follow;
        priorFrameAt = now;
        device.queue.writeBuffer(uniformBuffer, 0, new Float32Array([
          width, height, trackedOrigin.x * scale, trackedOrigin.y * scale, Math.min(elapsed, settlesAt), intensity, hasScene, 0,
        ]));
        const encoder = device.createCommandEncoder({ label: 'Spatial ripple frame' });
        const pass = encoder.beginRenderPass({ colorAttachments: [{
          view: context.getCurrentTexture().createView(),
          clearValue: { r: 0, g: 0, b: 0, a: 0 }, loadOp: 'clear', storeOp: 'store',
        }] });
        pass.setPipeline(pipeline); pass.setBindGroup(0, bindGroup); pass.draw(3); pass.end();
        device.queue.submit([encoder.finish()]);
        if (elapsed < settlesAt) frame = requestAnimationFrame(draw);
        else canvas.dataset.renderer = 'webgpu-settled';
      } catch (error) { fallback(error); }
    };
    frame = requestAnimationFrame(draw);
  })().catch(fallback);

  return () => {
    stopped = true; cancelAnimationFrame(frame); clearTimeout(fallbackTimer);
    try { sceneTexture?.destroy?.(); } catch {}
    try { uniformBuffer?.destroy?.(); } catch {}
    if (device) { try { device.destroy(); } catch {} }
  };
}
})();
