/** Content channels use the same persisted {v, kf, expr} model as transforms. */
export function isProperty(value: any): boolean {
  return !!value && typeof value === 'object' && Array.isArray(value.kf) && 'v' in value;
}

export const contentFields: Record<string, string[]> = {
  text: ['text', 'font', 'weight', 'size', 'tracking', 'leading', 'align', 'color', 'italic', 'boxWidth', 'boxHeight'],
  solid: ['color', 'w', 'h', 'radius'],
  shape: ['color', 'shape', 'w', 'h', 'radius', 'stroke', 'strokeColor', 'points'],
  image: ['fit', 'w', 'h'],
  video: ['fit', 'w', 'h', 'trim', 'speed', 'timeRemap', 'sourceTime'],
  audio: ['trim', 'gain', 'fadeIn', 'fadeOut'],
  shader: ['w', 'h'],
  extension: ['w', 'h'],
  precomp: ['w', 'h', 'trim', 'sourceTime', 'timeRemap']
};

export function canAnimateContent(layer: any, key: string): boolean {
  return !!contentFields[layer.type]?.includes(key) || (layer.type === 'text' && /^fontAxis\.[\x20-\x7e]{4}$/.test(key));
}

export function resolveContent(PM: any, layer: any, time: number): any {
  const content = { ...layer.d };
  for (const [key, value] of Object.entries(content)) {
    if (canAnimateContent(layer, key) && isProperty(value)) content[key] = PM.evP ? PM.evP(layer, value, time, `c.${key}`) : (value as any).v;
  }
  return content;
}

export function contentLabel(key: string): string {
  const names: Record<string, string> = { w: 'Width', h: 'Height', text: 'Text', font: 'Font', weight: 'Weight', size: 'Size', tracking: 'Tracking', leading: 'Leading', align: 'Align', color: 'Color', italic: 'Italic', boxWidth: 'Text Box Width', boxHeight: 'Text Box Height', shape: 'Shape', radius: 'Corner radius', stroke: 'Stroke', strokeColor: 'Stroke color', points: 'Points', fit: 'Fit', trim: 'Trim start', speed: 'Speed', gain: 'Gain', fadeIn: 'Fade in', fadeOut: 'Fade out' };
  return names[key] ?? key;
}

export function evaluatedValue(PM: any, layer: any, value: any, time: number, path: string): any {
  return isProperty(value) ? PM.evP(layer, value, time, path) : value;
}
