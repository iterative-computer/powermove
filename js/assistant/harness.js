/* No longer loaded — superseded by src/renderer/src/legacy/assistant/harness.ts; kept for the legacy test oracle until Phase 6. */
/* Powermove — bounded composition-agent harness.
   Observe semantic source + rendered pixels, propose typed source edits, apply
   behind a checkpoint, visually review, repair at most twice, then let the user
   keep or roll back the complete run. */
(() => {
const PM = window.PM;
const MAX_COMMANDS = 80;
const MAX_REPAIRS = 2;
const MAX_KEYFRAMES = 80;
const SCENE_OPERATIONS = new Set([
  'set_property', 'replace_keyframes', 'set_easing', 'set_expression', 'set_content',
  'set_layer', 'set_composition', 'add_layer', 'delete_layers',
  'reorder_layer', 'add_effect', 'remove_effect', 'set_effect',
  'set_scene_parameter', 'add_marker', 'create_section', 'update_section',
  'transform_layers',
]);
const FIELDS = {
  set_property: ['type', 'target', 'path', 'value', 'time', 'mode', 'ease', 'hold'],
  replace_keyframes: ['type', 'target', 'path', 'keyframes', 'replace', 'expression'],
  set_easing: ['type', 'keyframes', 'curve'],
  set_expression: ['type', 'target', 'path', 'expression'],
  set_content: ['type', 'target', 'patch'],
  set_layer: ['type', 'target', 'patch'],
  set_composition: ['type', 'patch'],
  add_layer: ['type', 'id', 'layerType', 'name', 'from', 'duration', 'content', 'properties', 'color', 'index', 'select', 'parent', 'blend', 'motionBlur', 'visible', 'solo', 'shy', 'collapsed'],
  delete_layers: ['type', 'target', 'targets'],
  reorder_layer: ['type', 'target', 'index'],
  add_effect: ['type', 'target', 'effect', 'parameters', 'open'],
  remove_effect: ['type', 'target', 'effect'],
  set_effect: ['type', 'target', 'effect', 'patch'],
  set_scene_parameter: ['type', 'name', 'label', 'control', 'value', 'min', 'max', 'options'],
  add_marker: ['type', 'id', 'time', 'name'],
  create_section: ['type', 'section'],
  update_section: ['type', 'sectionId', 'layers', 'thumb', 'at', 'version'],
  transform_layers: ['type', 'transform', 'state'],
};

const clone = value => value === undefined ? undefined : JSON.parse(JSON.stringify(value));
const text = (value, fallback, max) => typeof value === 'string' && value.trim()
  ? value.trim().slice(0, max) : fallback;

function safeTimes(values, fallback) {
  const dur = Math.max(.1, Number(PM.proj?.dur) || 10);
  const source = Array.isArray(values) && values.length ? values : fallback;
  return [...new Set((source || []).map(Number).filter(Number.isFinite)
    .map(value => PM.clamp(value, 0, dur)).map(value => PM.round(value, 3)))].slice(0, 5);
}

function defaultTimes() {
  const p = PM.proj;
  const start = Array.isArray(p.work) ? p.work[0] : 0;
  const end = Array.isArray(p.work) ? p.work[1] : p.dur;
  const selected = PM.selLayers?.() || [];
  const candidates = selected.length
    ? [PM.time, ...selected.flatMap(layer => [layer.from, layer.from + layer.dur * .5, layer.from + layer.dur - 1 / p.fps])]
    : [start, start + (end - start) * .33, start + (end - start) * .67, Math.max(start, end - 1 / p.fps)];
  return safeTimes(candidates, [PM.time]);
}

function propertyDigest(layer) {
  return PM.allProps(layer).map(item => ({
    path: item.key,
    value: PM.evP(layer, item.prop, PM.time, item.key),
    keyframes: item.prop.kf.slice(0, MAX_KEYFRAMES).map(key => ({
      time: PM.round(key.t, 3), value: clone(key.v), hold: !!key.hold,
    })),
    expression: item.prop.expr || null,
    handEdited: !!(layer.locked_intent?.[item.key] || layer.locked_intent?.[item.key.split('.')[0]]),
  }));
}

function projectState() {
  const p = PM.proj;
  const selectedIds = new Set(PM.sel?.layers || []);
  return {
    composition: {
      id: p.id, name: p.name, width: p.w, height: p.h, fps: p.fps,
      duration: p.dur, workArea: clone(p.work), background: p.backgroundFill || p.bg,
      playhead: PM.round(PM.time, 3), revision: Number(p.revision) || 0,
    },
    selection: clone(PM.sel),
    layers: p.layers.slice(0, 120).map((layer, index) => ({
      index, id: layer.id, name: layer.name, type: layer.type, from: layer.from,
      duration: layer.dur, visible: layer.on, locked: layer.lock, parent: layer.parent,
      blend: layer.blend, motionBlur: layer.mblur, color: layer.color,
      content: Object.fromEntries(Object.entries(layer.d || {}).map(([key, value]) => [
        key,
        typeof value === 'string' ? value.slice(0, key === 'code' ? 8000 : 500) : clone(value),
      ])),
      properties: propertyDigest(layer),
      effects: (layer.fx || []).map(effect => ({ id: effect.id, type: effect.type, enabled: effect.on })),
      textLayout: layer.type === 'text' && selectedIds.has(layer.id) && PM.textLayout
        ? PM.textLayout(layer.d) : null,
    })),
    parameters: clone(p.params || {}),
    markers: clone(p.markers || []),
    availableOperations: [...SCENE_OPERATIONS],
    availableCapabilities: PM.Capabilities?.catalog?.() || null,
    editableSource: PM.Edit?.sourceCatalog?.() || null,
  };
}

async function capture(times = defaultTimes(), width = 480) {
  const chosen = safeTimes(times, defaultTimes());
  const images = [];
  for (const time of chosen) {
    await new Promise(resolve => requestAnimationFrame(resolve));
    images.push(PM.Export.snapshot(time, width));
  }
  return { times: chosen, images };
}

async function observe(times) {
  const frames = await capture(times);
  return { state: projectState(), ...frames };
}

function cleanCommand(raw) {
  let source = raw;
  if (typeof raw === 'string') {
    if (raw.length > 50_000) return null;
    try { source = JSON.parse(raw); } catch { return null; }
  }
  if (!source || typeof source !== 'object' || Array.isArray(source) || !SCENE_OPERATIONS.has(source.type)) return null;
  const out = {};
  for (const key of FIELDS[source.type]) if (source[key] !== undefined) out[key] = clone(source[key]);
  if (JSON.stringify(out).length > 50_000) return null;
  if (out.type === 'replace_keyframes') {
    if (!Array.isArray(out.keyframes)) return null;
    out.keyframes = out.keyframes.slice(0, MAX_KEYFRAMES);
  }
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
  /* Model-authored edits never bypass layer locks or hand-authored intent. */
  if (out.type === 'set_property' || out.type === 'replace_keyframes') out.preserveHandEdits = true;
  return out;
}

function sanitizeProposal(raw) {
  const commands = (Array.isArray(raw?.commands) ? raw.commands : [])
    .slice(0, MAX_COMMANDS).map(cleanCommand).filter(Boolean);
  return {
    label: text(raw?.label, 'Agent composition edit', 80),
    summary: text(raw?.summary, 'A structured composition change is ready to review.', 260),
    commands,
    reviewTimes: safeTimes(raw?.reviewTimes, defaultTimes()),
    baseRevision: Number(PM.proj.revision) || 0,
  };
}

function describeCommand(command) {
  const target = command.target || (Array.isArray(command.targets) ? `${command.targets.length} layers` : '') || command.name || '';
  const detail = command.path || command.layerType || command.effect || Object.keys(command.patch || {}).join(', ');
  const labels = {
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

function promptContext(observation) {
  return `\nLIVE COMPOSITION SOURCE\n${JSON.stringify(observation?.state || projectState())}\n\nRENDERED FRAME TIMES\n${JSON.stringify(observation?.times || [])}`;
}

function reviewPrompt(request, applied, pass, observation) {
  return `You are the visual review stage inside Powermove's bounded motion-editing harness. The attached images are real rendered frames after source edits. Judge the result against the user's request and the live semantic source.

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
- Preserve locked layers and hand-edited channels. Do not broaden the user's scope.
- This is repair pass ${pass + 1} of ${MAX_REPAIRS}. Return no repair commands after the limit.
- Keep message and critique concise.`;
}

async function execute(request, proposal, progress = () => {}) {
  const historyMark = PM.hist.mark?.() || null;
  const historyGroup = PM.uid('agent-history');
  const editStart = Array.isArray(PM.proj.edits) ? PM.proj.edits.length : 0;
  const checkpoint = {
    id: PM.uid('agent-checkpoint'),
    label: `Before agent · ${text(request, 'composition edit', 42)}`,
    json: JSON.stringify(PM.proj),
  };
  try { checkpoint.takeId = PM.takes?.save(checkpoint.label)?.id || null; } catch { checkpoint.takeId = null; }
  progress('Applying structured source edit…');
  const first = PM.Edit.apply(proposal.commands, {
    label: proposal.label, origin: 'agent', baseRevision: proposal.baseRevision, historyGroup,
  });
  if (!first.ok) throw new Error(first.message);
  /* Review passes must see what the policy actually executed (locked layers
     may have been skipped), not the raw proposal. Surface skips to the user. */
  const results = Array.isArray(first.data?.results) ? first.data.results : null;
  const applied = results ? results.map(item => item.command) : proposal.commands.slice();
  const skipped = (results || []).flatMap(item => item.data?.skippedLocked || []);
  if (skipped.length && first.message) progress(first.message.split('. ').find(part => part.startsWith('Skipped locked')) || first.message);
  let review = { status: 'pass', message: 'The rendered change is ready.', critique: '' };
  let frames = null;
  let reviewError = '';
  try {
    for (let pass = 0; pass <= MAX_REPAIRS; pass++) {
      progress(pass ? `Reviewing repair ${pass} of ${MAX_REPAIRS}…` : 'Reviewing rendered frames…');
      frames = await observe(proposal.reviewTimes);
      const raw = await PM.CodexBridge.request(
        reviewPrompt(request, applied, pass, frames), reviewSchema(), frames.images,
        { onProgress: summary => progress(summary) },
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
  } catch (error) {
    reviewError = String(error.message || error);
  }
  /* Always present pixels from the final source, including after the last repair. */
  try { frames = await observe(proposal.reviewTimes); }
  catch (error) {
    reviewError = reviewError || String(error.message || error);
    frames = frames || { state: projectState(), times: proposal.reviewTimes, images: [] };
  }
  const newEdits = (PM.proj.edits || []).slice(editStart);
  const agentOnly = newEdits.length > 0 && newEdits.every(edit => edit.origin === 'agent');
  const historyId = agentOnly
    ? PM.hist.squash?.(historyMark, `Agent · ${proposal.label}`, historyGroup) || null
    : null;
  checkpoint.historyId = historyId;
  return { checkpoint, applied, review, frames, reviewError, historyId, revision: Number(PM.proj.revision) || 0 };
}

function rollback(checkpoint) {
  if (!checkpoint?.json) return false;
  if (checkpoint.historyId && PM.hist.undoIfTop?.(checkpoint.historyId)) return true;
  return !!PM.hist.restoreSnapshot(checkpoint.json, 'Undo agent run');
}

PM.AgentHarness = {
  MAX_REPAIRS, projectState, defaultTimes, observe, capture, sanitizeProposal,
  describeCommand, sceneSchema, promptContext, execute, rollback, cleanCommand,
  test: { cleanCommand, safeTimes, reviewSchema },
};
})();
