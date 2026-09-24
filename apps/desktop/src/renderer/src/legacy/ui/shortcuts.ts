import { installLayerMenu } from './layer-menu';
import { installParentPickwhip } from './parent-pickwhip';
import { evaluatedValue } from '../core/content-properties';
import { materializeSvgPaths } from '../core/svg-import';
import { inspectorService, timelineService, toolService, viewerService } from '../core/services';
/* Ported from js/ui/shortcuts.js — behavior-preserving.
 *
 * The command table lives in the kernel now:
 *   - `def()` registers a `CommandDefinition` under owner `legacy`;
 *     `PM.commands` is a live view over `kernel.commands` in the old
 *     `{ id, label, kb, run, cat }` shape (see kernel-view.ts).
 *   - keybindings are contributed by the built-in `keymap-default` extension
 *     and dispatched by the one key listener the kernel installs.
 */
import type { CommandDefinition } from '../../kernel/api';
import { IMPORT_DEFAULTS_SERVICE } from '../../kernel/import-defaults';
import { hasTextSelection, isFieldTarget, selectableTextRoot, selectTextContents } from '../../kernel/keychord';
import { ensureKernel, registryView } from '../kernel-view';
import type { PMRegistry } from '../registry';
import { bridge as hostBridge } from '../../kernel/bridge';

export const LEGACY_OWNER = 'legacy';

