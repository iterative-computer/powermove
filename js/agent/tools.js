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

reg('set_composition', 'Set composition name, dimensions, frame rate, duration, background, shutter, or work area.', {
  type: 'object', properties: {
    name: { type: 'string' }, width: { type: 'number' }, height: { type: 'number' }, fps: { type: 'number' },
    duration: { type: 'number' }, background: { type: 'string' }, shutter: { type: 'number' }, workArea: { type: 'array', items: { type: 'number' } },
  },
}, async (x) => {
  PM.hist.do('Agent · composition', () => {
    const p = PM.proj;
    if (x.name != null) p.name = x.name;
    if (x.width != null) p.w = Math.max(16, Math.round(x.width));
    if (x.height != null) p.h = Math.max(16, Math.round(x.height));
    if (x.fps != null) p.fps = PM.clamp(Math.round(x.fps), 1, 240);
    if (x.duration != null) { p.dur = Math.max(.1, x.duration); if (!x.workArea) p.work = [0, p.dur]; }
    if (x.background != null) p.bg = x.background;
    if (x.shutter != null) p.shutter = PM.clamp(x.shutter, 0, 2);
    if (x.workArea && x.workArea.length === 2) p.work = [Math.max(0, x.workArea[0]), Math.min(p.dur, x.workArea[1])];
    PM.bus.emit('project');
  });
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
  let L;
  PM.hist.do('Agent · add ' + x.type, () => {
    L = PM.mkLayer(x.type, { name: x.name, from: x.from, dur: x.duration, d: x.content, p: x.properties, color: x.color });
    if (x.from != null) L.from = x.from;
    if (x.duration != null) L.dur = x.duration;
    else L.dur = Math.max(.1, PM.proj.dur - L.from);
    PM.addLayer(L, 0);
    if (x.type === 'shader') PM.syncShaderUniforms(L);
    if (x.select !== false) PM.selectLayers(L.id);
  });
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
  PM.hist.do('Agent · update ' + L.name, () => {
    if (x.name != null) L.name = x.name;
    if (x.from != null) L.from = Math.max(0, x.from);
    if (x.duration != null) L.dur = Math.max(1 / PM.proj.fps, x.duration);
    if (x.visible != null) L.on = x.visible;
    if (x.locked != null) L.lock = x.locked;
    if (x.blend != null && PM.BLENDS.includes(x.blend)) L.blend = x.blend;
    if (x.motionBlur != null) L.mblur = x.motionBlur;
    if (x.parent !== undefined) { const p = layer(x.parent); L.parent = p && p.id !== L.id ? p.id : null; }
    if (x.content) Object.assign(L.d, x.content);
    if (x.properties) for (const k in x.properties) {
      const intent = k.split('.')[0];
      if (x.preserveHandEdits !== false && L.locked_intent && L.locked_intent[intent]) { skipped.push(k); continue; }
      const p = L.p[k]; if (p) p.v = x.properties[k];
    }
    if (L.type === 'shader') PM.syncShaderUniforms(L);
    PM.touch(); PM.bus.emit('layers');
  });
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
  let p = PM.findProp(L, x.channel);
  if (!p && L.p[x.channel]) p = L.p[x.channel];
  if (!p) return fail(`Channel “${x.channel}” not found on “${L.name}”`);
  PM.hist.do('Agent · animate ' + x.channel, () => {
    if (x.replace !== false) p.kf = [];
    for (const k of x.keyframes) {
      const z = PM.setKeyOn(p, Math.max(0, k.time), k.value, k.ease || 'power', PM.proj.fps);
      if (k.hold) z.hold = true;
    }
    if (x.expression !== undefined) p.expr = x.expression || null;
    PM.touch(); L.collapsed = false; L._reveal = [x.channel];
  });
  PM.sel.chan = x.channel; PM.invalidate();
  return ok(`Animated ${L.name} · ${x.channel} with ${x.keyframes.length} keyframes`);
});

