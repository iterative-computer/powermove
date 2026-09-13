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

export function cloneLayer(api: Pick<PowermoveAPI, 'anim' | 'project' | 'util'>, layer: Layer): Layer {
  const clone = JSON.parse(JSON.stringify(layer)) as Layer;
  clone.id = api.util.uid('L');
  const baseName = layer.name.replace(/ (\d+)$/, '');
  clone.name = `${baseName} ${api.project.get().layers.filter((item) => item.name.startsWith(baseName)).length + 1}`;
  const renew = (property: unknown): void => {
    const channel = property as { kf?: Array<{ i: string }> } | null;
    for (const keyframe of channel?.kf ?? []) keyframe.i = api.util.uid('k');
  };
  Object.values(clone.p ?? {}).forEach(renew);
  for (const effect of clone.fx ?? []) {
    renew(effect.on);
    Object.values(effect.p ?? {}).forEach(renew);
  }
  for (const mask of clone.masks ?? []) {
    Object.values(mask.p ?? {}).forEach(renew);
    renew(mask.shape);
    renew(mask.mode);
    renew(mask.on);
  }
  Object.values(clone.d ?? {}).forEach(renew);
  renew(clone.blend);
  renew(clone.mblur);
  renew(clone.on);
  const dynamic = clone as Layer & {
    d: Layer['d'] & { uniforms?: Record<string, unknown>; params?: Record<string, unknown> };
    transitionIn?: { p?: Record<string, unknown> } | null;
    transitionOut?: { p?: Record<string, unknown> } | null;
  };
  Object.values(dynamic.d?.uniforms ?? {}).forEach(renew);
  Object.values(dynamic.d?.params ?? {}).forEach(renew);
  for (const transition of [dynamic.transitionIn, dynamic.transitionOut]) Object.values(transition?.p ?? {}).forEach(renew);
  return clone;
}

export function trimAtStart(api: Pick<PowermoveAPI, 'anim' | 'media' | 'transport'>, layer: Layer, nextFrom: number): number {
  const content = api.anim.resolveContent(layer, api.transport.time()) as Layer['d'] & { trim?: number };
  const trim = Math.max(0, Number(content?.trim) || 0);
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