export function install(PM: PMRegistry): void {
installLayerMenu(PM);
installParentPickwhip(PM);
const h: any = PM.h;
const kernel = ensureKernel(PM);

/* One live registration per command id: re-registering (`def` on reinstall,
   or `PM.commands.palette.run = …` from overlays/install.ts) replaces rather
   than stacking, so the legacy layer never buries its own entries. */
const owned = new Map<string, { dispose(): void }>();
const put = (definition: CommandDefinition): void => {
  owned.get(definition.id)?.dispose();
  owned.set(definition.id, kernel.commands.register(LEGACY_OWNER, definition));
};

const def: any = (id?: any, label?: any, kb?: any, run?: any, cat: any = 'General', opts: any = {}) => {
  put({
    id, label, kb, category: cat,
    /* The kernel key listener treats a `false` return as "not handled" and
       skips preventDefault. Legacy always prevented the default, so a command
       that happens to return false must not change that. */
    run: (...a: any[]) => {
      const result = run(...a);
      return opts.preserveFalse ? result : result === false ? undefined : result;
    },
    ...(opts.when ? { when: opts.when } : {})
  });
};
const hidden = { when: () => false };
const preserveFalse = { preserveFalse: true };

const delegateCommand = (id: string, ...args: unknown[]): unknown => {
  const command = kernel.commands.get(id);
  return command ? command.run(...args) : false;
};

const activeTextField = (): boolean => {
  if (typeof document === 'undefined') return false;
  let active = document.activeElement;
  while (active?.shadowRoot?.activeElement) active = active.shadowRoot.activeElement;
  return isFieldTarget(active);
};

const readingText = () => typeof document !== 'undefined' ? selectableTextRoot(document.activeElement) : null;

const nativeEdit = (action: string): unknown => {
  const bridge: any = typeof window === 'undefined' ? null : (hostBridge() as any);
  if (typeof bridge?.nativeEdit === 'function') {
    bridge.nativeEdit(action);
    return true;
  }
  if (typeof document === 'undefined' || typeof document.execCommand !== 'function') return false;
  return document.execCommand(action);
};

PM.commands = registryView<CommandDefinition, any>(kernel.commands, {
  read: (_item, id) => commandView(kernel, id, put),
  write: (id, value: any) => {
    if (!value || typeof value !== 'object') return false;
    put({ ...value, id, category: value.cat ?? value.category ?? 'General' });
    return true;
  },
  remove: (id) => {
    owned.get(id)?.dispose();
    owned.delete(id);
    return true;
  }
});
PM.cmd = (id?: any, ...a: any[]) => {
  const c: any = kernel.commands.get(id);
  if (c) return c.run(...a);
  /* A missing command is deliberately an unhandled dispatch. Extensions can
     replace/remove commands while a stale keybinding is still in flight; the
     kernel uses `false` to let lower-priority bindings continue. */
  console.warn('no cmd', id);
  return false;
};

const center: any = () => ({ 'position.x': PM.proj.w / 2, 'position.y': PM.proj.h / 2 });
function addLayer(type?: any, opts: any = {}) {
  const from: any = opts.from == null ? PM.snapF(PM.time, PM.proj.fps) : opts.from;
  const result: any = PM.Edit.apply({
    type: 'add_layer', layerType: type, name: opts.name,
    from, duration: opts.dur == null ? Math.max(1, PM.proj.dur - from) : opts.dur,
    content: opts.d || {}, properties: opts.p || {}, color: opts.color, select: true,
  }, { label: 'New ' + type, origin: 'command' });
  const id: any = result.ok && result.data.results[0].data.id;
  return id ? PM.L(id) : null;
}
PM.addLayerCmd = addLayer;

/* ── layer creation ────────────────────────────────────── */
def('newText', 'New text layer', '⌘T', () => addLayer('text', { name: 'Headline', p: center() }), 'Create');
def('newSolid', 'New solid', '⌘Y', () => addLayer('solid', { name: 'Solid' }), 'Create');
def('newShape', 'New shape', '⌘⇧Y', () => addLayer('shape', { name: 'Shape', p: center() }), 'Create');
def('newNull', 'New null object', '⌘⌥⇧Y', () => addLayer('null', { name: 'Null', p: center() }), 'Create');
def('import', 'Import media…', '⌘I', () => PM.pickFiles(), 'Create');
def('importSequence', 'Import image sequence…', null, () => PM.pickFiles(true), 'Create');
const selectTool = (tool: string, detail?: string): boolean => {
  const tools = toolService(PM);
  if (!tools) return false;
  tools.setTool(tool, detail);
  return true;
};
def('toolSelect', 'Selection tool', 'V', () => selectTool('select'), 'Tool');
def('toolHand', 'Hand tool', 'H', () => selectTool('hand'), 'Tool');
def('toolZoom', 'Zoom tool', 'Z', () => selectTool('zoom'), 'Tool');
def('toolRotate', 'Rotation tool', 'W', () => selectTool('rotate'), 'Tool');
def('toolAnchor', 'Pan Behind (Anchor Point) tool', 'Y', () => selectTool('anchor'), 'Tool');
def('toolShape', 'Shape tool', 'Q', () => {
  const tools = toolService(PM);
  if (!tools) return false;
  const shapes: any[] = ['rect', 'rounded', 'ellipse', 'polygon', 'star'];
  const current: any = shapes.includes(tools.toolShape) ? tools.toolShape : 'rect';
  const next: any = tools.tool === 'shape' ? shapes[(shapes.indexOf(current) + 1) % shapes.length] : current;
  tools.setTool('shape', next);
}, 'Tool');
def('renderQueue', 'Render queue', '', () => PM.Export?.queueDialog?.(), 'File');
def('toolPen', 'Pen tool', 'G', () => selectTool('pen'), 'Tool');
def('toolText', 'Horizontal Type tool', '⌘T', () => selectTool('text'), 'Tool');
def('centerAnchor', 'Center anchor point in layer content', '⌘⌥Home', () => {
  const commands: any[] = [];
  for (const layer of PM.selLayers().filter((item: any) => !item.lock)) {
    const bounds: any = PM.GL.bounds(layer, PM.time); if (!bounds) continue;
    const anchorX: any = PM.ev(layer, 'anchor.x', PM.time);
    const anchorY: any = PM.ev(layer, 'anchor.y', PM.time);
    const nextX: any = (bounds.x0 + bounds.x1) / 2;
    const nextY: any = (bounds.y0 + bounds.y1) / 2;
    const matrix: any = PM.localMatrix(layer, PM.time);
    const positionX: any = PM.ev(layer, 'position.x', PM.time);
    const positionY: any = PM.ev(layer, 'position.y', PM.time);
    const dx: any = nextX - anchorX, dy: any = nextY - anchorY;
    commands.push(
      { type: 'set_property', target: layer.id, path: 'anchor.x', value: nextX, time: PM.time, mode: 'auto', preserveHandEdits: false, markIntent: 'human' },
      { type: 'set_property', target: layer.id, path: 'anchor.y', value: nextY, time: PM.time, mode: 'auto', preserveHandEdits: false, markIntent: 'human' },
      { type: 'set_property', target: layer.id, path: 'position.x', value: positionX + matrix[0] * dx + matrix[2] * dy, time: PM.time, mode: 'auto', preserveHandEdits: false, markIntent: 'human' },
      { type: 'set_property', target: layer.id, path: 'position.y', value: positionY + matrix[1] * dx + matrix[3] * dy, time: PM.time, mode: 'auto', preserveHandEdits: false, markIntent: 'human' },
    );
  }
  if (!commands.length) return false;
  return PM.Edit.apply(commands, { label: 'Center anchor point', origin: 'command' });
}, 'Tool');
const nativeCommandForAsset = (id?: any, at: any = PM.time) => {
  const meta: any = PM.proj.assets[id]; if (!meta) return;
  const a: any = PM.assets?.get?.(id) || meta;
  if (a.kind === 'model') {
    const definition = a.layerDefinition || 'powermove.3d.obj-model';
    if (!PM.layerDefinition?.(definition)) return;
    return {
      type: 'add_layer', layerType: 'extension', name: a.name,
      from: PM.snapF(at, PM.proj.fps),
      duration: Math.max(1 / PM.proj.fps, PM.proj.dur - at),
      content: { definition, data: { assetId: id, objects: [{ id: 'model', assetId: id }] } },
      select: true,
    };
  }
  if (a.kind === 'image' && a.format === 'svg' && a.svg?.paths?.length) {
    return {
      type: 'add_layer', layerType: 'shape', name: String(a.name || 'SVG').replace(/\.svg$/i, ''),
      from: PM.snapF(at, PM.proj.fps),
      duration: Math.max(1 / PM.proj.fps, PM.proj.dur - at),
      content: {
        paths: materializeSvgPaths(PM, a.svg),
        svgSourceAsset: id,
        svgWidth: a.svg.width,
        svgHeight: a.svg.height,
        svgWarnings: a.svg.warnings,
      },
      select: true,
    };
  }
  const type: any = a.kind === 'audio' ? 'audio' : a.kind === 'video' ? 'video' : 'image';
  const content: any = type === 'audio'
    ? { asset: id, trim: 0, gain: 1, fadeIn: 0, fadeOut: 0 }
    : type === 'video'
      ? { asset: id, trim: 0, speed: 1, embeddedAudio: a.hasAudio === true, w: a.w || PM.proj.w, h: a.h || PM.proj.h }
      : { asset: id, w: a.w || PM.proj.w, h: a.h || PM.proj.h };
  return {
    type: 'add_layer', layerType: type, name: a.name,
    from: PM.snapF(at, PM.proj.fps),
    /* Keep the whole source even when it extends beyond the current comp. The
       composition still clips playback/export, but extending it later reveals
       the rest instead of permanently discarding the imported media. */
    duration: a.dur || Math.max(1 / PM.proj.fps, PM.proj.dur - at),
    content,
    select: true,
  };
};
PM.commandForAsset = (id?: any, at: any = PM.time) => {
  const command = nativeCommandForAsset(id, at);
  const defaults = kernel.services.get<import('../../kernel/api').ImportDefaults>(IMPORT_DEFAULTS_SERVICE);
  if (!command || !defaults || !['image', 'video', 'shape'].includes(command.layerType)) return command;
  // Measure a detached layer before insertion. No existing channel, keyframe,
  // selection, revision, or history entry is modified by the default itself.
  const layer = PM.mkLayer(command.layerType, { d: command.content });
  const bounds = PM.GL?.bounds?.(layer, command.from);
  if (!bounds) throw new Error('Import defaults could not measure the new layer bounds.');
  const x = bounds.x0 + (bounds.x1 - bounds.x0) * defaults.anchor.x;
  const y = bounds.y0 + (bounds.y1 - bounds.y0) * defaults.anchor.y;
  if (![x, y].every(Number.isFinite)) throw new Error('Import defaults received invalid layer bounds.');
  return { ...command, properties: {
    'anchor.x': x, 'anchor.y': y,
    // Compensate the new anchor so the imported content stays in place.
    'position.x': layer.p['position.x'].v + x - layer.p['anchor.x'].v,
    'position.y': layer.p['position.y'].v + y - layer.p['anchor.y'].v,
  } };
};
def('addFromAsset', 'Add layer from asset', null, (id?: any) => {
  const command: any = PM.commandForAsset(id);
  if (!command) return;
  const result: any = PM.Edit.apply(command, { label: 'New ' + command.layerType, origin: 'command' });
  const layerId: any = result.ok && result.data.results[0].data.id;
  return layerId ? PM.L(layerId) : null;
}, 'Create');

/* ── editing ───────────────────────────────────────────── */
def('duplicate', 'Duplicate layers', '⌘D', () => {
  const selected = selectedStackLayers(PM);
  if (!selected.length || selected.some((layer: any) => layer.lock || (PM.groupAncestors?.(layer) || []).some((group: any) => group.lock))) return;
  return pasteLayers(PM, () => selected, { atPlayhead: false });
}, 'Edit');
def('delete', 'Delete selection', '⌫', () => deleteSelection(PM), 'Edit');
def('split', 'Split at playhead', '⌘⇧D', () => splitLayers(PM), 'Edit');
def('separateAudio', 'Separate audio', null, (id?: any) => separateVideoAudio(PM, id), 'Edit', hidden);
def('selectAll', 'Select all layers', '⌘A', () => PM.selectLayers(PM.proj.layers.map((l: any) => l.id)), 'Edit');
def('deselect', 'Deselect', '⎋', () => { PM.selectLayers([]); PM.sel.keys = []; }, 'Edit');
def('groupLayers', 'Group layers', '⌘G', () => PM.Edit.apply({ type: 'group_layers', targets: PM.sel.layers }, { label: 'Group layers', origin: 'timeline' }), 'Edit');
def('ungroupLayers', 'Ungroup layers', '⌘⇧G', () => PM.Edit.apply({ type: 'ungroup_layers', targets: PM.sel.layers }, { label: 'Ungroup layers', origin: 'timeline' }), 'Edit');
def('precompose', 'Group layers', null, () => PM.cmd('groupLayers'), 'Edit', hidden);

/* ── layer clipboard ───────────────────────────────────── */
let layerClip: any = null;
def('copyLayers', 'Copy layers', '⌘C', () => {
  const inspector = inspectorService(PM);
  if (inspector?.copySelectedEffects()) return;
  const sels: any = selectedStackLayers(PM); if (!sels.length) return;
  /* An explicit layer copy becomes the active app-local clipboard payload.
     Effect rows stop propagation before this command, so copying an effect
     keeps the effect payload active while the user selects its destination. */
  inspector?.clearEffectClipboard();
  layerClip = sels.map((L: any) => JSON.parse(JSON.stringify(L)));
  PM.toast(`Copied ${layerClip.length} ${layerClip.length === 1 ? 'layer' : 'layers'}`);
}, 'Edit');
def('cutLayers', 'Cut layers', '⌘X', () => cutLayers(PM, (value: any[]) => { layerClip = value; }), 'Edit');
def('pasteLayers', 'Paste layers', '⌘V', () => {
  /* Effect paste deliberately routes through the ordinary global shortcut:
     select effect → ⌘C → select destination layer → ⌘V. */
  if (inspectorService(PM)?.pasteCopiedEffects()) return;
  return pasteLayers(PM, () => layerClip);
}, 'Edit');
def('contextUndo', 'Undo', null, () => activeTextField() ? nativeEdit('undo') : readingText() ? false : PM.cmd('undo'), 'Edit', hidden);
def('contextRedo', 'Redo', null, () => activeTextField() ? nativeEdit('redo') : readingText() ? false : PM.cmd('redo'), 'Edit', hidden);
def('contextCut', 'Cut', null, () => activeTextField() ? nativeEdit('cut') : readingText() ? false : PM.cmd('cutLayers'), 'Edit', hidden);
def('contextCopy', 'Copy', null, () => activeTextField() || readingText() || hasTextSelection() ? nativeEdit('copy') : PM.cmd('copyLayers'), 'Edit', hidden);
def('contextPaste', 'Paste', null, () => activeTextField() ? nativeEdit('paste') : readingText() ? false : PM.cmd('pasteLayers'), 'Edit', hidden);
def('contextSelectAll', 'Select all', null, () => { const root = readingText(); return activeTextField() ? nativeEdit('selectAll') : root ? selectTextContents(root) : PM.cmd('selectAll'); }, 'Edit', hidden);
def('toggleVisibility', 'Hide/show selected layers', null, () => toggleVisibility(PM), 'Edit');
def('toggleLayerControls', 'Show/hide layer controls', '⌘⇧H', () => toggleLayerControls(PM), 'View', preserveFalse);
def('lockSelectedLayers', 'Lock selected layers', '⌘L', () => setLayerLocks(PM, true), 'Edit');
def('unlockAllLayers', 'Unlock all layers', '⌘⇧L', () => setLayerLocks(PM, false, true), 'Edit');
def('selectPreviousLayer', 'Select previous layer', '⌘↑', () => selectAdjacentLayer(PM, -1), 'Edit');
def('selectNextLayer', 'Select next layer', '⌘↓', () => selectAdjacentLayer(PM, 1), 'Edit');
def('extendSelectionPreviousLayer', 'Extend selection to previous layer', '⌘⇧↑', () => selectAdjacentLayer(PM, -1, true), 'Edit');
def('extendSelectionNextLayer', 'Extend selection to next layer', '⌘⇧↓', () => selectAdjacentLayer(PM, 1, true), 'Edit');
def('bringForward', 'Bring forward', '⌘]', () => orderLayers(PM, 'forward'), 'Edit');
def('sendBackward', 'Send backward', '⌘[', () => orderLayers(PM, 'backward'), 'Edit');
def('bringToFront', 'Bring to front', '⌘⇧]', () => orderLayers(PM, 'front'), 'Edit');
def('sendToBack', 'Send to back', '⌘⇧[', () => orderLayers(PM, 'back'), 'Edit');
def('nudgeSelection', 'Nudge selection', null, (dx?: any, dy?: any) => nudgeSelection(PM, dx, dy), 'Edit');
def('undo', 'Undo', '⌘Z', () => PM.hist.undo(), 'Edit');
def('redo', 'Redo', '⌘⇧Z', () => PM.hist.redo(), 'Edit');

/* ── transport ─────────────────────────────────────────── */
def('play', 'Play / pause', '␣', () => PM.toggle(), 'Transport');
def('gotoStart', 'Go to start', '⇱', () => PM.setTime(0), 'Transport');
def('gotoEnd', 'Go to end', '⇲', () => PM.setTime(PM.proj.dur), 'Transport');
def('nextFrame', 'Next frame', 'Page Down', () => PM.step(1), 'Transport');
def('prevFrame', 'Previous frame', 'Page Up', () => PM.step(-1), 'Transport');
def('stepFrames', 'Step frames', null, (frames?: any) => {
  if (!Number.isFinite(Number(frames)) || Number(frames) === 0) return false;
  return PM.step(Number(frames));
}, 'Transport', hidden);
def('nextEdge', 'Next edge', '⇧→', () => { const edge = timelineService(PM)?.nextEdge(); if (Number.isFinite(edge)) PM.setTime(edge); }, 'Transport');
def('prevEdge', 'Previous edge', '⇧←', () => { const edge = timelineService(PM)?.prevEdge(); if (Number.isFinite(edge)) PM.setTime(edge); }, 'Transport');
def('nextVisibleEvent', 'Next visible timeline event', 'K', () => goToTimelineEvent(PM, 1), 'Transport');
def('prevVisibleEvent', 'Previous visible timeline event', 'J', () => goToTimelineEvent(PM, -1), 'Transport');
def('nextKeyframe', 'Next keyframe', '⌃→ / ⇧K', () => delegateCommand('timeline.adjacentKeyframe:next'), 'Transport', preserveFalse);
def('prevKeyframe', 'Previous keyframe', '⌃← / ⇧J', () => delegateCommand('timeline.adjacentKeyframe:prev'), 'Transport', preserveFalse);
def('nextSelectedEvent', 'Next selected timeline event', '⇧K', () => goToTimelineEvent(PM, 1, true), 'Transport');
def('prevSelectedEvent', 'Previous selected timeline event', '⇧J', () => goToTimelineEvent(PM, -1, true), 'Transport');
def('gotoLayerIn', 'Go to selected layer In point', 'I', () => goToSelectedLayerBoundary(PM, 'in'), 'Transport');
def('gotoLayerOut', 'Go to selected layer Out point', 'O', () => goToSelectedLayerBoundary(PM, 'out'), 'Transport');
def('gotoWorkIn', 'Go to work area start', '⇧Home', () => goToWorkAreaBoundary(PM, 'in'), 'Transport');
def('gotoWorkOut', 'Go to work area end', '⇧End', () => goToWorkAreaBoundary(PM, 'out'), 'Transport');
def('workIn', 'Work area in', 'B', () => PM.Edit.apply({ type: 'set_composition', patch: { workArea: [Math.min(PM.time, PM.proj.work[1] - 1 / PM.proj.fps), PM.proj.work[1]] } }, { label: 'Work area', origin: 'command' }), 'Transport');
def('workOut', 'Work area out', 'N', () => PM.Edit.apply({ type: 'set_composition', patch: { workArea: [PM.proj.work[0], Math.max(PM.time, PM.proj.work[0] + 1 / PM.proj.fps)] } }, { label: 'Work area', origin: 'command' }), 'Transport');

/* ── reveal properties (AE muscle memory) ──────────────── */
const propertyShortcuts = [
  ['p', 'revealPos', 'Position'], ['s', 'revealScale', 'Scale'],
  ['r', 'revealRot', 'Rotation'], ['t', 'revealOpacity', 'Opacity'],
  ['a', 'revealAnchor', 'Anchor point'], ['u', 'revealKeys', 'Animated properties'],
  ['m', 'revealMasks', 'Mask controls'], ['f', 'revealFeather', 'Mask feather'],
  ['e', 'revealEffects', 'Effects'], ['l', 'revealAudio', 'Audio levels'],
] as const;
for (const [key, id, label] of propertyShortcuts) {
  def(id, `Toggle ${label.toLowerCase()}`, key.toUpperCase(), (shift = false) => delegateCommand(`timeline.revealProperty:${key}`, shift), 'Reveal', preserveFalse);
}
def('revealAll', 'Toggle all layer properties', '⌘`', () => delegateCommand('timeline.revealAll'), 'Reveal', preserveFalse);
def('graph', 'Toggle graph editor', '⇧F3', () => { const timeline = timelineService(PM); if (!timeline) return; timeline.graph = !timeline.graph; PM.invalidate('timeline'); }, 'Reveal');

/* ── keyframes ─────────────────────────────────────────── */
/* sel.keys holds keyframe ids (Phase 3a); easing needs the live objects */
const easeTargets: any = () => PM.sel.keys.length ? PM.resolveSelectedKeys() : allSelKeys();
def('easyEase', 'Easy Ease', 'F9', () => PM.hist.do('Easy Ease', () => {
  PM.applyEaseTo(easeTargets(), 'easeInOut'); PM.invalidate();
}), 'Keyframes');
def('easyEaseIn', 'Easy Ease In', '⇧F9', () => PM.hist.do('Easy Ease In', () => {
  PM.applyEaseTo(easeTargets(), 'easeIn'); PM.invalidate();
}), 'Keyframes');
def('easyEaseOut', 'Easy Ease Out', '⌘⇧F9', () => PM.hist.do('Easy Ease Out', () => {
  PM.applyEaseTo(easeTargets(), 'easeOut'); PM.invalidate();
}), 'Keyframes');
def('nudgeKeyframes', 'Move selected keyframes', null, (frames?: any) => nudgeKeyframes(PM, frames), 'Keyframes', hidden);
function allSelKeys() {
  const out: any = [];
  PM.selLayers().forEach((L: any) => PM.allProps(L).forEach((p: any) => out.push(...p.prop.kf)));
  return out;
}

/* ── view / files ──────────────────────────────────────── */
def('fitView', 'Fit composition and timeline', '⇧F', () => {
  const viewer = viewerService(PM);
  const timeline = timelineService(PM);
  if (!viewer && !timeline) return false;
  if (viewer) {
    viewer.fit = true;
    if (Array.isArray(viewer.pan)) viewer.pan = [0, 0];
    viewer.layout();
  }
  timeline?.frameView();
  return true;
}, 'View', preserveFalse);
def('fitComposition', 'Fit composition', '⇧/', () => {
  const viewer = viewerService(PM);
  if (!viewer) return false;
  viewer.fit = true;
  if (Array.isArray(viewer.pan)) viewer.pan = [0, 0];
  viewer.layout();
}, 'View', preserveFalse);
def('zoomIn', 'Zoom in', '.', () => zoomViewer(PM, 1.25), 'View', preserveFalse);
def('zoomOut', 'Zoom out', ',', () => zoomViewer(PM, .8), 'View', preserveFalse);
def('actualSize', 'Actual size', '/', () => setViewerZoom(PM, 1), 'View', preserveFalse);
def('palette', 'Command palette', '⌘K', () => PM.palette(), 'View');
def('agent', 'Ask Powermove agent', '⌘⇧K', () => PM.SpatialAssistant?.open?.(), 'View');
def('settings', 'Settings…', '⌘,', () => PM.SettingsUI?.isOpen ? PM.SettingsUI.close?.() : PM.SettingsUI?.open?.(), 'View');
def('projectSettings', 'Project settings…', '⌘⇧,', () => PM.SettingsUI?.open?.('project'), 'File');
def('save', 'Save project', '⌘S', () => PM.saveProject(), 'File');
def('saveAs', 'Save project as…', '⌘⇧S', () => PM.saveProject({ saveAs: true }), 'File');
def('open', 'Open project…', '⌘O', () => PM.openProject(), 'File');
def('export', 'Export…', '⌘E', () => PM.Export.dialog(), 'File');
def('projects', 'Projects screen', '⌘P', () => PM.ProjectsScreen && PM.ProjectsScreen.toggle(), 'File');
def('newProject', 'New project', '⌘N', () => PM.newProject(), 'File');
def('takeSave', 'Save take', '', () => { PM.takes.save(); PM.toast('Take saved'); }, 'File');

/* ── AE layer timing ───────────────────────────────────── */
def('moveLayerIn', 'Move layer In point to current time', '[', () => editSelectedLayerTiming(PM, 'moveIn'), 'Edit');
def('moveLayerOut', 'Move layer Out point to current time', ']', () => editSelectedLayerTiming(PM, 'moveOut'), 'Edit');
def('trimIn', 'Trim In point to current time', '⌥[', () => editSelectedLayerTiming(PM, 'trimIn'), 'Edit');
def('trimOut', 'Trim Out point to current time', '⌥]', () => editSelectedLayerTiming(PM, 'trimOut'), 'Edit');
def('moveLayerInToStart', 'Move layer In point to composition start', '⌥Home', () => editSelectedLayerTiming(PM, 'inToStart'), 'Edit');
def('moveLayerOutToEnd', 'Move layer Out point to composition end', '⌥End', () => editSelectedLayerTiming(PM, 'outToEnd'), 'Edit');

/* ── keymap ────────────────────────────────────────────── */
/* The keymap-default extension binds Escape in fields to this hidden command.
   Returning false leaves the browser default alone after blurring the field. */
put({
  id: 'blurField',
  label: 'Blur focused field',
  category: 'Edit',
  when: () => false,
  run: () => {
    const active: any = typeof document === 'undefined' ? null : document.activeElement;
    const tag = typeof active?.tagName === 'string' ? active.tagName.toUpperCase() : '';
    if (tag === 'INPUT' || tag === 'TEXTAREA' || active?.isContentEditable) active.blur?.();
    return false;
  }
});
}

