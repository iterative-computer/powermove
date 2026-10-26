<script lang="ts">
  import { mount, onDestroy, unmount } from 'svelte';
  import type { PanelProps } from './registerSveltePanel';
  import Icon from './Icon.svelte';
  import Row from '../controls/Row.svelte';
  import ColorField from '../controls/ColorField.svelte';
  import NumField from '../controls/NumField.svelte';
  import SelectField from '../controls/SelectField.svelte';
  import ToggleField from '../controls/ToggleField.svelte';
  import type { EditBinding } from '../controls/gesture';
  import { doc } from '../state/document.svelte';
  import { sel } from '../state/selection.svelte';
  import { bridge } from '../kernel/bridge';
  import CurveEditor from './easing/CurveEditor.svelte';
  import { bounds, clone, cubic, evaluate, mirror, path, type Curve } from './easing/curve';
  import { handleControlPoint, setHandleFromPoint } from '../core/anim/temporal-ease';
  import { STORAGE_KEY, mergeLibrary, movePreset, readLibrary, type Group, type Library, type Preferences, type Preset } from './easing/library';
  import { ALL_SECTIONS, applyCurve, copyKeys, duplicate, expressionCommands, interpolate, loadCurve, pasteKeys, sections, tracks, type CopiedTrack, type Sections, type Track } from './easing/operations';
  import type { EditCommand } from '../core/types/commands';
  import { cleanupPath, positionController } from './easing/position';
  import { importPresetFiles } from './easing/preset-files';

  let { panelId }: PanelProps = $props();
  const PM = window.PM as Record<string, any>;
  const api = PM.Kernel.api('easing-panel');
  let library = $state<Library>(readLibrary(window.localStorage));
  let curve = $state<Curve>(cubic());
  let clipboard = $state<CopiedTrack[]>([]);
  let autoApply = $state(false);
  let coordinates = $state(false);
  let settings = $state(false);
  let presetColor = $state('#ff6b1a');
  let status = $state('');
  let hint = $state('');
  let hover = $state<Preset | null>(null);
  let preview = $state(0);
  let fileInput: HTMLInputElement;
  let importFolder: HTMLInputElement;
  let panel: HTMLDivElement;
  let undo = $state<Curve[]>([]), redo = $state<Curve[]>([]);
  let coordinatePoint = $state(0);
  let before: Curve | undefined;
  let gestureTracks: Track[] = [];
  let transaction = false;
  let gestureFailed = false;
  const group = $derived(library.groups.find(g => g.id === library.active) || library.groups[0]!);
  const prefs = $derived(library.preferences);
  const available = $derived((sel.keys, doc.tick.values, doc.tick.structure, doc.proj, tracks(PM)));
  const count = $derived(available.reduce((n, t) => n + t.selected.length, 0));
  const info = $derived(hover ? `${hover.name}${hover.curve.points.length === 2 ? ` · ${Math.round(hover.curve.points[0]!.outEase.influence)} / ${Math.round(hover.curve.points[1]!.inEase.influence)}%` : ` · ${hover.curve.points.length} points`}` : hint || status || (count ? `${count} keyframes selected` : 'Select keyframes to apply easing'));
  const uid = () => PM.uid('ease');
  const keyId = () => PM.uid('k');
  const local = (label: string, set: (v: any) => void): EditBinding => ({ mode: 'local', label, set });
  function persist(next = library) {
    library = next;
    try { window.localStorage.setItem(STORAGE_KEY, JSON.stringify(next)); }
    catch { status = 'Library could not be saved. Export a backup.'; }
  }
  function preference<K extends keyof Preferences>(key: K, value: Preferences[K]) {
    persist({ ...library, preferences: { ...prefs, [key]: value } });
  }
  function run(action: () => void) {
    try { action(); } catch (error) { status = error instanceof Error ? error.message : String(error); }
  }
  function edit(commands: EditCommand[], label: string, includeSpan = false) {
    if (!commands.length) throw new Error('Select keyframes on an unlocked layer');
    const result = PM.Edit.apply(commands, { label, origin: 'interface' });
    if (!result.ok) throw new Error(result.error || result.message || 'The animation could not be changed');
    selectSurvivingKeys(commands,includeSpan);
    status = label;
  }
  function selectSurvivingKeys(commands: EditCommand[], includeSpan = false) {
    const selected = new Set(PM.sel.keys);
    for (const command of commands) if (command.type === 'replace_keyframes') {
      const previous = gestureTracks.find(t => t.layerId === command.target && t.path === command.path);
      const source = previous?.keys.filter(k => previous.selected.includes(k.i));
      if (includeSpan && source?.length && curve.points.length > 2) {
        const first = source[0]!.t, last = source.at(-1)!.t;
        const prop = PM.findProp(PM.L(command.target), command.path);
        for (const k of prop?.kf || []) if (k.t >= first && k.t <= last) selected.add(k.i);
      }
    }
    PM.sel.keys = [...selected].filter(id => PM.resolveSelectedKeys().some((k: any) => k.i === id) || commands.some(c => c.type === 'replace_keyframes' && c.keyframes.some(k => k.id === id)));
    PM.bus.emit('sel');
  }
  function apply(next = curve, target: Sections = ALL_SECTIONS) {
    gestureTracks = tracks(PM);
    edit(applyCurve(gestureTracks, next, keyId, target), 'Apply easing',true);
  }
  function change(next: Curve, phase: 'begin' | 'update' | 'end' | 'cancel') {
    if (phase === 'begin') { before = clone(curve); gestureTracks = tracks(PM); gestureFailed = false; return; }
    if (phase === 'cancel') {
      if (transaction) PM.Edit.cancel(); transaction = false; curve = clone(next); before = undefined; return;
    }
    if (phase === 'update') {
      curve = clone(next);
      if (autoApply && gestureTracks.length && !gestureFailed) run(() => {
        if (!transaction) { PM.Edit.begin('Adjust easing', { origin: 'interface' }); transaction = true; }
        let commands;
        try { commands = applyCurve(gestureTracks, curve, keyId); }
        catch (error) { PM.Edit.cancel(); transaction = false; gestureFailed = true; throw error; }
        for (const command of commands) {
          const result = PM.Edit.dispatch(command);
          if (result?.ok === false) { PM.Edit.cancel(); transaction = false; gestureFailed = true; throw new Error(result.error || 'Could not apply easing'); }
        }
      });
      return;
    }
    if (before && JSON.stringify(before) !== JSON.stringify(curve)) { undo = [...undo.slice(-99), before]; redo = []; }
    if (transaction) {
      PM.Edit.commit('Adjust easing'); transaction = false;
      selectSurvivingKeys(gestureTracks.map(t => ({ type: 'replace_keyframes', target: t.layerId, path: t.path, keyframes: PM.findProp(PM.L(t.layerId),t.path).kf.map((k:any) => ({time:k.t,value:k.v,id:k.i})) })),true);
    }
    before = undefined;
  }
  onDestroy(() => { if (transaction) PM.Edit.cancel(); });
  function setCurve(next: Curve, applyToSelection = false, event?: MouseEvent, passive = false) {
    if (applyToSelection || passive) {
      undo = [...undo.slice(-99), clone(curve)]; redo = []; curve = clone(next);
      if (applyToSelection) run(() => apply(next, sections(event)));
    } else { change(curve, 'begin'); change(next, 'update'); change(next, 'end'); }
  }
  function graphUndo(forward = false) {
    const source = forward ? redo : undo, next = source.at(-1);
    if (!next) return;
    if (forward) { redo = redo.slice(0, -1); undo = [...undo, clone(curve)]; }
    else { undo = undo.slice(0, -1); redo = [...redo, clone(curve)]; }
    curve = clone(next); if (autoApply) run(() => apply());
  }
  function prompt(title: string, current: string, accept: (value: string) => void) {
    const input = document.createElement('input'); input.className = 'pm-control-input'; input.value = current; input.maxLength = 100; input.setAttribute('aria-label', title);
    const body = document.createElement('div'); body.className = 'field'; body.appendChild(input);
    PM.modal({ title, body, width: 360, actions: [{ label: 'Cancel' }, { label: 'Save', pri: true, run: () => { const value = input.value.trim(); if (value) accept(value); } }] });
    queueMicrotask(() => { input.focus(); input.select(); });
  }
  function addGroup() { prompt('New preset group', String(library.groups.length + 1), name => { const id = uid(); persist({ ...library, active: id, groups: [...library.groups, { id, name, presets: [] }] }); }); }
  function savePreset() { prompt('Save easing preset', 'Custom easing', name => { const preset: Preset = { id: uid(), name, color: '', curve: clone(curve) }; persist({ ...library, groups: library.groups.map(g => g.id === group.id ? { ...g, presets: [...g.presets, preset] } : g) }); status = 'Preset saved'; }); }
  function copy(event?: MouseEvent) {
    run(() => {
      const current = tracks(PM); clipboard = copyKeys(current); status = `${clipboard.reduce((n, t) => n + t.keys.length, 0)} keyframes copied`;
      if (current.length === 1) { try { setCurve(loadCurve(current[0]!),false,undefined,true); } catch { /* Values and easing are still copied for flat or discrete tracks. */ } }
      else if (event) PM.menu(event.currentTarget, current.map(t => ({ label: `${PM.L(t.layerId)?.name} · ${t.path}`, run: () => run(() => setCurve(loadCurve(t),false,undefined,true)) })));
    });
  }
  function paste(values: boolean, event?: Pick<MouseEvent, 'shiftKey' | 'altKey' | 'metaKey' | 'ctrlKey'>, target?: Sections) { run(() => edit(pasteKeys(tracks(PM), clipboard, values, target || sections(event)), values ? 'Paste values' : 'Paste easing')); }
  async function exportLibrary(groups = library.groups) {
    try {
      const data = JSON.stringify({ ...library, groups, active: groups[0]!.id }, null, 2);
      const result = await bridge()!.saveFile({ name: 'Easing presets.json', data: new TextEncoder().encode(data), saveAs: true });
      if (!result.ok && !result.cancelled) throw new Error(result.error || 'Could not export library');
      if (result.ok) status = 'Presets exported';
    } catch (e) { status = e instanceof Error ? e.message : String(e); }
  }
  async function importFiles(files: FileList | File[]) {
    try { const incoming = await importPresetFiles(Array.from(files), uid); persist(mergeLibrary(library, incoming, uid)); status = 'Presets imported'; }
    catch (e) { status = e instanceof Error ? e.message : String(e); }
    if (fileInput) fileInput.value = ''; if (importFolder) importFolder.value = '';
  }
  function menuAt(event: MouseEvent, items: any[]) {
    const anchor = event.target instanceof Element ? event.target.closest('button') || panel : panel;
    PM.menu(anchor, items, { x: event.clientX, y: event.clientY });
  }
  function targets(run: (s: Sections) => void) {
    return [
      { label: 'All sections', run: () => run(ALL_SECTIONS) },
      ...[1, 2, 4, 3, 6, 5, 7].map(mask => ({ label: ['Incoming', 'Middle', 'Outgoing'].filter((_, i) => mask & (1 << i)).join(' + '),
        run: () => run({ incoming: !!(mask & 1), middle: !!(mask & 2), outgoing: !!(mask & 4) }) })) ];
  }
  function presetMenu(event: MouseEvent, preset: Preset) {
    event.preventDefault(); event.stopPropagation();
    menuAt(event, [
      { label: 'Apply easing to…', run: () => menuAt(event, targets(s => { setCurve(preset.curve,false,undefined,true); run(() => apply(preset.curve, s)); })) },
      { label: 'Load into graph', run: () => setCurve(preset.curve,false,undefined,true) }, '-',
      { label: 'Rename…', run: () => prompt('Rename preset', preset.name, name => updatePreset(preset.id, { name })) },
      { label: 'Color…', run: () => colorPreset(preset) },
      { label: 'Duplicate', run: () => { const copy = { ...clone(preset), id: uid(), name: `${preset.name} copy`.slice(0, 100) }; persist({ ...library, groups: library.groups.map(g => g.id === group.id ? { ...g, presets: [...g.presets, copy] } : g) }); } },
      { label: 'Move to group…', run: () => menuAt(event, library.groups.map(g => ({ label: g.name, run: () => persist(movePreset(library, preset.id, g.id)) }))) },
      { label: 'Export preset…', run: () => void exportLibrary([{ ...group, presets: [preset] }]) },
      { label: 'Delete', run: () => persist({ ...library, groups: library.groups.map(g => ({ ...g, presets: g.presets.filter(p => p.id !== preset.id) })) }) }
    ]);
  }
  function updatePreset(id: string, patch: Partial<Preset>) { persist({ ...library, groups: library.groups.map(g => ({ ...g, presets: g.presets.map(p => p.id === id ? { ...p, ...patch } : p) })) }); }
  function colorPreset(preset: Preset) {
    presetColor = preset.color || '#ff6b1a';
    const body = document.createElement('div'); body.className = 'field';
    const picker = mount(ColorField, { target: body, props: { api, label: 'Preset color', get: () => presetColor, edit: local('Preset color',v => presetColor = String(v)) } });
    PM.modal({ title: 'Preset color', body, onClose: () => void unmount(picker), actions: [{ label: 'Cancel' }, { label: 'Use native color', run: () => updatePreset(preset.id, { color: '' }) }, { label: 'Save', pri: true, run: () => updatePreset(preset.id, { color: presetColor }) }] });
  }
  function groupMenu(event: MouseEvent, g: Group) {
    event.preventDefault(); event.stopPropagation();
    menuAt(event, [{ label: 'Rename group…', run: () => prompt('Rename group', g.name, name => persist({ ...library, groups: library.groups.map(v => v.id === g.id ? { ...v, name } : v) })) },
      { label: 'Export group…', run: () => void exportLibrary([g]) },
      { label: 'Delete group', disabled: library.groups.length === 1, run: () => { const groups = library.groups.filter(v => v.id !== g.id); if (groups.length) persist({ ...library, groups, active: groups[0]!.id }); } }]);
  }
  function pathTool(clean = false) {
    run(() => {
      const commands = clean ? cleanupPath(PM) : positionController(PM);
      edit(commands, clean ? 'Clean motion path' : 'Create position controller');
      const controllers = commands.flatMap(command => command.type === 'replace_keyframes' && command.path === 'opacity' && typeof command.target === 'string' && PM.L(command.target)?.d?.easingPath ? [command.target] : []);
      if (controllers.length) {
        PM.selectLayers(controllers);
        PM.sel.keys = controllers.flatMap(id => PM.L(id).p.opacity.kf.map((k:any) => k.i));
        PM.sel.chan = 'opacity'; PM.bus.emit('sel');
        status = clean ? 'Path cleaned · Path progress keys selected' : 'Path progress keys selected';
      }
    });
  }
  function more(event: MouseEvent) {
    PM.menu(event.currentTarget, [
      { label: 'Load selected curve', run: () => run(() => { const ts = tracks(PM); if (ts.length !== 1) throw new Error('Select keys from one property, or use Copy to choose a curve'); setCurve(loadCurve(ts[0]!),false,undefined,true); }) },
      { label: 'Overshoot expression', run: () => run(() => edit(expressionCommands(tracks(PM), 'overshoot'), 'Add overshoot expression')) },
      { label: 'Bounce expression', run: () => run(() => edit(expressionCommands(tracks(PM), 'bounce'), 'Add bounce expression')) },
      { label: 'Remove expressions', run: () => run(() => edit(tracks(PM).map(t => ({ type: 'set_expression', target: t.layerId, path: t.path, expression: null })), 'Remove expressions')) }, '-',
      { label: 'Linear interpolation', run: () => run(() => edit(interpolate(tracks(PM), 'linear'), 'Linear interpolation')) },
      { label: 'Continuous interpolation', run: () => run(() => edit(interpolate(tracks(PM), 'continuous'), 'Continuous interpolation')) },
      { label: 'Auto Bezier interpolation', run: () => run(() => edit(interpolate(tracks(PM), 'auto'), 'Auto Bezier interpolation')) },
      { label: 'Clean motion path', run: () => pathTool(true) },
      { label: 'Create position controller', run: () => pathTool() }, '-',
      { label: 'Import presets…', run: () => fileInput.click() },
      { label: 'Import preset folder…', run: () => importFolder.click() },
      { label: 'Export library…', run: () => void exportLibrary() },
      { label: 'Panel settings', on: settings, run: () => settings = !settings }
    ]);
  }
  function copyMenu(event: MouseEvent) {
    event.preventDefault(); menuAt(event, [
      { label: 'Copy keyframes', run: () => copy() },
      { label: 'Duplicate at playhead', run: () => run(() => edit(duplicate(tracks(PM), PM.time, keyId), 'Duplicate keyframes')) },
      { label: 'Mirror at playhead', run: () => run(() => edit(duplicate(tracks(PM), PM.time, keyId, true), 'Mirror keyframes')) }
    ]);
  }
  function coordinate(index: number, side: 'in' | 'out', axis: 't' | 'v', value?: number) {
    const p = handleControlPoint(curve.points, index, side);
    if (value === undefined) return p?.[axis] ?? 0;
    if (!p) return 0;
    const next = clone(curve); p[axis] = value; setHandleFromPoint(next.points, index, side, p); setCurve(next); return value;
  }
  function drag(event: DragEvent, kind: 'group' | 'preset', id: string) { event.dataTransfer?.setData('application/x-powermove-easing', JSON.stringify({ kind, id })); }
  function drop(event: DragEvent, groupId = group.id, beforeId?: string) {
    event.preventDefault(); event.stopPropagation();
    if (event.dataTransfer?.files.length) { void importFiles(event.dataTransfer.files); return; }
    run(() => {
      const data = JSON.parse(event.dataTransfer?.getData('application/x-powermove-easing') || '{}');
      if (data.kind === 'preset') persist(movePreset(library, data.id, groupId, beforeId));
      else if (data.kind === 'group' && data.id !== groupId) {
        const moved = library.groups.find(g => g.id === data.id);
        if (!moved) return;
        const groups = library.groups.filter(g => g.id !== moved.id); groups.splice(groups.findIndex(g => g.id === groupId), 0, moved); persist({ ...library, groups });
      }
    });
  }
  $effect(() => {
    if (!hover || !prefs.preview || window.matchMedia('(prefers-reduced-motion: reduce)').matches) { preview = 0; return; }
    const current = hover.curve; let frame = 0, start = performance.now();
    const tick = (time: number) => { preview = evaluate(current, Math.min(1, ((time - start) % 1600) / 1000)); frame = requestAnimationFrame(tick); };
    frame = requestAnimationFrame(tick); return () => cancelAnimationFrame(frame);
  });
  function shortcuts(event: KeyboardEvent) {
    if ((event.target as HTMLElement).matches('input,textarea,select')) return;
    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'c') { event.preventDefault(); event.stopPropagation(); copy(); }
    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'v') { event.preventDefault(); event.stopPropagation(); paste(event.shiftKey); }
    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'z') { event.preventDefault(); event.stopPropagation(); graphUndo(event.shiftKey); }
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions (Panel-scoped shortcuts and file drop target; controls remain keyboard accessible.) -->
<div class="easing-panel" data-svelte-panel={panelId} bind:this={panel} onkeydown={shortcuts} role="group" aria-label="Easing"
  ondragover={e => e.preventDefault()} ondrop={e => drop(e)}
  onpointerover={e => { hint = e.target instanceof Element ? e.target.closest('button[title]')?.getAttribute('title') || '' : ''; }}
  onpointerleave={() => hint = ''}
  style={`--ease-ink:${({ native:'var(--accent)', blue:'var(--blue)', green:'var(--green)', violet:'var(--violet)', mono:'var(--tx)' })[prefs.palette]};`}>
  <input class="panel-sr-only" type="file" tabindex="-1" accept=".json,.kease" multiple bind:this={fileInput} onchange={e => { if (e.currentTarget.files) void importFiles(e.currentTarget.files); }} aria-label="Import easing presets" />
  <input class="panel-sr-only" type="file" tabindex="-1" webkitdirectory multiple bind:this={importFolder} onchange={e => { if (e.currentTarget.files) void importFiles(e.currentTarget.files); }} aria-label="Import easing preset folder" />
  <div class="easing-toolbar" role="toolbar" aria-label="Easing tools">
    <button class="iconbtn" title="More easing tools" aria-label="More easing tools" onclick={more}><Icon {PM} name="more" /></button>
    <button class="iconbtn copy" title="Copy keyframes" aria-label="Copy keyframes" onclick={copy} oncontextmenu={copyMenu}><Icon {PM} name="copy" />{#if clipboard.length}<span>{clipboard.reduce((n,t) => n+t.keys.length,0)}</span>{/if}</button>
    <button class="iconbtn" title="Paste easing" aria-label="Paste easing" disabled={!clipboard.length} onclick={e => paste(false,e)} oncontextmenu={e => { e.preventDefault(); menuAt(e, targets(s => paste(false, undefined, s))); }}><Icon {PM} name="bezier" /></button>
    <button class="iconbtn" title="Paste values" aria-label="Paste values" disabled={!clipboard.length} onclick={e => paste(true,e)}><Icon {PM} name="clock" /></button>
    {#if prefs.view === 'toggle'}<button class="iconbtn" title="Toggle graph and library" aria-label="Toggle graph and library" onclick={() => preference('tab',prefs.tab === 'graph' ? 'library' : 'graph')}><Icon {PM} name="panel" /></button>{/if}
  </div>
  {#if prefs.view === 'both' || prefs.tab === 'graph'}
    <div class="graph-wrap" role="group" aria-label="Curve editor" oncontextmenu={e => { e.preventDefault(); menuAt(e,[{label:'Reset curve',run:() => setCurve(cubic())},{label:'Mirror curve',run:() => setCurve(mirror(curve))},{label:'Save preset…',run:savePreset},{label:'Input coordinates',on:coordinates,run:() => coordinates=!coordinates}]); }}><CurveEditor {curve} onChange={change} /></div>
    <div class="easing-toolbar graph-tools" role="toolbar" aria-label="Graph tools">
      <button class="iconbtn" title="Undo curve" aria-label="Undo curve" disabled={!undo.length} onclick={() => graphUndo()}><Icon {PM} name="undo" /></button>
      <button class="iconbtn" title="Redo curve" aria-label="Redo curve" disabled={!redo.length} onclick={() => graphUndo(true)}><Icon {PM} name="redo" /></button>
      <button class="iconbtn" class:on={coordinates} title="Input coordinates" aria-label="Input coordinates" aria-pressed={coordinates} onclick={() => coordinates = !coordinates}><Icon {PM} name="sliders" /></button>
      <button class="iconbtn" title="Mirror curve" aria-label="Mirror curve" onclick={() => setCurve(mirror(curve))}><Icon {PM} name="rotate" /></button>
      <button class="iconbtn" title="Save easing preset" aria-label="Save easing preset" onclick={savePreset}><Icon {PM} name="plus" /></button>
      <button class="iconbtn" class:on={autoApply} title="Auto apply" aria-label="Auto apply" aria-pressed={autoApply} onclick={() => autoApply = !autoApply}><Icon {PM} name="type" /></button>
      <button class="iconbtn" title="Apply curve" aria-label="Apply curve" onclick={e => run(() => apply(curve, sections(e)))} oncontextmenu={e => { e.preventDefault(); menuAt(e, targets(s => run(() => apply(curve,s)))); }}><Icon {PM} name="enter" /></button>
    </div>
    {#if coordinates}
      <div class="easing-fields">
        {#if curve.points.length > 2}<Row {api} label="Point"><SelectField {api} label="Curve point" get={() => coordinatePoint} options={curve.points.map((_,i) => ({v:i,label:String(i+1)}))} edit={local('Curve point',v => coordinatePoint = Number(v))} /></Row>{/if}
        {#each (curve.points.length === 2 ? [{index:0,side:'out' as const},{index:1,side:'in' as const}] : [{index:Math.min(coordinatePoint,curve.points.length-1),side:'out' as const},{index:Math.min(coordinatePoint,curve.points.length-1),side:'in' as const}]) as field}
          {#if handleControlPoint(curve.points,field.index,field.side)}
            <Row {api} label={field.side === 'out' ? 'Outgoing' : 'Incoming'} pair>
              <NumField {api} label="X" ariaLabel={`${field.side} handle X`} get={() => coordinate(field.index,field.side,'t')} min={0} max={1} step={.01} precision={3} edit={local('Handle X',v => coordinate(field.index,field.side,'t',Number(v)))} />
              <NumField {api} label="Y" ariaLabel={`${field.side} handle Y`} get={() => coordinate(field.index,field.side,'v')} step={.01} precision={3} edit={local('Handle Y',v => coordinate(field.index,field.side,'v',Number(v)))} />
            </Row>
          {/if}
        {/each}
      </div>
    {/if}
  {/if}
  {#if prefs.view === 'both' || prefs.tab === 'library'}
    <div class="preset-groups" role="tablist" aria-label="Preset groups">
      {#each library.groups as g (g.id)}
        <button class="group-tab" class:on={g.id === group.id} role="tab" aria-selected={g.id === group.id} tabindex={g.id === group.id ? 0 : -1} draggable="true" ondragstart={e => drag(e,'group',g.id)} ondragover={e => e.preventDefault()} ondrop={e => drop(e,g.id)} onclick={() => persist({...library,active:g.id})} oncontextmenu={e => groupMenu(e,g)}
          onkeydown={e => { if (['ArrowLeft','ArrowRight','Home','End'].includes(e.key)) { e.preventDefault(); const i=library.groups.indexOf(g), n=e.key === 'Home' ? 0 : e.key === 'End' ? library.groups.length-1 : (i+(e.key === 'ArrowRight'?1:-1)+library.groups.length)%library.groups.length; persist({...library,active:library.groups[n]!.id}); (e.currentTarget.parentElement?.querySelectorAll('button')[n] as HTMLButtonElement)?.focus(); } }}>{g.name}</button>
      {/each}
      <button class="iconbtn" title="New preset group" aria-label="New preset group" onclick={addGroup}><Icon {PM} name="plus" /></button>
    </div>
    <div class="preset-library" role="group" aria-label="Easing presets" oncontextmenu={e => { e.preventDefault(); menuAt(e,[{label:'Import presets…',run:() => fileInput.click()},{label:'Export library…',run:() => void exportLibrary()},{label:'Panel settings',run:() => settings = true}]); }} style={`--preset-columns:${prefs.columns || 'auto-fill'};--preset-size:${prefs.size}px;`}>
      {#each [false,true] as multi}
        <div class="preset-grid" class:multi role="group" aria-label={multi?'Multi-keyframe presets':'Standard presets'} style={`grid-template-columns:${prefs.columns ? `repeat(${prefs.columns},minmax(0,1fr))` : `repeat(auto-fill,minmax(${prefs.size}px,1fr))`};`}>
          {#each group.presets.filter(p => (p.curve.points.length > 2) === multi) as preset (preset.id)}
            <button class="easing-preset" title={preset.name} aria-label={`Apply ${preset.name}`} style={`--preset-ink:${preset.color || 'var(--tx-2)'};`} draggable="true"
              ondragstart={e => drag(e,'preset',preset.id)} ondragover={e => e.preventDefault()} ondrop={e => drop(e,group.id,preset.id)}
              onpointerenter={() => hover = preset} onpointerleave={() => hover = null} onfocus={() => hover = preset} onblur={() => hover = null}
              onclick={e => setCurve(preset.curve,true,e)} oncontextmenu={e => presetMenu(e,preset)}>
              <svg viewBox="0 0 44 32" aria-hidden="true"><path d={path(preset.curve,44,32,3,...bounds(preset.curve))} fill="none" stroke="currentColor" stroke-width="1.35" /></svg>
              {#if prefs.labels !== 'none'}<span>{prefs.labels === 'name' ? preset.name : `${Math.round(preset.curve.points[0]!.outEase.influence)} / ${Math.round(preset.curve.points.at(-1)!.inEase.influence)}`}</span>{/if}
            </button>
          {/each}
        </div>
      {/each}
      {#if !group.presets.length}<div class="empty">Save a curve or import presets</div>{/if}
    </div>
  {/if}
  {#if settings}
    <div class="easing-fields">
      <Row {api} label="Layout"><SelectField {api} get={() => prefs.view} label="Panel layout" options={[{v:'both',label:'Graph + library'},{v:'toggle',label:'Toggle views'}]} edit={local('Layout',v => preference('view',v))} /></Row>
      <Row {api} label="Columns"><NumField {api} get={() => prefs.columns} min={0} max={12} step={1} label="Columns (0 = automatic)" edit={local('Columns',v => preference('columns',Number(v)))} /></Row>
      <Row {api} label="Preset size"><SelectField {api} get={() => prefs.size} options={[32,44,56,72].map(v=>({v,label:`${v} px`}))} edit={local('Preset size',v => preference('size',Number(v)))} /></Row>
      <Row {api} label="Labels"><SelectField {api} get={() => prefs.labels} options={['none','name','influence']} edit={local('Labels',v => preference('labels',v))} /></Row>
      <Row {api} label="Curve color"><SelectField {api} get={() => prefs.palette} options={['native','blue','green','violet','mono']} edit={local('Curve color',v => preference('palette',v))} /></Row>
      <Row {api} label="Preview"><ToggleField {api} get={() => prefs.preview} edit={local('Preview',v => preference('preview',!!v))} /></Row>
    </div>
  {/if}
  <div class="easing-preview" aria-hidden="true">{#if hover && prefs.preview}<i style={`left:${Math.max(0,Math.min(1,preview))*100}%;`}></i>{/if}</div>
  <div class="easing-info" role="status" aria-live="polite">{info}</div>
</div>

<style>
  .easing-panel { display:flex;flex-direction:column;width:100%;min-width:0;padding:0 12px 8px;color:var(--tx); }
  .easing-toolbar { display:flex;align-items:center;gap:8px;height:40px;flex:none; }
  .easing-toolbar .iconbtn { flex:1;min-width:0;height:28px; }
  .easing-toolbar .iconbtn.on { color:var(--accent-tx);background:var(--ctl-raised); }
  .easing-toolbar .copy { position:relative; }
  .copy span { position:absolute;right:0;top:0;font-size:9px;color:var(--accent-tx); }
  .graph-wrap { --accent:var(--ease-ink); }
  .graph-tools { gap:3px;height:42px; }
  .easing-fields { padding:4px 0 8px; }
  .preset-groups { display:flex;align-items:center;gap:4px;padding:2px 0 6px;overflow-x:auto;min-height:34px; }
  .preset-groups .iconbtn { width:28px;height:26px;flex:none; }
  .group-tab { flex:1 0 28px;height:26px;max-width:140px;border:0;border-radius:var(--r-sm);background:transparent;color:var(--tx-2);font-size:var(--fs-sm);overflow:hidden;text-overflow:ellipsis;white-space:nowrap;padding:0 8px; }
  .group-tab.on { background:var(--ctl-raised);color:var(--accent-tx); }
  .group-tab:focus-visible,.easing-preset:focus-visible { outline:2px solid var(--accent);outline-offset:1px; }
  .preset-library { display:flex;flex-direction:column;gap:16px;padding:4px 0 2px; }
  .preset-grid { display:grid;gap:6px;min-width:0; }
  .preset-grid:empty { display:none; }
  .easing-preset { display:flex;flex-direction:column;align-items:center;justify-content:center;min-width:0;gap:4px;padding:2px;border:0;border-radius:var(--r-xs);background:transparent;color:var(--preset-ink); }
  .easing-preset svg { width:100%;height:auto;max-height:var(--preset-size);aspect-ratio:44 / 32; }
  .easing-preset:hover,.easing-preset:focus-visible { background:var(--ctl-raised); }
  .easing-preset span { width:100%;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-size:10px;line-height:14px;color:var(--tx-2); }
  .easing-preview { position:relative;height:8px;overflow:hidden;flex:none; }
  .easing-preview i { position:absolute;bottom:2px;width:4px;height:4px;border-radius:50%;background:var(--ease-ink);transform:translateX(-100%); }
  .easing-info { min-height:24px;display:flex;align-items:center;font-size:var(--fs-xs);line-height:16px;color:var(--tx-2);overflow-wrap:anywhere; }
</style>
