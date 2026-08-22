/* Powermove — spatial coding assistant.
   Shake the pointer to summon a full-window shader, circle interface, describe
   the change, preview the generated section manifest, then apply it safely. */
(() => {
const PM = window.PM, h = PM.h;

/* The native Codex client owns ChatGPT authentication. This bridge only moves a
   prompt and a strict JSON schema across WKWebView; no account secret enters JS. */
const pending = new Map();
PM.CodexBridge = {
  request(prompt, schema) {
    const bridge = window.webkit?.messageHandlers?.pmCodex;
    if (!bridge) return Promise.reject(new Error('The coding agent is available in the Powermove macOS app'));
    const id = PM.uid('spatial-codex-');
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        pending.delete(id);
        reject(new Error('The coding agent took too long to respond'));
      }, 120000);
      pending.set(id, { resolve, reject, timer });
      bridge.postMessage({ id, prompt, schema });
    });
  },
  resolve(id, result) {
    const job = pending.get(id); if (!job) return;
    pending.delete(id); clearTimeout(job.timer);
    let text = '';
    try {
      const bytes = Uint8Array.from(atob(result.dataBase64 || ''), c => c.charCodeAt(0));
      text = new TextDecoder().decode(bytes);
    } catch { text = 'The coding-agent response could not be decoded'; }
    if (result.ok) job.resolve(text); else job.reject(new Error(text));
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
  rippleWarmup: null, sceneCache: null, sceneCacheAt: 0, cachePending: null,
  hintFrame: 0, hintPoint: null,
};

const Spatial = {
  init,
  activate,
  cancel,
  get active() { return S.active; },
  /* Small pure seams are exposed for deterministic regression tests. */
  math: { motionProfile, shakeReady, shakeIntent, loopInfo, isClickGesture, pointInPolygon, sanitizePlan, hintPosition, clampFloatingPosition },
  lifecycle: { requestAdapter: requestRippleAdapter },
};
PM.SpatialAssistant = Spatial;

function init() {
  if (S.initialized) return;
  S.initialized = true;
  addEventListener('pointerdown', () => { S.pressed = true; }, true);
  addEventListener('pointerup', () => { S.pressed = false; }, true);
  addEventListener('pointercancel', () => { S.pressed = false; }, true);
  addEventListener('pointermove', watchShake, true);
  refreshSceneCache();
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
    capture: S.sceneCache ? null : PM.WindowCapture.request(),
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
  S.hint = h('div.spatial-hint', h('span', 'Circle any part of the interface'));
  S.root = h('div#spatial-assistant', { role: 'dialog', 'aria-label': 'Spatial coding assistant' },
    canvas, h('div.spatial-wash'), S.ink, S.hint);
  addEventListener('keydown', onKey, true);
  /* Capture first, while the overlay is not in the DOM. This is the texture the
     WGSL pass genuinely displaces instead of merely painting over the UI. */
  const cachedScene = S.sceneCache;
  if (cachedScene) { S.sceneCache = null; S.sceneCacheAt = 0; }
  (cachedScene ? Promise.resolve(cachedScene) : warmup?.capture || PM.WindowCapture.request()).then(sceneBitmap => {
    if (!S.active) { sceneBitmap?.close?.(); return; }
    S.root.addEventListener('pointerdown', beginCircle);
    document.body.appendChild(S.root);
    scheduleHint(x, y);
    S.renderStop = startRipple(canvas, S.origin, sceneBitmap, warmup?.adapter || null);
    setTimeout(() => { if (S.active && S.phase === 'arming') S.phase = 'selecting'; }, 340);
  });
}

function beginCircle(event) {
  if (S.phase !== 'selecting' || event.button !== 0 || event.target.closest('.spatial-compose')) return;
  event.preventDefault();
  S.phase = 'drawing'; S.points = [{ x: event.clientX, y: event.clientY }];
  S.path.setAttribute('d', `M ${event.clientX} ${event.clientY}`);
  S.root.setPointerCapture?.(event.pointerId);
  const move = e => {
    const p = { x: e.clientX, y: e.clientY };
    const last = S.points.at(-1);
    if (Math.hypot(p.x - last.x, p.y - last.y) < 3) return;
    S.points.push(p);
    S.path.setAttribute('d', pathData(S.points));
  };
  const finish = e => {
    S.root.removeEventListener('pointermove', move);
    S.root.removeEventListener('pointerup', finish);
    S.root.removeEventListener('pointercancel', abort);
    S.root.releasePointerCapture?.(event.pointerId);
    finishCircle(e);
  };
  const abort = () => {
    S.root.removeEventListener('pointermove', move);
    S.root.removeEventListener('pointerup', finish);
    S.root.removeEventListener('pointercancel', abort);
    resetSelection('Circle any part of the interface');
  };
  S.root.addEventListener('pointermove', move);
  S.root.addEventListener('pointerup', finish);
  S.root.addEventListener('pointercancel', abort);
}

