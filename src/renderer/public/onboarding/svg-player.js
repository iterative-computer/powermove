import { createEngine } from './player.js';
import { createOnboardingHdrOutput } from './hdr-output.js';

const NS = 'http://www.w3.org/2000/svg';
export const EXPECTED_DURATION = 9.766666666666667;
export const SDR_EMISSIVE_GAIN = 3;
export const WARM_WHITE_TINT = .7;

const svgNode = (name) => document.createElementNS(NS, name);
const finite = (value, fallback = 0) => Number.isFinite(Number(value)) ? Number(value) : fallback;
const clamp = (value, min, max) => Math.max(min, Math.min(max, value));

export function pathData(engine, layer, path, time) {
  const prefix = `g.${path.id}`;
  const vertices = path.vertices.map((vertex) => {
    const vertexPrefix = `${prefix}.v.${vertex.id}`;
    const read = (key) => finite(engine.evP(layer, vertex.p[key], time, `${vertexPrefix}.${key}`));
    return { x: read('x'), y: read('y'), inX: read('inX'), inY: read('inY'), outX: read('outX'), outY: read('outY') };
  });
  if (!vertices.length) return '';
  let data = `M ${vertices[0].x} ${vertices[0].y}`;
  const closed = Boolean(engine.evP(layer, path.p.closed, time, `${prefix}.closed`));
  for (let index = 0; index < vertices.length - (closed ? 0 : 1); index++) {
    const from = vertices[index], to = vertices[(index + 1) % vertices.length];
    data += ` C ${from.x + from.outX} ${from.y + from.outY} ${to.x + to.inX} ${to.y + to.inY} ${to.x} ${to.y}`;
  }
  return closed ? `${data} Z` : data;
}

export function pathMatrix(engine, layer, path, time, allPaths = layer.d.paths, seen = new Set()) {
  if (seen.has(path.id)) return [1, 0, 0, 1, 0, 0];
  seen.add(path.id);
  const prefix = `g.${path.id}`;
  const read = (key, fallback = 0) => finite(engine.evP(layer, path.p[key], time, `${prefix}.${key}`), fallback);
  const radians = read('rotation') * Math.PI / 180;
  const cosine = Math.cos(radians), sine = Math.sin(radians);
  const local = [
    cosine * read('scaleX', 100) / 100,
    sine * read('scaleX', 100) / 100,
    -sine * read('scaleY', 100) / 100,
    cosine * read('scaleY', 100) / 100,
    read('x'), read('y')
  ];
  const parent = allPaths.find((candidate) => candidate.id === path.parent);
  return parent ? engine.mul(pathMatrix(engine, layer, parent, time, allPaths, seen), local) : local;
}

export function combinedPathMatrix(engine, layer, path, time) {
  return engine.mul(engine.worldMatrix(layer, time), pathMatrix(engine, layer, path, time));
}

/** Remove the neutral-white component while preserving the color's composite over white. */
export function unmixWhite(color) {
  const raw = String(color || '#ffffff').replace('#', '');
  const value = /^[\da-f]{6}$/i.test(raw) ? raw : 'ffffff';
  const rgb = [0, 2, 4].map((offset) => Number.parseInt(value.slice(offset, offset + 2), 16) / 255);
  const white = Math.min(...rgb), opacity = 1 - white;
  const straight = opacity <= 1e-6 ? [0, 0, 0] : rgb.map((channel) => (channel - white) / opacity);
  return {
    color: `#${straight.map((channel) => Math.round(clamp(channel, 0, 1) * 255).toString(16).padStart(2, '0')).join('').toUpperCase()}`,
    opacity
  };
}

export function tintVisibleGlow(color, amount = WARM_WHITE_TINT) {
  const raw = String(color || '#000000').replace('#', '');
  const value = /^[\da-f]{6}$/i.test(raw) ? raw : '000000';
  const tint = clamp(finite(amount), 0, 1);
  return `#${[0, 2, 4].map((offset) => {
    const channel = Number.parseInt(value.slice(offset, offset + 2), 16);
    return Math.round(channel + (255 - channel) * tint).toString(16).padStart(2, '0');
  }).join('').toUpperCase()}`;
}

