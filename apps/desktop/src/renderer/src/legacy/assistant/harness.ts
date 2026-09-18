import { createAgentCheckpoint } from './checkpoint';
import { openPanel, readPanel, interactPanel, panelBounds, preparePanelInput } from './panel-tools';
import { records as extensionRecords } from '../../kernel/extensions.svelte';
import { editVideo, videoAssets } from './video-editing';
import { validateEffect } from '../../kernel/glsl';
import { AGENT_RESPONSE_STYLE } from '../../../../shared/response-style';
/* Ported from js/assistant/harness.js — behavior-preserving. */
import type { PMRegistry } from '../registry';
import type {
  AgentToolContent,
  AgentToolRequestEvent,
  AgentToolResponseEvent
} from '../../../../shared/ipc';

export function install(PM: PMRegistry): void {
const MAX_COMMANDS = 80;
const MAX_REPAIRS = 2;
const MAX_KEYFRAMES = 80;
const SCENE_OPERATIONS = new Set([
  'set_property', 'replace_keyframes', 'set_easing', 'set_expression', 'set_content',
  'set_layer', 'set_composition', 'add_layer', 'delete_layers',
  'reorder_layer', 'group_layers', 'ungroup_layers', 'move_to_group', 'add_effect', 'remove_effect', 'set_effect', 'set_transition',
  'set_scene_parameter', 'add_marker', 'create_section', 'update_section',
  'transform_layers',
]);
const FIELDS: any = {
  set_property: ['type', 'target', 'path', 'value', 'time', 'mode', 'ease', 'hold', 'preserveHandEdits'],
  replace_keyframes: ['type', 'target', 'path', 'keyframes', 'replace', 'expression', 'preserveHandEdits'],
  set_easing: ['type', 'keyframes', 'curve'],
  set_expression: ['type', 'target', 'path', 'expression'],
  set_content: ['type', 'target', 'patch'],
  set_layer: ['type', 'target', 'patch'],
  set_composition: ['type', 'patch'],
  add_layer: ['type', 'id', 'layerType', 'name', 'from', 'duration', 'content', 'properties', 'color', 'index', 'select', 'parent', 'blend', 'motionBlur', 'visible', 'shy', 'collapsed'],
  delete_layers: ['type', 'target', 'targets'],
  group_layers: ['type', 'targets', 'name'],
  ungroup_layers: ['type', 'targets'],
  move_to_group: ['type', 'targets', 'group'],
  reorder_layer: ['type', 'target', 'index'],
  add_effect: ['type', 'target', 'effect', 'parameters', 'open'],
  remove_effect: ['type', 'target', 'effect'],
  set_effect: ['type', 'target', 'effect', 'patch'],
  set_transition: ['type', 'layer', 'edge', 'transition'],
  set_scene_parameter: ['type', 'name', 'label', 'control', 'value', 'min', 'max', 'options'],
  add_marker: ['type', 'id', 'time', 'name'],
  create_section: ['type', 'section'],
  update_section: ['type', 'sectionId', 'layers', 'thumb', 'at', 'version'],
  transform_layers: ['type', 'transform', 'state'],
};

const clone = (value: any) => value === undefined ? undefined : JSON.parse(JSON.stringify(value));
const text = (value: any, fallback: any, max: any) => typeof value === 'string' && value.trim()
  ? value.trim().slice(0, max) : fallback;

function safeTimes(values: any, fallback: any) {
  const dur = Math.max(.1, Number(PM.proj?.dur) || 10);
  const source = Array.isArray(values) && values.length ? values : fallback;
  return [...new Set((source || []).map(Number).filter(Number.isFinite)
    .map((value: any) => PM.clamp(value, 0, dur)).map((value: any) => PM.round(value, 3)))].slice(0, 5);
}

function defaultTimes() {
  const p = PM.proj;
  const start = Array.isArray(p.work) ? p.work[0] : 0;
  const end = Array.isArray(p.work) ? p.work[1] : p.dur;
  const selected = PM.selLayers?.() || [];
  const candidates = selected.length
    ? [PM.time, ...selected.flatMap((layer: any) => [layer.from, layer.from + layer.dur * .5, layer.from + layer.dur - 1 / p.fps])]
    : [start, start + (end - start) * .33, start + (end - start) * .67, Math.max(start, end - 1 / p.fps)];
  return safeTimes(candidates, [PM.time]);
}

function propertyDigest(layer: any, options: any = {}) {
  const offset = Math.max(0, Math.trunc(Number(options.propertyOffset) || 0));
  const limit = Math.max(1, Math.min(100, Math.trunc(Number(options.propertyLimit) || 100)));
  const keys = options.keyframeLimit === undefined ? 8 : Math.max(0, Math.min(MAX_KEYFRAMES, Math.trunc(Number(options.keyframeLimit) || 0)));
  const keyOffset = Math.max(0, Math.trunc(Number(options.keyframeOffset) || 0));
  return PM.allProps(layer).slice(offset, offset + limit).map((item: any) => ({
    path: item.key,
    value: PM.evP(layer, item.prop, PM.time, item.key),
    keyframeCount: item.prop.kf.length,
    keyframes: item.prop.kf.slice(keyOffset, keyOffset + keys).map((key: any) => ({
      time: PM.round(key.t, 3), compositionTime: PM.round(layer.from + key.t, 3),
      value: clone(key.v), hold: !!key.hold,
    })),
    expression: item.prop.expr || null,
    handEdited: !!(layer.locked_intent?.[item.key] || layer.locked_intent?.[item.key.split('.')[0]]),
  }));
}

function projectState(options: any = {}) {
  const p = PM.proj;
  const selectedIds = new Set(PM.sel?.layers || []);
  const layerOffset = Math.max(0, Math.trunc(Number(options.layerOffset) || 0));
  const layerLimit = Math.max(1, Math.min(20, Math.trunc(Number(options.layerLimit) || 12)));
  return {
    composition: {
      id: p.id, name: p.name, width: p.w, height: p.h, fps: p.fps,
      duration: p.dur, workArea: clone(p.work), background: p.backgroundFill || p.bg,
      playhead: PM.round(PM.time, 3), revision: Number(p.revision) || 0,
    },
    layerCount: p.layers.length,
    mediaAssets: videoAssets(PM),
    videoEditingTool: 'edit_video',
    selection: clone(PM.sel),
    layers: p.layers.filter((layer: any) => !options.layerId || layer.id === options.layerId).slice(layerOffset, layerOffset + layerLimit).map((layer: any) => ({
      index: p.layers.indexOf(layer), id: layer.id, name: layer.name, type: layer.type, from: layer.from,
      duration: layer.dur, visible: layer.on, locked: layer.lock, parent: layer.parent, group: layer.group || null,
      blend: layer.blend, motionBlur: layer.mblur, color: layer.color,
      content: Object.fromEntries(Object.entries(layer.d || {}).map(([key, value]) => [
        key,
        typeof value === 'string' ? value.slice(0, key === 'code' ? 8000 : 500) : clone(value),
      ])),
      masks: (layer.masks || []).map((m: any) => ({ id: m.id, shape: m.shape, mode: m.mode, hasPath: !!m.path, vertices: m.path?.vertices?.length || 0 })),
      propertyCount: PM.allProps(layer).length,
      properties: propertyDigest(layer, options),
      effects: (layer.fx || []).map((effect: any) => ({ id: effect.id, type: effect.type, enabled: effect.on })),
      textLayout: layer.type === 'text' && selectedIds.has(layer.id) && PM.textLayout
        ? PM.textLayout(layer, PM.time) : null,
    })),
    parameters: clone(p.params || {}),
    markers: clone(p.markers || []),
    availableOperations: [...SCENE_OPERATIONS],
    availableCapabilities: PM.Capabilities?.catalog?.() || null,
    pagination: { layerOffset, layerLimit, keyframeOffset: Number(options.keyframeOffset) || 0, propertyOffset: Number(options.propertyOffset) || 0, propertyLimit: Math.min(100, Number(options.propertyLimit) || 100), note: 'Properties and keyframes are sampled. Use layerOffset/layerLimit, layerId, propertyOffset/propertyLimit and keyframeOffset/keyframeLimit for focused reads. Full source is in inputs/powermove-project.json (run-start snapshot).' },
  };
}

async function capture(times = defaultTimes(), width = 480) {
  const chosen = safeTimes(times, defaultTimes());
  const images = [];
  for (const time of chosen) {
    await new Promise((resolve: any) => window.requestAnimationFrame(resolve));
    images.push(await (PM.Export.snapshotAsync?.(time, width) ?? PM.Export.snapshot(time, width)));
  }
  return { times: chosen, images };
}

async function observe(times?: any) {
  // Reading context must not decode frames or touch playback. Only explicit
  // agent-selected review times opt into capture; render_frames calls capture directly.
  const chosen = safeTimes(times, []);
  const frames = chosen.length ? await capture(chosen) : { times: [], images: [] };
  return { state: projectState(), ...frames };
}

function cleanCommand(raw: any) {
  let source: any = raw;
  if (typeof raw === 'string') {
    if (raw.length > 50_000) return null;
    try { source = JSON.parse(raw); } catch { return null; }
  }
  if (!source || typeof source !== 'object' || Array.isArray(source) || !SCENE_OPERATIONS.has(source.type)) return null;
  const out: any = {};
  for (const key of FIELDS[source.type]) if (source[key] !== undefined) out[key] = clone(source[key]);
  if (JSON.stringify(out).length > 50_000) return null;
  if (out.type === 'replace_keyframes') {
    if (!Array.isArray(out.keyframes)) return null;
    out.keyframes = out.keyframes.slice(0, MAX_KEYFRAMES);
  }
  /* Agent-authored layers are deterministic and reviewable regardless of where
     the human happened to leave the playhead. Interactive add-layer commands
     retain their existing playhead-relative behavior. */
  if (out.type === 'add_layer' && out.from === undefined) out.from = 0;
  if (out.type === 'set_easing') {
    if (!Array.isArray(out.keyframes)) return null;
    out.keyframes = out.keyframes.slice(0, MAX_KEYFRAMES);
  }
  if (out.type === 'create_section') {
    if (!out.section || !Array.isArray(out.section.layers)) return null;
    /* Bound the section like every other list, and never persist proto keys. */
    out.section = Object.fromEntries(Object.entries(out.section)
      .filter(([key]) => !['__proto__', 'prototype', 'constructor'].includes(key)));
    out.section.layers = out.section.layers.slice(0, 200);
  }
  if (out.type === 'update_section') {
    if (!out.sectionId || !Array.isArray(out.layers) || !out.layers.length) return null;
    out.layers = out.layers.slice(0, 200);
  }
  if (out.type === 'delete_layers' && Array.isArray(out.targets)) out.targets = out.targets.slice(0, 20);
  /* Model-authored edits preserve hand intent by default; explicit overrides survive. */
  if (out.type === 'set_property' || out.type === 'replace_keyframes') out.preserveHandEdits = out.preserveHandEdits !== false;
  return out;
}

function sanitizeProposal(raw: any) {
  const commands = (Array.isArray(raw?.commands) ? raw.commands : [])
    .slice(0, MAX_COMMANDS).map(cleanCommand).filter(Boolean);
  return {
    label: text(raw?.label, 'Agent composition edit', 80),
    summary: text(raw?.summary, 'A structured composition change is ready to review.', 260),
    commands,
    reviewTimes: safeTimes(raw?.reviewTimes, []),
    baseRevision: Number(PM.proj.revision) || 0,
  };
}

function describeCommand(command: any) {
  const target = command.target || (Array.isArray(command.targets) ? `${command.targets.length} layers` : '') || command.name || '';
  const detail = command.path || command.layerType || command.effect || Object.keys(command.patch || {}).join(', ');
  const labels: any = {
    set_property: 'Set property', replace_keyframes: 'Animate', set_easing: 'Set easing', set_expression: 'Set expression',
    set_content: 'Edit content', set_layer: 'Edit layer', set_composition: 'Edit composition',
    add_layer: 'Add layer', delete_layers: 'Delete', reorder_layer: 'Reorder layer',
    add_effect: 'Add effect', remove_effect: 'Remove effect', set_effect: 'Edit effect',
    set_scene_parameter: 'Set scene control', add_marker: 'Add marker',
    create_section: 'Create section', update_section: 'Update section',
    transform_layers: 'Transform layers',
  };
  return [labels[command.type] || command.type, target, detail].filter(Boolean).join(' · ').slice(0, 150);
}

function sceneSchema() {
  return {
    type: 'object', additionalProperties: false,
    required: ['label', 'summary', 'commands', 'reviewTimes'],
    properties: {
      label: { type: 'string' }, summary: { type: 'string' },
      commands: { type: 'array', maxItems: MAX_COMMANDS, items: { type: 'string' } },
      reviewTimes: { type: 'array', maxItems: 5, items: { type: 'number' } },
    },
  };
}

function reviewSchema() {
  return {
    type: 'object', additionalProperties: false,
    required: ['status', 'message', 'critique', 'commands', 'reviewTimes'],
    properties: {
      status: { type: 'string', enum: ['pass', 'repair'] },
      message: { type: 'string' }, critique: { type: 'string' },
      commands: { type: 'array', maxItems: MAX_COMMANDS, items: { type: 'string' } },
      reviewTimes: { type: 'array', maxItems: 5, items: { type: 'number' } },
    },
  };
}

function promptContext(observation: any) {
  return `\nLIVE COMPOSITION SOURCE\n${JSON.stringify(observation?.state || projectState())}\n\nRENDERED FRAME TIMES\n${JSON.stringify(observation?.times || [])}`;
}

function reviewPrompt(request: any, applied: any, pass: any, observation: any) {
  return `You are the review stage inside Powermove's bounded motion-editing harness. Judge the result against the user's request and the live semantic source. Frames are only attached when explicitly requested through reviewTimes; when there are no frames, do not claim visual verification. Leave reviewTimes empty unless you need to inspect specific moments.

USER REQUEST
${request}

APPLIED EDITS
${JSON.stringify(applied.map(describeCommand))}
${promptContext(observation)}

REVIEW CONTRACT
- Check brief fidelity, hierarchy, legibility, temporal development, transitions, dead time, and finish.
- status=pass when there is no clear repair required.
- status=repair only for an obvious, bounded defect you can fix with the typed edit language.
- Each commands item must be one JSON-encoded source-edit object. Never output JavaScript, shell commands, or project JSON.
- Preserve locked layers and hand-edited channels by default. When the user explicitly requests changing a hand-edited channel, use preserveHandEdits: false on that set_property or replace_keyframes command. Do not bypass this through inspector controls or broaden the user's scope.
- This is repair pass ${pass + 1} of ${MAX_REPAIRS}. Return no repair commands after the limit.
- message and critique follow the response style below: one or two sentences each, naming the layer or property at issue.

${AGENT_RESPONSE_STYLE}`;
}

async function execute(request: any, proposal: any, progress: any = () => {}) {
  const historyMark = PM.hist.mark?.() || null;
  const historyGroup = PM.uid('agent-history');
  const editStart = Array.isArray(PM.proj.edits) ? PM.proj.edits.length : 0;
  const checkpoint = createAgentCheckpoint(PM, `Before agent · ${text(request, 'composition edit', 42)}`);
  progress('Applying structured source edit…');
  const first = PM.Edit.apply(proposal.commands, {
    label: proposal.label, origin: 'agent', baseRevision: proposal.baseRevision, historyGroup,
  });
  if (!first.ok) throw new Error(first.message);
  /* Review passes must see what the policy actually executed (locked layers
     may have been skipped), not the raw proposal. Surface skips to the user. */
  const results = Array.isArray(first.data?.results) ? first.data.results : null;
  const applied = results ? results.map((item: any) => item.command) : proposal.commands.slice();
  const skipped = (results || []).flatMap((item: any) => item.data?.skippedLocked || []);
  if (skipped.length && first.message) progress(first.message.split('. ').find((part: any) => part.startsWith('Skipped locked')) || first.message);
  let review = { status: 'pass', message: 'The editable change is ready.', critique: '' };
  let frames = null;
  let reviewError = '';
  try {
    for (let pass = 0; pass <= MAX_REPAIRS; pass++) {
      progress(pass ? `Reviewing repair ${pass} of ${MAX_REPAIRS}…` : 'Reviewing the source edit…');
      frames = await observe(proposal.reviewTimes);
      const raw = await PM.CodexBridge.request(
        reviewPrompt(request, applied, pass, frames), reviewSchema(), frames.images,
        { onProgress: (summary: any) => progress(summary) },
      );
      review = JSON.parse(raw);
      const repair = sanitizeProposal(review);
      if (review.status !== 'repair' || !repair.commands.length || pass >= MAX_REPAIRS) break;
      progress(`Applying repair ${pass + 1} of ${MAX_REPAIRS}…`);
      const result = PM.Edit.apply(repair.commands, {
        label: `Agent repair ${pass + 1}`, origin: 'agent', baseRevision: Number(PM.proj.revision) || 0, historyGroup,
      });
      if (!result.ok) { reviewError = result.message; break; }
      applied.push(...repair.commands);
      proposal.reviewTimes = repair.reviewTimes;
    }
  } catch (error: any) {
    reviewError = String(error.message || error);
  }
  /* Refresh final source, capturing pixels only at agent-requested times. */
  try { frames = await observe(proposal.reviewTimes); }
  catch (error: any) {
    reviewError = reviewError || String(error.message || error);
    frames = frames || { state: projectState(), times: proposal.reviewTimes, images: [] };
  }
  const newEdits = (PM.proj.edits || []).slice(editStart);
  const agentOnly = newEdits.length > 0 && newEdits.every((edit: any) => edit.origin === 'agent');
  const historyId = agentOnly
    ? PM.hist.squash?.(historyMark, `Agent · ${proposal.label}`, historyGroup) || null
    : null;
  checkpoint.historyId = historyId;
  return { checkpoint, applied, review, frames, reviewError, historyId, revision: Number(PM.proj.revision) || 0 };
}

function rollback(checkpoint: any) {
  if (checkpoint.historyId && PM.hist.undoIfTop?.(checkpoint.historyId)) return true;
  const take = checkpoint?.takeId && PM.takes?.all?.().find((item: any) => item.id === checkpoint.takeId);
  const json = checkpoint?.json || take?.json;
  return !!json && !!PM.hist.restoreSnapshot(json, 'Undo agent run');
}

interface LiveToolTransaction {
  baseRevision: number;
  revision: number;
  historyMark: unknown;
  historyGroup: string;
  snapshot: string;
  label: string;
  changed: boolean;
  panelActions?: boolean;
}

const liveToolTransactions = new Map<string, LiveToolTransaction>();

function toolText(value: unknown): AgentToolContent {
  const serialized = JSON.stringify(value);
  if (serialized.length > 1_900_000) throw new Error('State exceeds the response budget. Read a single layerId with propertyLimit: 10 and keyframeLimit: 0, then page the properties/keyframes. Use capture_panel for visual inspection.');
  return { type: 'text', text: serialized };
}

function currentRevision(): number {
  return Math.max(0, Number(PM.proj?.revision) || 0);
}

function beginLiveTransaction(request: AgentToolRequestEvent, label = 'Agent composition edit'): LiveToolTransaction {
  const existing = liveToolTransactions.get(request.runId);
  if (existing) return existing;
  const revision = currentRevision();
  if (revision !== request.baseRevision) {
    throw new Error(`The project changed before the agent could edit it (expected revision ${request.baseRevision}, found ${revision}). Inspect the live project again and retry.`);
  }
  const transaction: LiveToolTransaction = {
    baseRevision: revision,
    revision,
    historyMark: PM.hist.mark?.() ?? null,
    historyGroup: PM.uid('native-agent-history-'),
    snapshot: JSON.stringify(PM.proj),
    label,
    changed: false
  };
  liveToolTransactions.set(request.runId, transaction);
  return transaction;
}

function dataUrlImage(value: unknown): AgentToolContent | null {
  if (typeof value !== 'string') return null;
  const match = /^data:(image\/(?:png|jpeg));base64,([A-Za-z0-9+/=]+)$/.exec(value);
  if (!match) return null;
  const binary = window.atob(match[2]!);
  const data = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) data[index] = binary.charCodeAt(index);
  return { type: 'image', data, mimeType: match[1]! as 'image/png' | 'image/jpeg' };
}

