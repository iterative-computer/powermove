/* Ported from js/core/ui-state.js — behavior-preserving. */
import type { PMRegistry } from '../registry';

export function install(PM: PMRegistry): void {

const keyHandles: any = new Map();
const reveal: any = new Map();
const shaderMeta: any = new Map();
const fxOpen: any = new Map();
const layerCollapsed: any = new Map();
const layerCollapsedBase: any = new Map();
const groupCollapsed: any = new Map();
let activeProject: any = null;

const idOf = (value: any) => typeof value === 'string' ? value : value && (value.i || value.id);
const emptyShaderMeta = () => ({ udefs: [], shaderKey: null });
const clear = () => {
  keyHandles.clear();
  reveal.clear();
  shaderMeta.clear();
  fxOpen.clear();
  layerCollapsed.clear();
  layerCollapsedBase.clear();
  groupCollapsed.clear();
};

function setKeyPatch(id: any, patch: any) {
  if (!id) return null;
  const next = { ...(keyHandles.get(id) || {}), ...(patch || {}) };
  keyHandles.set(id, next);
  return next;
}

function setRevealValue(id: any, keys: any) {
  if (!id) return null;
  if (!Array.isArray(keys)) {
    reveal.delete(id);
    return null;
  }
  const next = [...keys];
  reveal.set(id, next);
  return next;
}

function setShaderPatch(id: any, patch: any) {
  if (!id) return null;
  const next = { ...(shaderMeta.get(id) || emptyShaderMeta()), ...(patch || {}) };
  shaderMeta.set(id, next);
  return next;
}

function defineCompat(object: any, key: any, get: any, set: any) {
  const descriptor = Object.getOwnPropertyDescriptor(object, key);
  if (descriptor && !descriptor.enumerable && descriptor.get && descriptor.set) return;
  /* A few boundary-locked legacy modules still use these old properties.
     Accessors preserve that contract while keeping the values out of JSON. */
  Object.defineProperty(object, key, { configurable: true, enumerable: false, get, set });
}

function installKeyCompat(key: any) {
  const id = idOf(key);
  if (!id) return;
  const legacy: any = {};
  for (const [name, field] of [['_ho', 'ho'], ['_hi', 'hi'], ['_pt', 'pt']] as any) {
    const descriptor = Object.getOwnPropertyDescriptor(key, name);
    if (descriptor && descriptor.enumerable && descriptor.value != null) legacy[field] = descriptor.value;
    defineCompat(key, name,
      () => keyHandles.get(id)?.[field],
      (value: any) => setKeyPatch(id, { [field]: value }));
  }
  if (Object.keys(legacy).length) setKeyPatch(id, legacy);
}

function installEffectCompat(effect: any) {
  const id = idOf(effect);
  if (!id) return;
  const descriptor = Object.getOwnPropertyDescriptor(effect, 'open');
  if (descriptor && descriptor.enumerable && typeof descriptor.value === 'boolean') fxOpen.set(id, descriptor.value);
  defineCompat(effect, 'open', () => !!fxOpen.get(id), (value: any) => fxOpen.set(id, !!value));
}

function syncLayerCollapsed(layer: any) {
  const id = idOf(layer);
  if (!id) return;
  /* Before hierarchy disclosure had its own state, a group's `collapsed`
     field represented its children. Do not reinterpret an open legacy group
     as a request to expose its transform properties. */
  const persistent = layer.type === 'group' ? true : layer.collapsed !== false;
  if (!layerCollapsedBase.has(id)) {
    layerCollapsedBase.set(id, persistent);
    if (!layerCollapsed.has(id)) layerCollapsed.set(id, persistent);
  } else if (layerCollapsedBase.get(id) !== persistent) {
    layerCollapsedBase.set(id, persistent);
    layerCollapsed.set(id, persistent);
  }
}

function installLayerCompat(layer: any) {
  const id = idOf(layer);
  if (!id) return;
  const revealDescriptor = Object.getOwnPropertyDescriptor(layer, '_reveal');
  const shaderKeyDescriptor = Object.getOwnPropertyDescriptor(layer, '_shaderKey');
  const udefsDescriptor = Object.getOwnPropertyDescriptor(layer, '_udefs');
  if (revealDescriptor?.enumerable) setRevealValue(id, revealDescriptor.value);
  if (shaderKeyDescriptor?.enumerable && shaderKeyDescriptor.value != null) setShaderPatch(id, { shaderKey: shaderKeyDescriptor.value });
  if (udefsDescriptor?.enumerable && Array.isArray(udefsDescriptor.value)) setShaderPatch(id, { udefs: udefsDescriptor.value });
  defineCompat(layer, '_reveal', () => reveal.get(id) || null, (value: any) => {
    setRevealValue(id, value);
    /* Boundary-locked reveal shortcuts update the persistent field first. */
    syncLayerCollapsed(layer);
  });
  defineCompat(layer, '_shaderKey', () => shaderMeta.get(id)?.shaderKey, (value: any) => setShaderPatch(id, { shaderKey: value }));
  defineCompat(layer, '_udefs', () => shaderMeta.get(id)?.udefs, (value: any) => setShaderPatch(id, { udefs: value }));
}

function collectProject(proj: any) {
  const ids: any = { keys: new Set(), layers: new Set(), effects: new Set() };
  const visitProps = (props: any) => Object.values(props || {}).forEach((prop: any) => {
    if (!prop || !Array.isArray(prop.kf)) return;
    prop.kf.forEach((key: any) => {
      const id = idOf(key);
      if (!id) return;
      ids.keys.add(id);
      installKeyCompat(key);
    });
  });
  const visitLayers = (layers: any) => (layers || []).forEach((layer: any) => {
    const id = idOf(layer);
    if (!id) return;
    ids.layers.add(id);
    syncLayerCollapsed(layer);
    installLayerCompat(layer);
    visitProps(layer.p);
    (layer.fx || []).forEach((effect: any) => {
      const effectId = idOf(effect);
      if (effectId) ids.effects.add(effectId);
      installEffectCompat(effect);
      visitProps(effect && effect.p);
    });
    (layer.masks || []).forEach((mask: any) => visitProps(mask && mask.p));
  });
  const visitProject = (project: any) => {
    if (!project || typeof project !== 'object') return;
    visitLayers(project.layers);
    Object.values(project.comps || {}).forEach(visitProject);
  };
  visitProject(proj);
  return ids;
}

function prune(proj: any) {
  if (activeProject && activeProject !== proj) clear();
  activeProject = proj || null;
  const ids = collectProject(proj);
  for (const id of keyHandles.keys()) if (!ids.keys.has(id)) keyHandles.delete(id);
  for (const id of reveal.keys()) if (!ids.layers.has(id)) reveal.delete(id);
  for (const id of shaderMeta.keys()) if (!ids.layers.has(id)) shaderMeta.delete(id);
  for (const id of fxOpen.keys()) if (!ids.effects.has(id)) fxOpen.delete(id);
  for (const id of layerCollapsed.keys()) if (!ids.layers.has(id)) layerCollapsed.delete(id);
  for (const id of layerCollapsedBase.keys()) if (!ids.layers.has(id)) layerCollapsedBase.delete(id);
  for (const id of groupCollapsed.keys()) if (!ids.layers.has(id)) groupCollapsed.delete(id);
  return UIState;
}

function current() {
  if (PM.proj !== activeProject) prune(PM.proj);
}

const UIState = PM.UIState = {
  keyHandles,
  reveal,
  shaderMeta,
  fxOpen,
  layerCollapsed,
  groupCollapsed,
  prune,

  getKeyHandles(key: any) {
    current();
    if (key && typeof key === 'object') installKeyCompat(key);
    return keyHandles.get(idOf(key)) || null;
  },
  setKeyHandles(key: any, patch: any) {
    current();
    if (key && typeof key === 'object') installKeyCompat(key);
    return setKeyPatch(idOf(key), patch);
  },

  getReveal(layer: any) {
    current();
    if (layer && typeof layer === 'object') installLayerCompat(layer);
    return reveal.get(idOf(layer)) || null;
  },
  setReveal(layer: any, keys: any) {
    current();
    if (layer && typeof layer === 'object') installLayerCompat(layer);
    return setRevealValue(idOf(layer), keys);
  },

  getShaderMeta(layer: any) {
    current();
    if (layer && typeof layer === 'object') installLayerCompat(layer);
    return shaderMeta.get(idOf(layer)) || emptyShaderMeta();
  },
  setShaderMeta(layer: any, patch: any) {
    current();
    if (layer && typeof layer === 'object') installLayerCompat(layer);
    return setShaderPatch(idOf(layer), patch);
  },

  getFxOpen(effect: any) {
    current();
    if (effect && typeof effect === 'object') installEffectCompat(effect);
    return !!fxOpen.get(idOf(effect));
  },
  setFxOpen(effect: any, open: any) {
    current();
    if (effect && typeof effect === 'object') installEffectCompat(effect);
    const id = idOf(effect);
    if (!id) return false;
    fxOpen.set(id, !!open);
    return !!open;
  },

  getLayerCollapsed(layer: any) {
    current();
    const id = idOf(layer);
    if (!id) return true;
    if (layer && typeof layer === 'object') syncLayerCollapsed(layer);
    if (!layerCollapsed.has(id)) layerCollapsed.set(id, true);
    return layerCollapsed.get(id);
  },
  setLayerCollapsed(layer: any, collapsed: any) {
    current();
    const id = idOf(layer);
    if (!id) return true;
    const next = !!collapsed;
    if (layer && typeof layer === 'object') {
      const persistent = layer.type === 'group' ? true : layer.collapsed !== false;
      if (!layerCollapsedBase.has(id)) layerCollapsedBase.set(id, persistent);
      if (persistent === next) layerCollapsedBase.set(id, next);
    }
    layerCollapsed.set(id, next);
    return next;
  },

  /* Group hierarchy disclosure is independent from a group's own property
     strip. Opening a group reveals its child layers without also exposing
     every transform row on the group itself. */
  getGroupCollapsed(layer: any) {
    current();
    const id = idOf(layer);
    if (!id) return true;
    if (!groupCollapsed.has(id)) {
      /* Preserve the old hierarchy state during the one-time split from the
         legacy shared disclosure field. */
      groupCollapsed.set(id, layer && typeof layer === 'object' ? layer.collapsed !== false : true);
    }
    return groupCollapsed.get(id);
  },
  setGroupCollapsed(layer: any, collapsed: any) {
    current();
    const id = idOf(layer);
    if (!id) return true;
    const next = !!collapsed;
    groupCollapsed.set(id, next);
    return next;
  },
};

PM.bus.on('project', () => prune(PM.proj));
PM.bus.on('layers', () => { if (PM.proj) prune(PM.proj); });
}
