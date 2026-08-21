/* Powermove — agent tools. Every mutation is undoable and returns semantic evidence. */
(() => {
const PM = window.PM;
const A = PM.Agent;
const tools = {};
PM.Tools = tools;

function reg(name, description, parameters, run) {
  tools[name] = { name, description, parameters, run };
}
const ok = (message, data) => ({ ok: true, message, data });
const fail = (message) => ({ ok: false, message });
const layer = (ref) => {
  if (!ref) return PM.firstSel();
  if (typeof ref === 'number') return PM.proj.layers[ref - 1] || null;
  return PM.L(ref) || PM.byName(ref);
};
const arr = (v) => Array.isArray(v) ? v : v == null ? [] : [v];

reg('inspect_project', 'Read semantic state for the composition, selection, layers, animation, effects, assets, and workspace.', {
  type: 'object', properties: { detail: { type: 'string', enum: ['compact', 'full'] } },
}, async ({ detail = 'compact' } = {}) => ok('Project inspected', detail === 'full' ? A.digest() : A.state()));

reg('edit_source', 'Apply one atomic transaction using the same typed source edits as the canvas, inspector, timeline, and generated controls.', {
  type: 'object', required: ['commands'], properties: {
    label: { type: 'string' }, baseRevision: { type: 'number' },
    commands: { type: 'array', items: { type: 'object', required: ['type'], properties: {
      type: { type: 'string', enum: Object.keys(PM.Edit.operations) },
      target: {}, targets: { type: 'array' }, path: { type: 'string' }, value: {}, patch: { type: 'object' },
      layerType: { type: 'string' }, name: { type: 'string' }, content: { type: 'object' }, properties: { type: 'object' },
      keyframes: { type: 'array' }, expression: {}, effect: { type: 'string' }, parameters: { type: 'object' }, index: { type: 'number' },
    } } },
  },
}, async (x) => {
  const result = PM.Edit.apply(x.commands, { label: x.label || 'Agent · source edit', origin: 'agent', baseRevision: x.baseRevision });
  return result.ok ? ok(result.message, result.data) : fail(result.message);
});

reg('set_composition', 'Set composition name, dimensions, frame rate, duration, background, shutter, or work area.', {
  type: 'object', properties: {
    name: { type: 'string' }, width: { type: 'number' }, height: { type: 'number' }, fps: { type: 'number' },
    duration: { type: 'number' }, background: { type: 'string' }, shutter: { type: 'number' }, workArea: { type: 'array', items: { type: 'number' } },
  },
}, async (x) => {
  const patch = {};
  for (const key of ['name', 'width', 'height', 'fps', 'duration', 'background', 'shutter']) if (x[key] != null) patch[key] = x[key];
  if (x.workArea) patch.workArea = x.workArea;
  const result = PM.Edit.apply({ type: 'set_composition', patch }, { label: 'Agent · composition', origin: 'agent' });
  if (!result.ok) return fail(result.message);
  PM.Viewer.layout();
  return ok(`Composition is ${PM.proj.w}×${PM.proj.h}, ${PM.proj.fps} fps, ${PM.proj.dur}s`);
});

reg('add_layer', 'Add a text, solid, shape, shader, null, image, video, or audio layer.', {
  type: 'object', required: ['type'], properties: {
    type: { type: 'string', enum: ['text', 'solid', 'shape', 'shader', 'null', 'image', 'video', 'audio'] },
    name: { type: 'string' }, from: { type: 'number' }, duration: { type: 'number' },
    content: { type: 'object' }, properties: { type: 'object' }, color: { type: 'string' }, select: { type: 'boolean' },
  },
}, async (x) => {
  const result = PM.Edit.apply({
    type: 'add_layer', layerType: x.type, name: x.name, from: x.from,
    duration: x.duration, content: x.content || {}, properties: x.properties || {},
    color: x.color, select: x.select,
  }, { label: 'Agent · add ' + x.type, origin: 'agent' });
  if (!result.ok) return fail(result.message);
  const L = PM.L(result.data.results[0].data.id);
  PM.invalidate();
  return ok(`Added ${x.type} layer “${L.name}”`, { id: L.id, name: L.name });
});

reg('update_layer', 'Update a layer’s content, timing, transform, visibility, blend, parenting, motion blur, or label.', {
  type: 'object', required: ['layer'], properties: {
    layer: { description: 'Layer id, exact/partial name, or 1-based index' },
    name: { type: 'string' }, from: { type: 'number' }, duration: { type: 'number' }, visible: { type: 'boolean' }, locked: { type: 'boolean' },
    blend: { type: 'string' }, motionBlur: { type: 'boolean' }, parent: {}, content: { type: 'object' }, properties: { type: 'object' },
    preserveHandEdits: { type: 'boolean' },
  },
}, async (x) => {
  const L = layer(x.layer); if (!L) return fail('Layer not found: ' + x.layer);
  const skipped = [];
  const changes = [];
  const fields = {};
  for (const [input, output] of [['name','name'],['from','from'],['duration','duration'],['visible','visible'],['locked','locked'],['blend','blend'],['motionBlur','motionBlur']]) {
    if (x[input] != null) fields[output] = x[input];
  }
  if (x.parent !== undefined) fields.parent = x.parent;
  if (x.content) changes.push({ type: 'set_content', target: L.id, patch: x.content });
  if (x.properties) for (const k in x.properties) {
    const root = k.split('.')[0];
    if (x.preserveHandEdits !== false && L.locked_intent && (L.locked_intent[k] || L.locked_intent[root])) { skipped.push(k); continue; }
    changes.push({ type: 'set_property', target: L.id, path: k, value: x.properties[k], mode: 'static', preserveHandEdits: false });
  }
  if (Object.keys(fields).length) changes.push({ type: 'set_layer', target: L.id, patch: fields });
  if (changes.length) {
    const result = PM.Edit.apply(changes, { label: 'Agent · update ' + L.name, origin: 'agent' });
    if (!result.ok) return fail(result.message);
  }
  PM.invalidate();
  return ok(`Updated “${L.name}”${skipped.length ? '; preserved hand-edited ' + skipped.join(', ') : ''}`, { id: L.id, skipped });
});

reg('animate', 'Create or replace keyframes on one channel. Times are layer-local seconds. Use position.x, position.y, scale.x, scale.y, rotation, opacity, skew, u.UniformName, or an effect channel id.', {
  type: 'object', required: ['layer', 'channel', 'keyframes'], properties: {
    layer: {}, channel: { type: 'string' },
    keyframes: { type: 'array', items: { type: 'object', required: ['time', 'value'], properties: { time: { type: 'number' }, value: { type: 'number' }, ease: { type: 'string' }, hold: { type: 'boolean' } } } },
    replace: { type: 'boolean' }, preserveHandEdits: { type: 'boolean' }, expression: { type: ['string', 'null'] },
  },
}, async (x) => {
  const L = layer(x.layer); if (!L) return fail('Layer not found: ' + x.layer);
  const root = x.channel.split('.')[0];
  if (x.preserveHandEdits !== false && L.locked_intent && L.locked_intent[root]) return fail(`Preserved hand-edited ${root}; ask explicitly to overwrite it`);
  const result = PM.Edit.apply({
    type: 'replace_keyframes', target: L.id, path: x.channel,
    keyframes: x.keyframes, replace: x.replace, expression: x.expression,
    preserveHandEdits: x.preserveHandEdits,
  }, { label: 'Agent · animate ' + x.channel, origin: 'agent' });
  if (!result.ok) return fail(result.message);
  L.collapsed = false; L._reveal = [x.channel];
  PM.sel.chan = x.channel; PM.invalidate();
  return ok(`Animated ${L.name} · ${x.channel} with ${x.keyframes.length} keyframes`);
});

reg('set_expression', 'Set a time-based expression on a layer channel. Helpers: t, T, value, wiggle, random, linear, ease, loop, pingpong, param.', {
  type: 'object', required: ['layer', 'channel', 'expression'], properties: { layer: {}, channel: { type: 'string' }, expression: { type: ['string', 'null'] } },
}, async (x) => {
  const L = layer(x.layer); if (!L) return fail('Layer not found');
  const result = PM.Edit.apply({ type: 'set_expression', target: L.id, path: x.channel, expression: x.expression }, { label: 'Agent · expression', origin: 'agent' });
  if (!result.ok) return fail(result.message);
  PM.invalidate();
  return ok(`${x.expression ? 'Set' : 'Removed'} expression on ${L.name} · ${x.channel}`);
});

reg('add_effect', 'Add a GPU effect to a layer and optionally set its parameters.', {
  type: 'object', required: ['layer', 'effect'], properties: {
    layer: {}, effect: { type: 'string', enum: ['blur', 'motionblurDir', 'sharpen', 'glow', 'color', 'levels', 'duotone', 'grain', 'vignette', 'chroma', 'pixelate', 'posterize', 'displace', 'shadow', 'invert'] },
    parameters: { type: 'object' },
  },
}, async (x) => {
  const L = layer(x.layer); if (!L) return fail('Layer not found');
  const result = PM.Edit.apply({ type: 'add_effect', target: L.id, effect: x.effect, parameters: x.parameters || {} }, { label: 'Agent · effect', origin: 'agent' });
  if (!result.ok) return fail(result.message);
  const effectId = result.data.results[0].data.effectId;
  PM.Inspector.refresh(); PM.invalidate();
  return ok(`Added ${PM.FX[x.effect].label} to “${L.name}”`, { effectId });
});

reg('remove_effect', 'Remove one effect by type or effect id from a layer.', {
  type: 'object', required: ['layer', 'effect'], properties: { layer: {}, effect: { type: 'string' } },
}, async (x) => {
  const L = layer(x.layer); if (!L) return fail('Layer not found');
  const result = PM.Edit.apply({ type: 'remove_effect', target: L.id, effect: x.effect }, { label: 'Agent · remove effect', origin: 'agent' });
  if (!result.ok) return fail(result.message);
  PM.invalidate();
  return ok(`Removed ${result.data.results[0].data.removed} effect(s) from “${L.name}”`);
});

reg('write_shader', 'Create or replace GLSL source on a shader layer. Annotated uniforms become controls.', {
  type: 'object', required: ['layer', 'source'], properties: { layer: {}, source: { type: 'string' }, openEditor: { type: 'boolean' } },
}, async (x) => {
  const L = layer(x.layer); if (!L || L.type !== 'shader') return fail('Shader layer not found');
  const result = PM.Edit.apply({ type: 'set_content', target: L.id, patch: { code: x.source } }, { label: 'Agent · shader', origin: 'agent' });
  if (!result.ok) return fail(result.message);
  PM.invalidate();
  await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));
  const err = PM.GL.compileError(L._shaderKey);
  if (x.openEditor) PM.openShaderEditor(L);
  return err ? fail('Shader compile error: ' + err) : ok(`Shader compiled on “${L.name}” with ${(L._udefs || []).length} controls`, { uniforms: (L._udefs || []).map(u => u.name) });
});