export function gradientState(engine, layer, effect, time) {
  const parameter = (key, fallback) => effect?.p[key]
    ? engine.evP(layer, effect.p[key], time, key)
    : fallback;
  const count = clamp(Math.round(finite(parameter('gradientPointCount', 3), 3)), 2, 8);
  const colors = ['gradientStart', 'gradientMiddle', 'gradientEnd', 'gradientPoint4', 'gradientPoint5', 'gradientPoint6', 'gradientPoint7', 'gradientPoint8'];
  const positions = ['gradientPoint1Position', 'gradientMiddlePosition', 'gradientPoint3Position', 'gradientPoint4Position', 'gradientPoint5Position', 'gradientPoint6Position', 'gradientPoint7Position', 'gradientPoint8Position'];
  const stops = Array.from({ length: count }, (_, index) => {
    const unmixed = unmixWhite(parameter(colors[index], '#ffffff'));
    return {
      ...unmixed,
      color: unmixed.opacity > 1e-6 ? tintVisibleGlow(unmixed.color) : unmixed.color,
      offset: clamp(finite(parameter(positions[index], index / Math.max(1, count - 1) * 100)) / 100, 0, 1)
    };
  }).sort((left, right) => left.offset - right.offset);
  // A fully transparent stop keeps its nearest colored hue to avoid a dark fringe.
  stops.forEach((stop, index) => {
    if (stop.opacity > 1e-6) return;
    const neighbor = stops.slice().sort((a, b) => Math.abs(a.offset - stop.offset) - Math.abs(b.offset - stop.offset))
      .find((candidate) => candidate.opacity > 1e-6);
    if (neighbor) stop.color = neighbor.color;
  });
  const angle = finite(parameter('gradientAngle', 0));
  const offset = finite(parameter('gradientOffset', 0)) / 100;
  const repeat = Boolean(parameter('gradientRepeat', false));
  const scale = repeat ? Math.max(.01, finite(parameter('gradientRepeatSize', 100), 100) / 100) : 1;
  const radians = angle * Math.PI / 180, x = Math.cos(radians), y = Math.sin(radians);
  const x1 = .5 - (.5 + offset) * x, y1 = .5 - (.5 + offset) * y;
  const intensity = Math.max(0, finite(parameter('intensity', 0)) / 100);
  const pulseAmount = clamp(finite(parameter('pulseAmount', 0)) / 100, 0, 1);
  const pulse = 1 - pulseAmount * .5 + pulseAmount * .5
    * Math.sin(2 * Math.PI * time * finite(parameter('pulseSpeed', 1), 1));
  return {
    stops, x1, y1, x2: x1 + x * scale, y2: y1 + y * scale,
    spread: repeat ? 'reflect' : 'pad',
    radius: Math.max(.5, finite(parameter('radius', 0))),
    strength: Math.max(0, intensity * pulse)
  };
}

const smoothstep = (edge0, edge1, value) => {
  const x = clamp((value - edge0) / (edge1 - edge0), 0, 1);
  return x * x * (3 - 2 * x);
};

export function sourceGate(engine, layer, path, effect, time) {
  const prefix = `g.${path.id}`;
  if (!engine.evP(layer, path.p.fillEnabled, time, `${prefix}.fillEnabled`)) return 0;
  const fill = String(engine.evP(layer, path.p.fill, time, `${prefix}.fill`) || '#ffffff').replace('#', '');
  const rgb = /^[\da-f]{6}$/i.test(fill)
    ? [0, 2, 4].map((offset) => Number.parseInt(fill.slice(offset, offset + 2), 16) / 255)
    : [1, 1, 1];
  const brightness = rgb[0] * .2126 + rgb[1] * .7152 + rgb[2] * .0722;
  const threshold = finite(effect?.p.threshold
    ? engine.evP(layer, effect.p.threshold, time, 'threshold')
    : 55) / 100;
  const fillOpacity = clamp(finite(engine.evP(layer, path.p.fillOpacity, time, `${prefix}.fillOpacity`), 100) / 100, 0, 1);
  return fillOpacity * smoothstep(threshold - .08, threshold + .08, brightness);
}

