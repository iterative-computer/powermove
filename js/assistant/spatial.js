/* Powermove — spatial coding assistant.
   Shake the pointer to summon a full-window shader, drag over interface, describe
   the change, preview the generated section manifest, then apply it safely. */
(() => {
const PM = window.PM, h = PM.h;

/* The native Codex client owns ChatGPT authentication. This bridge only moves a
   prompt and a strict JSON schema across WKWebView; no account secret enters JS. */
const pending = new Map();
PM.CodexBridge = {
  request(prompt, schema, images = []) {
    const bridge = window.webkit?.messageHandlers?.pmCodex;
    if (!bridge) return Promise.reject(new Error('The coding agent is available in the Powermove macOS app'));
    const id = PM.uid('spatial-codex-');
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        pending.delete(id);
        reject(new Error('The coding agent took too long to respond'));
      }, 120000);
      pending.set(id, { resolve, reject, timer });
      bridge.postMessage({ id, prompt, schema, images: images.slice(0, 6) });
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
  requestText: '', run: null, conversation: [], activity: '',
  rippleWarmup: null, sceneCache: null, sceneCacheAt: 0, cachePending: null,
  sceneFrame: null, regionImage: null,
  hintFrame: 0, hintPoint: null,
};

const Spatial = {
  init,
  activate,
  open: () => activate(Math.round(innerWidth / 2), Math.round(innerHeight / 2), {
    claimed: true, capture: PM.WindowCapture.request(), adapter: requestRippleAdapter(),
  }),
  cancel,
  get active() { return S.active; },
  /* Small pure seams are exposed for deterministic regression tests. */
  math: { motionProfile, shakeReady, shakeIntent, selectionRect, bitmapCropRect, isClickGesture, overlayPointerAction, pointInPolygon, sanitizePlan, applyChromeEdit, hintPosition, clampFloatingPosition },
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
  const input = h('textarea', {
    placeholder: S.context.targetPanelId ? 'How should this area change?' : 'Describe a change…',
    rows: '3',
  });
  input.value = draft;
  const status = h('span.spatial-status', 'Enter to send');
  const cancelBtn = h('button.spatial-action', { onclick: cancel }, 'Cancel');
  const sendBtn = h('button.spatial-action.pri', { 'aria-label': 'Enter to send', onclick: () => sendRequest(input) }, '↵');
  input.addEventListener('keydown', e => {
    e.stopPropagation();
    if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); sendRequest(input); }
  });
  const handle = h('div.spatial-target', S.context.targetPanelId ? `Selected · ${S.context.targetTitle}` : 'Powermove agent · full composition');
  const contextLabel = h('span.spatial-context-label', S.context.targetPanelId ? `Selected ${S.context.targetTitle}` : 'Full composition');
  S.card = h('div.spatial-compose.compact.full',
    handle,
    h('div.spatial-input-row', contextLabel, input, sendBtn),
    h('div.spatial-actions', status, cancelBtn));
  S.root.appendChild(S.card);
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
  if (plan.kind === 'workspace') return plan.message || `I prepared the ${plan.workspaceEdit.name} workspace.`;
  if (plan.kind === 'chrome') return plan.message || 'I prepared an interface change.';
  return plan.message || `I prepared the ${plan.section.title} section.`;
}

function promoteToConversation() {
  if (!S.root || !S.card) return;
  const rect = S.card.getBoundingClientRect();
  S.root.classList.add('conversation-mode');
  S.root.classList.remove('target-selected');
  S.hint.style.display = 'none';
  S.path?.setAttribute('d', ''); S.shadePath?.setAttribute('d', '');
  S.outline?.remove(); S.outline = null;
  if (S.renderStop) { S.renderStop(); S.renderStop = null; }
  Object.assign(S.card.style, { left: Math.round(rect.left) + 'px', top: Math.round(rect.top) + 'px' });
}