function panelLayoutDigest() {
  const workspace = PM.WS?.current;
  return {
    workspace: workspace ? { id: workspace.id, name: workspace.name } : null,
    docks: (workspace?.layout?.docks || []).map((dock: any) => ({
      id: String(dock.id || ''),
      size: Number.isFinite(dock.size) ? dock.size : null,
      panels: (dock.panels || []).map((panel: any) => ({
        id: String(panel.id || ''),
        title: String(panel.title || PM.PANELS?.[panel.id]?.title || panel.id || ''),
        collapsed: panel.collapsed === true,
        flex: panel.flex === true,
        size: Number.isFinite(panel.size) ? panel.size : null
      }))
    })),
    registeredPanels: Object.values(PM.PANELS || {}).map((panel: any) => ({
      id: String(panel.id || ''),
      title: String(panel.title || panel.id || '')
    })).filter((panel: any) => panel.id)
  };
}

async function rollBackLiveTransaction(transaction: LiveToolTransaction): Promise<void> {
  if (transaction.panelActions) throw new Error('Panel controls use their normal editor Undo. Their actions cannot be automatically rolled back; use the editor Undo command.');
  if (!transaction.changed) return;
  const revision = currentRevision();
  if (revision !== transaction.revision) {
    throw new Error('The project changed after the agent edit, so Powermove left both the user work and the agent transaction intact instead of restoring an older snapshot.');
  }
  if (!PM.hist.restoreSnapshot(transaction.snapshot, 'Roll back agent changes', 'agent')) {
    throw new Error('Powermove could not restore the pre-agent project snapshot.');
  }
  transaction.baseRevision = currentRevision();
  transaction.revision = transaction.baseRevision;
  transaction.historyMark = PM.hist.mark?.() ?? null;
  transaction.historyGroup = PM.uid('native-agent-history-');
  transaction.snapshot = JSON.stringify(PM.proj);
  transaction.changed = false;
}