type LayerOrder = 'forward' | 'backward' | 'front' | 'back';

function currentLayers(PM: PMRegistry): any[] {
  const comp = typeof PM.curComp === 'function' ? PM.curComp() : PM.proj;
  return Array.isArray(comp?.layers) ? comp.layers : [];
}

function selectedStackLayers(PM: PMRegistry): any[] {
  const layers = currentLayers(PM);
  const ids = new Set(PM.expandGroups?.((PM.sel?.layers || []).filter(Boolean)) || (PM.sel?.layers || []).filter(Boolean));
  if (ids.size) return layers.filter((layer: any) => ids.has(layer.id));
  return typeof PM.selLayers === 'function' ? PM.selLayers().filter(Boolean) : [];
}

function selectLayerIds(PM: PMRegistry, ids: any[]): void {
  if (typeof PM.selectLayers === 'function') {
    PM.selectLayers(ids);
    return;
  }
  PM.sel = PM.sel || { layers: [], keys: [] };
  PM.sel.layers = [...ids];
  PM.bus?.emit?.('sel');
  PM.invalidate?.();
}

function finishLayerMutation(PM: PMRegistry): void {
  if (PM.Edit?.mutate) return;
  PM.bus?.emit?.('layers');
  /* PM.selectLayers already invalidates when a command changes selection. */
  if (typeof PM.selectLayers !== 'function') PM.invalidate?.();
}