function planPreview() {
  const plan = S.plan;
  if (!plan) return null;
  const chrome = plan.kind === 'chrome';
  const scene = plan.kind === 'scene';
  const workspace = plan.kind === 'workspace';
  const title = chrome ? 'Interface edit' : scene ? plan.sceneEdit.label : workspace ? plan.workspaceEdit.name : plan.section.title;
  const preview = h('div.spatial-proposal',
    h('div.spatial-proposal-kicker', 'Proposed change'),
    h('h3', title));
  if (chrome) preview.appendChild(h('div.spatial-preview-control', h('span', 'Corner style'), h('span', plan.chromeEdit.value)));
  else if (scene) plan.sceneEdit.commands.forEach(command => preview.appendChild(h('div.spatial-preview-control', h('span', PM.AgentHarness.describeCommand(command)))));
  else if (workspace) {
    plan.workspaceEdit.docks.forEach(dock => preview.appendChild(h('div.spatial-preview-control', h('span', dock.id), h('span', dock.panels.map(panel => panel.id).join(' · ')))));
    plan.workspaceEdit.sections.forEach(section => preview.appendChild(h('div.spatial-preview-control', h('span', section.title), h('span', `${section.controls.length} connected controls`))));
  } else {
    plan.section.controls.forEach(control => preview.appendChild(h('div.spatial-preview-control', h('span', control.label), h('span', control.connection || control.type), control.type === 'slider' ? h('i') : null)));
  }
  preview.appendChild(h('div.spatial-proposal-actions',
    h('button.spatial-action', { onclick: () => { S.plan = null; renderConversation(true); } }, 'Dismiss'),
    h('button.spatial-action.pri', { onclick: applyPlan }, chrome ? 'Apply interface edit' : scene ? 'Apply scene edit' : workspace ? 'Create workspace' : 'Apply section')));
  return preview;
}