function finishCircle(event) {
  if (event) S.points.push({ x: event.clientX, y: event.clientY });
  if (isClickGesture(S.points)) { cancel(); return; }
  const info = loopInfo(S.points);
  if (!info.closed) {
    resetSelection(info.reason || 'Close the loop around a section');
    return;
  }
  S.path.setAttribute('d', pathData(S.points) + ' Z');
  S.shadePath.setAttribute('fill-rule', 'evenodd');
  S.shadePath.setAttribute('d', `M 0 0 H ${innerWidth} V ${innerHeight} H 0 Z ${pathData(S.points)} Z`);
  S.root.classList.add('target-selected');
  S.region = info.rect;
  S.context = inspectRegion(S.points, S.region);
  S.phase = 'composing';
  S.hint.style.display = 'none';
  showOutline(S.region);
  showComposer();
}

function isClickGesture(points) {
  if (!Array.isArray(points) || !points.length) return false;
  const start = points[0];
  return points.every(p => Math.hypot(p.x - start.x, p.y - start.y) <= 7);
}

function loopInfo(points) {
  if (!Array.isArray(points) || points.length < 12) return { closed: false, reason: 'Draw a fuller loop around a section' };
  let length = 0, minX = Infinity, maxX = -Infinity, minY = Infinity, maxY = -Infinity;
  points.forEach((p, i) => {
    if (i) length += Math.hypot(p.x - points[i - 1].x, p.y - points[i - 1].y);
    minX = Math.min(minX, p.x); maxX = Math.max(maxX, p.x);
    minY = Math.min(minY, p.y); maxY = Math.max(maxY, p.y);
  });
  const width = maxX - minX, height = maxY - minY;
  const gap = Math.hypot(points.at(-1).x - points[0].x, points.at(-1).y - points[0].y);
  const closeLimit = Math.max(34, Math.min(74, Math.hypot(width, height) * .25));
  if (length < 150 || width < 34 || height < 34) return { closed: false, reason: 'Circle a larger interface area' };
  if (gap > closeLimit) return { closed: false, reason: 'Close the loop to select that area' };
  return { closed: true, rect: { x: minX, y: minY, width, height }, length, gap };
}

function pathData(points) { return points.map((p, i) => `${i ? 'L' : 'M'} ${Math.round(p.x)} ${Math.round(p.y)}`).join(' '); }

