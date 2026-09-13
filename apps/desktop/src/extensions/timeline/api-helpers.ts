import type { Layer, PowermoveAPI } from 'powermove';

export interface TimelineChrome {
  rowHeight: number;
  gutterWidth: number;
  rulerHeight: number;
  clipRadius: number;
  keyframeSize: number;
  showLayerNumbers: boolean;
  showTypeBadges: boolean;
  toolbarDensity: 'compact' | 'normal';
}

export function selectedLayers(api: Pick<PowermoveAPI, 'model' | 'selection'>): Layer[] {
  return api.selection.layers().map((id) => api.model.layer(id)).filter((layer): layer is Layer => layer != null);
}

export function trimAtStart(api: Pick<PowermoveAPI, 'media'>, layer: Layer, nextFrom: number): number {
  // Pre-migration timeline.ts:2028,2043-2045 captured raw d.trim at drag start,
  // then advanced it by the layer's media rate as the in-point moved.
  const trim = Math.max(0, Number((layer.d as Layer['d'] & { trim?: number })?.trim) || 0);
  return Math.max(0, trim + (nextFrom - Number(layer.from || 0)) * api.media.timing.rate(layer));
}

export function normalizeTimelineChrome(api: Pick<PowermoveAPI, 'util'>, value: unknown): TimelineChrome {
  const raw = value && typeof value === 'object' ? value as Partial<TimelineChrome> : {};
  const finite = (candidate: unknown): candidate is number => Number.isFinite(candidate);
  return {
    rowHeight: api.util.clamp(Math.round(finite(raw.rowHeight) ? raw.rowHeight : 30), 22, 48),
    gutterWidth: api.util.clamp(Math.round(finite(raw.gutterWidth) ? raw.gutterWidth : 224), 160, 360),
    rulerHeight: api.util.clamp(Math.round(finite(raw.rulerHeight) ? raw.rulerHeight : 28), 20, 42),
    clipRadius: api.util.clamp(finite(raw.clipRadius) ? raw.clipRadius : 5, 0, 12),
    keyframeSize: api.util.clamp(finite(raw.keyframeSize) ? raw.keyframeSize : 8, 4, 12),
    showLayerNumbers: raw.showLayerNumbers !== false,
    showTypeBadges: raw.showTypeBadges !== false,
    toolbarDensity: raw.toolbarDensity === 'normal' ? 'normal' : 'compact',
  };
}

export function iconNode(api: Pick<PowermoveAPI, 'ui'>, name: string): SVGElement {
  const template = document.createElement('template');
  template.innerHTML = api.ui.icon(name).trim();
  return template.content.firstElementChild as SVGElement;
}

export function element(selector: string, ...args: unknown[]): any {
  const match = selector.match(/^([\w-]+)?(?:#([\w-]+))?((?:\.[\w-]+)*)$/);
  const tag = match?.[1] || 'div';
  const node = document.createElement(tag);
  if (match?.[2]) node.id = match[2];
  if (match?.[3]) node.className = match[3].slice(1).replace(/\./g, ' ');
  const first = args[0];
  if (first && typeof first === 'object' && !(first instanceof Node) && !Array.isArray(first)) {
    const attributes = args.shift() as Record<string, unknown>;
    for (const [key, value] of Object.entries(attributes)) {
      if (key === 'style' && value && typeof value === 'object') Object.assign(node.style, value);
      else if (key in node) (node as unknown as Record<string, unknown>)[key] = value;
      else if (value != null) node.setAttribute(key, String(value));
    }
  }
  for (const child of args.flat()) {
    if (child == null || child === false) continue;
    node.append(child instanceof Node ? child : document.createTextNode(String(child)));
  }
  return node;
}