function runAtomic(PM: PMRegistry, label: string, action: () => unknown): unknown {
  if (PM.Edit?.mutate) {
    return PM.Edit.mutate(label, action, { origin: 'command' });
  }
  if (PM.hist && typeof PM.hist.do === 'function') return PM.hist.do(label, action);
  return action();
}

function cloneForCommand(PM: PMRegistry, layer: any): any {
  const clone = typeof PM.cloneLayer === 'function'
    ? PM.cloneLayer(layer)
    : JSON.parse(JSON.stringify(layer));
  return clone;
}

function ensureLayerId(PM: PMRegistry, layer: any, used: Set<any>, source?: any): void {
  if (layer.id && layer.id !== source?.id && !used.has(layer.id)) {
    used.add(layer.id);
    return;
  }
  const next = typeof PM.uid === 'function' ? PM.uid('L') : `${source?.id || 'layer'}-copy`;
  layer.id = next;
  used.add(next);
}

function rebaseTailAnimation(PM: PMRegistry, layer: any, offset: number): void {
  if (!Number.isFinite(offset) || offset === 0 || typeof PM.allProps !== 'function') return;
  for (const { prop } of PM.allProps(layer)) {
    if (!Array.isArray(prop?.kf)) continue;
    for (const keyframe of prop.kf) {
      if (Number.isFinite(Number(keyframe.t))) keyframe.t = Number(keyframe.t) - offset;
    }
  }
  /* Expressions deliberately remain layer-local: `t` restarts on the new
     clip, while authors who need composition continuity can use global `T`.
     Keyframes, by contrast, are rebased so their value is continuous. */
}

function splitSpan(PM: PMRegistry, layer: any): { from: number; dur: number } {
  const value = layer?.type === 'group' && typeof PM.groupSpan === 'function'
    ? PM.groupSpan(layer)
    : layer;
  return { from: Number(value?.from), dur: Number(value?.dur) };
}

function crossesPlayhead(span: { from: number; dur: number }, time: number): boolean {
  return Number.isFinite(span.from) && Number.isFinite(span.dur)
    && time > span.from && time < span.from + span.dur;
}

