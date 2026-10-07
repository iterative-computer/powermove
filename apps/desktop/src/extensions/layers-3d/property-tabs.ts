/* Blender's Properties tabs for 3D layers, in Blender's order and colors. */
export type PropertyTab = 'render' | 'world' | 'object' | 'modifiers' | 'data' | 'material';

const line = 'fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"';
export const PROPERTY_TABS: Record<PropertyTab, { label: string; color: string; icon: (lightType?: string, camera?: boolean) => string }> = {
  render: { label: 'Render Properties', color: '#b4b4b4', icon: () => `<rect x="3" y="7" width="13" height="10" rx="2" ${line}/><path d="m16 10.5 5-3v9l-5-3" ${line}/>` },
  world: { label: 'World Properties', color: '#e65c5c', icon: () => `<circle cx="12" cy="12" r="8" ${line}/><path d="M4 12h16M12 4c2.5 2.4 2.5 13.6 0 16M12 4c-2.5 2.4-2.5 13.6 0 16" ${line}/>` },
  object: { label: 'Object Properties', color: '#f0a03c', icon: () => `<rect x="5" y="5" width="14" height="14" rx="2" fill="currentColor" opacity=".9"/>` },
  modifiers: { label: 'Modifier Properties', color: '#6c9be0', icon: () => `<path d="M14.5 4.5a4 4 0 0 0-4.7 5.4L4 15.7 8.3 20l5.8-5.8a4 4 0 0 0 5.4-4.7l-2.6 2.6-2.7-.6-.6-2.7z" ${line}/>` },
  data: { label: 'Data Properties', color: '#6fcf6f', icon: (lightType, camera) => camera
    ? `<rect x="3" y="7" width="13" height="10" rx="2" ${line}/><path d="m16 10.5 5-3v9l-5-3" ${line}/>`
    : lightType ? `<path d="M9 17h6M10 20h4M12 3a6 6 0 0 0-3.5 10.9V15h7v-1.1A6 6 0 0 0 12 3z" ${line}/>`
    : `<path d="M12 3 21 19H3z" ${line}/>` },
  material: { label: 'Material Properties', color: '#e8706f', icon: () => `<circle cx="12" cy="12" r="8" ${line}/><path d="M12 4v16M4 12h16" ${line} opacity=".55"/><circle cx="9" cy="9" r="1.6" fill="currentColor"/>` }
};

/** Which tabs a layer has: models get Object, Modifiers (generated models) and Material; lights and cameras get Data. */
export function tabsFor(kind: { object: boolean; light: boolean; camera: boolean; model: boolean }): PropertyTab[] {
  const tabs: PropertyTab[] = ['render', 'world'];
  if (kind.object) tabs.push('object');
  if (kind.model) tabs.push('modifiers');
  if (kind.light || kind.camera) tabs.push('data');
  if (kind.object) tabs.push('material');
  return tabs;
}