function makeLayerNodes(svg, layer, index, width, height) {
  const defs = svg.querySelector('defs');
  const gradient = svgNode('linearGradient'), gradientId = `onboarding-gradient-${index}`;
  gradient.id = gradientId; gradient.setAttribute('gradientUnits', 'objectBoundingBox');
  const filter = svgNode('filter'), filterId = `onboarding-blur-${index}`;
  filter.id = filterId;
  for (const [key, value] of Object.entries({ filterUnits: 'userSpaceOnUse', primitiveUnits: 'userSpaceOnUse', x: '0', y: '0', width: String(width), height: String(height) })) filter.setAttribute(key, value);
  const blur = svgNode('feGaussianBlur');
  const transfer = svgNode('feComponentTransfer'), alphaSlope = svgNode('feFuncA');
  alphaSlope.setAttribute('type', 'linear'); alphaSlope.setAttribute('slope', '1'); alphaSlope.setAttribute('intercept', '0');
  transfer.append(alphaSlope); filter.append(blur, transfer);
  const bloomMask = svgNode('mask'), bloomMaskId = `onboarding-bloom-${index}`;
  bloomMask.id = bloomMaskId; bloomMask.setAttribute('maskUnits', 'userSpaceOnUse'); bloomMask.setAttribute('mask-type', 'alpha');
  const blurGroup = svgNode('g'); blurGroup.setAttribute('filter', `url(#${filterId})`);
  const glowPath = svgNode('path'); glowPath.setAttribute('fill', '#ffffff');
  blurGroup.append(glowPath); bloomMask.append(blurGroup);
  const hollowMask = svgNode('mask'), hollowMaskId = `onboarding-hollow-${index}`;
  hollowMask.id = hollowMaskId; hollowMask.setAttribute('maskUnits', 'userSpaceOnUse'); hollowMask.setAttribute('mask-type', 'luminance');
  const maskBase = svgNode('rect');
  maskBase.setAttribute('width', String(width)); maskBase.setAttribute('height', String(height)); maskBase.setAttribute('fill', '#ffffff');
  const hollowPath = svgNode('path'); hollowPath.setAttribute('fill', '#000000');
  hollowMask.append(maskBase, hollowPath); defs.append(gradient, filter, bloomMask, hollowMask);
  const output = svgNode('g'); output.dataset.layerId = layer.id; output.setAttribute('mask', `url(#${hollowMaskId})`);
  const rect = svgNode('rect');
  rect.setAttribute('width', String(width)); rect.setAttribute('height', String(height));
  rect.setAttribute('fill', `url(#${gradientId})`); rect.setAttribute('mask', `url(#${bloomMaskId})`);
  output.append(rect); svg.append(output);
  return { layer, output, rect, gradient, blur, alphaSlope, glowPath, hollowPath };
}

function setStops(gradient, stops) {
  while (gradient.children.length < stops.length) gradient.append(svgNode('stop'));
  while (gradient.children.length > stops.length) gradient.lastElementChild.remove();
  stops.forEach((state, index) => {
    const stop = gradient.children[index];
    stop.setAttribute('offset', String(state.offset));
    stop.setAttribute('stop-color', state.color);
    stop.dataset.hdrOpacity = String(state.opacity);
    stop.setAttribute('stop-opacity', String(clamp(state.opacity * SDR_EMISSIVE_GAIN, 0, 1)));
  });
}

async function readyAudio(url) {
  const audio = new Audio();
  audio.preload = 'auto'; audio.src = url.href;
  await new Promise((resolve, reject) => {
    const cleanup = () => { audio.removeEventListener('canplaythrough', done); audio.removeEventListener('error', failed); };
    const done = () => { cleanup(); resolve(); };
    const failed = () => { cleanup(); reject(new Error('Could not load onboarding sound effect')); };
    audio.addEventListener('canplaythrough', done, { once: true });
    audio.addEventListener('error', failed, { once: true });
    if (audio.readyState >= 3) done(); else audio.load();
  });
  return audio;
}