function splitLayerClone(PM: PMRegistry, layer: any, tail: any, time: number): void {
  const sourceEnd = Number(layer.from) + Number(layer.dur);
  const sourceOffset = time - Number(layer.from);
  tail.from = time;
  tail.dur = sourceEnd - time;
  rebaseTailAnimation(PM, tail, sourceOffset);
  if (PM.MediaTiming?.isTimed?.(layer)) {
    tail.d = tail.d && typeof tail.d === 'object' ? tail.d : {};
    if (typeof PM.MediaTiming.trimAtStart === 'function') {
      tail.d.trim = PM.MediaTiming.trimAtStart(layer, time);
    }
  }
  /* The cut is an internal boundary, not a new entrance/exit. Keep the
     original entrance on the head and the original exit on the tail. */
  layer.transitionOut = null;
  tail.transitionIn = null;
  if (layer.type === 'audio') {
    if (layer.d && typeof layer.d === 'object') layer.d.fadeOut = 0;
    if (tail.d && typeof tail.d === 'object') tail.d.fadeIn = 0;
  }
  layer.dur = time - Number(layer.from);
}

/**
 * Split the selected active layers, or all visible active root layers when
 * there is no selection. The tail is inserted immediately above its source,
 * preserving the source's parent and timed-media continuity.
 */
export function splitLayers(PM: PMRegistry): unknown {
  const layers = currentLayers(PM);
  const selected = selectedStackLayers(PM);
  const selectedIds = (PM.sel?.layers || []).filter(Boolean);
  const selectionRequested = Boolean(selectedIds.length || selected.length);
  const T = Number(PM.time);
  if (!Number.isFinite(T)) return false;

  const explicit = selectionRequested
    ? layers.filter((layer: any) => selectedIds.includes(layer.id))
    : layers.filter((layer: any) => !layer.parent && layer.on !== false);
  const explicitSet = new Set(explicit.map((layer: any) => layer.id));
  const groupRoots = explicit.filter((layer: any) => layer.type === 'group'
    && !(PM.groupAncestors?.(layer) || []).some((group: any) => explicitSet.has(group.id)));
  const groupedIds = new Set(groupRoots.flatMap((group: any) => PM.expandGroups?.([group.id]) || [group.id]));
  const ordinaryTargets = (selectionRequested ? selected : explicit)
    .filter((layer: any) => layer.type !== 'group' && !groupedIds.has(layer.id))
    .filter((layer: any) => !layer.lock && crossesPlayhead(splitSpan(PM, layer), T));
  const groupPlans = groupRoots.map((root: any) => {
    const ids = new Set(PM.expandGroups?.([root.id]) || [root.id]);
    const hierarchy = layers.filter((layer: any) => ids.has(layer.id));
    return { root, hierarchy };
  }).filter(({ root, hierarchy }: any) => crossesPlayhead(splitSpan(PM, root), T)
    // A group with one child is still a real editable layer strip.  Do not
    // reject it here: ⌘⇧D must split singleton groups just like larger
    // hierarchies.
    && hierarchy.length > 0
    && !hierarchy.some((layer: any) => layer.lock
      || (PM.groupAncestors?.(layer) || []).some((group: any) => group.lock && !hierarchy.includes(group))));
  if (!ordinaryTargets.length && !groupPlans.length) return false;

  return runAtomic(PM, 'Split', () => {
    const selectedTails: any[] = [];
    const used = new Set(layers.map((layer: any) => layer.id));
    for (const { root, hierarchy } of groupPlans) {
      const hierarchyIds = new Set(hierarchy.map((layer: any) => layer.id));
      const side = new Map<any, 'head' | 'tail' | 'both'>();
      for (const layer of hierarchy) {
        const span = splitSpan(PM, layer);
        side.set(layer, crossesPlayhead(span, T) ? 'both'
          : span.from >= T ? 'tail' : 'head');
      }

      /* Crossing rows get a real tail copy. Rows wholly after the playhead
         move into that tail hierarchy, while rows wholly before it stay in
         the head. This keeps staggered and nested groups as two coherent
         strips instead of leaving future-only rows attached to the head. */
      const tailById = new Map<any, any>();
      const clonePairs: Array<[any, any]> = [];
      for (const layer of hierarchy) {
        if (side.get(layer) === 'tail') {
          tailById.set(layer.id, layer);
          continue;
        }
        if (side.get(layer) !== 'both') continue;
        const index = layers.indexOf(layer);
        if (index < 0) continue;
        const tail = cloneForCommand(PM, layer);
        ensureLayerId(PM, tail, used, layer);
        splitLayerClone(PM, layer, tail, T);
        layers.splice(index, 0, tail);
        tailById.set(layer.id, tail);
        clonePairs.push([layer, tail]);
      }

      for (const layer of hierarchy) {
        if (side.get(layer) !== 'tail') continue;
        const tailGroup = tailById.get(layer.group);
        if (tailGroup) layer.group = tailGroup.id;
        const tailParent = tailById.get(layer.parent);
        if (tailParent) layer.parent = tailParent.id;
        const tailMatte = tailById.get(layer.matteSource);
        if (tailMatte) layer.matteSource = tailMatte.id;
      }
      for (const [source, tail] of clonePairs) {
        tail.group = tailById.get(source.group)?.id
          || (hierarchyIds.has(source.group) ? null : source.group);
        tail.parent = tailById.get(source.parent)?.id || source.parent || null;
        if (source.matteSource) tail.matteSource = tailById.get(source.matteSource)?.id || source.matteSource;
      }
      const tailRoot = tailById.get(root.id);
      if (tailRoot) selectedTails.push(tailRoot);
    }

    /* Ordinary layer targets follow stack order so each source/tail pair stays
       adjacent even when several selected layers are split in one operation. */
    for (const layer of ordinaryTargets) {
      const index = layers.indexOf(layer);
      if (index < 0) continue;
      const tail = cloneForCommand(PM, layer);
      ensureLayerId(PM, tail, used, layer);
      splitLayerClone(PM, layer, tail, T);
      layers.splice(index, 0, tail);
      selectedTails.push(tail);
    }
    if (!selectedTails.length) return false;
    PM.normalizeGroupStack?.();
    selectLayerIds(PM, selectedTails.map((layer: any) => layer.id));
    finishLayerMutation(PM);
    return selectedTails;
  });
}

/** Turn a combined video clip into adjacent video and audio layers. The two
 * source edits share one transaction, so Undo recombines them in one step. */
export function separateVideoAudio(PM: PMRegistry, layerId?: any): unknown {
  const video: any = layerId ? PM.L?.(layerId) : PM.firstSel?.();
  if (!video || video.type !== 'video' || video.d?.embeddedAudio !== true || !video.d.asset) return false;
  if (video.lock) {
    PM.toast?.(`Layer “${video.name}” is locked`);
    return false;
  }
  const layers = currentLayers(PM);
  const index = layers.indexOf(video);
  const result: any = PM.Edit.apply([
    { type: 'set_content', target: video.id, patch: { embeddedAudio: false } },
    {
      type: 'add_layer', layerType: 'audio', name: `${video.name} Audio`,
      from: video.from, duration: video.dur, index: index < 0 ? 0 : index,
      content: {
        asset: video.d.asset,
        trim: Math.max(0, Number(video.d.trim) || 0),
        gain: 1,
        fadeIn: 0,
        fadeOut: 0,
      },
      select: true,
    },
  ], { label: 'Separate audio', origin: 'command' });
  return result;
}

/** Cut only unlocked selected layers and retain the cut payload for paste. */
export function cutLayers(PM: PMRegistry, setClipboard: (value: any[]) => void = () => {}): unknown {
  const layers = currentLayers(PM);
  const selected = selectedStackLayers(PM);
  const editable = selected.filter((layer: any) => !layer.lock);
  if (!editable.length) {
    PM.toast?.('No unlocked layers to cut');
    return false;
  }

  const payload = editable.map((layer: any) => JSON.parse(JSON.stringify(layer)));
  setClipboard(payload);
  const ids = new Set(editable.map((layer: any) => layer.id));
  return runAtomic(PM, 'Cut layers', () => {
    const comp = typeof PM.curComp === 'function' ? PM.curComp() : PM.proj;
    if (comp === PM.proj && typeof PM.removeLayers === 'function') {
      /* The model helper also unparents survivors and garbage-collects unused
         nested compositions. */
      PM.removeLayers([...ids]);
    } else {
      comp.layers = layers.filter((layer: any) => !ids.has(layer.id));
      comp.layers.forEach((layer: any) => {
        if (ids.has(layer.parent)) layer.parent = null;
      });
      finishLayerMutation(PM);
    }
    const remainingSelection = selected.filter((layer: any) => !ids.has(layer.id)).map((layer: any) => layer.id);
    selectLayerIds(PM, remainingSelection);
    PM.toast?.(`Cut ${editable.length} ${editable.length === 1 ? 'layer' : 'layers'}`);
    return editable;
  });
}