async function handleLiveAgentTool(request: AgentToolRequestEvent): Promise<Omit<AgentToolResponseEvent, 'runId' | 'callId'>> {
  if (request.tool === 'validate_effect') {
    const definition = validateEffect(request.arguments.definition as Parameters<typeof validateEffect>[0]);
    return { ok: true, content: [toolText({
      valid: true, id: definition.id, parameterCount: definition.params.length,
      note: 'Definition validation only. This does not register the effect or compile/render its shader.'
    })], revision: currentRevision() };
  }
  if (request.tool === 'get_project_state') {
    const transaction = liveToolTransactions.get(request.runId);
    if (transaction?.panelActions) transaction.revision = currentRevision();
    return { ok: true, content: [toolText(projectState(request.arguments))], revision: currentRevision() };
  }
  if (request.tool === 'get_workspace_state') {
    const file = PM.projectFileState?.(PM.proj.id);
    const projects = PM.Projects?.list?.() || [];
    const state = {
      project: {
        id: PM.proj.id, name: PM.proj.name, revision: currentRevision(),
        layerCount: PM.proj.layers.length, file: file?.path || null, unsaved: file?.dirty ?? null,
      },
      /* Every project with an editor window, across all of them. `active` marks
         the one this window — the one the agent is running in — is editing. */
      openProjects: (PM.Projects?.openProjects?.() || []).map((id: string) => ({
        id, name: projects.find((p: any) => p.id === id)?.name || null, active: id === PM.proj.id,
      })),
      extensions: extensionRecords().map(record => ({
        id: record.id, version: record.manifest?.version, enabled: record.enabled, health: record.health,
      })),
      registeredEffects: (PM.Kernel?.effects?.list?.() || []).map((effect: any) => ({
        id: effect.id, label: effect.label, group: effect.group,
        parameters: effect.params.map((param: any) => param.k)
      })),
      projectsScreenOpen: PM.ProjectsScreen?.isOpen || false,
      selection: clone(PM.sel), layout: panelLayoutDigest(),
      viewport: { width: innerWidth, height: innerHeight, devicePixelRatio },
      errors: PM.agentDiagnostics || [],
    };
    return { ok: true, content: [toolText(state)], revision: currentRevision() };
  }
  if (request.tool === '__panel_bounds') return { ok: true, content: [toolText(panelBounds(PM, request.arguments.panelId))], revision: currentRevision() };
  if (request.tool === '__prepare_panel_input') {
    const target = preparePanelInput(PM, request.arguments);
    const transaction = beginLiveTransaction(request, 'Use panel');
    if (currentRevision() !== transaction.revision) throw new Error('The project changed. Read get_project_state before using another panel control.');
    transaction.panelActions = true;
    return { ok: true, content: [toolText(target)], revision: currentRevision() };
  }
  if (request.tool === 'get_panel_layout') {
    return { ok: true, content: [toolText(panelLayoutDigest())], revision: currentRevision() };
  }
  if (request.tool === 'get_panel_state' || request.tool === 'open_panel' || request.tool === 'interact_panel') {
    if (request.tool === 'open_panel') await openPanel(PM, request.arguments.panelId);
    if (request.tool !== 'interact_panel') return { ok: true, content: [toolText(readPanel(PM, request.arguments.panelId))], revision: currentRevision() };
    const transaction = beginLiveTransaction(request, 'Use panel');
    if (currentRevision() !== transaction.revision) throw new Error('The project changed. Read get_project_state before using another panel control.');
    // Arbitrary panel handlers may import asynchronously or use their own history.
    // Keep their native Undo entries; never snapshot-rollback an external panel action.
    const state = await interactPanel(PM, request.arguments, () => { transaction.panelActions = true; });
    transaction.revision = currentRevision();
    transaction.changed ||= transaction.revision !== transaction.baseRevision;
    return { ok: true, content: [toolText(state)], changed: transaction.changed, revision: transaction.revision };
  }
  if (request.tool === 'render_frames') {
    const rawTimes = Array.isArray(request.arguments.times) ? request.arguments.times : undefined;
    const times = rawTimes?.slice(0, 5).map(Number).filter(Number.isFinite);
    const width = PM.clamp(Math.round(Number(request.arguments.width) || 720), 160, 1280);
    const frames = await capture(times, width);
    const images = frames.images.map(dataUrlImage).filter((item: AgentToolContent | null): item is AgentToolContent => item !== null);
    return {
      ok: true,
      content: [toolText({ times: frames.times, width, rendered: images.length }), ...images],
      revision: currentRevision()
    };
  }
  if (request.tool === 'apply_commands' || request.tool === 'edit_video') {
    const rawCommands = Array.isArray(request.arguments.commands) ? request.arguments.commands : [];
    const commands = rawCommands.slice(0, MAX_COMMANDS).map(cleanCommand).filter(Boolean);
    if (request.tool === 'apply_commands' && !commands.length) throw new Error('No valid Powermove edit commands were supplied.');
    const label = text(request.arguments.label, 'Agent composition edit', 80);
    const transaction = beginLiveTransaction(request, label);
    const revision = currentRevision();
    if (revision !== transaction.revision) {
      throw new Error(`The project changed during the agent run (expected revision ${transaction.revision}, found ${revision}). No commands were applied.`);
    }
    const editMeta = {
      label,
      origin: 'agent',
      baseRevision: transaction.revision,
      historyGroup: transaction.historyGroup
    };
    const applied = request.tool === 'edit_video'
      ? editVideo(PM, request.arguments, editMeta)
      : PM.Edit.apply(commands, editMeta);
    if (!applied.ok) throw new Error(applied.message);
    transaction.label = label;
    transaction.revision = currentRevision();
    transaction.changed = true;
    return {
      ok: true,
      content: [toolText({
        applied: commands.map(describeCommand),
        clip: applied.data?.result ?? null,
        message: applied.message || 'Applied editable Powermove commands.',
        revision: transaction.revision
      })],
      changed: true,
      revision: transaction.revision
    };
  }
  if (request.tool === 'rollback_changes') {
    const transaction = liveToolTransactions.get(request.runId);
    if (transaction) await rollBackLiveTransaction(transaction);
    return {
      ok: true,
      content: [toolText({ rolledBack: true, revision: currentRevision() })],
      changed: false,
      revision: currentRevision()
    };
  }
  if (request.tool === '__finish_run') {
    const transaction = liveToolTransactions.get(request.runId);
    if (!transaction) {
      return { ok: true, content: [toolText({ changed: false })], changed: false, revision: currentRevision() };
    }
    const commit = request.arguments.commit === true;
    try {
      let historyId: string | undefined;
      if (transaction.panelActions) {
        transaction.changed ||= currentRevision() !== transaction.baseRevision;
        if (!commit && transaction.changed) return { ok: false, content: [], changed: true, revision: currentRevision(), error: 'Panel actions were preserved after the run stopped. Use the editor Undo command to undo project changes.' };
      } else if (!commit) {
        await rollBackLiveTransaction(transaction);
      } else if (transaction.changed) {
        if (currentRevision() !== transaction.revision) {
          throw new Error('The project changed after the agent edit. Powermove preserved all work but could not safely combine the agent changes into one Undo step.');
        }
        historyId = PM.hist.squash?.(
          transaction.historyMark,
          `Agent · ${transaction.label}`,
          transaction.historyGroup
        ) || undefined;
      }
      return {
        ok: true,
        content: [toolText({ changed: (commit || transaction.panelActions) && transaction.changed, historyId: historyId || null })],
        changed: (commit || transaction.panelActions === true) && transaction.changed,
        revision: currentRevision(),
        ...(historyId ? { historyId } : {})
      };
    } catch (error) {
      return {
        ok: false,
        content: [],
        error: error instanceof Error ? error.message : String(error),
        changed: transaction.changed,
        revision: currentRevision()
      };
    } finally {
      liveToolTransactions.delete(request.runId);
    }
  }
  throw new Error(`Unknown Powermove agent tool: ${request.tool}`);
}

const nativeAgentTools = typeof window === 'undefined' ? undefined : window.powermove?.agentTools;
nativeAgentTools?.onRequest((request) => {
  void handleLiveAgentTool(request).then(
    (result) => nativeAgentTools.respond({ runId: request.runId, callId: request.callId, ...result }),
    (error) => nativeAgentTools.respond({
      runId: request.runId,
      callId: request.callId,
      ok: false,
      content: [],
      error: error instanceof Error ? error.message : String(error),
      changed: liveToolTransactions.get(request.runId)?.changed === true,
      revision: currentRevision()
    })
  );
});

PM.AgentHarness = {
  MAX_REPAIRS, projectState, defaultTimes, observe, capture, sanitizeProposal,
  describeCommand, sceneSchema, promptContext, execute, rollback, cleanCommand,
  test: { cleanCommand, safeTimes, reviewSchema, handleLiveAgentTool },
};
}