export async function createOnboardingSvgPlayer(options) {
  const { svg, scene } = options;
  if (!(svg instanceof SVGSVGElement)) throw new Error('An SVG element is required');
  if (Math.abs(scene.project?.dur - EXPECTED_DURATION) > 1e-9 || scene.project?.fps !== 30) throw new Error('The supplied onboarding timing has changed');
  const project = structuredClone(scene.project), engine = createEngine(project);
  const shapeLayers = project.layers.filter((layer) => layer.type === 'shape');
  const audioLayer = project.layers.find((layer) => layer.type === 'audio');
  const effectDefinition = scene.effects.find((effect) => effect.id === 'glow');
  if (!effectDefinition || !shapeLayers.length) throw new Error('The supplied onboarding vectors are unavailable');
  svg.setAttribute('viewBox', `0 0 ${project.w} ${project.h}`); svg.setAttribute('preserveAspectRatio', 'xMidYMid meet');
  svg.dataset.sdrGain = String(SDR_EMISSIVE_GAIN);
  svg.replaceChildren(svgNode('defs'));
  const nodes = shapeLayers.map((layer, index) => makeLayerNodes(svg, layer, index, project.w, project.h));
  const base = new URL(options.baseURL || document.baseURI, document.baseURI);
  const audio = options.audio === false || !audioLayer ? null : await readyAudio(new URL(scene.assets[audioLayer.d.asset], base));
  let dead = false, playing = false, frame = 0, time = 0, startTime = 0, origin = 0, hdrOutput = null;

  const render = (nextTime) => {
    time = nextTime; engine.time = nextTime; engine.beginEval?.(nextTime);
    for (const node of nodes) {
      const active = engine.active(node.layer, nextTime);
      node.output.setAttribute('display', active ? 'inline' : 'none');
      if (!active) continue;
      const path = node.layer.d.paths[0];
      const data = pathData(engine, node.layer, path, nextTime);
      const transform = `matrix(${combinedPathMatrix(engine, node.layer, path, nextTime).join(' ')})`;
      node.glowPath.setAttribute('d', data); node.glowPath.setAttribute('transform', transform);
      node.hollowPath.setAttribute('d', data); node.hollowPath.setAttribute('transform', transform);
      const effect = node.layer.fx.find((candidate) => candidate.type === effectDefinition.id);
      const state = gradientState(engine, node.layer, effect, nextTime);
      for (const [key, value] of Object.entries({ x1: state.x1, y1: state.y1, x2: state.x2, y2: state.y2 })) node.gradient.setAttribute(key, String(value));
      node.gradient.setAttribute('spreadMethod', state.spread); setStops(node.gradient, state.stops);
      node.blur.setAttribute('stdDeviation', String(state.radius * .5));
      node.alphaSlope.setAttribute('slope', String(state.strength));
      node.glowPath.setAttribute('opacity', String(sourceGate(engine, node.layer, path, effect, nextTime)));
      node.rect.setAttribute('opacity', String(engine.worldOpacity(node.layer, nextTime)));
    }
    void hdrOutput?.present();
  };
  const pauseAudio = () => audio?.pause();
  const tick = (now) => {
    if (!playing || dead) return;
    const next = origin + (now - startTime) / 1000;
    if (next >= project.dur) {
      playing = false; render(project.dur - 1 / project.fps); pauseAudio(); options.onComplete?.(); return;
    }
    try { render(next); frame = requestAnimationFrame(tick); }
    catch (error) { playing = false; pauseAudio(); options.onError?.(error instanceof Error ? error : new Error(String(error))); }
  };
  const player = {
    renderer: 'svg', element: svg, engine, get hdrOutput() { return hdrOutput; },
    get duration() { return project.dur; }, get currentTime() { return time; }, get playing() { return playing; },
    play() {
      if (dead || playing) return;
      playing = true; startTime = performance.now(); origin = time;
      if (audio && audioLayer && time < audioLayer.from + audioLayer.dur) {
        audio.currentTime = Math.max(0, time - audioLayer.from + finite(audioLayer.d.trim));
        audio.volume = clamp(finite(audioLayer.d.gain, 1), 0, 1);
        void audio.play().catch((error) => options.onError?.(error instanceof Error ? error : new Error(String(error))));
      }
      frame = requestAnimationFrame(tick);
    },
    pause() { playing = false; cancelAnimationFrame(frame); pauseAudio(); },
    async seek(value) {
      if (dead || !Number.isFinite(value)) throw new Error('Seek time must be finite');
      origin = clamp(value, 0, project.dur - 1 / project.fps); startTime = performance.now(); render(origin);
      await hdrOutput?.present();
      if (audio && audioLayer) {
        audio.pause();
        const audioEnd = Number.isFinite(audio.duration) ? audio.duration : audioLayer.dur;
        audio.currentTime = clamp(origin - audioLayer.from + finite(audioLayer.d.trim), 0, audioEnd);
      }
    },
    destroy() {
      if (dead) return;
      dead = true; playing = false; cancelAnimationFrame(frame);
      if (audio) { audio.pause(); audio.removeAttribute('src'); audio.load(); }
      hdrOutput?.destroy(); hdrOutput = null; svg.replaceChildren(); engine.bus.clear();
    }
  };
  render(0);
  if (options.hdr !== false) {
    hdrOutput = await createOnboardingHdrOutput(svg, (error) => console.warn('[onboarding] HDR fallback', error));
    await hdrOutput?.present();
  }
  if (options.autoplay) player.play(); return player;
}