/** Paste at the playhead beside the topmost selection, preserving relative timing and links. */
export function pasteLayers(PM: PMRegistry, getClipboard: () => any[] | null = () => null, options: { atPlayhead?: boolean } = {}): unknown {
  const clipboard = getClipboard();
  if (!Array.isArray(clipboard) || !clipboard.length) return false;
  const layers = currentLayers(PM);
  const selectedIds = new Set((PM.sel?.layers || []).filter(Boolean));
  const selectedIndexes = layers
    .map((layer: any, index: number) => selectedIds.has(layer.id) ? index : -1)
    .filter((index: number) => index >= 0);
  const insertAt = selectedIndexes.length ? Math.min(...selectedIndexes) : 0;
  const currentIds = new Set(layers.map((layer: any) => layer.id));
  // Group clocks are independent of their visible spans. Anchor to the copied
  // contents, then shift every clock equally so group animation stays in sync.
  const contents = clipboard.filter((layer: any) => layer.type !== 'group');
  const start = Math.min(...(contents.length ? contents : clipboard).map((layer: any) => Number(layer.from) || 0));
  const offset = options.atPlayhead === false ? 0 : (Number(PM.time) || 0) - start;

  return runAtomic(PM, 'Paste layers', () => {
    const pairs = clipboard.map((source: any) => [source, cloneForCommand(PM, JSON.parse(JSON.stringify(source)))]);
    const used = new Set(currentIds);
    const idMap = new Map<any, any>();
    pairs.forEach(([source, clone]: any) => {
      ensureLayerId(PM, clone, used, source);
      idMap.set(source.id, clone.id);
    });
    const pasted = pairs.map(([source, clone]: any) => {
      // Animation keys and media trim are layer-local and move with this start.
      clone.from = (Number(source.from) || 0) + offset;
      if (source.parent && idMap.has(source.parent)) clone.parent = idMap.get(source.parent);
      else clone.parent = source.parent && currentIds.has(source.parent) ? source.parent : null;
      clone.group = idMap.get(source.group) || (currentIds.has(source.group) ? source.group : null);
      if (source.matteSource && idMap.has(source.matteSource)) clone.matteSource = idMap.get(source.matteSource);
      return clone;
    });
    layers.splice(insertAt, 0, ...pasted);
    PM.normalizeGroupStack?.();
    selectLayerIds(PM, pasted.map((layer: any) => layer.id));
    finishLayerMutation(PM);
    PM.toast?.(`Pasted ${pasted.length} ${pasted.length === 1 ? 'layer' : 'layers'}`);
    return pasted;
  });
}

/** Toggle visibility of the editable portion of the current selection. */
export function toggleVisibility(PM: PMRegistry): unknown {
  const editable = selectedStackLayers(PM).filter((layer: any) => !layer.lock);
  if (!editable.length) return false;
  const show = editable.every((layer: any) => evaluatedValue(PM, layer, layer.on, PM.time, 'l.on') === false);
  const commands = editable.map((layer: any) => ({
    type: 'set_layer', target: layer.id, patch: { visible: show },
  }));
  if (PM.Edit?.apply) {
    return PM.Edit.apply(commands, { label: show ? 'Show layers' : 'Hide layers', origin: 'command' });
  }
  return runAtomic(PM, show ? 'Show layers' : 'Hide layers', () => {
    editable.forEach((layer: any) => { layer.on = show; });
    finishLayerMutation(PM);
    return editable;
  });
}

/** Hide selection boxes and handles without changing layer visibility. */
export function toggleLayerControls(PM: PMRegistry): unknown {
  const viewer = viewerService(PM);
  if (!viewer) return false;
  viewer.showControls = viewer.showControls === false;
  PM.invalidate?.('render');
  return viewer.showControls;
}

/** Lock the selection, or unlock every layer in the current composition. */
export function setLayerLocks(PM: PMRegistry, locked: boolean, all = false): unknown {
  const layers = (all ? currentLayers(PM) : selectedStackLayers(PM))
    .filter((layer: any) => Boolean(layer.lock) !== locked);
  if (!layers.length || !PM.Edit?.apply) return false;
  return PM.Edit.apply(
    layers.map((layer: any) => ({ type: 'set_layer', target: layer.id, patch: { locked } })),
    { label: locked ? 'Lock layers' : 'Unlock layers', origin: 'command' },
  );
}

/** Cmd/Ctrl+Up/Down follows the visible layer stack; Shift extends the range. */
export function selectAdjacentLayer(PM: PMRegistry, direction: -1 | 1, extend = false): unknown {
  const layers = currentLayers(PM);
  if (!layers.length) return false;
  const selected = new Set((PM.sel?.layers || []).filter(Boolean));
  const selectedIndexes = layers
    .map((layer: any, index: number) => selected.has(layer.id) ? index : -1)
    .filter((index: number) => index >= 0);
  const anchor = selectedIndexes.length
    ? (direction < 0 ? Math.min(...selectedIndexes) : Math.max(...selectedIndexes))
    : (direction < 0 ? layers.length : -1);
  const target = layers[anchor + direction];
  if (!target) return false;
  const ids = extend
    ? layers.filter((layer: any) => selected.has(layer.id) || layer === target).map((layer: any) => layer.id)
    : [target.id];
  selectLayerIds(PM, ids);
  return target;
}

type TimelineDirection = -1 | 1;
type LayerBoundary = 'in' | 'out';
type LayerTimingEdit = 'moveIn' | 'moveOut' | 'trimIn' | 'trimOut' | 'inToStart' | 'outToEnd';

function finiteTimelineTimes(values: unknown[], duration: number): number[] {
  return [...new Set(values
    .map(value => Number(value))
    .filter(value => Number.isFinite(value) && value >= 0 && value <= duration)
    .map(value => Math.round(value * 1e6) / 1e6))]
    .sort((a, b) => a - b);
}

/** Collect the same items AE treats as visible timeline events: composition
 * bounds, work-area ends, markers, layer boundaries, and property keyframes. */
export function timelineEventTimes(PM: PMRegistry, selectedOnly = false): number[] {
  const comp: any = typeof PM.curComp === 'function' ? PM.curComp() : PM.proj;
  const duration = Math.max(0, Number(comp?.dur) || 0);
  const selected = new Set((PM.sel?.layers || []).filter(Boolean));
  const layers = (Array.isArray(comp?.layers) ? comp.layers : [])
    .filter((layer: any) => !selectedOnly || selected.has(layer.id));
  if (selectedOnly && !layers.length) return [];

  const times: unknown[] = selectedOnly ? [] : [0, duration, ...(comp.work || [])];
  if (!selectedOnly) times.push(...(comp.markers || []).map((marker: any) => marker?.t));
  for (const layer of layers) {
    times.push(layer.from, Number(layer.from) + Number(layer.dur));
    for (const entry of PM.allProps?.(layer) || []) {
      for (const key of entry?.prop?.kf || []) times.push(Number(layer.from) + Number(key.t));
    }
  }
  return finiteTimelineTimes(times, duration);
}

export function goToTimelineEvent(PM: PMRegistry, direction: TimelineDirection, selectedOnly = false): unknown {
  if (direction !== -1 && direction !== 1) return false;
  const times = timelineEventTimes(PM, selectedOnly);
  const current = Number(PM.time) || 0;
  const next = direction > 0
    ? times.find(time => time > current + 1e-5)
    : [...times].reverse().find(time => time < current - 1e-5);
  if (!Number.isFinite(next)) return false;
  PM.setTime?.(next);
  return next;
}