function resultPreview() {
  const run = S.run;
  if (!run || S.phase !== 'result') return null;
  const preview = h('div.spatial-proposal', h('div.spatial-proposal-kicker', 'Rendered result'), h('h3', 'Review the actual result'));
  const message = run.review?.message || 'The rendered change is ready.';
  preview.appendChild(h('p', message));
  if (run.review?.critique) preview.appendChild(h('p', run.review.critique));
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

function renderConversation(focusInput = false) {
  if (!S.card) return;
  /* CSS pop-out animation uses transform. Read the committed left/top values so
     progress updates cannot make the floating conversation drift mid-animation. */
  const rect = cardCoordinates(S.card);
  S.card.className = 'spatial-compose conversation';
  S.card.textContent = '';
  const handle = h('div.spatial-target', { title: 'Drag to move. Arrow keys also move this conversation.' },
    `Powermove agent · ${S.context?.targetPanelId ? S.context.targetTitle : 'full composition'}`);
  const close = h('button.spatial-close', { type: 'button', 'aria-label': 'Close agent conversation', title: 'Close', onclick: cancel }, '×');
  const messages = h('div.spatial-conversation-log', { role: 'log', 'aria-live': 'polite' });
  S.conversation.slice(-30).forEach(message => messages.appendChild(h(`div.spatial-message.${message.role}`, message.text)));
  if (S.activity) messages.appendChild(h('div.spatial-message.assistant.pending', h('i'), S.activity));
  const proposal = resultPreview() || planPreview();
  if (proposal) messages.appendChild(proposal);
  const input = h('textarea.spatial-followup', {
    rows: '2', placeholder: 'Reply or ask for an adjustment…', 'aria-label': 'Reply to Powermove agent',
    disabled: S.phase === 'working' || S.phase === 'applying',
  });
  const send = h('button.spatial-action.pri', {
    type: 'button', 'aria-label': 'Send reply', disabled: input.disabled,
    onclick: () => sendRequest(input),
  }, '↑');
  input.addEventListener('keydown', event => {
    event.stopPropagation();
    if (event.key === 'Enter' && !event.shiftKey) { event.preventDefault(); sendRequest(input); }
  });
  S.card.append(h('div.spatial-conversation-head', handle, close), messages, h('div.spatial-followup-row', input, send));
  makeCardMovable(S.card, handle);
  moveCardTo(S.card, rect.left, rect.top);
  requestAnimationFrame(() => {
    messages.scrollTop = messages.scrollHeight;
    if (focusInput && !input.disabled) input.focus();
  });
}

async function sendRequest(input) {
  const request = input.value.trim();
  if (!request || S.phase === 'working' || S.phase === 'applying') return;
  S.requestText = request;
  S.conversation.push({ role: 'user', text: request });
  S.plan = null; S.phase = 'working'; S.activity = 'Looking at the composition…';
  promoteToConversation(); renderConversation();
  const token = ++S.requestToken;
  try {
    const observation = PM.AgentHarness ? await PM.AgentHarness.observe() : { state: {}, times: [], images: [] };
    if (!S.active || token !== S.requestToken) return;
    S.activity = 'Designing a safe change…'; renderConversation();
    const attachedImages = S.regionImage ? [S.regionImage, ...observation.images] : observation.images;
    const raw = await PM.CodexBridge.request(agentPrompt(request, observation), responseSchema(), attachedImages);
    if (!S.active || token !== S.requestToken) return;
    let decoded;
    try { decoded = JSON.parse(raw); } catch { throw new Error('The coding agent returned an invalid section'); }
    const plan = sanitizePlan(decoded, S.context, request);
    if (plan.operation === 'noop') throw new Error(plan.message || 'No safe interface change was generated');
    if (plan.kind === 'scene' && !plan.sceneEdit.commands.length) throw new Error(plan.message || 'No safe composition edit was generated');
    if (plan.kind === 'section' && !plan.section.controls.length) throw new Error('The generated section had no controls connected to editable source');
    if (plan.kind === 'workspace' && !plan.workspaceEdit) throw new Error('The generated workspace was not safe or complete enough to preview');
    S.conversation.push({ role: 'assistant', text: conversationReply(plan) });
    S.activity = ''; S.plan = plan; showPreview();
  } catch (error) {
    if (!S.active || token !== S.requestToken) return;
    S.activity = ''; S.phase = 'conversation';
    S.conversation.push({ role: 'assistant', text: String(error.message || error).slice(0, 220) });
    renderConversation(true);
  }
}

function responseSchema() {
  return {
    type: 'object', additionalProperties: false,
    required: ['kind', 'operation', 'targetPanelId', 'dockId', 'placement', 'message', 'chromeEdit', 'section', 'sceneEdit', 'workspaceEdit'],
    properties: {
      kind: { type: 'string', enum: ['section', 'chrome', 'scene', 'workspace'] },
      operation: { type: 'string', enum: ['create', 'modify', 'noop'] },
      targetPanelId: { type: 'string' }, dockId: { type: 'string' },
      placement: { type: 'string', enum: ['before', 'after', 'replace'] },
      message: { type: 'string' },
      chromeEdit: {
        type: 'object', additionalProperties: false, required: ['target', 'value'],
        properties: {
          target: { type: 'string', enum: ['preview.cornerRadius'] },
          value: { type: 'string', enum: ['square', 'rounded'] },
        },
      },
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
      sceneEdit: PM.AgentHarness.sceneSchema(),
      workspaceEdit: { type: 'string' },
    },
  };
}

function agentPrompt(request, observation) {
  const workspace = PM.WS.current;
  const commands = Object.values(PM.commands || {}).map(c => ({ id: c.id, label: c.label })).slice(0, 80);
  const sourcePaths = ['properties.position.x', 'properties.position.y', 'properties.scale.x', 'properties.scale.y', 'properties.rotation', 'properties.opacity', 'content.text', 'content.color', 'content.size', 'layer.visible', 'layer.locked', 'layer.duration', 'layer.motionBlur', 'composition.background.type', 'composition.background.startColor', 'composition.background.endColor', 'composition.background.angle', 'composition.background.midpoint'];
  return `You are the bounded visual editing agent inside Powermove. Decide whether the request changes the rendered composition or the app interface, then return one structured proposal. Return only the requested JSON object.

RULES
- kind=scene for changes to layers, content, motion, timing, effects, or composition settings. Each sceneEdit.commands item must be one JSON-encoded source-edit object using only availableOperations. Use stable explicit ids for new layers that later commands target. Never output JavaScript, shell commands, or whole-project JSON.
- For scene requests, inspect the live source and attached rendered frames. Preserve locked layers, hand-edited channels, and unrelated work. Set reviewTimes to the most revealing moments. Return neutral section and chromeEdit fields.
- kind=workspace when the user asks for a complete workspace, layout, editing environment, or a coordinated group of panels. workspaceEdit must be one JSON-encoded manifest shaped like {"name":"...","density":"compact|normal|comfy","accent":"#RRGGBB","docks":[{"id":"left|center|right","size":number,"flex":boolean,"panels":[{"id":"viewer|timeline|inspector|assets|fxbrowser|takes|notes|CUSTOM_ID","size":number,"flex":boolean}]}],"sections":[SECTION_OBJECTS]}. Include viewer, keep all panels reachable, and make every generated section control source-connected under the same rules below.
- For non-workspace requests return workspaceEdit="{}". For non-scene requests return an empty neutral sceneEdit. For non-section requests return a neutral empty section.
- kind=chrome for a supported app-interface style change. The supported editable chrome target is preview.cornerRadius with value square or rounded. Use operation=modify, and return a neutral empty section object.
- kind=section for editable panels/controls. Return a neutral chromeEdit of {"target":"preview.cornerRadius","value":"square"}.
- operation=create when adding a section; operation=modify when replacing the selected section; noop only when the request cannot be represented safely.
- A section is a compact native Powermove panel made from slider, color, toggle, select, and button controls.
- Every non-button control must bind to real editable source. Use target="$selection" for the selected layer, an exact layer id from LIVE COMPOSITION SOURCE, or target="$composition" for composition.background.* paths.
- Supported control binding paths are: ${sourcePaths.join(', ')}. For exact layers, other primitive content.* fields and real properties.* channels shown in live source are also valid.
- Do not create decorative or disconnected scene parameters. If no compatible source exists, return operation=noop and explain what must be selected or created.
- Buttons may use only one of the listed command ids. Never invent commands.
- Prefer 3–8 focused controls, a short title, and a useful one-sentence note. Avoid decorative filler.
- For unused control fields, still return schema-safe neutral values: empty string/array, 0, or false.
- App chrome is a valid editable source target when it is in the whitelist above. Never reject preview corner styling merely because it is interface chrome.
- Keep the current interface reachable. Do not remove unrelated docks or panels. Never alter rendered composition shapes or export geometry for a chrome request.

VISUAL REFERENCES
${S.regionImage ? '- The FIRST attached image is an exact screenshot of the selected editor region before the Ripple overlay appeared. Treat its geometry and visible controls as the primary visual target.\n- Remaining attached images are rendered composition frames at the times listed below.' : '- No editor region was selected. Attached images are rendered composition frames.'}

SELECTED REGION SEMANTICS
${JSON.stringify(S.context)}

CONVERSATION SO FAR
${JSON.stringify(S.conversation.slice(0, -1).slice(-12))}

SEMANTIC WORKSPACE MAP
${JSON.stringify(workspaceSemanticContext(workspace))}

CURRENT WORKSPACE MANIFEST
${JSON.stringify(workspace)}

AVAILABLE COMMANDS
${JSON.stringify(commands)}

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

function controlConnection(target, path, controlType) {
  const compositionPaths = new Set(['composition.background.type', 'composition.background.startColor', 'composition.background.endColor', 'composition.background.angle', 'composition.background.midpoint']);
  if ((target === '$composition' || target === 'composition') && compositionPaths.has(path)) {
    const expected = path.endsWith('.type') ? 'select' : path.endsWith('Color') ? 'color' : 'slider';
    if (controlType !== expected) return null;
    return { target: '$composition', path, connection: 'Composition background' };
  }
  const layer = target === '$selection' || target === 'selection' ? PM.firstSel() : (PM.L(target) || PM.byName(target));
  if (!layer) return null;
  if (path.startsWith('properties.')) {
    const channel = path.slice('properties.'.length);
    if (!PM.findProp(layer, channel) || controlType !== 'slider') return null;
  } else if (path.startsWith('content.')) {
    const key = path.slice('content.'.length);
    const current = layer.d?.[key];
    if (!['string', 'number', 'boolean'].includes(typeof current)) return null;
    const expected = typeof current === 'number' ? 'slider' : typeof current === 'boolean' ? 'toggle'
      : /^#[0-9a-f]{6}$/i.test(current) ? 'color' : 'select';
    if (controlType !== expected) return null;
  } else if (path.startsWith('layer.')) {
    const key = path.slice('layer.'.length);
    const expected = ['visible', 'locked', 'motionBlur'].includes(key) ? 'toggle' : key === 'duration' ? 'slider' : key === 'blend' ? 'select' : '';
    if (!expected || controlType !== expected) return null;
  } else return null;
  return { target: target === '$selection' || target === 'selection' ? '$selection' : layer.id, path, connection: `Layer · ${layer.name}` };
}

function sanitizePlan(raw, context, request = '') {
  const operation = ['create', 'modify', 'noop'].includes(raw?.operation) ? raw.operation : 'noop';
  const chromeTarget = raw?.chromeEdit?.target === 'preview.cornerRadius' ? 'preview.cornerRadius' : '';
  const chromeValue = ['square', 'rounded'].includes(raw?.chromeEdit?.value) ? raw.chromeEdit.value : '';
  const requestedKind = raw?.kind;
  const kind = requestedKind === 'scene' ? 'scene'
    : requestedKind === 'workspace' ? 'workspace'
    : requestedKind === 'chrome' && chromeTarget && chromeValue ? 'chrome' : 'section';
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
    const connection = controlConnection(target, path, type);
    if (!connection) return null;
    Object.assign(out, connection);
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
  }).filter(c => c && (c.type !== 'button' || c.cmd));
  return {
    kind,
    operation,
    targetPanelId: cleanText(raw?.targetPanelId, context?.targetPanelId || '', 100),
    dockId: cleanText(raw?.dockId, '', 100),
    placement: ['before', 'after', 'replace'].includes(raw?.placement) ? raw.placement : (operation === 'modify' ? 'replace' : 'after'),
    message: cleanText(raw?.message, operation === 'modify' ? 'The redesigned section is ready.' : 'The new section is ready.', 220),
    chromeEdit: kind === 'chrome' ? { target: chromeTarget, value: chromeValue } : null,
    section: { id: baseId, title: cleanText(source.title, 'Generated section', 70), size: PM.clamp(Number(source.size) || 220, 120, 700), note: cleanText(source.note, '', 240), controls },
    sceneEdit: kind === 'scene' ? PM.AgentHarness.sanitizeProposal(raw?.sceneEdit, request) : null,
    workspaceEdit: kind === 'workspace' ? sanitizeWorkspaceEdit(raw?.workspaceEdit, context) : null,
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
  if (!workspace || edit?.target !== 'preview.cornerRadius' || !['square', 'rounded'].includes(edit.value)) return false;
  workspace.chrome = workspace.chrome && typeof workspace.chrome === 'object' ? workspace.chrome : {};
  workspace.chrome.previewCornerRadius = edit.value;
  return true;
}

function slug(value) {
  const out = String(value || '').toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '').slice(0, 48);
  return out || 'generated-section';
}

function showPreview() {
  S.phase = 'conversation';
  renderConversation(true);
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

async function applyPlan() {
  if (!S.plan || !['conversation', 'preview'].includes(S.phase)) return;
  const plan = S.plan;
  if (plan.kind === 'scene') {
    await applyScenePlan(plan);
    return;
  }
  if (plan.kind === 'workspace') {
    const manifest = plan.workspaceEdit;
    const created = PM.WS.create({
      name: manifest.name, base: PM.WS.current.id, density: manifest.density,
      theme: { ...(PM.WS.current.theme || {}), accent: manifest.accent },
      custom: manifest.sections,
      layout: { docks: manifest.docks },
    });
    PM.toast(`Created workspace · ${created.name}`);
    S.plan = null; S.conversation.push({ role: 'assistant', text: `Created ${created.name}. You can keep asking me to adjust it.` });
    S.phase = 'conversation'; renderConversation(true);
    return;
  }
  if (plan.kind === 'chrome') {
    let changed = false;
    PM.WS.mutate(workspace => { changed = applyChromeEdit(workspace, plan.chromeEdit); });
    if (changed) PM.toast('Updated preview corner style');
    S.plan = null; S.conversation.push({ role: 'assistant', text: changed ? 'Applied the interface edit.' : 'That interface setting was already in place.' });
    S.phase = 'conversation'; renderConversation(true);
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
  S.plan = null; S.conversation.push({ role: 'assistant', text: `${replacing ? 'Redesigned' : 'Added'} ${plan.section.title}. You can keep refining it here.` });
  S.phase = 'conversation'; renderConversation(true);
}

async function applyScenePlan(plan) {
  S.phase = 'applying'; S.plan = null; S.activity = 'Applying structured source edit…';
  renderConversation();
  try {
    const run = await PM.AgentHarness.execute(S.requestText, plan.sceneEdit, value => {
      S.activity = value; renderConversation();
    });
    if (!S.active) return;
    S.activity = ''; S.run = run;
    showSceneResult(run);
  } catch (error) {
    if (!S.active) return;
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
  S.conversation.push({ role: 'assistant', text: `Kept ${S.run.applied.length} editable source changes. What should we adjust next?` });
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

function cancel() {
  if (!S.active) return;
  S.active = false; S.phase = 'idle'; S.requestToken++;
  removeEventListener('keydown', onKey, true);
  cancelAnimationFrame(S.hintFrame); S.hintFrame = 0; S.hintPoint = null;
  if (S.renderStop) S.renderStop();
  S.root?.remove();
  Object.assign(S, { root: null, ink: null, path: null, shadePath: null, hint: null, card: null, outline: null, region: null, context: null, plan: null, run: null, requestText: '', renderStop: null, points: [], conversation: [], activity: '', sceneFrame: null, regionImage: null });
  setTimeout(refreshSceneCache, 80);
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