reg('delete_layers', 'Delete layers by name/id/index. Defaults to the current selection.', {
  type: 'object', properties: { layers: { type: 'array' } },
}, async (x = {}) => {
  const ls = x.layers && x.layers.length ? x.layers.map(layer).filter(Boolean) : PM.selLayers();
  if (!ls.length) return fail('No layers to delete');
  const result = PM.Edit.apply({ type: 'delete_layers', targets: ls.map(l => l.id) }, { label: 'Agent · delete', origin: 'agent' });
  if (!result.ok) return fail(result.message);
  PM.invalidate();
  return ok('Deleted ' + ls.map(l => '“' + l.name + '”').join(', '));
});

reg('select', 'Select layers and optionally a channel or playhead time.', {
  type: 'object', properties: { layers: { type: 'array' }, channel: { type: ['string', 'null'] }, time: { type: 'number' } },
}, async (x) => {
  const ls = (x.layers || []).map(layer).filter(Boolean);
  PM.selectLayers(ls.map(l => l.id));
  if (x.channel !== undefined) PM.sel.chan = x.channel;
  if (x.time != null) PM.setTime(x.time);
  return ok(`Selected ${ls.length ? ls.map(l => l.name).join(', ') : 'full composition'}`);
});

reg('set_workspace', 'Create or edit an authored workspace: panels, docks, dimensions, theme, features, density, and custom parameter controls.', {
  type: 'object', properties: {
    name: { type: 'string' }, base: { type: 'string' }, create: { type: 'boolean' }, density: { type: 'string', enum: ['compact', 'normal', 'comfy'] },
    theme: { type: 'object' }, features: { type: 'object' },
    docks: { type: 'array', items: { type: 'object', required: ['id', 'panels'], properties: {
      id: { type: 'string', description: 'Use left, center, or right. The center dock expands to fill the window.' },
      size: { type: 'number' }, flex: { type: 'boolean' },
      panels: { type: 'array', items: { type: 'object', required: ['id'], properties: {
        id: { type: 'string', description: 'A panel id from available panels or a custom panel id.' },
        size: { type: 'number' }, flex: { type: 'boolean' }, min: { type: 'number' },
      } } },
    } } },
    show: { type: 'array', items: { type: 'string' } }, hide: { type: 'array', items: { type: 'string' } },
    move: { type: 'array', items: { type: 'object', properties: { panel: { type: 'string' }, dock: { type: 'string' } } } },
    customPanels: { type: 'array', items: { type: 'object', required: ['title', 'controls'], properties: {
      id: { type: 'string' }, title: { type: 'string' }, note: { type: 'string' }, size: { type: 'number' },
      controls: { type: 'array', items: { type: 'object', required: ['type', 'label'], properties: {
        type: { type: 'string', enum: ['slider', 'color', 'toggle', 'select', 'button'] },
        label: { type: 'string' }, param: { type: 'string', description: 'Stable scene parameter name used by expressions.' }, def: {}, min: { type: 'number' }, max: { type: 'number' },
        step: { type: 'number' }, unit: { type: 'string' }, options: { type: 'array', items: { type: 'string' } },
        target: { description: 'Layer id/name or $selection for a source-bound control.' },
        path: { type: 'string', description: 'properties.*, content.*, or layer.* source path.' },
        commands: { type: 'array', items: { type: 'object' } },
        cmd: { type: 'string' }, prompt: { type: 'string' },
      } } },
    } } },
  },
}, async (x) => {
  if (x.docks && (!x.docks.length || !x.docks.some(d => d && Array.isArray(d.panels) && d.panels.length))) {
    return fail('Workspace needs at least one dock with a panel');
  }
  let w;
  if (x.create) w = PM.WS.create({ name: x.name || 'Custom', base: x.base || PM.WS.current.id });
  const apply = (z) => {
    if (x.name) z.name = x.name;
    if (x.density) z.density = x.density;
    if (x.theme) z.theme = Object.assign(z.theme || {}, x.theme);
    if (x.features) z.features = Object.assign(z.features || {}, x.features);
    if (x.docks) z.layout.docks = x.docks.map(d => ({ id: d.id, size: d.size, panels: (d.panels || []).map(q => typeof q === 'string' ? { id: q, flex: true } : q) }));
    for (const id of x.hide || []) PM.Layout.removePanel(z, id);
    for (const id of x.show || []) PM.Layout.addPanel(z, id, id === 'chat' ? 'left' : 'right');
    for (const m of x.move || []) PM.Layout.addPanel(z, m.panel, m.dock);
    if (x.customPanels) {
      z.custom = x.customPanels.map((cp, i) => ({ id: cp.id || 'custom-' + PM.uid('p'), title: cp.title || 'Controls', size: cp.size || 220, note: cp.note, controls: cp.controls || [] }));
      PM.WS.registerCustom(z);
      z.custom.forEach(cp => { if (!PM.Layout.hasPanel(z, cp.id)) PM.Layout.addPanel(z, cp.id, 'right'); });
    }
  };
  if (x.create) PM.WS.mutate(apply, { inPlace: true });
  else PM.WS.mutate(apply);
  return ok(`Workspace is now “${PM.WS.current.name}”`, { docks: PM.WS.current.layout.docks });
});