/** I/O go to the earliest In and latest Out across the selected layers. */
export function goToSelectedLayerBoundary(PM: PMRegistry, boundary: LayerBoundary): unknown {
  const layers = selectedStackLayers(PM);
  if (!layers.length) return false;
  const times = layers.map((layer: any) => boundary === 'in'
    ? Number(layer.from)
    : Number(layer.from) + Number(layer.dur)).filter(Number.isFinite);
  if (!times.length) return false;
  const time = boundary === 'in' ? Math.min(...times) : Math.max(...times);
  PM.setTime?.(time);
  return time;
}

/** Shift+Home/End jumps to the preview range without changing it. This keeps
 * B/N as editing commands and makes auditioning a trimmed work area cheap. */
export function goToWorkAreaBoundary(PM: PMRegistry, boundary: LayerBoundary): unknown {
  const comp: any = typeof PM.curComp === 'function' ? PM.curComp() : PM.proj;
  const duration = Math.max(0, Number(comp?.dur) || 0);
  const work = Array.isArray(comp?.work) ? comp.work : [0, duration];
  const candidate = Number(work[boundary === 'in' ? 0 : 1]);
  if (!Number.isFinite(candidate)) return false;
  const time = Math.max(0, Math.min(duration, candidate));
  PM.setTime?.(time);
  return time;
}

/** Move or trim selected layer bars using AE's bracket-key model. Every
 * operation is one typed, undoable source edit; timed media retains continuity
 * when its In point is trimmed. */
export function editSelectedLayerTiming(PM: PMRegistry, mode: LayerTimingEdit): unknown {
  const frame = 1 / Math.max(1, Number(PM.proj?.fps) || 30);
  const now = Number(PM.time);
  if (!Number.isFinite(now)) return false;
  const commands: any[] = [];
  const targets = PM.transformRoots?.(PM.sel.layers) || selectedStackLayers(PM).filter((item: any) => !item.lock);
  for (const layer of targets) {
    const from = Number(layer.from), duration = Number(layer.dur), out = from + duration;
    if (!Number.isFinite(from) || !Number.isFinite(duration)) continue;
    if (mode === 'moveIn') {
      commands.push({ type: 'set_layer', target: layer.id, patch: { from: Math.max(0, now) } });
    } else if (mode === 'moveOut') {
      commands.push({ type: 'set_layer', target: layer.id, patch: { from: Math.max(0, now - duration) } });
    } else if (mode === 'inToStart') {
      commands.push({ type: 'set_layer', target: layer.id, patch: { from: 0 } });
    } else if (mode === 'outToEnd') {
      commands.push({ type: 'set_layer', target: layer.id, patch: { from: Math.max(0, Number(PM.proj.dur) - duration) } });
    } else if (mode === 'trimIn' && now > from && now < out) {
      commands.push({ type: 'set_layer', target: layer.id, patch: { from: now, duration: out - now } });
      if (PM.MediaTiming?.isTimed?.(layer)) {
        commands.push({ type: 'set_content', target: layer.id, patch: { trim: PM.MediaTiming.trimAtStart(layer, now) } });
      }
    } else if (mode === 'trimOut' && now > from) {
      commands.push({ type: 'set_layer', target: layer.id, patch: { duration: Math.max(frame, now - from) } });
    }
  }
  if (!commands.length || !PM.Edit?.apply) return false;
  const labels: Record<LayerTimingEdit, string> = {
    moveIn: 'Move layer In point', moveOut: 'Move layer Out point',
    trimIn: 'Trim layer In point', trimOut: 'Trim layer Out point',
    inToStart: 'Move layer to start', outToEnd: 'Move layer to end',
  };
  return PM.Edit.apply(commands, { label: labels[mode], origin: 'command' });
}

interface SelectedKeyframeEntry {
  comp: any;
  layer: any;
  prop: any;
  key: any;
}

function selectedKeyframeEntries(PM: PMRegistry): SelectedKeyframeEntry[] {
  const selected = new Set((PM.sel?.keys || []).filter(Boolean));
  if (!selected.size) return [];
  const entries: SelectedKeyframeEntry[] = [];
  const comps = [PM.proj];
  const seen = new Set<any>();
  while (comps.length) {
    const comp: any = comps.shift();
    if (!comp || seen.has(comp)) continue;
    seen.add(comp);
    for (const layer of comp.layers || []) for (const item of PM.allProps?.(layer) || []) {
      for (const key of item?.prop?.kf || []) if (selected.has(key?.i)) {
        entries.push({ comp, layer, prop: item.prop, key });
      }
    }
    comps.push(...Object.values(comp.comps || {}));
  }
  return entries;
}

/** Option/Alt+Left/Right moves selected keyframes by exact frames. The group
 * stops at composition bounds and selected keys replace destination collisions
 * in the same property, matching timeline drag semantics. */
export function nudgeKeyframes(PM: PMRegistry, frames?: any): unknown {
  const amount = Number(frames);
  const fps = Math.max(1, Number(PM.proj?.fps) || 30);
  if (!Number.isFinite(amount) || amount === 0) return false;
  const selected = selectedKeyframeEntries(PM);
  if (!selected.length) return false;

  let delta = Math.round(amount) / fps;
  const minDelta = Math.max(...selected.map(({ layer, key }) => -Number(layer.from) - Number(key.t)));
  const maxDelta = Math.min(...selected.map(({ comp, layer, key }) => Number(comp.dur) - Number(layer.from) - Number(key.t)));
  delta = Math.max(minDelta, Math.min(maxDelta, delta));
  delta = Math.round(delta * fps) / fps;
  if (Math.abs(delta) < 1e-12) return false;

  const selectedKeys = new Set(selected.map(entry => entry.key));
  const destinationFrames = new Map<any, Set<number>>();
  for (const entry of selected) {
    let destinations = destinationFrames.get(entry.prop);
    if (!destinations) destinationFrames.set(entry.prop, destinations = new Set());
    destinations.add(Math.round((Number(entry.key.t) + delta) * fps));
  }

  return PM.hist.do('Move keyframes', () => {
    for (const [prop, destinations] of destinationFrames) {
      prop.kf = prop.kf.filter((key: any) => selectedKeys.has(key)
        || !destinations.has(Math.round(Number(key.t) * fps)));
    }
    for (const entry of selected) entry.key.t = Math.round((Number(entry.key.t) + delta) * fps) / fps;
    for (const prop of destinationFrames.keys()) prop.kf.sort((a: any, b: any) => Number(a.t) - Number(b.t));
    PM.touch?.();
    PM.bus?.emit?.('sel');
    PM.invalidate?.();
  });
}

function stepLayerOrder(layers: any[], selected: Set<any>, movable: Set<any>, direction: 'forward' | 'backward'): boolean {
  let changed = false;
  if (direction === 'forward') {
    for (let index = 0; index < layers.length; index++) {
      const layer = layers[index];
      if (!movable.has(layer.id) || index === 0) continue;
      const previous = layers[index - 1];
      if (selected.has(previous.id) || previous.lock) continue;
      layers[index - 1] = layer;
      layers[index] = previous;
      changed = true;
    }
  } else {
    for (let index = layers.length - 1; index >= 0; index--) {
      const layer = layers[index];
      if (!movable.has(layer.id) || index === layers.length - 1) continue;
      const next = layers[index + 1];
      if (selected.has(next.id) || next.lock) continue;
      layers[index + 1] = layer;
      layers[index] = next;
      changed = true;
    }
  }
  return changed;
}

/** Reorder unlocked selected layers without crossing locked layer barriers. */
export function orderLayers(PM: PMRegistry, mode: LayerOrder): unknown {
  if (!['forward', 'backward', 'front', 'back'].includes(mode)) return false;
  const layers = currentLayers(PM);
  const selectedLayers = selectedStackLayers(PM);
  const selected = new Set(selectedLayers.map((layer: any) => layer.id));
  const movable = new Set(selectedLayers.filter((layer: any) => !layer.lock).map((layer: any) => layer.id));
  if (!movable.size) return false;

  const labels: Record<LayerOrder, string> = {
    forward: 'Bring forward', backward: 'Send backward',
    front: 'Bring to front', back: 'Send to back',
  };
  return runAtomic(PM, labels[mode], () => {
    let changed = false;
    if (mode === 'forward' || mode === 'backward') {
      changed = stepLayerOrder(layers, selected, movable, mode);
    } else {
      const direction = mode === 'front' ? 'forward' : 'backward';
      while (stepLayerOrder(layers, selected, movable, direction)) changed = true;
    }
    if (!changed) return false;
    finishLayerMutation(PM);
    return selectedLayers;
  });
}

