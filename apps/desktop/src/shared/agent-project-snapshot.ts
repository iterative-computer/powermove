import { LIMITS } from './ipc';

const INDEX_ENTRIES = 1_000;
const LABEL_CHARS = 256;

function metadata(value: Record<string, any>, fields: readonly string[]): Record<string, string | number | boolean | null> {
  const result: Record<string, string | number | boolean | null> = {};
  for (const field of fields) {
    const item = value[field];
    if (typeof item === 'string') result[field] = item.slice(0, LABEL_CHARS);
    else if (typeof item === 'number' && Number.isFinite(item)) result[field] = item;
    else if (typeof item === 'boolean' || item === null) result[field] = item;
  }
  return result;
}

/** Read-only context for agent workspaces, never a save or undo snapshot. */
export function agentProjectSnapshot(project: Record<string, any>): string {
  const serialized = JSON.stringify(project);
  if (serialized.length <= LIMITS.codexProjectJsonBytes &&
      new TextEncoder().encode(serialized).byteLength <= LIMITS.codexProjectJsonBytes) return serialized;

  const layers = Array.isArray(project.layers) ? project.layers : [];
  const comps = Array.isArray(project.comps) ? project.comps : [];
  const assets = Array.isArray(project.assets) ? project.assets : [];
  const index = (items: any[], fields: readonly string[]) => items.slice(0, INDEX_ENTRIES)
    .filter(item => item && typeof item === 'object')
    .map(item => metadata(item, fields));
  return JSON.stringify({
    ...metadata(project, ['id', 'name', 'revision', 'w', 'h', 'fps', 'dur', 'activeCompId']),
    agentSnapshot: {
      complete: false,
      note: 'This is a bounded run-start index, not the full project. Layer properties, geometry, keyframes and media data are omitted. Use get_project_state with indexOnly=true and follow pagination, then read layerId for details. Use list_media for assets. Never replace the project from this index.',
    },
    layerCount: layers.length,
    layers: index(layers, ['id', 'name', 'type', 'parent', 'compId']),
    compositionCount: comps.length,
    comps: index(comps, ['id', 'name', 'w', 'h', 'fps', 'dur']),
    assetCount: assets.length,
    assets: index(assets, ['id', 'name', 'type']),
  });
}