function resetSelection(message) {
  S.phase = 'selecting'; S.points = []; S.path.setAttribute('d', '');
  S.shadePath?.setAttribute('d', ''); S.root?.classList.remove('target-selected');
  S.hint.style.display = ''; S.hint.querySelector('span').textContent = message;
  setTimeout(() => {
    if (S.active && S.phase === 'selecting') S.hint.querySelector('span').textContent = 'Circle any part of the interface';
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

function showOutline(rect) {
  S.outline = h('div.spatial-outline', { style: {
    left: Math.max(4, rect.x - 5) + 'px', top: Math.max(4, rect.y - 5) + 'px',
    width: Math.min(innerWidth - 8, rect.width + 10) + 'px', height: Math.min(innerHeight - 8, rect.height + 10) + 'px',
  } });
  S.root.appendChild(S.outline);
}

function cardPosition(card, rect) {
  const left = PM.clamp(rect.x + rect.width + 14, 14, innerWidth - Math.min(420, innerWidth - 28) - 14);
  let top = PM.clamp(rect.y, 60, innerHeight - card.offsetHeight - 14);
  if (left < rect.x + rect.width && rect.y + rect.height + 14 + card.offsetHeight < innerHeight) top = rect.y + rect.height + 14;
  Object.assign(card.style, { left: Math.round(left) + 'px', top: Math.round(top) + 'px' });
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

function makeCardMovable(card, handle) {
  handle.tabIndex = 0; handle.setAttribute('role', 'button');
  handle.setAttribute('aria-label', 'Move assistant result');
  handle.addEventListener('pointerdown', event => {
    if (event.button !== 0) return;
    event.preventDefault(); event.stopPropagation();
    const startX = event.clientX, startY = event.clientY;
    const rect = card.getBoundingClientRect();
    handle.setPointerCapture?.(event.pointerId);
    const move = e => moveCardTo(card, rect.left + e.clientX - startX, rect.top + e.clientY - startY);
    const end = e => {
      handle.removeEventListener('pointermove', move); handle.removeEventListener('pointerup', end); handle.removeEventListener('pointercancel', end);
      handle.releasePointerCapture?.(e.pointerId);
    };
    handle.addEventListener('pointermove', move); handle.addEventListener('pointerup', end); handle.addEventListener('pointercancel', end);
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

function showComposer() {
  const input = h('textarea', { placeholder: S.context.targetPanelId ? 'How should this section change?' : 'What should be created here?', rows: '3' });
  const status = h('span.spatial-status', 'Enter to send');
  const cancelBtn = h('button.spatial-action', { onclick: cancel }, 'Cancel');
  const pinBtn = h('button.spatial-action', { onclick: () => cardPosition(S.card, S.region), title: 'Return beside the circled region' }, 'Return to layout');
  const sendBtn = h('button.spatial-action.pri', { onclick: () => sendRequest(input, status, sendBtn, cancelBtn) }, 'Send to Codex');
  input.addEventListener('keydown', e => {
    e.stopPropagation();
    if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); sendRequest(input, status, sendBtn, cancelBtn); }
  });
  const handle = h('div.spatial-target', S.context.targetPanelId ? `Selected · ${S.context.targetTitle}` : 'Selected · open interface area');
  S.card = h('div.spatial-compose',
    handle,
    input,
    h('div.spatial-actions', status, cancelBtn, pinBtn, sendBtn));
  S.root.appendChild(S.card);
  makeCardMovable(S.card, handle);
  requestAnimationFrame(() => { cardPosition(S.card, S.region); input.focus(); });
}

async function sendRequest(input, status, sendBtn, cancelBtn) {
  const request = input.value.trim();
  if (!request || S.phase === 'working') return;
  S.phase = 'working'; input.disabled = true; sendBtn.disabled = true; cancelBtn.disabled = true;
  status.textContent = 'Designing section…';
  const token = ++S.requestToken;
  try {
    const raw = await PM.CodexBridge.request(agentPrompt(request), responseSchema());
    if (!S.active || token !== S.requestToken) return;
    let decoded;
    try { decoded = JSON.parse(raw); } catch { throw new Error('The coding agent returned an invalid section'); }
    const plan = sanitizePlan(decoded, S.context);
    if (plan.operation === 'noop') throw new Error(plan.message || 'No safe interface change was generated');
    S.plan = plan; showPreview();
  } catch (error) {
    if (!S.active || token !== S.requestToken) return;
    S.phase = 'composing'; input.disabled = false; sendBtn.disabled = false; cancelBtn.disabled = false;
    status.textContent = String(error.message || error).slice(0, 110); input.focus();
  }
}

function responseSchema() {
  return {
    type: 'object', additionalProperties: false,
    required: ['operation', 'targetPanelId', 'dockId', 'placement', 'message', 'section'],
    properties: {
      operation: { type: 'string', enum: ['create', 'modify', 'noop'] },
      targetPanelId: { type: 'string' }, dockId: { type: 'string' },
      placement: { type: 'string', enum: ['before', 'after', 'replace'] },
      message: { type: 'string' },
      section: {
        type: 'object', additionalProperties: false, required: ['id', 'title', 'size', 'note', 'controls'],
        properties: {
          id: { type: 'string' }, title: { type: 'string' }, size: { type: 'number' }, note: { type: 'string' },
          controls: { type: 'array', maxItems: 12, items: {
            type: 'object', additionalProperties: false,
            required: ['type', 'label', 'parameter', 'defaultValue', 'min', 'max', 'step', 'options', 'target', 'path', 'command'],
            properties: {
              type: { type: 'string', enum: ['slider', 'color', 'toggle', 'select', 'button'] }, label: { type: 'string' },
              parameter: { type: 'string' }, defaultValue: { anyOf: [{ type: 'string' }, { type: 'number' }, { type: 'boolean' }, { type: 'null' }] }, min: { type: 'number' }, max: { type: 'number' }, step: { type: 'number' },
              options: { type: 'array', items: { type: 'string' } }, target: { type: 'string' }, path: { type: 'string' }, command: { type: 'string' },
            },
          } },
        },
      },
    },
  };
}

function agentPrompt(request) {
  const workspace = PM.WS.current;
  const commands = Object.values(PM.commands || {}).map(c => ({ id: c.id, label: c.label })).slice(0, 80);
  const sourcePaths = ['properties.position.x', 'properties.position.y', 'properties.scale.x', 'properties.scale.y', 'properties.rotation', 'properties.opacity', 'content.text', 'content.color', 'content.size', 'layer.visible', 'layer.locked', 'layer.duration', 'layer.motionBlur'];
  return `You are the interface coding agent inside Powermove. Design one structured, editable interface section for the circled region. Return only the requested JSON object.

RULES
- operation=create when adding a section; operation=modify when replacing the selected section; noop only when the request cannot be represented safely.
- A section is a compact native Powermove panel made from slider, color, toggle, select, and button controls.
- Controls can bind to the selected composition layer with target="$selection" and one of these paths: ${sourcePaths.join(', ')}.
- Unbound controls must use a stable parameter name and become editable scene parameters.
- Buttons may use only one of the listed command ids. Never invent commands.
- Prefer 3–8 focused controls, a short title, and a useful one-sentence note. Avoid decorative filler.
- For unused control fields, still return schema-safe neutral values: empty string/array, 0, or false.
- Keep the current interface reachable. Do not remove unrelated docks or panels.

CIRCLED REGION
${JSON.stringify(S.context)}

CURRENT WORKSPACE
${JSON.stringify(workspace)}

AVAILABLE COMMANDS
${JSON.stringify(commands)}

USER REQUEST
${request}`;
}

function sanitizePlan(raw, context) {
  const safePaths = new Set(['properties.position.x', 'properties.position.y', 'properties.scale.x', 'properties.scale.y', 'properties.rotation', 'properties.opacity', 'content.text', 'content.color', 'content.size', 'layer.visible', 'layer.locked', 'layer.duration', 'layer.motionBlur']);
  const operation = ['create', 'modify', 'noop'].includes(raw?.operation) ? raw.operation : 'noop';
  const cleanText = (v, fallback = '', max = 100) => typeof v === 'string' && v.trim() ? v.trim().slice(0, max) : fallback;
  const source = raw?.section && typeof raw.section === 'object' ? raw.section : {};
  const baseId = slug(cleanText(source.id, cleanText(source.title, 'Generated section')));
  const controls = (Array.isArray(source.controls) ? source.controls : []).slice(0, 12).map((control, index) => {
    const c = control && typeof control === 'object' ? control : {};
    const type = ['slider', 'color', 'toggle', 'select', 'button'].includes(c.type) ? c.type : 'slider';
    const label = cleanText(c.label, `Control ${index + 1}`, 60);
    const out = { type, label };
    if (type === 'button') {
      if (c.command && PM.commands?.[c.command]) out.cmd = c.command;
      return out;
    }
    const target = cleanText(c.target, '', 120), path = cleanText(c.path, '', 120);
    if ((target === '$selection' || target === 'selection') && safePaths.has(path)) { out.target = target; out.path = path; }
    else out.param = cleanText(c.parameter, label, 80);
    if (type === 'color') out.def = /^#[0-9a-f]{6}$/i.test(c.defaultValue) ? c.defaultValue : '#FF6B1A';
    else if (type === 'toggle') out.def = !!c.defaultValue;
    else if (type === 'select') {
      out.options = (Array.isArray(c.options) ? c.options : []).filter(v => typeof v === 'string').slice(0, 12);
      out.def = out.options.includes(c.defaultValue) ? c.defaultValue : (out.options[0] || 'Default');
      if (!out.options.length) out.options = [out.def];
    } else {
      out.min = Number.isFinite(c.min) ? c.min : 0; out.max = Number.isFinite(c.max) ? c.max : 100;
      if (out.max < out.min) [out.min, out.max] = [out.max, out.min];
      out.step = Number.isFinite(c.step) && c.step > 0 ? c.step : Math.max((out.max - out.min) / 100, .01);
      out.def = Number.isFinite(c.defaultValue) ? PM.clamp(c.defaultValue, out.min, out.max) : out.min;
    }
    return out;
  }).filter(c => c.type !== 'button' || c.cmd);
  return {
    operation,
    targetPanelId: cleanText(raw?.targetPanelId, context?.targetPanelId || '', 100),
    dockId: cleanText(raw?.dockId, '', 100),
    placement: ['before', 'after', 'replace'].includes(raw?.placement) ? raw.placement : (operation === 'modify' ? 'replace' : 'after'),
    message: cleanText(raw?.message, operation === 'modify' ? 'The redesigned section is ready.' : 'The new section is ready.', 220),
    section: { id: baseId, title: cleanText(source.title, 'Generated section', 70), size: PM.clamp(Number(source.size) || 220, 120, 700), note: cleanText(source.note, '', 240), controls },
  };
}

function slug(value) {
  const out = String(value || '').toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '').slice(0, 48);
  return out || 'generated-section';
}

function showPreview() {
  S.phase = 'preview'; S.card.textContent = '';
  const handle = h('div.spatial-target', { title: 'Drag to move. Arrow keys also move this box.' }, `Proposed change · ${S.context.targetTitle}`);
  const body = h('div.spatial-preview', h('h3', S.plan.section.title), h('p', S.plan.message));
  if (S.plan.section.note) body.appendChild(h('p', S.plan.section.note));
  S.plan.section.controls.forEach(c => body.appendChild(h('div.spatial-preview-control', h('span', c.label), h('span', c.type), c.type === 'slider' ? h('i') : null)));
  if (!S.plan.section.controls.length) body.appendChild(h('div.spatial-preview-control', 'Empty section'));
  const actions = h('div.spatial-actions', h('span.spatial-status', S.plan.operation === 'modify' ? 'Replaces circled section' : 'Adds beside circled section'),
    h('button.spatial-action', { onclick: cancel }, 'Cancel'),
    h('button.spatial-action', { onclick: () => cardPosition(S.card, S.region), title: 'Return beside the circled region' }, 'Return to layout'),
    h('button.spatial-action.pri', { onclick: applyPlan }, 'Apply section'));
  S.card.append(handle, body, actions); makeCardMovable(S.card, handle);
  requestAnimationFrame(() => {
    const rect = S.card.getBoundingClientRect();
    moveCardTo(S.card, rect.left, rect.top);
    handle.focus();
  });
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

function applyPlan() {
  if (!S.plan || S.phase !== 'preview') return;
  const plan = S.plan;
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
  cancel();
}

function onKey(event) {
  if (!S.active) return;
  if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); cancel(); }
}

function cancel() {
  if (!S.active) return;
  S.active = false; S.phase = 'idle'; S.requestToken++;
  removeEventListener('keydown', onKey, true);
  cancelAnimationFrame(S.hintFrame); S.hintFrame = 0; S.hintPoint = null;
  if (S.renderStop) S.renderStop();
  S.root?.remove();
  Object.assign(S, { root: null, ink: null, path: null, shadePath: null, hint: null, card: null, outline: null, region: null, context: null, plan: null, renderStop: null, points: [] });
  setTimeout(refreshSceneCache, 80);
}

function startRipple(canvas, origin, sceneBitmap, adapterPromise = null) {
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
        let propagationFade = exp(-uniforms.time * 0.38);
        let front = uniforms.time * 1.32;
        let lightIn = smoothstep(0.0, 0.16, uniforms.time);

        // Broad, low-energy bands occupy more of the window without creating
        // a harsh lens or a conspicuous edge.
        let crest = exp(-pow((distanceFromSource - front) * 7.0, 2.0)) * propagationFade;
        let echo = exp(-pow((distanceFromSource - front + 0.18) * 10.5, 2.0)) * propagationFade;
        let wakeMask = smoothstep(front + 0.32, front - 0.14, distanceFromSource);
        let wake = (0.5 + 0.5 * sin(distanceFromSource * 76.0 - uniforms.time * 14.0))
          * wakeMask * exp(-distanceFromSource * 1.8);
        let core = exp(-distanceFromSource * 5.4) * exp(-uniforms.time * 0.74);
        let cursorLens = exp(-pow(distanceFromSource * 3.4, 2.0));
        let cursorRipple = sin(distanceFromSource * 38.0 - uniforms.time * 17.0)
          * cursorLens * 0.006;
        let shimmer = 0.5 + 0.5 * cos(atan2(delta.y, delta.x) * 3.0 - uniforms.time * 2.0);

        let ringAlpha = clamp((crest * 0.24 + echo * 0.08 + wake * 0.04 + core * 0.13)
          * uniforms.intensity * lightIn, 0.0, 0.42);
        let violet = vec3f(0.34, 0.18, 0.55);
        let hot = clamp(crest + core + shimmer * echo * 0.35, 0.0, 1.0);
        // Keep the defined displacement edge neutral; orange belongs only to
        // the very broad HDR haze below, never to a crisp ring.
        let ringColor = mix(violet, vec3f(0.92, 0.86, 0.82), hot * 0.24);

        // True radial displacement: the interface texture itself is sampled at
        // offset coordinates around the wave crest, with a restrained RGB split.
        let radialDirection = delta / max(distanceFromSource, 0.0001);
        let displacementStrength = (crest * 0.018 - echo * 0.006 + wake * 0.0015 + cursorRipple)
          * uniforms.intensity;
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
          * propagationFade * uniforms.intensity * lightIn;
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
