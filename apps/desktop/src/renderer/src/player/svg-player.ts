import { createEngine } from './player';
import { validateScene, type WebScene } from './scene';
import { inspectSvgExport } from './svg-compatibility';
import { fontAnchorOffset } from '../legacy/core/font-anchor';

const NS = 'http://www.w3.org/2000/svg';
export interface SvgPlayerOptions {
  svg: SVGSVGElement;
  scene: WebScene;
  baseURL?: string | URL;
  autoplay?: boolean;
  loop?: boolean;
  transparent?: boolean;
  onError?: (error: Error) => void;
}

/** SVG is only a drawing backend. All animation and hierarchy evaluation is shared. */
export async function createSvgPlayer(options: SvgPlayerOptions) {
  validateScene(options.scene);
  const scene = structuredClone(options.scene);
  const compatibility = inspectSvgExport(scene.project);
  if (!compatibility.supported) throw new Error(compatibility.reasons.join('\n'));
  const PM = createEngine(scene.project), svg = options.svg;
  const base = new URL(options.baseURL || document.baseURI, document.baseURI);
  const faces: FontFace[] = [];
  const group = document.createElementNS(NS, 'g');
  let dead = false, playing = false, loop = options.loop ?? true, frame = 0, time = 0, start = 0, origin = 0;
  const assertAlive = () => { if (dead) throw new Error('Player has been destroyed'); };
  const dispose = () => { if (dead) return; dead = true; playing = false; cancelAnimationFrame(frame); group.remove(); faces.forEach(face => document.fonts.delete(face)); PM.bus.clear(); };
  try {
    for (const font of scene.fonts) {
      const response = await fetch(new URL(font.src, base));
      if (!response.ok) throw new Error(`Could not load font ${font.family}`);
      const face = new FontFace(font.family, await response.arrayBuffer(), { weight: font.weight, style: font.style, ...(font.unicodeRange ? { unicodeRange: font.unicodeRange } : {}) });
      await face.load(); document.fonts.add(face); faces.push(face);
    }
    const families = [...new Set(scene.project.layers.filter((l: any) => l.type === 'text').map((l: any) => PM.resolveContent(l, 0).font))];
    await Promise.all(families.map(family => document.fonts.load(`400 16px ${JSON.stringify(family)}`)));
    svg.setAttribute('viewBox', `0 0 ${scene.project.w} ${scene.project.h}`);
    svg.append(group);
    if (!options.transparent) {
      const bg = document.createElementNS(NS, 'rect');
      bg.setAttribute('width', String(scene.project.w)); bg.setAttribute('height', String(scene.project.h));
      bg.setAttribute('fill', scene.project.bg || '#000000'); group.append(bg);
    }
    const nodes = [...scene.project.layers].reverse().filter((l: any) => l.type !== 'group').map((layer: any) => {
      const outer = document.createElementNS(NS, 'g'); outer.dataset.layerId = layer.id;
      const node = document.createElementNS(NS, layer.type === 'text' ? 'text' : layer.d.shape === 'ellipse' ? 'ellipse' : 'rect');
      outer.append(node); group.append(outer); return { layer, outer, node };
    });
    // Measurement only: no rasterized glyphs/textures are used by SVG output.
    const measure = document.createElement('canvas').getContext('2d')!;
    function render(t: number) {
      PM.time = time = t;
      for (const { layer, outer, node } of nodes) {
        outer.setAttribute('display', PM.active(layer, t) ? 'inline' : 'none');
        const d = PM.resolveContent(layer, t);
        outer.setAttribute('transform', `matrix(${PM.worldMatrix(layer, t).join(' ')})`);
        outer.setAttribute('opacity', String(PM.worldOpacity(layer, t)));
        node.setAttribute('fill', d.color || '#ffffff');
        if (layer.type === 'text') {
          const size = Math.max(1, Number(d.size) || 16), weight = d['fontAxis.wght'] ?? d.weight ?? 500;
          const font = `${d.italic ? 'italic ' : ''}${weight} ${size}px ${JSON.stringify(d.font)}`;
          node.setAttribute('style', `font:${font};letter-spacing:${Number(d.tracking) || 0}px;white-space:pre`);
          node.setAttribute('text-anchor', d.align === 'right' ? 'end' : d.align === 'center' ? 'middle' : 'start');
          node.replaceChildren();
          let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
          measure.font = font; measure.textAlign = d.align || 'left'; measure.textBaseline = 'alphabetic';
          measure.letterSpacing = `${Number(d.tracking) || 0}px`;
          String(d.text).split('\n').forEach((line, index) => {
            const y = size * .82 + index * size * (d.leading || 1.15), span = document.createElementNS(NS, 'tspan');
            span.setAttribute('x', '0'); span.setAttribute('y', String(y)); span.textContent = line; node.append(span);
            const m = measure.measureText(line);
            x0 = Math.min(x0, -m.actualBoundingBoxLeft); x1 = Math.max(x1, m.actualBoundingBoxRight);
            y0 = Math.min(y0, y - m.actualBoundingBoxAscent); y1 = Math.max(y1, y + m.actualBoundingBoxDescent);
          });
          const pad = Math.max(3, Math.min(12, size * .055));
          const offset = fontAnchorOffset(d.fontAnchorBounds, { x0: x0 - pad, x1: x1 + pad, y0: y0 - pad, y1: y1 + pad }, PM.ev(layer, 'anchor.x', t), PM.ev(layer, 'anchor.y', t));
          node.setAttribute('transform', `translate(${offset.x} ${offset.y})`);
        } else {
          const w = d.w || scene.project.w, h = d.h || scene.project.h;
          if (layer.type === 'shape') node.setAttribute('transform', `translate(${-w / 2} ${-h / 2})`);
          if (node.tagName === 'ellipse') { for (const [k, v] of Object.entries({ cx: w / 2, cy: h / 2, rx: w / 2, ry: h / 2 })) node.setAttribute(k, String(v)); }
          else { node.setAttribute('width', String(w)); node.setAttribute('height', String(h)); node.setAttribute('rx', String(d.radius || 0)); }
          if (layer.type === 'shape') { node.setAttribute('stroke', d.strokeColor || '#ffffff'); node.setAttribute('stroke-width', String(d.stroke || 0)); }
        }
      }
    }
    function tick(now: number) {
      if (!playing || dead) return;
      let t = origin + (now - start) / 1000;
      if (t >= scene.project.dur) {
        if (loop) { t %= scene.project.dur; origin = t; start = now; }
        else { playing = false; t = Math.max(0, scene.project.dur - 1 / scene.project.fps); }
      }
      try { render(t); if (playing) frame = requestAnimationFrame(tick); }
      catch (error) { playing = false; options.onError?.(error instanceof Error ? error : new Error(String(error))); }
    }
    const player = {
      renderer: 'svg' as const, element: svg,
      get duration() { return scene.project.dur; }, get currentTime() { return time; }, get playing() { return playing; },
      get loop() { return loop; }, set loop(value: boolean) { assertAlive(); loop = value; },
      get parameters() { return structuredClone(scene.project.params || {}); },
      play() { assertAlive(); if (playing) return; playing = true; start = performance.now(); origin = time; frame = requestAnimationFrame(tick); },
      pause() { assertAlive(); playing = false; cancelAnimationFrame(frame); },
      async seek(value: number) { assertAlive(); if (!Number.isFinite(value)) throw new Error('Seek time must be finite'); origin = Math.max(0, Math.min(scene.project.dur - 1 / scene.project.fps, value)); start = performance.now(); render(origin); },
      async setText(id: string, text: string) { assertAlive(); const layer: any = scene.project.layers.find(l => l.id === id); if (layer?.type !== 'text' || typeof text !== 'string') throw new Error('setText requires a root text layer ID and a string'); layer.d.text = PM.P(text); PM.touch(); render(time); },
      async setParameter(name: string, value: unknown) { assertAlive(); const p: any = scene.project.params?.[name]; if (!p) throw new Error(`Unknown parameter: ${name}`); if (p.control === 'num' && (typeof value !== 'number' || !Number.isFinite(value) || p.min != null && value < p.min || p.max != null && value > p.max) || p.control === 'toggle' && typeof value !== 'boolean' || p.control === 'color' && (typeof value !== 'string' || !/^#(?:[\da-f]{2}){3,4}$/i.test(value)) || p.control === 'select' && !p.options?.some((o: any) => o.v === value)) throw new Error(`Invalid value for ${name}`); p.value = value; PM.touch(); render(time); },
      destroy: dispose,
    };
    render(0); if (options.autoplay) player.play(); return player;
  } catch (error) { dispose(); throw error; }
}
