/* Powermove — pure workspace manifest model.
   This file deliberately has no DOM dependency so manifests can be validated,
   migrated and tested before the interface is touched. */
(function (root, factory) {
  const api = factory();
  if (typeof module !== 'undefined' && module.exports) module.exports = api;
  if (root) root.PMWorkspaceSchema = api;
})(typeof window !== 'undefined' ? window : globalThis, function () {
  'use strict';

  const TYPES = new Set(['panel', 'split', 'stack', 'tabs']);
  const clone = (value) => value == null ? value : JSON.parse(JSON.stringify(value));
  const cleanNumber = (value, fallback, min, max) => {
    const number = Number(value);
    if (!Number.isFinite(number)) return fallback;
    return Math.max(min, Math.min(max, number));
  };
  const safeId = (value, fallback) => {
    const id = String(value || fallback || '').trim().replace(/[^a-zA-Z0-9_.:-]+/g, '-');
    return id || fallback || 'section';
  };

  function panel(type, options = {}) {
    return {
      type: 'panel',
      panel: safeId(type, 'layers'),
      instance: safeId(options.instance, safeId(type, 'layers')),
      ...(options.title ? { title: String(options.title) } : {}),
      ...(options.size ? { size: cleanNumber(options.size, 240, 48, 4000) } : {}),
      ...(options.minWidth ? { minWidth: cleanNumber(options.minWidth, 160, 80, 2000) } : {}),
      ...(options.minHeight ? { minHeight: cleanNumber(options.minHeight, 80, 48, 2000) } : {}),
      ...(options.flex ? { flex: true } : {}),
      ...(options.headless ? { headless: true } : {}),
    };
  }

  function split(direction, children, options = {}) {
    return {
      type: direction === 'column' ? 'stack' : 'split',
      direction: direction === 'column' ? 'column' : 'row',
      children: children || [],
      ...(Array.isArray(options.sizes) ? { sizes: options.sizes.map(Number) } : {}),
    };
  }

  function tabs(children, active) {
    const list = children || [];
    return { type: 'tabs', active: safeId(active, list[0]?.instance), children: list };
  }

  function legacyToRoot(layout) {
    const docks = Array.isArray(layout?.docks) ? layout.docks.filter((dock) => !dock.hidden) : [];
    const nodes = docks.map((dock) => {
      const children = (dock.panels || []).map((spec) => panel(spec.id || spec.panel, spec));
      if (!children.length) return null;
      const node = children.length === 1 ? children[0] : split('column', children, {
        /* Old workspaces mixed pixel sizes with a boolean "flex" flag. Treating
           flex as the number 1 made the flexible viewer almost disappear next
           to a 300px timeline. Convert both forms to comparable layout weights. */
        sizes: children.map((child, index) => child.flex
          ? 3
          : Math.max(.75, (Number(dock.panels[index]?.size) || 180) / 150)),
      });
      node.region = safeId(dock.id, 'center');
      node.preferredSize = cleanNumber(dock.size, dock.id === 'right' ? 300 : 250, 120, 1600);
      node.dockFlex = !!dock.flex;
      if (dock.id === 'center' || dock.flex) node.flex = true;
      return node;
    }).filter(Boolean);
    if (!nodes.length) return panel('viewer', { instance: 'viewer-main', flex: true, headless: true });
    if (nodes.length === 1) return nodes[0];
    return split('row', nodes, { sizes: nodes.map((node) => node.region === 'center' || node.dockFlex ? 4 : (node.preferredSize || 250) / 180) });
  }

  function normalizeNode(input, context, depth = 0) {
    if (!input || typeof input !== 'object' || depth > 24) return null;
    const kind = TYPES.has(input.type) ? input.type : (input.panel || input.id ? 'panel' : null);
    if (kind === 'panel') {
      const panelId = safeId(input.panel || input.id, 'layers');
      const base = safeId(input.instance, panelId);
      const count = context.instances.get(base) || 0;
      context.instances.set(base, count + 1);
      const instance = count ? `${base}-${count + 1}` : base;
      return {
        ...panel(panelId, { ...input, instance }),
        ...(input.bindings && typeof input.bindings === 'object' ? { bindings: clone(input.bindings) } : {}),
      };
    }
    const children = (Array.isArray(input.children) ? input.children : [])
      .map((child) => normalizeNode(child, context, depth + 1)).filter(Boolean);
    if (!children.length) return null;
    if (kind === 'tabs') {
      const activeExists = children.some((child) => child.instance === input.active);
      return { type: 'tabs', active: activeExists ? input.active : children[0].instance, children };
    }
    return {
      type: kind === 'stack' || input.direction === 'column' ? 'stack' : 'split',
      direction: kind === 'stack' || input.direction === 'column' ? 'column' : 'row',
      children,
      sizes: normalizeSizes(input.sizes, children.length),
      ...(input.region ? { region: safeId(input.region) } : {}),
      ...(input.preferredSize ? { preferredSize: cleanNumber(input.preferredSize, 250, 80, 4000) } : {}),
      ...(input.flex ? { flex: true } : {}),
    };
  }

  function normalizeSizes(sizes, length) {
    const raw = Array.isArray(sizes) ? sizes.slice(0, length).map((value) => Math.max(0.01, Number(value) || 1)) : [];
    while (raw.length < length) raw.push(1);
    const total = raw.reduce((sum, value) => sum + value, 0) || length;
    const normalized = raw.map((value) => value / total);
    /* A stale/corrupt saved ratio must never make a section effectively
       unreachable. Keep every sibling large enough to grab and resize. */
    const floor = Math.min(.12, .6 / Math.max(1, length));
    const raised = normalized.map((value) => Math.max(floor, value));
    const raisedTotal = raised.reduce((sum, value) => sum + value, 0);
    return raised.map((value) => value / raisedTotal);
  }

  function normalizeFloating(list, context, mode) {
    return (Array.isArray(list) ? list : []).map((item, index) => {
      const node = normalizeNode(item.node || item, context, 0);
      if (!node) return null;
      const defaults = mode === 'overlay'
        ? { x: 24, y: 24, width: 320, height: 220, anchor: 'top-right' }
        : { x: 90 + index * 24, y: 80 + index * 24, width: 420, height: 360, anchor: 'free' };
      return {
        id: safeId(item.id, `${mode}-${index + 1}`), node,
        x: cleanNumber(item.x, defaults.x, -4000, 12000),
        y: cleanNumber(item.y, defaults.y, -4000, 12000),
        width: cleanNumber(item.width, defaults.width, 140, 4000),
        height: cleanNumber(item.height, defaults.height, 90, 4000),
        anchor: ['top-left', 'top-right', 'bottom-left', 'bottom-right', 'center', 'free'].includes(item.anchor) ? item.anchor : defaults.anchor,
        ...(item.viewerOverlay ? { viewerOverlay: true } : {}),
      };
    }).filter(Boolean);
  }

  function normalizeWorkspace(workspace) {
    const source = clone(workspace || {});
    const context = { instances: new Map() };
    const root = normalizeNode(source.layout?.root || legacyToRoot(source.layout), context) || panel('viewer', { instance: 'viewer-main' });
    const overlays = normalizeFloating(source.layout?.overlays, context, 'overlay');
    const floating = normalizeFloating(source.layout?.floating, context, 'floating');
    return {
      ...source,
      manifestVersion: 2,
      id: safeId(source.id, 'workspace'),
      name: String(source.name || 'Workspace'),
      density: ['compact', 'normal', 'comfy'].includes(source.density) ? source.density : 'normal',
      theme: source.theme && typeof source.theme === 'object' ? source.theme : {},
      features: source.features && typeof source.features === 'object' ? source.features : {},
      custom: Array.isArray(source.custom) ? source.custom : [],
      layout: { root, overlays, floating },
    };
  }

  function walk(node, visit, path = []) {
    if (!node) return;
    visit(node, path);
    (node.children || []).forEach((child, index) => walk(child, visit, path.concat(index)));
  }

  function atPath(root, path) {
    let node = root;
    for (const index of path) node = node?.children?.[index];
    return node || null;
  }

  function replaceAt(root, path, replacement) {
    if (!path.length) return replacement;
    const copy = clone(root);
    let parent = copy;
    for (let i = 0; i < path.length - 1; i += 1) parent = parent.children[path[i]];
    parent.children[path[path.length - 1]] = replacement;
    return copy;
  }

  function findPath(root, instance) {
    let found = null;
    walk(root, (node, path) => { if (!found && node.type === 'panel' && node.instance === instance) found = path; });
    return found;
  }

  function prune(node) {
    if (!node || node.type === 'panel') return node;
    node.children = (node.children || []).map(prune).filter(Boolean);
    if (!node.children.length) return null;
    if (node.children.length === 1) return node.children[0];
    if (node.type === 'tabs' && !node.children.some((child) => child.instance === node.active)) node.active = node.children[0].instance;
    if (node.type !== 'tabs') node.sizes = normalizeSizes(node.sizes, node.children.length);
    return node;
  }

  function removeFromRoot(root, instance) {
    const path = findPath(root, instance);
    if (!path) return { root, removed: null };
    const removed = clone(atPath(root, path));
    if (!path.length) return { root: null, removed };
    const copy = clone(root);
    const parent = atPath(copy, path.slice(0, -1));
    parent.children.splice(path[path.length - 1], 1);
    return { root: prune(copy), removed };
  }

  function removeInstance(workspace, instance) {
    const next = clone(workspace);
    const result = removeFromRoot(next.layout.root, instance);
    next.layout.root = result.root;
    let removed = result.removed;
    for (const key of ['overlays', 'floating']) {
      const kept = [];
      for (const item of next.layout[key] || []) {
        const inner = removeFromRoot(item.node, instance);
        if (!removed && inner.removed) removed = inner.removed;
        if (inner.root) kept.push({ ...item, node: inner.root });
      }
      next.layout[key] = kept;
    }
    return { workspace: next, removed };
  }

  function uniqueInstance(workspace, base) {
    const used = new Set();
    walk(workspace.layout.root, (node) => { if (node.type === 'panel') used.add(node.instance); });
    for (const key of ['overlays', 'floating']) for (const item of workspace.layout[key] || []) walk(item.node, (node) => { if (node.type === 'panel') used.add(node.instance); });
    let candidate = safeId(base, 'section');
    let index = 2;
    while (used.has(candidate)) candidate = `${safeId(base, 'section')}-${index++}`;
    return candidate;
  }

  function placePanel(workspace, panelNode, placement = {}) {
    let next = normalizeWorkspace(workspace);
    let moving = normalizeNode(panelNode, { instances: new Map() }) || panel('layers');
    if (placement.duplicate || findPath(next.layout.root, moving.instance)) moving.instance = uniqueInstance(next, moving.instance || moving.panel);
    else {
      const detached = removeInstance(next, moving.instance);
      next = detached.workspace;
      moving = detached.removed || moving;
    }
    if (placement.mode === 'overlay' || placement.mode === 'floating') {
      const key = placement.mode === 'overlay' ? 'overlays' : 'floating';
      next.layout[key].push({
        id: safeId(placement.id, `${placement.mode}-${moving.instance}`), node: moving,
        x: cleanNumber(placement.x, placement.mode === 'overlay' ? 24 : 90, -4000, 12000),
        y: cleanNumber(placement.y, placement.mode === 'overlay' ? 24 : 80, -4000, 12000),
        width: cleanNumber(placement.width, placement.mode === 'overlay' ? 320 : 420, 140, 4000),
        height: cleanNumber(placement.height, placement.mode === 'overlay' ? 220 : 360, 90, 4000),
        anchor: placement.anchor || (placement.mode === 'overlay' ? 'top-right' : 'free'),
        ...(placement.viewerOverlay ? { viewerOverlay: true } : {}),
      });
      return next;
    }
    if (!next.layout.root) { next.layout.root = moving; return next; }
    const targetPath = placement.target ? findPath(next.layout.root, placement.target) : [];
    const path = targetPath || [];
    const target = atPath(next.layout.root, path) || next.layout.root;
    const where = placement.where || 'right';
    let replacement;
    if (where === 'tab') {
      replacement = target.type === 'tabs'
        ? { ...target, active: moving.instance, children: [...target.children, moving] }
        : tabs([target, moving], moving.instance);
    } else {
      const direction = where === 'top' || where === 'bottom' ? 'column' : 'row';
      const before = where === 'left' || where === 'top';
      replacement = split(direction, before ? [moving, target] : [target, moving], { sizes: placement.sizes || [1, 1] });
    }
    next.layout.root = replaceAt(next.layout.root, path, replacement);
    return next;
  }

  function validate(workspace, availablePanels) {
    const errors = [];
    const seen = new Set();
    const allowed = availablePanels ? new Set(availablePanels) : null;
    const check = (node, path) => {
      if (node.type !== 'panel') return;
      if (seen.has(node.instance)) errors.push(`Duplicate section instance “${node.instance}”`);
      seen.add(node.instance);
      if (allowed && !allowed.has(node.panel)) errors.push(`Unknown section “${node.panel}” at ${path.join('.') || 'root'}`);
    };
    walk(workspace.layout?.root, check);
    for (const key of ['overlays', 'floating']) for (const item of workspace.layout?.[key] || []) walk(item.node, check);
    if (!workspace.layout?.root) errors.push('Workspace needs a root section');
    return { ok: errors.length === 0, errors };
  }

  function listPanels(workspace) {
    const out = [];
    walk(workspace.layout?.root, (node, path) => { if (node.type === 'panel') out.push({ ...node, region: 'root', path }); });
    for (const key of ['overlays', 'floating']) for (const item of workspace.layout?.[key] || []) {
      walk(item.node, (node, path) => { if (node.type === 'panel') out.push({ ...node, region: key, container: item.id, path }); });
    }
    return out;
  }

  return {
    clone, panel, split, tabs, legacyToRoot, normalizeNode, normalizeWorkspace,
    normalizeSizes, walk, findPath, atPath, removeInstance, placePanel,
    uniqueInstance, validate, listPanels,
  };
});