reg('add_scene_parameter', 'Expose a purposeful global scene control that expressions can read with param("Name").', {
  type: 'object', required: ['name', 'value'], properties: {
    name: { type: 'string' }, label: { type: 'string' }, control: { type: 'string', enum: ['num', 'color', 'toggle', 'select'] }, value: {}, min: { type: 'number' }, max: { type: 'number' }, options: { type: 'array' },
  },
}, async (x) => {
  const result = PM.Edit.apply({
    type: 'set_scene_parameter', name: x.name, label: x.label, control: x.control,
    value: x.value, min: x.min == null ? 0 : x.min,
    max: x.max == null ? Math.max(1, Number(x.value) * 2) : x.max,
    options: x.options || [],
  }, { label: 'Agent · scene parameter', origin: 'agent' });
  if (!result.ok) return fail(result.message);
  PM.Inspector.refresh();
  return ok(`Added scene control “${x.label || x.name}”`);
});

reg('look', 'Render real frames of the live composition. Use representative beat moments and boundary triplets.', {
  type: 'object', properties: { times: { type: 'array', items: { type: 'number' } }, width: { type: 'number' }, note: { type: 'string' } },
}, async (x = {}) => {
  const p = PM.proj;
  let times = x.times && x.times.length ? x.times : [0, p.dur * .25, p.dur * .5, p.dur * .75, Math.max(0, p.dur - 1 / p.fps)];
  times = [...new Set(times.map(t => PM.clamp(t, 0, p.dur)).map(t => PM.round(t, 3)))].slice(0, 12);
  const old = PM.time;
  const shots = [];
  for (const t of times) {
    PM.setTime(t, { raw: true, force: true });
    await new Promise(r => requestAnimationFrame(r));
    shots.push({ time: t, image: PM.Export.snapshot(t, x.width || 420) });
  }
  PM.setTime(old, { raw: true, force: true });
  const msg = A.thread[A.thread.length - 1];
  if (msg && msg.role === 'assistant') msg.shots = (msg.shots || []).concat(shots);
  PM.bus.emit('chat');
  return ok(`Rendered ${shots.length} live frames at ${times.join(', ')}s`, { times, shots });
});

