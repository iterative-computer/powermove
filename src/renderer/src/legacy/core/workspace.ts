/* Ported from js/core/workspace.js — behavior-preserving. */
import type { PMRegistry } from '../registry';

export function install(PM: PMRegistry): void {
const h: any = PM.h;

const dock: any = (id: any, panels: any, size: any) => ({ id, size, panels });
const p: any = (id: any, o: any = {}) => ({ id, ...o });

const copy: any = (value: any) => JSON.parse(JSON.stringify(value));
const finite: any = (value: any) => typeof value === 'number' && Number.isFinite(value);
const text: any = (value: any, fallback: any = '') => typeof value === 'string' && value.trim() ? value.trim() : fallback;
const LEGACY_TIMELINE_DEFAULTS: any = Object.freeze({
  rowHeight: 30, gutterWidth: 214, rulerHeight: 26, clipRadius: 6,
  keyframeSize: 8.8, showLayerNumbers: true, showTypeBadges: true, toolbarDensity: 'normal',
});
const V2_TIMELINE_DEFAULTS: any = Object.freeze({
  rowHeight: 26, gutterWidth: 192, rulerHeight: 22, clipRadius: 5,
  keyframeSize: 7.5, showLayerNumbers: true, showTypeBadges: true,
  toolbarDensity: 'compact',
});
const TIMELINE_DEFAULTS: any = Object.freeze({
  rowHeight: 32, gutterWidth: 224, rulerHeight: 28, clipRadius: 5,
  keyframeSize: 8, showLayerNumbers: true, showTypeBadges: true,
  toolbarDensity: 'compact',
});
const TIMELINE_CHROME_SCHEMA: any = 3;

function isLegacyTimelineChrome(value: any) {
  if (!value || typeof value !== 'object') return true;
  return [LEGACY_TIMELINE_DEFAULTS, V2_TIMELINE_DEFAULTS].some((defaults: any) =>
    Object.entries(defaults).every(([key, expected]: any) => value[key] === undefined || value[key] === expected));
}

function normalizeTimelineChrome(value: any) {
  const raw: any = value && typeof value === 'object' ? value : {};
  return {
    rowHeight: PM.clamp(Math.round(finite(raw.rowHeight) ? raw.rowHeight : TIMELINE_DEFAULTS.rowHeight), 22, 48),
    gutterWidth: PM.clamp(Math.round(finite(raw.gutterWidth) ? raw.gutterWidth : TIMELINE_DEFAULTS.gutterWidth), 160, 360),
    rulerHeight: PM.clamp(Math.round(finite(raw.rulerHeight) ? raw.rulerHeight : TIMELINE_DEFAULTS.rulerHeight), 20, 42),
    clipRadius: PM.clamp(finite(raw.clipRadius) ? raw.clipRadius : TIMELINE_DEFAULTS.clipRadius, 0, 12),
    keyframeSize: PM.clamp(finite(raw.keyframeSize) ? raw.keyframeSize : TIMELINE_DEFAULTS.keyframeSize, 4, 12),
    showLayerNumbers: raw.showLayerNumbers !== false,
    showTypeBadges: raw.showTypeBadges !== false,
    toolbarDensity: ['compact', 'normal'].includes(raw.toolbarDensity) ? raw.toolbarDensity : TIMELINE_DEFAULTS.toolbarDensity,
  };
}

function sanitizeInterfaceEdit(value: any) {
  let raw: any = value;
  if (typeof raw === 'string') {
    if (raw.length > 30_000) return null;
    try { raw = JSON.parse(raw); } catch { return null; }
  }
  if (!raw || typeof raw !== 'object' || Array.isArray(raw) || raw.target !== 'timeline') return null;
  const patch: any = raw.patch && typeof raw.patch === 'object' && !Array.isArray(raw.patch) ? raw.patch : {};
  const out: any = {};
  if (finite(patch.rowHeight)) out.rowHeight = PM.clamp(Math.round(patch.rowHeight), 22, 48);
  if (finite(patch.gutterWidth)) out.gutterWidth = PM.clamp(Math.round(patch.gutterWidth), 160, 360);
  if (finite(patch.rulerHeight)) out.rulerHeight = PM.clamp(Math.round(patch.rulerHeight), 20, 42);
  if (finite(patch.clipRadius)) out.clipRadius = PM.clamp(patch.clipRadius, 0, 12);
  if (finite(patch.keyframeSize)) out.keyframeSize = PM.clamp(patch.keyframeSize, 4, 12);
  if (typeof patch.showLayerNumbers === 'boolean') out.showLayerNumbers = patch.showLayerNumbers;
  if (typeof patch.showTypeBadges === 'boolean') out.showTypeBadges = patch.showTypeBadges;
  if (['compact', 'normal'].includes(patch.toolbarDensity)) out.toolbarDensity = patch.toolbarDensity;
  if (['normal', 'reversed'].includes(patch.surfaceOrder)) out.surfaceOrder = patch.surfaceOrder;
  return Object.keys(out).length ? { target: 'timeline', patch: out } : null;
}

function applyInterfaceEdit(workspace: any, edit: any) {
  const clean: any = sanitizeInterfaceEdit(edit);
  if (!workspace || !clean) return false;
  workspace.chrome = workspace.chrome && typeof workspace.chrome === 'object' ? workspace.chrome : {};
  workspace.chrome.timeline = { ...normalizeTimelineChrome(workspace.chrome.timeline), ...clean.patch };
  if (clean.patch.surfaceOrder) workspace.chrome.timelineSurfaceOrder = clean.patch.surfaceOrder;
  return true;
}

/* Agent-authored workspace JSON is persisted, so tolerate common model aliases and
   repair older malformed workspaces before they reach the layout or controls UI. */
function normalizeControl(control: any, index: any) {
  const raw: any = control && typeof control === 'object' ? control : {};
  const rawDefault: any = raw.def !== undefined ? raw.def : raw.default !== undefined ? raw.default : raw.value;
  let type: any = text(raw.type).toLowerCase();
  if (!['slider', 'text', 'color', 'fill', 'toggle', 'select', 'button', 'readout', 'curve'].includes(type)) {
    if (Array.isArray(raw.options)) type = 'select';
    else if (typeof rawDefault === 'boolean') type = 'toggle';
    else if (typeof rawDefault === 'string' && /^#[0-9a-f]{3,8}$/i.test(rawDefault)) type = 'color';
    else if (typeof rawDefault === 'string') type = 'text';
    else type = 'slider';
  }
  const legacyParam: any = text(raw.parameter);
  const savedParam: any = text(raw.param);
  const generatedParam: any = /^Control \d+$/.test(savedParam);
  const authoredParam: any = generatedParam && legacyParam ? legacyParam : text(savedParam, legacyParam);
  const savedLabel: any = text(raw.label);
  const label: any = generatedParam && /^Control \d+$/.test(savedLabel) && legacyParam
    ? legacyParam
    : text(savedLabel, text(raw.name, text(authoredParam, `Control ${index + 1}`)));
  const out: any = { ...raw, type, label };
  if (type === 'readout') {
    out.source = ['selection.summary', 'selection.count', 'keyframes.summary', 'keyframes.count'].includes(raw.source) ? raw.source : 'selection.summary';
    return out;
  }
  if (type === 'button') {
    out.primary = raw.primary === true;
    out.action = PM.Capabilities?.sanitizeControlAction?.(raw.action) || null;
    out.cmd = text(raw.cmd, text(raw.command));
    return out;
  }
  out.stateKey = /^[a-z][a-z0-9_.-]{0,79}$/i.test(raw.stateKey || '') ? raw.stateKey : '';
  if (type === 'curve') {
    out.stateKey ||= `curve${index + 1}`;
    out.def = PM.Capabilities?.sanitizeCurve?.(rawDefault, [.62, .05, 0, 1]) || [.62, .05, 0, 1];
    out.minY = PM.clamp(finite(raw.minY) ? raw.minY : -1, -4, 0);
    out.maxY = PM.clamp(finite(raw.maxY) ? raw.maxY : 2, 1, 4);
    out.presets = (Array.isArray(raw.presets) ? raw.presets : Array.isArray(raw.options) ? raw.options : [])
      .filter((name: any) => typeof name === 'string' && PM.Ease?.PRESETS?.[name]).slice(0, 16);
    return out;
  }
  out.param = text(authoredParam, text(raw.name, label));
  if (type === 'text') out.def = rawDefault == null ? '' : String(rawDefault);
  else if (type === 'color') out.def = typeof rawDefault === 'string' ? rawDefault : '#FF6B1A';
  else if (type === 'fill') out.def = rawDefault && typeof rawDefault === 'object' ? copy(rawDefault) : null;
  else if (type === 'toggle') out.def = rawDefault === undefined ? false : !!rawDefault;
  else if (type === 'select') {
    out.options = Array.isArray(raw.options) ? raw.options.filter((v: any) => typeof v === 'string' || (v && typeof v === 'object' && 'v' in v && typeof v.label === 'string')).map((v: any) => typeof v === 'string' ? v : ({ v: v.v, label: v.label })) : [];
    out.def = rawDefault !== undefined ? rawDefault : (out.options[0] || '');
  } else {
    out.min = finite(raw.min) ? raw.min : 0;
    out.max = finite(raw.max) ? raw.max : Math.max(1, finite(rawDefault) ? Math.abs(rawDefault) * 2 : 1);
    if (out.max < out.min) [out.min, out.max] = [out.max, out.min];
    out.def = finite(rawDefault) ? rawDefault : out.min;
    out.step = finite(raw.step) && raw.step > 0 ? raw.step : Math.max((out.max - out.min) / 100, .01);
  }
  return out;
}

function isLegacyBrokenStagger(panel: any) {
  if (!panel || panel.id !== 'layer-stagger-tools' || panel.title !== 'Layer Stagger') return false;
  const controls: any = Array.isArray(panel.controls) ? panel.controls : [];
  const commands: any = controls.map((control: any) => control?.cmd || control?.command || '');
  return commands.length === 7 && commands.join('|') === 'selectAll|deselect|prevEdge|nextEdge|prevFrame|nextFrame|split';
}

function isLegacyBrokenTextSplitter(panel: any) {
  if (!panel || panel.title !== 'Text Splitter') return false;
  const labels: any = (Array.isArray(panel.controls) ? panel.controls : []).map((control: any) => control?.label || '');
  return labels.includes('Duplicate source layer')
    && labels.includes('Add text layer')
    && labels.includes('Undo last split step');
}

function normalizeWorkspace(workspace: any, fallback?: any) {
  const raw: any = workspace && typeof workspace === 'object' ? copy(workspace) : {};
  const backup: any = fallback && typeof fallback === 'object' ? copy(fallback) : null;
  raw.schemaVersion = 1;
  raw.id = text(raw.id, PM.uid('ws'));
  raw.name = text(raw.name, 'Workspace');
  raw.density = ['compact', 'normal', 'comfy'].includes(raw.density) ? raw.density : 'normal';
  raw.theme = raw.theme && typeof raw.theme === 'object' ? raw.theme : {};
  raw.chrome = raw.chrome && typeof raw.chrome === 'object' ? raw.chrome : {};
  raw.chrome.previewCornerRadius = raw.chrome.previewCornerRadius === 'rounded' ? 'rounded' : 'square';
  /* Surface schema 3 restores the panel-first hierarchy: gutter/ruler on the
     panel surface, tracks one step down. (Schema 2's reversed default made the
     dark-theme gutter sit below the window color and dissolve into the app
     background.) Existing workspaces migrate once; choices
     made after this version continue to round-trip normally. */
  if (raw.chrome.timelineSurfaceSchema !== 3) raw.chrome.timelineSurfaceOrder = 'normal';
  else raw.chrome.timelineSurfaceOrder = raw.chrome.timelineSurfaceOrder === 'reversed' ? 'reversed' : 'normal';
  raw.chrome.timelineSurfaceSchema = 3;
  /* Compact the old stock Timeline once, while preserving any authored or
     agent-authored Timeline configuration that differs from the old default. */
  if (raw.chrome.timelineChromeSchema !== TIMELINE_CHROME_SCHEMA && isLegacyTimelineChrome(raw.chrome.timeline)) {
    raw.chrome.timeline = copy(TIMELINE_DEFAULTS);
  }
  raw.chrome.timelineChromeSchema = TIMELINE_CHROME_SCHEMA;
  raw.chrome.timeline = normalizeTimelineChrome(raw.chrome.timeline);
  raw.features = raw.features && typeof raw.features === 'object' ? raw.features : {};
  delete raw.features.motionBlur;
  delete raw.features.guides;
  raw.scope = raw.scope === 'project' ? 'project' : 'global';
  raw.projectId = raw.scope === 'project' ? text(raw.projectId, PM.proj?.id || '') : null;
  raw.custom = (Array.isArray(raw.custom) ? raw.custom : []).map((panel: any, index: any) => {
    const source: any = panel && typeof panel === 'object' ? panel : {};
    const legacyStagger: any = isLegacyBrokenStagger(source);
    const legacyTextSplitter: any = isLegacyBrokenTextSplitter(source);
    const recipeId: any = legacyStagger ? 'layer-stagger' : legacyTextSplitter ? 'decompose-text' : text(source.tool);
    const recipe: any = PM.Capabilities?.panelRecipe?.(recipeId) || null;
    const cp: any = recipe ? {
      ...recipe, ...source,
      tool: recipe.id,
      note: legacyStagger || legacyTextSplitter ? recipe.note : text(source.note, recipe.note),
      state: { ...(recipe.state || {}), ...(source.state && typeof source.state === 'object' ? source.state : {}) },
      controls: legacyStagger || legacyTextSplitter
        ? recipe.controls
        : Array.isArray(source.controls) && source.controls.length ? source.controls : recipe.controls,
    } : source;
    const state: any = Object.fromEntries(Object.entries(cp.state && typeof cp.state === 'object' ? cp.state : {})
      .filter(([key, value]: any) => /^[a-z][a-z0-9_.-]{0,79}$/i.test(key) && (value === null || ['string', 'number', 'boolean'].includes(typeof value)))
      .slice(0, 64));
    return {
      ...cp,
      id: text(cp.id, `custom-${raw.id}-${index + 1}`),
      title: text(cp.title, text(cp.name, 'Controls')),
      size: finite(cp.size) ? PM.clamp(cp.size, 72, 1200) : 220,
      state,
      controls: (Array.isArray(cp.controls) ? cp.controls : []).map(normalizeControl),
    };
  });
  raw.hiddenPanels = (Array.isArray(raw.hiddenPanels) ? raw.hiddenPanels : []).map((item: any) => {
    const hidden: any = item && typeof item === 'object' ? item : {};
    const spec: any = hidden.spec && typeof hidden.spec === 'object' ? hidden.spec : { id: hidden.id };
    return {
      id: text(hidden.id, text(spec.id)), dockId: text(hidden.dockId, 'right'),
      index: Math.max(0, Math.floor(finite(hidden.index) ? hidden.index : 0)),
      dockIndex: Math.max(0, Math.floor(finite(hidden.dockIndex) ? hidden.dockIndex : 0)),
      spec: { ...spec, id: text(spec.id, text(hidden.id)) },
      dock: hidden.dock && typeof hidden.dock === 'object' ? hidden.dock : null,
    };
  }).filter((item: any) => item.id && item.id !== 'viewer' && item.id !== 'library');

  let docks: any = raw.layout && Array.isArray(raw.layout.docks) ? raw.layout.docks : null;
  if (!docks || !docks.some((d: any) => d && Array.isArray(d.panels) && d.panels.length)) {
    docks = backup && backup.layout && Array.isArray(backup.layout.docks)
      ? backup.layout.docks
      : [{ id: 'center', panels: [{ id: 'viewer', flex: true }] }];
  }
  const usedPanels: any = new Set();
  docks = docks.map((item: any, index: any) => {
    const d: any = item && typeof item === 'object' ? item : {};
    const panels: any = (Array.isArray(d.panels) ? d.panels : []).map((spec: any) => {
      const q: any = typeof spec === 'string' ? { id: spec } : (spec && typeof spec === 'object' ? spec : {});
      const id: any = text(q.id);
      /* Narrowly retire obsolete dock panels. Saved Section/Look content lives
         in project.library and is intentionally untouched by this migration. */
      if (!id || id === 'chat' || id === 'layers' || id === 'library' || usedPanels.has(id)) return null;
      usedPanels.add(id);
      const clean: any = { id };
      if (q.flex) clean.flex = true;
      if (q.collapsed) clean.collapsed = true;
      if (finite(q.size)) clean.size = PM.clamp(q.size, 56, 1600);
      if (finite(q.min)) clean.min = PM.clamp(q.min, 32, 800);
      if (text(q.title)) clean.title = text(q.title);
      return clean;
    }).filter(Boolean);
    const out: any = { id: text(d.id, `dock-${index + 1}`), panels };
    /* A visible dock must consume its full height. Keep the saved pixel size as
       metadata, but let one panel flex so a short side panel cannot reserve a
       large blank column beneath itself. */
    if (panels.length && !panels.some((panel: any) => panel.flex)) panels[panels.length - 1].flex = true;
    if (finite(d.size)) out.size = PM.clamp(d.size, 200, 760);
    if (d.hidden) out.hidden = true;
    if (d.flex) out.flex = true;
    return out;
  }).filter((d: any) => d.panels.length);
  if (!docks.length) docks = [{ id: 'center', panels: [{ id: 'viewer', flex: true }], flex: true }];

  /* A workspace is never allowed to hide the composition completely. This is
     the recovery surface for every layout edit and generated panel. */
  if (!docks.some((d: any) => d.panels.some((panel: any) => panel.id === 'viewer'))) {
    const target: any = docks.find((d: any) => d.id === 'center') || docks.find((d: any) => d.flex) || docks[0];
    target.panels.unshift({ id: 'viewer', flex: true });
  }
  const visibleIds: any = new Set(docks.flatMap((d: any) => d.panels.map((panel: any) => panel.id)));
  raw.hiddenPanels = raw.hiddenPanels.filter((item: any) => !visibleIds.has(item.id));

  /* The layout must always have one dock that consumes unused width. Models often
     call it "main" or "canvas" instead of "center"; the viewer/timeline dock is
     the semantic center regardless of its authored name. */
  const fluid: any = docks.find((d: any) => d.id === 'center')
    || docks.find((d: any) => d.panels.some((q: any) => q.id === 'viewer' || q.id === 'timeline'))
    || docks.find((d: any) => d.flex)
    || docks[0];
  fluid.flex = true;
  raw.layout = { ...(raw.layout && typeof raw.layout === 'object' ? raw.layout : {}), docks };
  return raw;
}

const PRESETS: any = () => ([
  {
    id: 'design', name: 'Design', builtin: true, density: 'normal',
    theme: { accent: '#FF6B1A' },
    features: { snapping: true, autosave: true, adaptiveQuality: true },
    layout: {
      docks: [
        /* Default shape: project media above a tall agent on the left, the
           composition + timeline in the middle, and properties over effects
           on the right — the agent is a primary surface, not a footnote. */
        dock('left', [p('assets', { size: 220 }), p('agent', { flex: true })], 300),
        dock('center', [p('viewer', { flex: true }), p('timeline', { size: 340 })]),
        dock('right', [p('inspector', { flex: true }), p('fxbrowser', { size: 380 })], 320),
      ],
    },
  },
  {
    id: 'gradient', name: 'Gradient', builtin: true, density: 'compact',
    theme: { accent: '#FF6B1A', radius: 10 },
    features: { snapping: true, autosave: true, adaptiveQuality: true },
    layout: {
      docks: [
        dock('left', [p('gradient-controls', { flex: true }), p('assets', { size: 190 })], 300),
        dock('center', [p('viewer', { flex: true }), p('timeline', { size: 300 })]),
        dock('right', [p('inspector', { flex: true })], 340),
      ],
    },
    custom: [{
      id: 'gradient-controls', title: 'Gradient Editor', size: 220,
      controls: [
        { type: 'select', label: 'Type', target: '$composition', path: 'composition.background.type', options: ['linear', 'radial'], def: 'linear' },
        { type: 'color', label: 'Start color', target: '$composition', path: 'composition.background.startColor', def: '#FF6B1A' },
        { type: 'color', label: 'End color', target: '$composition', path: 'composition.background.endColor', def: '#34144F' },
        { type: 'slider', label: 'Angle', target: '$composition', path: 'composition.background.angle', min: -180, max: 180, def: 0, step: 1 },
        { type: 'slider', label: 'Midpoint', target: '$composition', path: 'composition.background.midpoint', min: 1, max: 100, def: 100, step: 1 },
      ],
    }],
  },
  {
    id: 'animate', name: 'Animate', builtin: true, density: 'compact',
    theme: { accent: '#FF6B1A' },
    features: { snapping: true, autosave: true, graphOnOpen: true },
    layout: {
      docks: [
        dock('left', [p('takes', { flex: true }), p('assets', { size: 170 })], 230),
        dock('center', [p('viewer', { size: 300 }), p('timeline', { flex: true })]),
        dock('right', [p('inspector', { flex: true })], 320),
      ],
    },
  },
  {
    id: 'shaderlab', name: 'Shader Lab', builtin: true, density: 'compact',
    theme: { accent: '#4C8DFF' },
    features: { snapping: true, autosave: true },
    layout: {
      docks: [
        dock('left', [p('shader', { flex: true })], 460),
        dock('center', [p('viewer', { flex: true }), p('timeline', { size: 200 })]),
        dock('right', [p('inspector', { flex: true }), p('perf', { size: 190 })], 290),
      ],
    },
  },
  {
    id: 'edit', name: 'Edit', builtin: true, density: 'normal',
    theme: { accent: '#3FCF8E' },
    features: { snapping: true, autosave: true },
    layout: {
      docks: [
        dock('left', [p('assets', { flex: true }), p('fxbrowser', { size: 240 })], 250),
        dock('center', [p('viewer', { flex: true }), p('timeline', { size: 360 })]),
        dock('right', [p('inspector', { flex: true })], 280),
      ],
    },
  },
  {
    id: 'review', name: 'Review', builtin: true, density: 'comfy',
    theme: { accent: '#FF6B1A' },
    features: { snapping: true, autosave: true },
    layout: {
      docks: [
        dock('center', [p('viewer', { flex: true }), p('timeline', { size: 180 })]),
        dock('right', [p('notes', { flex: true })], 300),
      ],
    },
  },
  {
    id: 'focus', name: 'Focus', builtin: true, density: 'normal',
    theme: { accent: '#FF6B1A' },
    features: { snapping: true, autosave: true },
    layout: { docks: [dock('center', [p('viewer', { flex: true })])] },
  },
]);

const WS: any = {
  current: null,
  all: [],
  editing: null,
  trashKey: 'workspaceTrash',
  list: () => WS.all,
  get: (id: any) => WS.all.find((w: any) => w.id === id),
};
PM.WS = WS;

/* The first agent-authored Gradient workspace used a two-dock layout that put
   the canvas, timeline, Layers, and Inspector into one vertical stack. Replace
   only that recognizable legacy shape with the shared repaired preset. */
WS.isLegacyGradient = (w: any) => {
  if (!w || w.builtin || w.name !== 'Gradient') return false;
  const docks: any = w.layout && Array.isArray(w.layout.docks) ? w.layout.docks : [];
  const panelIds: any = docks.flatMap((d: any) => Array.isArray(d.panels) ? d.panels.map((q: any) => q && q.id) : []);
  return docks.length === 2
    && panelIds.includes('gradient-controls')
    && panelIds.includes('viewer')
    && panelIds.includes('timeline')
    && panelIds.includes('layers');
};

WS.init = () => {
  const saved: any = PM.store.get('workspaces', null);
  WS.all = saved && saved.length ? saved.map((w: any) => normalizeWorkspace(w)) : PRESETS();
  const lastSavedId: any = PM.store.get('workspace', 'design');
  const legacyGradientIds: any = new Set(WS.all.filter(WS.isLegacyGradient).map((w: any) => w.id));
  if (legacyGradientIds.size) WS.all = WS.all.filter((w: any) => !legacyGradientIds.has(w.id));
  /* Built-ins are product source, not user-authored state. Refresh an older saved
     built-in definition so fixes (including real source bindings) reach existing
     installs; custom workspaces with different ids remain untouched. */
  PRESETS().forEach((preset: any) => {
    const index: any = WS.all.findIndex((workspace: any) => workspace.id === preset.id && workspace.builtin);
    if (index >= 0) WS.all[index] = normalizeWorkspace(preset);
    else if (!WS.all.some((workspace: any) => workspace.id === preset.id)) WS.all.push(normalizeWorkspace(preset));
  });
  if (legacyGradientIds.size) WS.save();
  const lastId: any = legacyGradientIds.has(lastSavedId) ? 'gradient' : lastSavedId;
  WS.activate(WS.get(lastId) ? lastId : 'design', true);
};

WS.activate = (id: any, silent: any) => {
  const w: any = WS.get(id); if (!w) return;
  WS.current = w;
  /* Phase 4: routes through the patched public property */
  WS.registerCustom(w);
  applyFeatures(w);
  PM.Layout.apply(w);
  PM.store.set('workspace', id);
  PM.bus.emit('workspaces');
  if (!silent) PM.toast('Workspace · ' + w.name);
};

WS.snapshot = () => WS.current ? copy(WS.current) : null;
/** Full workspace state for Undo/Redo. This includes newly created workspaces,
    so undoing an agent-created workspace does not leave an orphan behind. */
WS.historySnapshot = () => ({
  currentId: WS.current?.id || 'design',
  all: copy(WS.all),
});
WS.restoreHistorySnapshot = (snapshot: any) => {
  if (!snapshot || !Array.isArray(snapshot.all)) return false;
  WS.all = snapshot.all.map((workspace: any) => normalizeWorkspace(workspace));
  WS.save();
  const target: any = WS.get(snapshot.currentId) || WS.get('design') || WS.all[0];
  if (!target) return false;
  WS.activate(target.id, true);
  return true;
};
WS.restoreSnapshot = (snapshot: any) => {
  if (!snapshot || typeof snapshot !== 'object') return WS.activate('design', true);
  const restored: any = normalizeWorkspace(snapshot);
  const index: any = WS.all.findIndex((item: any) => item.id === restored.id);
  if (index >= 0) WS.all[index] = restored; else WS.all.push(restored);
  WS.current = restored;
  /* Phase 4: routes through the patched public property */
  WS.registerCustom(restored); applyFeatures(restored); PM.Layout.apply(restored);
  PM.bus.emit('workspaces');
  return restored;
};

function applyFeatures(w: any) {
  const f: any = w.features || {};
  if (f.snapping !== undefined) PM.snap = !!f.snapping;
  if (f.adaptiveQuality !== undefined) PM.perf.auto = !!f.adaptiveQuality;
  if (f.graphOnOpen !== undefined && PM.TL) PM.TL.graph = !!f.graphOnOpen;
  window.document.documentElement.dataset.previewCorners = w.chrome?.previewCornerRadius === 'rounded' ? 'rounded' : 'square';
  window.document.documentElement.dataset.timelineSurfaces = w.chrome?.timelineSurfaceOrder === 'reversed' ? 'reversed' : 'normal';
  PM.invalidate();
}

WS.save = () => {
  PM.store.set('workspaces', WS.all);
  PM.bus.emit('workspaces');
};

/** Mutate the active workspace and re-apply. All agent UI edits funnel through here. */
WS.mutate = (fn: any, opts: any = {}) => {
  if (WS.editing) {
    const fallback: any = copy(WS.editing.draft);
    fn(WS.editing.draft);
    WS.editing.draft = normalizeWorkspace(WS.editing.draft, fallback);
    WS.current = WS.editing.draft;
    /* Phase 4: routes through the patched public property */
    WS.registerCustom(WS.current); applyFeatures(WS.current); PM.Layout.apply(WS.current);
    PM.bus.emit('workspaces');
    return WS.current;
  }
  const w: any = WS.current;
  if (w.builtin && !opts.inPlace) {
    const copy: any = JSON.parse(JSON.stringify(w));
    copy.id = PM.uid('ws');
    copy.name = w.name + ' (edited)';
    copy.builtin = false;
    WS.all.push(copy);
    WS.current = copy;
  }
  const fallback: any = copy(WS.current);
  fn(WS.current);
  const normalized: any = normalizeWorkspace(WS.current, fallback);
  const index: any = WS.all.findIndex((item: any) => item.id === WS.current.id);
  if (index >= 0) WS.all[index] = normalized;
  WS.current = normalized;
  WS.save();
  WS.activate(WS.current.id, true);
  return WS.current;
};

WS.create = (spec: any) => {
  const base: any = copy(WS.get(spec.base) || WS.get('design'));
  const w: any = Object.assign(base, spec, { id: PM.uid('ws'), builtin: false });
  w.name = spec.name || 'Workspace';
  const normalized: any = normalizeWorkspace(w, base);
  WS.all.push(normalized);
  WS.save();
  WS.activate(normalized.id);
  return normalized;
};

WS.remove = (id: any) => {
  const w: any = WS.get(id);
  if (!w || w.builtin) return PM.toast('Built-in workspaces can’t be deleted');
  const trash: any = WS.trashList().filter((item: any) => item.id !== id);
  trash.unshift({ ...copy(w), deletedAt: Date.now() });
  PM.store.set(WS.trashKey, trash);
  WS.all = WS.all.filter((x: any) => x.id !== id);
  WS.save();
  if (WS.current.id === id) WS.activate('design');
  PM.bus.emit('workspaces');
};

WS.trashList = () => {
  const list: any = PM.store.get(WS.trashKey, []);
  return Array.isArray(list) ? list : [];
};
WS.restore = (id: any) => {
  const item: any = WS.trashList().find((workspace: any) => workspace.id === id);
  if (!item) return null;
  const restored: any = normalizeWorkspace({ ...copy(item), deletedAt: undefined });
  WS.all.push(restored);
  PM.store.set(WS.trashKey, WS.trashList().filter((workspace: any) => workspace.id !== id));
  WS.save(); return restored;
};
WS.rename = (id: any, name: any) => {
  const workspace: any = WS.get(id);
  if (!workspace || workspace.builtin) return false;
  workspace.name = text(name, workspace.name); WS.save(); return true;
};
WS.duplicate = (id: any, options: any = {}) => {
  const source: any = WS.get(id); if (!source) return null;
  const workspace: any = normalizeWorkspace({
    ...copy(source), id: PM.uid('ws'), name: options.name || source.name + ' copy', builtin: false,
    scope: options.scope || source.scope || 'global', projectId: options.projectId || source.projectId || null,
  }, source);
  WS.all.push(workspace); WS.save(); return workspace;
};
WS.resetBuiltin = (id: any) => {
  const preset: any = PRESETS().find((workspace: any) => workspace.id === id); if (!preset) return false;
  const index: any = WS.all.findIndex((workspace: any) => workspace.id === id);
  if (index >= 0) WS.all[index] = normalizeWorkspace(preset);
  else WS.all.push(normalizeWorkspace(preset));
  WS.save(); if (WS.current?.id === id) WS.activate(id, true); return true;
};

WS.beginEdit = (id: any = WS.current.id) => {
  const source: any = WS.get(id); if (!source || WS.editing) return null;
  WS.editing = { sourceId: source.id, beforeId: WS.current.id, draft: normalizeWorkspace(copy(source), source) };
  WS.current = WS.editing.draft;
  /* Phase 4: routes through the patched public property */
  WS.registerCustom(WS.current); applyFeatures(WS.current); PM.Layout.apply(WS.current);
  PM.bus.emit('workspaces');
  return WS.editing.draft;
};
WS.cancelEdit = () => {
  if (!WS.editing) return false;
  const beforeId: any = WS.editing.beforeId; WS.editing = null;
  WS.activate(WS.get(beforeId) ? beforeId : 'design', true); return true;
};
WS.saveEdit = (asNew: any = false, name: any) => {
  if (!WS.editing) return null;
  const session: any = WS.editing;
  let saved: any = normalizeWorkspace(copy(session.draft), WS.get(session.sourceId));
  const source: any = WS.get(session.sourceId);
  if (asNew || source?.builtin) {
    saved.id = PM.uid('ws'); saved.name = text(name, source?.name + ' copy'); saved.builtin = false;
    WS.all.push(saved);
  } else {
    saved.id = source.id; saved.name = text(name, source.name); saved.builtin = false;
    const index: any = WS.all.findIndex((workspace: any) => workspace.id === source.id); WS.all[index] = saved;
  }
  WS.editing = null; WS.save(); WS.activate(saved.id, true); PM.toast('Workspace saved'); return saved;
};

WS.saveAsNew = () => {
  const inp: any = h('input', { value: WS.current.name.replace(' (edited)', '') + ' copy' });
  PM.modal({
    title: 'Save workspace', body: h('div.field', inp), width: 400,
    actions: [{ label: 'Cancel' }, {
      label: 'Save', pri: true, run: () => {
        const w: any = JSON.parse(JSON.stringify(WS.current));
        w.id = PM.uid('ws'); w.name = inp.value.trim() || 'Workspace'; w.builtin = false;
        WS.all.push(w); WS.save(); WS.activate(w.id);
      },
    }],
  });
  window.setTimeout(() => inp.focus(), 30);
};

WS.editJSON = () => {
  const ta: any = h('textarea.code', { style: { height: '420px', borderRadius: '8px' }, spellcheck: 'false' }, JSON.stringify(WS.current, null, 2));
  ta.addEventListener('keydown', (e: any) => e.stopPropagation());
  PM.modal({
    title: 'Workspace definition', body: ta, width: 680,
    actions: [{ label: 'Cancel' }, {
      label: 'Apply', pri: true, run: () => {
        try {
          const o: any = JSON.parse(ta.value);
          const i: any = WS.all.findIndex((w: any) => w.id === WS.current.id);
          o.id = WS.current.id;
          /* Raw definitions still cross the same validator as visual editing;
             malformed docks can never strand the Composition surface. */
          WS.all[i] = normalizeWorkspace(o, WS.current); WS.save(); WS.activate(o.id, true);
          PM.toast('Workspace updated');
        } catch (e: any) { PM.toast('Invalid JSON: ' + e.message, 3200); return false; }
      },
    }],
  });
};

function curveControl(ct: any, get: any, set: any) {
  const field: any = h('section.generated-curve-field');
  const head: any = h('div.generated-curve-head', h('span', ct.label), h('code'));
  const canvas: any = h('canvas.generated-curve-canvas', {
    width: 600, height: 300, tabindex: 0, role: 'group',
    'aria-label': `${ct.label}. Drag either handle to shape the curve. Press Enter to switch handles.`,
  });
  const minY: any = Number.isFinite(ct.minY) ? ct.minY : -1;
  const maxY: any = Number.isFinite(ct.maxY) ? ct.maxY : 2;
  let curve: any = PM.Capabilities?.sanitizeCurve?.(get(), [.62, .05, 0, 1]) || [.62, .05, 0, 1];
  let active: any = 0, dragging: any = false;
  const pad: any = { x: 34, y: 24 };
  const x: any = (value: any) => pad.x + value * (canvas.width - pad.x * 2);
  const y: any = (value: any) => canvas.height - pad.y - (value - minY) / (maxY - minY) * (canvas.height - pad.y * 2);
  const point: any = (event: any) => {
    const rect: any = canvas.getBoundingClientRect();
    return {
      x: (event.clientX - rect.left) * canvas.width / Math.max(1, rect.width),
      y: (event.clientY - rect.top) * canvas.height / Math.max(1, rect.height),
    };
  };
  const draw: any = () => {
    const context: any = canvas.getContext('2d'); if (!context) return;
    const style: any = window.getComputedStyle(canvas);
    const ink: any = style.getPropertyValue('--tx').trim() || '#222';
    const sub: any = style.getPropertyValue('--tx-3').trim() || '#888';
    const line: any = style.getPropertyValue('--line').trim() || 'rgba(127,127,127,.2)';
    const accent: any = style.getPropertyValue('--accent').trim() || '#ff6b1a';
    context.clearRect(0, 0, canvas.width, canvas.height);
    context.lineWidth = 1; context.strokeStyle = line;
    for (let column: any = 0; column <= 4; column++) {
      const gx: any = x(column / 4); context.beginPath(); context.moveTo(gx, pad.y); context.lineTo(gx, canvas.height - pad.y); context.stroke();
    }
    for (const value of [minY, 0, .5, 1, maxY]) {
      if (value < minY || value > maxY) continue;
      const gy: any = y(value); context.beginPath(); context.moveTo(pad.x, gy); context.lineTo(canvas.width - pad.x, gy); context.stroke();
    }
    const points: any = [[x(curve[0]), y(curve[1])], [x(curve[2]), y(curve[3])]];
    context.strokeStyle = sub; context.lineWidth = 2;
    context.beginPath(); context.moveTo(x(0), y(0)); context.lineTo(...points[0]); context.moveTo(x(1), y(1)); context.lineTo(...points[1]); context.stroke();
    const easing: any = PM.Ease?.bezier?.(...curve) || ((value: any) => value);
    context.strokeStyle = accent; context.lineWidth = 4; context.beginPath();
    for (let index: any = 0; index <= 120; index++) {
      const t: any = index / 120, px: any = x(t), py: any = y(easing(t));
      index ? context.lineTo(px, py) : context.moveTo(px, py);
    }
    context.stroke();
    points.forEach((handle: any, index: any) => {
      context.fillStyle = index === active ? accent : ink;
      context.beginPath(); context.arc(handle[0], handle[1], index === active ? 9 : 7, 0, Math.PI * 2); context.fill();
      context.strokeStyle = style.getPropertyValue('--bg-panel').trim() || '#fff'; context.lineWidth = 3; context.stroke();
    });
    const name: any = PM.Ease?.nameOf?.([curve[0], curve[1]], [curve[2], curve[3]]) || 'custom';
    head.querySelector('code').textContent = name === 'custom' ? curve.map((value: any) => PM.round(value, 2)).join('  ') : name;
    canvas.setAttribute('aria-valuetext', `${name}: ${curve.join(', ')}`);
  };
  const update: any = (next: any, commit: any = true) => {
    curve = PM.Capabilities?.sanitizeCurve?.(next, curve) || curve;
    if (commit) set([...curve]);
    draw();
  };
  const moveHandle: any = (event: any) => {
    const p: any = point(event);
    const nx: any = PM.clamp((p.x - pad.x) / (canvas.width - pad.x * 2), 0, 1);
    const ny: any = PM.clamp(minY + (canvas.height - pad.y - p.y) / (canvas.height - pad.y * 2) * (maxY - minY), minY, maxY);
    const next: any = [...curve]; next[active * 2] = PM.round(nx, 3); next[active * 2 + 1] = PM.round(ny, 3); update(next);
  };
  canvas.addEventListener('pointerdown', (event: any) => {
    if (event.button !== 0) return;
    const p: any = point(event), handles: any = [[x(curve[0]), y(curve[1])], [x(curve[2]), y(curve[3])]];
    active = Math.hypot(p.x - handles[1][0], p.y - handles[1][1]) < Math.hypot(p.x - handles[0][0], p.y - handles[0][1]) ? 1 : 0;
    dragging = true; canvas.setPointerCapture?.(event.pointerId); canvas.focus(); moveHandle(event); event.preventDefault();
  });
  canvas.addEventListener('pointermove', (event: any) => { if (dragging) moveHandle(event); });
  const release: any = (event: any) => { dragging = false; canvas.releasePointerCapture?.(event.pointerId); };
  canvas.addEventListener('pointerup', release); canvas.addEventListener('pointercancel', release);
  canvas.addEventListener('keydown', (event: any) => {
    if (event.key === 'Enter' || event.key === ' ') { active = active ? 0 : 1; draw(); event.preventDefault(); return; }
    const axis: any = ['ArrowLeft', 'ArrowRight'].includes(event.key) ? 0 : ['ArrowUp', 'ArrowDown'].includes(event.key) ? 1 : -1;
    if (axis < 0) return;
    const next: any = [...curve], direction: any = ['ArrowLeft', 'ArrowDown'].includes(event.key) ? -1 : 1;
    next[active * 2 + axis] += direction * (event.shiftKey ? .1 : .01); update(next); event.preventDefault();
  });
  const presets: any = h('div.generated-curve-presets');
  (ct.presets || []).forEach((name: any) => presets.appendChild(h('button', {
    title: `Use ${name} easing`, onclick: () => update(PM.Ease.PRESETS[name]),
  }, h('i'), h('span', name))));
  field.append(head, canvas, presets);
  field.sync = () => { curve = PM.Capabilities?.sanitizeCurve?.(get(), curve) || curve; draw(); };
  field.sync();
  return field;
}

/* ── custom panels authored by prompt ──────────────────── */
function registerCustom(w: any) {
  (w.custom || []).forEach((cp: any) => {
    PM.registerPanel(cp.id, {
      title: cp.title || 'Panel',
      size: cp.size || 200,
      build(body: any) {
        const wrap: any = h('div.insp');
        const toolState: any = { ...(cp.state || {}) };
        const defaults: any = { ...toolState };
        const syncs: any = [];
        let offs: any = [], saveTimer: any = 0;
        const preview: any = h('div.generated-tool-preview', { hidden: true });
        const persistState: any = () => {
          cp.state = { ...toolState };
          window.clearTimeout(saveTimer);
          saveTimer = window.setTimeout(() => WS.save(), 140);
        };
        const fmt: any = (path: any, value: any) => {
          if (typeof value !== 'number') return String(value);
          if (path === 'layer.from' || path === 'layer.duration') return `${Math.round(value * Math.max(1, PM.proj.fps || 30))}fr`;
          return String(PM.round(value, 3));
        };
        const showPreview: any = (result: any, applied: any = false) => {
          preview.textContent = ''; preview.hidden = false;
          preview.classList.toggle('error', !result?.ok);
          preview.appendChild(h('strong', result?.ok ? (applied ? 'Applied to source' : 'Preview') : 'Nothing changed'));
          preview.appendChild(h('p', result?.message || 'The tool could not produce a valid source change.'));
          if (result?.ok) {
            const list: any = h('ol');
            (result.changes || result.preview?.changes || []).slice(0, 8).forEach((change: any) => {
              if (change.description) list.appendChild(h('li', h('span', change.description)));
              else list.appendChild(h('li', h('span', change.name), h('code', `${fmt(change.path, change.before)} → ${fmt(change.path, change.after)}`)));
            });
            const total: any = (result.changes || result.preview?.changes || []).length;
            if (total > 8) list.appendChild(h('li.more', `${total - 8} more changes`));
            preview.appendChild(list);
          }
        };
        const sync: any = () => {
          if (!body.isConnected) { offs.forEach((off: any) => off()); offs = []; return; }
          syncs.forEach((sync: any) => { try { sync(); } catch (e: any) { } });
        };
        offs = ['draw:ui', 'project', 'history', 'sel', 'layers'].map((event: any) => PM.bus.on(event, sync));
        body.appendChild(wrap);
        if (cp.note) wrap.appendChild(h('div', { style: { color: 'var(--tx-3)', fontSize: '11.5px', lineHeight: 1.6, padding: '2px 4px 8px' } }, cp.note));
        (cp.controls || []).forEach((ct: any) => {
          if (ct.type === 'readout') {
            const value: any = h('span.generated-tool-readout');
            value.sync = () => {
              const selected: any = PM.selLayers?.() || [];
              value.textContent = ct.source === 'selection.count' ? String(selected.length)
                : ct.source === 'keyframes.count' ? String(PM.Capabilities?.selectedKeyframes?.().length || 0)
                  : ct.source === 'keyframes.summary' ? (PM.Capabilities?.keyframeSummary?.() || 'No keyframes selected')
                    : (PM.Capabilities?.selectionSummary?.() || `${selected.length} selected`);
            };
            value.sync(); syncs.push(value.sync); wrap.appendChild(PM.row(ct.label, value));
            return;
          }
          if (ct.type === 'button') {
            let busy: any = false;
            const button: any = h('button.chip' + (ct.primary ? '.solid' : ''), {
              style: { width: '100%', justifyContent: 'center', height: '28px', marginBottom: '4px' },
              onclick: async () => {
                if (busy) return;
                if (ct.action?.type === 'transform') {
                  const result: any = ct.action.mode === 'apply'
                    ? PM.Capabilities.apply(ct.action.transform, toolState, { label: ct.label, origin: 'generated-tool' })
                    : PM.Capabilities.preview(ct.action.transform, toolState);
                  showPreview(result, ct.action.mode === 'apply' && !!result.ok);
                  PM.toast(result.ok ? (ct.action.mode === 'apply' ? `${ct.label} applied` : 'Preview ready') : result.message);
                } else if (ct.action?.type === 'easing') {
                  const result: any = ct.action.mode === 'apply'
                    ? PM.Capabilities.applyEasing(ct.action, toolState, { label: ct.label, origin: 'generated-tool' })
                    : PM.Capabilities.previewEasing(ct.action, toolState);
                  showPreview(result, ct.action.mode === 'apply' && !!result.ok); sync();
                  PM.toast(result.ok ? (ct.action.mode === 'apply' ? `${ct.label} applied` : 'Preview ready') : result.message);
                } else if (ct.action?.type === 'script') {
                  busy = true; button.disabled = true;
                  preview.hidden = false; preview.textContent = '';
                  preview.appendChild(h('strong', ct.action.mode === 'apply' ? 'Running isolated tool…' : 'Preparing preview…'));
                  let result: any;
                  try {
                    result = ct.action.mode === 'apply'
                      ? await PM.Script.apply(ct.action.code, toolState, { label: ct.action.label || ct.label, origin: 'generated-script' })
                      : await PM.Script.preview(ct.action.code, toolState, { label: ct.action.label || ct.label });
                  } catch (error: any) {
                    result = { ok: false, message: String(error.message || error), changes: [] };
                  } finally { busy = false; }
                  showPreview(result, ct.action.mode === 'apply' && !!result.ok); sync();
                  PM.toast(result.ok ? (ct.action.mode === 'apply' ? `${ct.label} applied` : 'Preview ready') : result.message);
                } else if (ct.action?.type === 'history') {
                  const changed: any = PM.hist.undo();
                  showPreview({ ok: changed, message: changed ? 'Restored the previous editable source.' : 'There is no edit to undo.', changes: [] });
                } else if (ct.action?.type === 'reset') {
                  Object.keys(toolState).forEach((key: any) => delete toolState[key]); Object.assign(toolState, defaults); persistState(); preview.hidden = true; sync();
                } else if (Array.isArray(ct.commands)) {
                  /* Manifest command arrays are agent-authored data — they never
                     carry trust-bearing fields, whatever the manifest claims. */
                  const cmds: any = ct.commands.map((c: any) => {
                    const s: any = { ...c };
                    delete s.overrideLock; delete s.preserveHandEdits; delete s.markIntent;
                    return s;
                  });
                  PM.Edit.apply(cmds, { label: ct.label, origin: 'generated-ui' });
                }
                else if (ct.cmd) PM.cmd(ct.cmd);
                else if (ct.prompt) PM.toast('Shake the pointer and drag across this section to change it');
              },
            }, ct.label);
            button.sync = () => {
              if (busy) { button.disabled = true; return; }
              if (ct.action?.type === 'transform') {
                const selector: any = ct.action.transform.selector || { scope: 'selection', types: [] };
                button.disabled = !(PM.Capabilities?.resolveTargets?.(selector)?.length);
              } else if (ct.action?.type === 'script') button.disabled = !PM.Script?.canRun?.(ct.action);
              else if (ct.action?.type === 'easing') button.disabled = !(PM.Capabilities?.selectedKeyframes?.().length);
            };
            button.sync(); syncs.push(button.sync); wrap.appendChild(button);
            return;
          }
          const binding: any = sourceBinding(ct);
          const local: any = !!ct.stateKey;
          const param: any = binding || local ? null : ensureParam(ct);
          /* History may restore the project object. Resolve generated scene
             parameters by stable name so the mounted control follows undo,
             redo, agent edits, and direct edits to the current source. */
          const currentParam: any = () => param && (PM.proj.params[param.name] || param);
          const get: any = binding ? binding.get : local ? () => toolState[ct.stateKey] ?? ct.def : () => currentParam().value;
          const set: any = binding ? () => {} : local
            ? (v: any) => { toolState[ct.stateKey] = v; persistState(); preview.hidden = true; }
            : (v: any) => { const active: any = currentParam(); active.value = v; applyParam(active); };
          const edit: any = local ? { label: ct.label, local: true } : binding ? {
            label: ct.label, origin: 'generated-ui', command: binding.command,
          } : {
            label: ct.label, origin: 'generated-ui',
            command: (value: any) => ({ type: 'set_scene_parameter', name: param.name, value }),
          };
          let field: any;
          if (ct.type === 'curve') {
            field = curveControl(ct, get, set); if (field.sync) syncs.push(field.sync); wrap.appendChild(field); return;
          }
          if (ct.type === 'text') field = PM.textField(get, set, { ...edit, mono: false });
          else if (ct.type === 'color') field = PM.colorField(get, set, edit);
          else if (ct.type === 'fill') field = PM.fillField(get, set, { ...edit, fallback: PM.proj.bg });
          else if (ct.type === 'toggle') field = PM.toggleField(get, set, edit);
          else if (ct.type === 'select') field = PM.selectField(get, set, ct.options || [], edit);
          else {
            field = PM.numField(get, set,
              { ...edit, min: ct.min, max: ct.max, step: ct.step || ((ct.max - ct.min) / 100) || .01, precision: 3, unit: ct.unit });
          }
          if (field.sync) syncs.push(field.sync);
          wrap.appendChild(PM.row(ct.label, field));
        });
        wrap.appendChild(preview);
      },
    });
  });
}
function sourceBinding(ct: any) {
  const target: any = ct.target || (ct.binding && ct.binding.target);
  const path: any = ct.path || (ct.binding && ct.binding.path);
  if (!target || !path) return null;
  if (target === '$composition' || target === 'composition') {
    const direct: any = ({
      'composition.name': ['name', () => PM.proj.name],
      'composition.width': ['width', () => PM.proj.w],
      'composition.height': ['height', () => PM.proj.h],
      'composition.fps': ['fps', () => PM.proj.fps],
      'composition.duration': ['duration', () => PM.proj.dur],
      'composition.shutter': ['shutter', () => PM.proj.shutter ?? .5],
      'composition.background': ['background', () => PM.proj.bg],
      'composition.backgroundFill': ['backgroundFill', () => PM.normalizeFill(PM.proj.backgroundFill, PM.proj.bg)],
    } as any)[path];
    if (direct) return { get: direct[1], command: (value: any) => ({ type: 'set_composition', patch: { [direct[0]]: value } }) };
    if (path === 'composition.workArea.start' || path === 'composition.workArea.end') {
      const index: any = path.endsWith('.start') ? 0 : 1;
      return {
        get: () => PM.proj.work?.[index] ?? (index ? PM.proj.dur : 0),
        command: (value: any) => {
          const workArea: any = [...(PM.proj.work || [0, PM.proj.dur])];
          const frame: any = 1 / Math.max(1, PM.proj.fps || 30);
          workArea[index] = index === 0
            ? PM.clamp(Number(value), 0, Math.max(0, workArea[1] - frame))
            : PM.clamp(Number(value), Math.min(PM.proj.dur, workArea[0] + frame), PM.proj.dur);
          return { type: 'set_composition', patch: { workArea } };
        },
      };
    }
  }
  if ((target === '$composition' || target === 'composition') && path.startsWith('composition.background.')) {
    const key: any = path.slice('composition.background.'.length);
    const value: any = () => {
      const fill: any = PM.normalizeFill(PM.proj.backgroundFill, PM.proj.bg);
      if (key === 'type') return fill.type;
      if (key === 'startColor') return fill.stops[0]?.color || ct.def;
      if (key === 'endColor') return fill.stops[1]?.color || fill.stops[0]?.color || ct.def;
      if (key === 'angle') return fill.angle;
      if (key === 'midpoint') return fill.stops[1]?.position ?? 100;
      return ct.def;
    };
    const command: any = (next: any) => {
      let fill: any = PM.normalizeFill(PM.proj.backgroundFill, PM.proj.bg);
      if (key === 'type') fill = PM.normalizeFill({ ...fill, type: ['solid', 'linear', 'radial', 'none'].includes(next) ? next : 'solid' }, PM.proj.bg);
      else {
        /* Gradient stop controls always operate on a real two-stop source. */
        if (!['linear', 'radial'].includes(fill.type)) fill = PM.normalizeFill({ ...fill, type: 'linear' }, PM.proj.bg);
        if (key === 'startColor') fill.stops[0].color = next;
      else if (key === 'endColor') fill.stops[1].color = next;
      else if (key === 'angle') fill.angle = next;
      else if (key === 'midpoint') fill.stops[1].position = next;
      }
      return { type: 'set_composition', patch: { backgroundFill: fill } };
    };
    return { get: value, command };
  }
  const layer: any = () => target === '$selection' || target === 'selection' ? PM.firstSel() : (PM.L(target) || PM.byName(target));
  const fallback: any = ct.def;
  if (path.startsWith('content.')) {
    const key: any = path.slice('content.'.length);
    return {
      get: () => { const L: any = layer(); return L && L.d[key] !== undefined ? L.d[key] : fallback; },
      command: (value: any) => ({ type: 'set_content', target, patch: { [key]: value } }),
    };
  }
  if (path.startsWith('properties.') || path.startsWith('transform.')) {
    const channel: any = path.replace(/^properties\./, '').replace(/^transform\./, '');
    return {
      get: () => { const L: any = layer(); const p: any = L && PM.findProp(L, channel); return L && p ? PM.evP(L, p, PM.time, channel) : fallback; },
      command: (value: any) => ({ type: 'set_property', target, path: channel, value, time: PM.time, mode: 'auto', preserveHandEdits: false, markIntent: 'human' }),
    };
  }
  if (path.startsWith('layer.')) {
    const key: any = path.slice('layer.'.length);
    const sourceKey: any = ({ visible: 'on', locked: 'lock', duration: 'dur', motionBlur: 'mblur' } as any)[key] || key;
    return {
      get: () => { const L: any = layer(); return L && L[sourceKey] !== undefined ? L[sourceKey] : fallback; },
      command: (value: any) => ({ type: 'set_layer', target, patch: { [key]: value } }),
    };
  }
  return null;
}
function ensureParam(ct: any) {
  const name: any = ct.param || ct.label;
  const ps: any = PM.proj.params;
  if (!ps[name]) ps[name] = { name, label: ct.label, control: ct.type === 'slider' ? 'num' : ct.type, value: ct.def, min: ct.min, max: ct.max, options: ct.options };
  return ps[name];
}
function applyParam(param: any) {
  /* bound layers pick this up through param("name") in expressions */
  PM.touch(); PM.invalidate();
}
WS.registerCustom = registerCustom;
WS.sourceBinding = sourceBinding;
WS.normalize = normalizeWorkspace;
WS.isLegacyBrokenStagger = isLegacyBrokenStagger;
WS.timelineDefaults = TIMELINE_DEFAULTS;
WS.normalizeTimelineChrome = normalizeTimelineChrome;
WS.sanitizeInterfaceEdit = sanitizeInterfaceEdit;
WS.applyInterfaceEdit = applyInterfaceEdit;
}