function worldNudgeToLocal(PM: PMRegistry, layer: any, dx: number, dy: number): [number, number] {
  const parent = layer.parent && PM.L?.(layer.parent);
  const matrix = PM.transformParentMatrix?.(layer,PM.time) || (parent && PM.worldMatrix?.(parent, PM.time));
  if (!Array.isArray(matrix) || matrix.length < 4) return [dx, dy];
  const determinant = matrix[0] * matrix[3] - matrix[1] * matrix[2];
  if (!Number.isFinite(determinant) || Math.abs(determinant) < 1e-9) return [0, 0];
  return [
    (dx * matrix[3] - dy * matrix[2]) / determinant,
    (dy * matrix[0] - dx * matrix[1]) / determinant,
  ];
}

const NUDGE_HISTORY_GROUP = 'keyboard-nudge';
const nudgeBatches = new WeakMap<object, { mark: any; timer: ReturnType<typeof setTimeout> }>();

function beginNudgeHistory(PM: PMRegistry): boolean {
  if (!PM.hist?.mark || !PM.hist?.squash || nudgeBatches.has(PM as object)) return false;
  nudgeBatches.set(PM as object, {
    mark: PM.hist.mark(), timer: 0 as unknown as ReturnType<typeof setTimeout>,
  });
  return true;
}

function coalesceNudgeHistory(PM: PMRegistry): void {
  const batch = nudgeBatches.get(PM as object);
  if (!batch) {
    return;
  }
  clearTimeout(batch.timer);
  batch.timer = setTimeout(() => {
    PM.hist.squash(batch.mark, 'Nudge selection', NUDGE_HISTORY_GROUP);
    nudgeBatches.delete(PM as object);
  }, 180);
}

/** Move selected layer positions through atomic, keyframe-aware source edits. */
export function nudgeSelection(PM: PMRegistry, dx?: any, dy?: any): unknown {
  if (typeof dx !== 'number' || typeof dy !== 'number'
    || !Number.isFinite(dx) || !Number.isFinite(dy) || (dx === 0 && dy === 0)) return false;
  const commands: any[] = [];
  const targets = PM.transformRoots?.(PM.sel.layers) || selectedStackLayers(PM).filter((item: any) => !item.lock);
  for (const layer of targets) {
    const [localX, localY] = worldNudgeToLocal(PM, layer, dx, dy);
    for (const [path, delta] of [['position.x', localX], ['position.y', localY]] as const) {
      if (!delta || !layer.p?.[path]) continue;
      const current = typeof PM.ev === 'function' ? PM.ev(layer, path, PM.time) : layer.p[path].v;
      if (!Number.isFinite(Number(current))) continue;
      commands.push({
        type: 'set_property', target: layer.id, path, value: Number(current) + delta,
        time: PM.time, mode: 'auto', preserveHandEdits: false, markIntent: 'human',
      });
    }
  }
  if (!commands.length || !PM.Edit?.apply) return false;
  const meta = { label: 'Nudge selection', origin: 'command', historyGroup: NUDGE_HISTORY_GROUP };
  const startedBatch = beginNudgeHistory(PM);
  const result = PM.Edit.apply(commands, meta);
  if (result?.ok) coalesceNudgeHistory(PM);
  else if (startedBatch) nudgeBatches.delete(PM as object);
  return result;
}

const VIEWER_MIN_ZOOM = .05;
const VIEWER_MAX_ZOOM = 8;

export function setViewerZoom(PM: PMRegistry, zoom: any): unknown {
  const viewer = viewerService(PM);
  if (!viewer || typeof zoom !== 'number' || !Number.isFinite(zoom)) return false;
  if (typeof viewer.setZoom === 'function') return viewer.setZoom(zoom);
  const clamp = typeof PM.clamp === 'function'
    ? PM.clamp.bind(PM)
    : (value: number, min: number, max: number) => Math.max(min, Math.min(max, value));
  const wasFit = viewer.fit === true;
  viewer.fit = false;
  if (wasFit && Array.isArray(viewer.pan)) viewer.pan = [0, 0];
  viewer.zoom = clamp(zoom, VIEWER_MIN_ZOOM, VIEWER_MAX_ZOOM);
  viewer.layout?.();
  return viewer.zoom;
}

export function zoomViewer(PM: PMRegistry, factor: any): unknown {
  const viewer = viewerService(PM);
  if (!viewer || typeof factor !== 'number' || !Number.isFinite(factor) || factor <= 0) return false;
  const current = Number.isFinite(Number(viewer.shown)) ? Number(viewer.shown)
    : Number.isFinite(Number(viewer.zoom)) ? Number(viewer.zoom) : 1;
  return setViewerZoom(PM, current * factor);
}

/** Legacy `{ id, label, kb, run, cat }` façade over one kernel command. */
function commandView(kernel: ReturnType<typeof ensureKernel>, id: string, put: (def: CommandDefinition) => void): any {
  const FIELDS = ['id', 'label', 'kb', 'run', 'cat'];
  const read = (): CommandDefinition | undefined => kernel.commands.get(id);
  const field = (definition: any, key: string): unknown => (key === 'cat' ? definition.category ?? 'General' : definition[key]);
  return new Proxy(Object.create(null) as Record<string, unknown>, {
    get(target, key) {
      if (typeof key === 'symbol') return Reflect.get(target, key);
      const definition = read();
      return definition ? field(definition, key) : undefined;
    },
    set(target, key, value) {
      if (typeof key === 'symbol') return Reflect.set(target, key, value);
      const definition = read();
      if (!definition) return false;
      put(key === 'cat' ? { ...definition, category: value as string } : ({ ...definition, [key]: value } as CommandDefinition));
      return true;
    },
    has(target, key) {
      if (typeof key === 'symbol') return Reflect.has(target, key);
      return !!read() && FIELDS.includes(key);
    },
    ownKeys: () => [...FIELDS],
    getOwnPropertyDescriptor(target, key) {
      if (typeof key === 'symbol') return Reflect.getOwnPropertyDescriptor(target, key);
      const definition = read();
      if (!definition || !FIELDS.includes(key)) return undefined;
      return { value: field(definition, key), enumerable: true, configurable: true, writable: true };
    }
  });
}

/** A keyframe deletion never falls through to its owning layer, including a
 * held Delete key after the first keyframe has already been removed. */
export function deleteSelection(PM: PMRegistry): unknown {
  const timeline = timelineService(PM);
  if (PM.sel.keys.length || timeline?.keySelectionActive) {
    if (timeline) timeline.keySelectionActive = true;
    const ids = new Set(PM.sel.keys);
    if (!ids.size) return;
    return PM.hist.do('Delete keyframes', () => {
      PM.proj.layers.forEach((layer: any) => PM.allProps(layer).forEach(({ prop, key: path }: any) => {
        const remaining = prop.kf.filter((key: any) => !ids.has(key.i));
        if (prop.kf.length && !remaining.length) prop.v = PM.evP(layer, prop, PM.time, path);
        prop.kf = remaining;
      }));
      PM.sel.keys = [];
      PM.touch(); PM.bus.emit('sel'); PM.invalidate();
    });
  }
  return PM.Edit.apply({ type: 'delete_layers', targets: PM.sel.layers }, { label: 'Delete', origin: 'command' });
}

/** Replace the legacy command set as one in-place registry update during HMR. */
export function reloadShortcuts(PM: PMRegistry): void {
  const kernel = ensureKernel(PM);
  const batch = kernel.commands.batchChanges();
  try {
    kernel.commands.disposeOwner(LEGACY_OWNER);
    install(PM);
    kernel.commands.demoteOwner(LEGACY_OWNER);
  } finally {
    batch.dispose();
  }
}

if (import.meta.hot) {
  import.meta.hot.accept(next => {
    if (next && window.PM) next.reloadShortcuts(window.PM);
  });
}
