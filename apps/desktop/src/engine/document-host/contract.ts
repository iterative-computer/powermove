/*
 * The agent-tool contract suite (contract 15, invariant 8; plan section 22's
 * gate): the same tool calls, from the same document, must leave
 * byte-identical documents and answers on the desktop (the editor's own
 * harness in Electron) and in a document host worker.
 *
 * Both sides install `deterministicSeam` right before the sequence, so the
 * ids `PM.uid` and `Math.random` mint and the clock `Date.now` reads are
 * the same sequence on both; everything else is the hosts' own code. The worker installs it
 * only when POWERMOVE_DOCUMENT_HOST_TEST_SEED is set (tests).
 */

export interface ContractCall {
  runId: string;
  tool: string;
  arguments: Record<string, unknown>;
  /** The call answers with an error on both hosts (the same error). */
  fails?: true;
}

/** Counter ids and a fixed clock, installed on `window.PM` (self-contained: Playwright sends its source). */
export function deterministicSeam(): void {
  const PM = (globalThis as any).window?.PM ?? (globalThis as any).PM;
  let n = 0;
  PM.uid = (prefix = 'l') => `${prefix}${(++n).toString(36).padStart(7, '0')}`;
  const fixed = 1_760_000_000_000;
  Date.now = () => fixed;
  // Modules that mint ids from Math.random directly (keys, cues, groups): a seeded PRNG (mulberry32).
  let seed = 0x5eed;
  Math.random = () => {
    seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const L = (id: string) => id;

/**
 * Document tools only (no pixels, no DOM, no jobs): the document host's
 * route. get_workspace_state is compared separately (its document fields;
 * the rest describes an editor window, WORKSPACE_DOCUMENT_FIELDS).
 */
export const CONTRACT_SEQUENCE: ContractCall[] = [
  { runId: 'run-1', tool: 'get_project_state', arguments: {} },
  { runId: 'run-1', tool: 'apply_commands', arguments: { label: 'Build the scene', commands: [
    { type: 'add_layer', id: L('Lrig'), layerType: 'null', name: 'Rig' },
    { type: 'add_layer', id: L('Lplate'), layerType: 'solid', name: 'Plate', parent: 'Lrig', content: { w: 320, h: 180, color: '#3355ff' } },
    { type: 'add_layer', id: L('Lbadge'), layerType: 'shape', name: 'Badge' },
    { type: 'add_layer', id: L('Ltitle'), layerType: 'text', name: 'Title', content: { text: 'Powermove' } },
  ] } },
  { runId: 'run-1', tool: 'apply_commands', arguments: { label: 'Animate', commands: [
    { type: 'set_property', target: 'Lrig', path: 'position.x', value: 100, time: 0 },
    { type: 'set_property', target: 'Lrig', path: 'position.x', value: 520, time: 2, ease: 'easeInOut' },
    { type: 'replace_keyframes', target: 'Lplate', path: 'rotation', keyframes: [{ time: 0, value: 0 }, { time: 1, value: 45, ease: 'easeOut' }, { time: 3, value: 90 }] },
    { type: 'set_expression', target: 'Lplate', path: 'opacity', expression: '50 + Math.sin(time * 2) * 50' },
    { type: 'offset_property', target: 'Lplate', path: 'position.y', delta: 30 },
    { type: 'set_layer', target: 'Lbadge', patch: { from: 0.5, duration: 4, name: 'Badge 2' } },
    { type: 'set_content', target: 'Ltitle', patch: { text: 'Powermove engine' } },
  ] } },
  { runId: 'run-1', tool: 'apply_commands', arguments: { label: 'Look', commands: [
    { type: 'add_effect', target: 'Lplate', effect: 'blur', parameters: { amount: 12 } },
    { type: 'add_effect', target: 'Lbadge', effect: 'invert' },
    { type: 'set_transition', layer: 'Lbadge', edge: 'in', transition: { type: 'crossfade', duration: 0.5 } },
    { type: 'set_composition', patch: { name: 'Contract', duration: 8 } },
    { type: 'add_marker', id: 'Mone', time: 1.5, name: 'Beat' },
    { type: 'set_scene_parameter', name: 'speed', label: 'Speed', control: 'slider', value: 1, min: 0, max: 4 },
  ] } },
  { runId: 'run-1', tool: 'apply_commands', arguments: { label: 'Structure', commands: [
    { type: 'group_layers', targets: ['Lplate', 'Lbadge'], name: 'Card' },
    { type: 'reorder_layer', target: 'Ltitle', index: 0 },
    { type: 'add_captions', id: 'Lcaps', name: 'Captions', cues: [{ start: 0.5, end: 2, text: 'Hello there' }, { start: 2.5, end: 4, text: 'Welcome back' }] },
  ] } },
  { runId: 'run-1', tool: 'apply_commands', arguments: { label: 'Captions', commands: [
    { type: 'edit_captions', target: 'Lcaps', op: 'move', by: 0.25 },
  ] } },
  // Type edits that record font-anchor bounds (glyph ink).
  { runId: 'run-1', tool: 'apply_commands', arguments: { label: 'Type', commands: [
    { type: 'set_content', target: 'Ltitle', patch: { size: 96 } },
  ] } },
  { runId: 'run-1', tool: 'apply_commands', arguments: { label: 'Type 2', commands: [
    { type: 'set_content', target: 'Ltitle', patch: { weight: 400, tracking: 4 } },
    { type: 'set_content', target: 'Ltitle', patch: { font: 'Helvetica Neue' } },
    { type: 'set_property', target: 'Ltitle', path: 'anchor.x', value: 30, time: 0 },
  ] } },
  { runId: 'run-1', tool: 'select_layers', arguments: { layerIds: ['Lrig', 'Ltitle'] } },
  { runId: 'run-1', tool: 'select_layers', arguments: { layerIds: ['Lvoice'], add: true } },
  { runId: 'run-1', tool: 'select_layers', arguments: { layerIds: ['Lnope'] }, fails: true },
  { runId: 'run-1', tool: 'validate_effect', arguments: { definition: { id: 'contract.tint', label: 'Tint', group: 'Color', params: [{ k: 'amount', label: 'Amount', def: 0.5, min: 0, max: 1 }],
    frag: '#version 300 es\nprecision highp float;\nin vec2 v_st;\nuniform sampler2D u_tex;\nuniform float u_p0;\nout vec4 o;\nvoid main(){ o = texture(u_tex, v_st) * u_p0; }' } } },
  { runId: 'run-1', tool: 'validate_effect', arguments: { definition: { id: '', params: [] } }, fails: true },
  // Clip edits on media the project references (no bytes needed).
  { runId: 'run-1', tool: 'edit_video', arguments: { label: 'Cut', operation: 'split', layerId: 'Lvoice', at: 2 } },
  { runId: 'run-1', tool: 'edit_video', arguments: { label: 'Trim', operation: 'trim', layerId: 'Lvoice', start: 0.5, end: 1.75 } },
  { runId: 'run-1', tool: 'edit_video', arguments: { label: 'Move', operation: 'move', layerId: 'Lvoice', from: 1 } },
  { runId: 'run-1', tool: 'edit_video', arguments: { label: 'Gain', operation: 'audio', layerId: 'Lvoice', gain: 0.8, fadeIn: 0.25 } },
  { runId: 'run-1', tool: 'edit_video', arguments: { label: 'Speed', operation: 'speed', layerId: 'Lvoice', speed: 2 }, fails: true },
  { runId: 'run-1', tool: 'export_captions', arguments: { layerId: 'Lcaps', format: 'srt' } },
  { runId: 'run-1', tool: 'export_captions', arguments: { layerId: 'Lcaps', format: 'vtt' } },
  { runId: 'run-1', tool: 'list_media', arguments: {} },
  { runId: 'run-1', tool: 'generate_captions', arguments: { layerIds: ['Lrig'] }, fails: true },
  { runId: 'run-1', tool: 'check_project', arguments: {} },
  { runId: 'run-1', tool: '__mcp_context', arguments: {} },
  { runId: 'run-1', tool: 'get_project_state', arguments: {} },
  { runId: 'run-1', tool: '__finish_run', arguments: { commit: true } },
  { runId: 'run-2', tool: 'apply_commands', arguments: { label: 'Rolled back', commands: [{ type: 'delete_layers', targets: ['Ltitle'] }] } },
  { runId: 'run-2', tool: 'rollback_changes', arguments: {} },
  { runId: 'run-2', tool: '__finish_run', arguments: { commit: false } },
  { runId: 'run-3', tool: 'check_project', arguments: {} },
  { runId: 'run-3', tool: 'get_project_state', arguments: {} },
  { runId: 'run-3', tool: '__finish_run', arguments: { commit: true } },
];

/** get_workspace_state's fields that come from the document and the kernel, not from an editor window. */
export const WORKSPACE_DOCUMENT_FIELDS = ['project', 'extensions', 'registeredEffects', 'selection', 'errors'] as const;