reg('review_motion', 'Score craft evidence after looking. A release requires 100/100 and all checks true.', {
  type: 'object', required: ['checks'], properties: {
    checks: { type: 'object', properties: {
      briefFidelity: { type: 'boolean' }, hierarchy: { type: 'boolean' }, legibility: { type: 'boolean' }, temporalDevelopment: { type: 'boolean' }, transitions: { type: 'boolean' }, noSlideshow: { type: 'boolean' }, noDeadTime: { type: 'boolean' }, finish: { type: 'boolean' },
    } }, critique: { type: 'string' },
  },
}, async (x) => {
  const names = ['briefFidelity', 'hierarchy', 'legibility', 'temporalDevelopment', 'transitions', 'noSlideshow', 'noDeadTime', 'finish'];
  const passed = names.filter(n => x.checks && x.checks[n]).length;
  const score = Math.round(passed / names.length * 100);
  const missing = names.filter(n => !x.checks || !x.checks[n]);
  return ok(`Motion review ${score}/100${missing.length ? ' — fix: ' + missing.join(', ') : ' — release approved'}`, { score, missing, critique: x.critique || '' });
});

reg('ask_user', 'Ask one compact multiple-choice creative checkpoint when essential direction is genuinely missing.', {
  type: 'object', required: ['question', 'options'], properties: { question: { type: 'string' }, options: { type: 'array', items: { type: 'string' } } },
}, async (x) => {
  A.push({ role: 'question', content: x.question, options: x.options });
  return ok('Question shown to user; stop and wait for their answer');
});

reg('save_take', 'Save a named project version before a major direction change.', {
  type: 'object', properties: { label: { type: 'string' } },
}, async (x = {}) => { const t = PM.takes.save(x.label); return ok('Saved take “' + t.label + '”'); });

A.toolSchemas = () => Object.values(tools).map(t => ({
  type: 'function', function: { name: t.name, description: t.description, parameters: t.parameters },
}));
A.anthropicTools = () => Object.values(tools).map(t => ({ name: t.name, description: t.description, input_schema: t.parameters }));

A.callTool = async (name, args = {}) => {
  const t = tools[name];
  if (!t) return fail('Unknown tool: ' + name);
  const msg = { role: 'tool', name, args, state: 'run', content: '' };
  A.thread.push(msg); PM.bus.emit('chat');
  try {
    const out = await t.run(args || {});
    msg.state = out && out.ok === false ? 'bad' : 'ok';
    msg.content = out && out.message ? out.message : JSON.stringify(out);
    msg.data = out && out.data;
    PM.bus.emit('chat');
    return out;
  } catch (e) {
    msg.state = 'bad'; msg.content = String(e.message || e); PM.bus.emit('chat');
    return fail(msg.content);
  }
};
})();