reg('set_expression', 'Set a time-based expression on a layer channel. Helpers: t, T, value, wiggle, random, linear, ease, loop, pingpong, param.', {
  type: 'object', required: ['layer', 'channel', 'expression'], properties: { layer: {}, channel: { type: 'string' }, expression: { type: ['string', 'null'] } },
}, async (x) => {
  const L = layer(x.layer); if (!L) return fail('Layer not found');
  const p = PM.findProp(L, x.channel); if (!p) return fail('Channel not found');
  PM.hist.do('Agent · expression', () => { p.expr = x.expression || null; PM.touch(); });
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
  const fx = PM.mkEffect(x.effect); if (!fx) return fail('Unknown effect');
  PM.hist.do('Agent · effect', () => {
    fx.open = true;
    if (x.parameters) for (const k in x.parameters) if (fx.p[k]) fx.p[k].v = x.parameters[k];
    L.fx.push(fx); PM.touch();
  });
  PM.Inspector.refresh(); PM.invalidate();
  return ok(`Added ${PM.FX[x.effect].label} to “${L.name}”`, { effectId: fx.id });
});

reg('remove_effect', 'Remove one effect by type or effect id from a layer.', {
  type: 'object', required: ['layer', 'effect'], properties: { layer: {}, effect: { type: 'string' } },
}, async (x) => {
  const L = layer(x.layer); if (!L) return fail('Layer not found');
  const old = L.fx.length;
  PM.hist.do('Agent · remove effect', () => { L.fx = L.fx.filter(f => f.id !== x.effect && f.type !== x.effect); PM.touch(); });
  PM.invalidate();
  return ok(`Removed ${old - L.fx.length} effect(s) from “${L.name}”`);
});

