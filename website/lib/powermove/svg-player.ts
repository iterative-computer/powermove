import { createEngine } from './player.js';

const NS = 'http://www.w3.org/2000/svg';
const node = (name: string) => document.createElementNS(NS, name);

/** SVG adapter for the exported scene's plain text layers and affine groups.
 * Evaluation is delegated to the original runtime, including temporal easing.
 * Missing export effects remain inactive, just as in the original web player.
 */
export function createSvgPlayer(svg: SVGSVGElement, source: any) {
  const scene = structuredClone(source);
  const engine = createEngine(scene.project);
  const unsupported = scene.project.layers.filter((l: any) => !['text', 'group'].includes(l.type));
  if (unsupported.length) throw new Error('SVG export currently supports text and affine groups only.');
  const defs = node('defs');
  svg.replaceChildren(defs);
  svg.setAttribute('viewBox', `0 0 ${scene.project.w} ${scene.project.h}`);
  const records = [...scene.project.layers].reverse().filter((l: any) => l.type === 'text').map((layer: any) => {
    if (layer.masks?.length || layer.d.paragraph || layer.d.animators?.length || layer.d.styles?.length) throw new Error(`Unsupported SVG text feature: ${layer.name}`);
    const group = node('g');
    group.setAttribute('data-layer-id', layer.id);
    const text = node('text') as SVGTextElement;
    text.setAttribute('xml:space', 'preserve');
    const fx = layer.fx?.find((f: any) => f.on && f.type === 'glow');
    let halo: SVGTextElement | undefined;
    let blur: Element | undefined;
    if (fx) {
      const filter = node('filter');
      const id = `scene-glow-${layer.id}`;
      filter.setAttribute('id', id);
      filter.setAttribute('x', '-50%'); filter.setAttribute('y', '-150%');
      filter.setAttribute('width', '200%'); filter.setAttribute('height', '400%');
      blur = node('feGaussianBlur'); filter.append(blur); defs.append(filter);
      halo = node('text') as SVGTextElement;
      halo.setAttribute('filter', `url(#${id})`);
      group.append(halo);
    }
    group.append(text); svg.append(group);
    return { layer, group, text, halo, blur, fx };
  });
  function seek(time: number) {
    engine.time = time;
    for (const {layer, group, text, halo, blur, fx} of records) {
      const active = engine.active(layer, time);
      group.setAttribute('display', active ? 'inline' : 'none');
      if (!active) continue;
      const d = engine.resolveContent(layer, time);
      const matrix = engine.worldMatrix(layer, time);
      group.setAttribute('transform', `matrix(${matrix.join(' ')})`);
      group.setAttribute('opacity', String(engine.worldOpacity(layer, time)));
      for (const element of [text, halo].filter(Boolean) as SVGTextElement[]) {
        element.textContent = String(d.text);
        element.setAttribute('font-family', d.font);
        element.setAttribute('font-size', String(d.size));
        element.setAttribute('font-weight', String(d.weight));
        element.setAttribute('font-style', d.italic ? 'italic' : 'normal');
        element.setAttribute('letter-spacing', String(d.tracking || 0));
        element.setAttribute('text-anchor', d.align === 'right' ? 'end' : d.align === 'center' ? 'middle' : 'start');
        element.setAttribute('y', String(d.size * .82));
        element.setAttribute('fill', d.color);
        element.style.fontVariationSettings = `'wght' ${d['fontAxis.wght'] ?? d.weight}`;
        element.removeAttribute('transform');
      }
      // Match the editor's saved typography anchor with vector glyph bounds.
      if (d.fontAnchorBounds) {
        const box = text.getBBox();
        const pad = Math.min(12, Math.max(3, d.size * .055));
        const ref = d.fontAnchorBounds;
        const offset = (start: number, end: number, nextStart: number, nextEnd: number, anchor: number) =>
          end > start ? anchor - (nextStart + (anchor - start) / (end - start) * (nextEnd - nextStart)) : 0;
        const x = offset(ref.x0, ref.x1, box.x - pad, box.x + box.width + pad, engine.ev(layer, 'anchor.x', time));
        const y = offset(ref.y0, ref.y1, box.y - pad, box.y + box.height + pad, engine.ev(layer, 'anchor.y', time));
        text.setAttribute('transform', `translate(${x} ${y})`);
        halo?.setAttribute('transform', `translate(${x} ${y})`);
      }
      if (halo && blur) {
        blur.setAttribute('stdDeviation', String(engine.evP(layer, fx.p.radius, time) / 2));
        halo.setAttribute('opacity', String(Math.min(1, engine.evP(layer, fx.p.intensity, time) / 100)));
      }
    }
    svg.dataset.time = String(time);
  }
  return { duration: scene.project.dur, seek, destroy() { engine.bus.clear(); svg.replaceChildren(); } };
}