reg('write_shader', 'Create or replace GLSL source on a shader layer. Annotated uniforms become controls.', {
  type: 'object', required: ['layer', 'source'], properties: { layer: {}, source: { type: 'string' }, openEditor: { type: 'boolean' } },
}, async (x) => {
  const L = layer(x.layer); if (!L || L.type !== 'shader') return fail('Shader layer not found');
  PM.hist.do('Agent · shader', () => {
    L.d.code = x.source;
    PM.syncShaderUniforms(L);
    if (L._shaderKey) PM.GL.dropProgram(L._shaderKey);
    PM.touch();
  });
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
  PM.hist.do('Agent · delete', () => PM.removeLayers(ls.map(l => l.id)));
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

reg('set_workspace', 'Generate or edit the app interface. Supports recursive horizontal/vertical splits, stacks, tabs, overlays, floating sections, duplicate linked viewers/timelines, theme, density, features, and generated component panels with safe project/selection/parameter bindings. Use preview:true for a reversible try-before-keep interface.', {
  type: 'object', properties: {
    name: { type: 'string' }, base: { type: 'string' }, create: { type: 'boolean' }, preview: { type: 'boolean' }, action: { type: 'string', enum: ['apply', 'preview', 'commit_preview', 'cancel_preview'] }, density: { type: 'string', enum: ['compact', 'normal', 'comfy'] },
    theme: { type: 'object' }, features: { type: 'object' },
    manifest: { type: 'object' }, layout: { type: 'object' },
    docks: { type: 'array', items: { type: 'object', properties: { id: { type: 'string' }, size: { type: 'number' }, panels: { type: 'array' } } } },
    show: { type: 'array', items: { type: 'string' } }, hide: { type: 'array', items: { type: 'string' } },
    move: { type: 'array', items: { type: 'object', properties: { panel: { type: 'string' }, dock: { type: 'string' } } } },
    placements: { type: 'array', items: { type: 'object', properties: { panel: { type: 'string' }, instance: { type: 'string' }, target: { type: 'string' }, where: { type: 'string', enum: ['left', 'right', 'top', 'bottom', 'tab'] }, mode: { type: 'string', enum: ['overlay', 'floating'] }, duplicate: { type: 'boolean' }, width: { type: 'number' }, height: { type: 'number' }, anchor: { type: 'string' } } } },
    customPanels: { type: 'array' },
  },
}, async (x) => {
  if (x.action === 'commit_preview') { PM.WS.commitPreview(); return ok('Generated workspace preview kept'); }
  if (x.action === 'cancel_preview') { PM.WS.cancelPreview(); return ok('Generated workspace preview reverted'); }
  const S = window.PMWorkspaceSchema;
  let draft = S.clone(x.manifest || PM.WS.get(x.base) || PM.WS.current);
  if (x.create) { draft.id = PM.uid('ws'); draft.builtin = false; draft.name = x.name || 'Generated workspace'; }
  if (x.name) draft.name = x.name;
  if (x.density) draft.density = x.density;
  if (x.theme) draft.theme = Object.assign(draft.theme || {}, x.theme);
  if (x.features) draft.features = Object.assign(draft.features || {}, x.features);
  if (x.layout) draft.layout = x.layout;
  if (x.docks) draft.layout = { docks: x.docks.map(d => ({ id: d.id, size: d.size, panels: (d.panels || []).map(q => typeof q === 'string' ? { id: q, flex: true } : q) })) };
  draft = S.normalizeWorkspace(draft);
  for (const id of x.hide || []) {
    const found = S.listPanels(draft).find(entry => entry.panel === id || entry.instance === id);
    if (found) draft = S.removeInstance(draft, found.instance).workspace;
  }
  for (const id of x.show || []) {
    const target = S.listPanels(draft)[0]?.instance;
    draft = S.placePanel(draft, S.panel(id, { instance: S.uniqueInstance(draft, id) }), { target, where: id === 'chat' ? 'left' : 'right', duplicate: true });
  }
  for (const movement of x.move || []) {
    const found = S.listPanels(draft).find(entry => entry.panel === movement.panel || entry.instance === movement.panel);
    const target = S.listPanels(draft).find(entry => entry.instance !== found?.instance)?.instance;
    if (found) draft = S.placePanel(draft, found, { target, where: movement.dock === 'left' ? 'left' : movement.dock === 'center' ? 'tab' : 'right' });
  }
  if (x.customPanels) {
    draft.custom = x.customPanels.map(cp => ({ id: cp.id || 'custom-' + PM.uid('p'), title: cp.title || 'Controls', size: cp.size || 220, note: cp.note, components: cp.components || cp.controls || [] }));
    PM.WS.registerCustom(draft);
  }
  for (const placement of x.placements || []) {
    const existing = S.listPanels(draft).find(entry => entry.instance === placement.instance || entry.panel === placement.panel);
    const panelNode = existing || S.panel(placement.panel, { instance: placement.instance || S.uniqueInstance(draft, placement.panel) });
    draft = S.placePanel(draft, panelNode, placement);
  }
  for (const custom of draft.custom || []) if (!S.listPanels(draft).some(entry => entry.panel === custom.id)) {
    const target = S.listPanels(draft)[0]?.instance;
    draft = S.placePanel(draft, S.panel(custom.id, { instance: S.uniqueInstance(draft, custom.id) }), { target, where: 'right', duplicate: true });
  }
  const preview = x.preview === true || x.action === 'preview';
  PM.WS.replace(draft, { preview, keepId: !x.create, label: 'Agent · interface' });
  const sections = S.listPanels(PM.WS.current).map(entry => ({ panel: entry.panel, instance: entry.instance, region: entry.region }));
  return ok(`${preview ? 'Previewing' : 'Applied'} “${PM.WS.current.name}” with ${sections.length} sections`, { manifestVersion: 2, sections, preview });
});

reg('add_scene_parameter', 'Expose a purposeful global scene control that expressions can read with param("Name").', {
  type: 'object', required: ['name', 'value'], properties: {
    name: { type: 'string' }, label: { type: 'string' }, control: { type: 'string', enum: ['num', 'color', 'toggle', 'select'] }, value: {}, min: { type: 'number' }, max: { type: 'number' }, options: { type: 'array' },
  },
}, async (x) => {
  PM.hist.do('Agent · scene parameter', () => {
    PM.proj.params[x.name] = { name: x.name, label: x.label || x.name, control: x.control || 'num', value: x.value, min: x.min == null ? 0 : x.min, max: x.max == null ? Math.max(1, Number(x.value) * 2) : x.max, options: x.options || [] };
    PM.touch();
  });
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
