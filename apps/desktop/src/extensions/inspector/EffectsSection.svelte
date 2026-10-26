<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { inspectorContext, type EditBinding } from './context';
  import { validMatteSource, type EditCommand } from 'powermove';
  import { evaluatedValue } from './multi-edit';
  import ParameterList from './ParameterList.svelte';
  /* App-local effect snapshots survive layer/inspector remounts. */
  import { effectPasteCommands } from './effect-clipboard';
  import Icon from './Icon.svelte';
  import { inspectorRefresh } from './refresh.svelte.js';
  import { provideEffectControls } from './effect-controls';

  const { api, doc, transport, mixed, edit: inspectorEdit, inspector } = inspectorContext();
  const { ToggleField, SelectField, Row } = api.ui.controls;
  provideEffectControls();

  let { layer }: { layer: any } = $props();

  let openVersion = $state(0);
  let selectedIds = $state<string[]>([]);
  let selectionAnchor = $state<string | null>(null);
  let effectsSection: HTMLElement;
  const effects = $derived((inspectorRefresh.version, doc.tick.structure, doc.proj, [...(layer.fx ?? [])]));

  function swapColors(effect: any, keys: [string, string]): void {
    const [first, second] = keys;
    const read = (key: string) => String(api.anim.evP(layer, effect.p[key], transport.time, key) ?? effect.p[key]?.v ?? '#000000');
    const values = [read(second), read(first)];
    inspectorEdit.apply(keys.map((key, index) => ({
      type: 'set_property' as const, target: layer.id, path: `${effect.id}.${key}`,
      value: values[index]!, time: transport.time, preserveHandEdits: false
    })), { label: 'Swap Colors', origin: 'inspector' });
    api.transport.invalidate?.();
  }

  function isOpen(effect: any): boolean {
    openVersion;
    return !!api.uiState.getFxOpen(effect);
  }

  function toggleOpen(event: MouseEvent, effect: any): void {
    event.stopPropagation();
    api.uiState.setFxOpen(effect, !isOpen(effect));
    openVersion++;
  }

  function clearSelection(): void {
    selectedIds = [];
    selectionAnchor = null;
    inspector()?.clearEffectSelection();
  }

  function clearSelectionFromBackground(event: PointerEvent): void {
    const target = event.target;
    if (!(target instanceof Element)) return;
    const effectSurface = target.closest('.fx-head, .fx-params');
    if (effectSurface && effectsSection?.contains(effectSurface)) return;
    clearSelection();
  }

  onMount(() => {
    const clearFromOutside = (event: PointerEvent) => clearSelectionFromBackground(event);
    document.addEventListener('pointerdown', clearFromOutside, true);
    return () => document.removeEventListener('pointerdown', clearFromOutside, true);
  });

  const enabled = (effect: any) => (doc.tick.values, doc.proj, transport.time, evaluatedValue(api, layer, effect.on, transport.time, `${effect.id}.$enabled`));

  function setEnabled(event: MouseEvent, effect: any): void {
    event.stopPropagation();
    inspectorEdit.apply(
      { type: 'set_effect', target: layer.id, effect: effect.id, patch: { enabled: !enabled(effect) } },
      { label: 'Toggle effect', origin: 'inspector' }
    );
    api.transport.invalidate?.();
  }

  /* After Effects' Reset: every parameter back to its default, as one step.
     Animated parameters take the default at the playhead, as a keyframe. */
  function reset(event: MouseEvent, effect: any, definition: any): void {
    event.stopPropagation();
    const commands: EditCommand[] = (definition.params ?? [])
      .filter((param: any) => param.def !== undefined && effect.p?.[param.k] !== undefined)
      .map((param: any) => ({
        type: 'set_property', target: layer.id, path: `${effect.id}.${param.k}`,
        value: param.def, time: transport.time, preserveHandEdits: false
      }));
    if (!commands.length) return;
    inspectorEdit.apply(commands.length === 1 ? commands[0]! : commands, { label: `Reset ${definition.label}`, origin: 'inspector' });
    api.transport.invalidate?.();
  }

  /* Track Matte reads as an effect, as in Effects & Presets: the layer's matte
     source and mode, shown and reset (removed) like any effect. */
  // A layer setting, not part of the effects list: read it through the document ticks.
  const matteSource = $derived((inspectorRefresh.version, doc.tick.structure, doc.tick.values, doc.tick.history, doc.proj, layer.matteSource ?? null));
  // Open until folded, as a newly applied effect is.
  const matteFold = () => ({ id: `matte-folded:${layer.id}` }) as any;
  const matteOpen = $derived.by(() => { void openVersion; return !api.uiState.getFxOpen(matteFold()); });
  const matteSources = $derived((doc.tick.structure, doc.proj,
    api.project.get().layers.filter((candidate: any) => candidate.id !== layer.id && validMatteSource(api.project.get().layers, layer, candidate.id))));
  const matteMode = $derived((doc.tick.values, transport.time, String(evaluatedValue(api, layer, layer.matteMode, transport.time, 'l.matteMode') ?? 'alpha')));
  function toggleMatte(event: MouseEvent): void {
    event.stopPropagation();
    api.uiState.setFxOpen(matteFold(), matteOpen);
    openVersion++;
  }
  function removeMatte(event: MouseEvent): void {
    event.stopPropagation();
    inspectorEdit.apply({ type: 'set_layer', target: layer.id, patch: { matteSource: null } }, { label: 'Remove Track Matte', origin: 'inspector' });
    api.transport.invalidate?.();
  }
  const matteModeEdit = (mode: (current: string) => string, label: string): EditBinding => ({
    mode: 'command', label, origin: 'inspector',
    command: (value) => ({ type: 'set_property', target: layer.id, path: 'l.matteMode', value: mode(String(value)) as any, time: transport.time, preserveHandEdits: false })
  });
  const matteUse = $derived(matteMode.startsWith('luma') ? 'luma' : 'alpha');
  const matteInverted = $derived(matteMode.endsWith('-inverted'));
  const modeFor = (use: string, inverted: boolean) => `${use}${inverted ? '-inverted' : ''}`;

  function remove(event: MouseEvent, effect: any): void {
    event.stopPropagation();
    const result = inspectorEdit.apply(
      { type: 'remove_effect', target: layer.id, effect: effect.id },
      { label: 'Remove effect', origin: 'inspector' }
    );
    if (result?.ok !== false) selectedIds = selectedIds.filter((id) => id !== effect.id);
    api.transport.invalidate?.();
  }

  function selected(effect: any): boolean {
    return selectedIds.includes(effect.id);
  }

  function selectEffect(event: MouseEvent, effect: any): void {
    const ordered = effects.map((candidate: any) => candidate.id);
    if (event.shiftKey && selectionAnchor && ordered.includes(selectionAnchor)) {
      const start = ordered.indexOf(selectionAnchor);
      const end = ordered.indexOf(effect.id);
      const range = ordered.slice(Math.min(start, end), Math.max(start, end) + 1);
      selectedIds = event.metaKey || event.ctrlKey ? [...new Set([...selectedIds, ...range])] : range;
    } else if (event.metaKey || event.ctrlKey) {
      selectedIds = selected(effect)
        ? selectedIds.filter((id) => id !== effect.id)
        : [...selectedIds, effect.id];
    } else selectedIds = [effect.id];
    selectionAnchor = effect.id;
    inspector()?.setEffectSelection(layer.id, selectedIds);
    (event.currentTarget as HTMLElement).focus();
  }

  function focusEffect(id: string | null): void {
    if (!id) return;
    void tick().then(() => document.querySelector<HTMLElement>(
      `[data-effect-id="${CSS.escape(id)}"]`
    )?.focus());
  }

  function copySelected(fallback: any): void {
    const available = new Set(effects.map((effect: any) => effect.id));
    const active = selectedIds.filter((id) => available.has(id));
    const ids = active.length ? new Set(active) : new Set([fallback.id]);
    const copied = effects.filter((effect: any) => ids.has(effect.id));
    selectedIds = copied.map((effect: any) => effect.id);
    selectionAnchor = fallback.id;
    inspector()?.setEffectSelection(layer.id, selectedIds);
    api.commands.run('copyLayers');
  }

  function pasteEffects(): void {
    const commands = effectPasteCommands(layer.id)
      .filter((command) => !!api.effects.get(command.effect));
    if (!commands.length) {
      api.ui.toast?.('Copy an effect first');
      return;
    }
    const before = new Set(effects.map((effect: any) => effect.id));
    const result = inspectorEdit.apply(commands.length === 1 ? commands[0]! : commands, {
      label: commands.length === 1 ? 'Paste effect' : 'Paste effects',
      origin: 'inspector'
    });
    if (result?.ok === false) return;
    const pasted = (layer.fx ?? []).filter((effect: any) => !before.has(effect.id));
    if (pasted.length) {
      selectedIds = pasted.map((effect: any) => effect.id);
      selectionAnchor = pasted.at(-1)?.id ?? null;
      focusEffect(selectionAnchor);
    }
    api.transport.invalidate?.();
  }

  function deleteSelected(fallback: any): void {
    const available = new Set(effects.map((effect: any) => effect.id));
    const active = selectedIds.filter((id) => available.has(id));
    const ids = active.length ? new Set(active) : new Set([fallback.id]);
    const removing = effects.filter((effect: any) => ids.has(effect.id));
    if (!removing.length) return;
    const first = effects.findIndex((effect: any) => ids.has(effect.id));
    const survivors = effects.filter((effect: any) => !ids.has(effect.id));
    const next = survivors[Math.min(first, Math.max(0, survivors.length - 1))] ?? null;
    const commands: EditCommand[] = removing.map((effect: any) => ({
      type: 'remove_effect', target: layer.id, effect: effect.id
    }));
    const result = inspectorEdit.apply(commands.length === 1 ? commands[0]! : commands, {
      label: commands.length === 1 ? 'Remove effect' : 'Remove effects',
      origin: 'inspector'
    });
    if (result?.ok === false) return;
    selectedIds = next ? [next.id] : [];
    selectionAnchor = next?.id ?? null;
    focusEffect(selectionAnchor);
    api.transport.invalidate?.();
  }

  function moveSelection(event: KeyboardEvent, effect: any, direction: number): void {
    const index = effects.findIndex((candidate: any) => candidate.id === effect.id);
    const next = effects[Math.max(0, Math.min(effects.length - 1, index + direction))];
    if (!next || next.id === effect.id) return;
    if (event.shiftKey) {
      const anchor = selectionAnchor ?? effect.id;
      const start = effects.findIndex((candidate: any) => candidate.id === anchor);
      const end = effects.findIndex((candidate: any) => candidate.id === next.id);
      selectedIds = effects.slice(Math.min(start, end), Math.max(start, end) + 1).map((candidate: any) => candidate.id);
    } else selectedIds = [next.id];
    selectionAnchor ??= effect.id;
    focusEffect(next.id);
  }

  /* Effects render in stack order, so the list index is the render order. */
  function reorder(effect: any, index: number): void {
    const from = effects.findIndex((candidate: any) => candidate.id === effect.id);
    const to = Math.max(0, Math.min(effects.length - 1, index));
    if (from < 0 || from === to) return;
    const result = inspectorEdit.apply(
      { type: 'set_effect', target: layer.id, effect: effect.id, patch: { index: to } },
      { label: 'Reorder effect', origin: 'inspector' }
    );
    if (result?.ok === false) return;
    focusEffect(effect.id);
    api.transport.invalidate?.();
  }

  let draggingId = $state<string | null>(null);
  /* Insertion slot between rows: 0 = above the first effect, length = below the last. */
  let dropSlot = $state<number | null>(null);

  function dragStart(event: DragEvent, effect: any): void {
    draggingId = effect.id;
    event.dataTransfer?.setData('application/x-powermove-effect', effect.id);
    if (event.dataTransfer) event.dataTransfer.effectAllowed = 'move';
  }

  function dragOver(event: DragEvent, effect: any): void {
    if (!draggingId) return;
    event.preventDefault();
    if (event.dataTransfer) event.dataTransfer.dropEffect = 'move';
    const row = event.currentTarget as HTMLElement;
    const bounds = row.getBoundingClientRect();
    const index = effects.findIndex((candidate: any) => candidate.id === effect.id);
    dropSlot = index + (event.clientY > bounds.top + bounds.height / 2 ? 1 : 0);
  }

  function drop(event: DragEvent): void {
    if (!draggingId || dropSlot == null) return dragEnd();
    event.preventDefault();
    const effect = effects.find((candidate: any) => candidate.id === draggingId);
    const from = effects.indexOf(effect);
    const slot = dropSlot;
    dragEnd();
    if (effect) reorder(effect, slot > from ? slot - 1 : slot);
  }

  function dragEnd(): void {
    draggingId = null;
    dropSlot = null;
  }

  function effectKeydown(event: KeyboardEvent, effect: any): void {
    const key = event.key.toLowerCase();
    const command = event.metaKey || event.ctrlKey;
    if (event.altKey && (key === 'arrowup' || key === 'arrowdown')) {
      const index = effects.findIndex((candidate: any) => candidate.id === effect.id);
      reorder(effect, index + (key === 'arrowup' ? -1 : 1));
    }
    else if (key === 'arrowup' || key === 'arrowdown') moveSelection(event, effect, key === 'arrowup' ? -1 : 1);
    else if (key === 'backspace' || key === 'delete') deleteSelected(effect);
    else if (command && key === 'c') copySelected(effect);
    else if (command && key === 'v') pasteEffects();
    else return;
    event.preventDefault();
    event.stopPropagation();
  }
</script>

<div
  bind:this={effectsSection}
  class="effects-section"
  data-effects-section
  role="group"
  aria-label="Effects section"
>
  {#if effects.length === 0 && !matteSource}
    <div class="fx-empty">Add effects from Effects &amp; Presets: double-click one, or drag it onto a layer.</div>
  {/if}
  {#if effects.length > 0}
  <div class="fx-list" role="listbox" aria-label="Effects" aria-multiselectable="true">
    {#each effects as effect, index (effect.id)}
      {@const definition = api.effects.get(effect.type, effect.version ?? 1)}
      {#if definition}
      {@const expanded = isOpen(effect)}
      {@const paramsId = `fx-params-${layer.id}-${effect.id}`}
      <div
        class="row fx-head"
        class:open={expanded}
        class:selected={selected(effect)}
        class:dragging={draggingId === effect.id}
        class:drop-before={dropSlot === index && draggingId !== null}
        class:drop-after={dropSlot === effects.length && index === effects.length - 1 && draggingId !== null}
        draggable="true"
        ondragstart={(event) => dragStart(event, effect)}
        ondragover={(event) => dragOver(event, effect)}
        ondrop={drop}
        ondragend={dragEnd}
        data-effect-id={effect.id}
        data-selected={selected(effect) ? 'true' : undefined}
        tabindex="0"
        role="option"
        aria-selected={selected(effect)}
        aria-label={`${definition.label} effect`}
        onclick={(event) => selectEffect(event, effect)}
        onkeydown={(event) => effectKeydown(event, effect)}
      >
        <button
          type="button"
          class="fx-expand"
          aria-label={`${expanded ? 'Collapse' : 'Expand'} ${definition.label}`}
          aria-expanded={expanded}
          aria-controls={paramsId}
          onclick={(event) => toggleOpen(event, effect)}
        >
          <span class="twirl" aria-hidden="true"><Icon name={expanded ? 'chevD' : 'chev'} /></span>
        </button>
        <button
          type="button"
          class="fx-badge"
          class:off={!enabled(effect)}
          title={enabled(effect) ? `Turn ${definition.label} off` : `Turn ${definition.label} on`}
          aria-label={`${enabled(effect) ? 'Disable' : 'Enable'} ${definition.label}`}
          aria-pressed={!!enabled(effect)}
          onclick={(event) => setEnabled(event, effect)}
        >fx</button>
        <span class="k fx-label">{definition.label}</span>
        <button type="button" class="fx-reset" title={`Reset ${definition.label} to its defaults`} onclick={(event) => reset(event, effect, definition)}>Reset</button>
        <button
          type="button"
          class="stopwatch fx-remove"
          title="Remove effect"
          aria-label={`Remove ${definition.label}`}
          onclick={(event) => remove(event, effect)}
        ><Icon name="x" /></button>
      </div>

      {#if expanded}
        <div class="grp fx-params" id={paramsId}>
          {#each definition.actions ?? [] as action}
            {#if action.kind === 'swap'}
              <Row {api} label=""><button type="button" class="chip" onclick={() => swapColors(effect, action.keys)}>{action.label}</button></Row>
            {/if}
          {/each}
          <!-- On/off is the fx badge, as in After Effects. -->
          <ParameterList {layer} params={definition.params ?? []} properties={effect.p ?? {}} inputs={effect.inputs ?? {}} prefix={effect.id} ui={definition.ui} />
        </div>
      {/if}
      {/if}
    {/each}
  </div>
  {/if}

  {#if matteSource}
    <div class="row fx-head fx-matte" class:open={matteOpen} data-effect-id="track-matte" role="group" aria-label="Track Matte effect">
      <button type="button" class="fx-expand" aria-label={`${matteOpen ? 'Collapse' : 'Expand'} Track Matte`} aria-expanded={matteOpen} onclick={toggleMatte}>
        <span class="twirl" aria-hidden="true"><Icon name={matteOpen ? 'chevD' : 'chev'} /></span>
      </button>
      <span class="fx-badge" aria-hidden="true">fx</span>
      <span class="k fx-label">Track Matte</span>
      <button type="button" class="fx-reset" title="Remove the track matte" onclick={removeMatte}>Reset</button>
    </div>
    {#if matteOpen}
      <div class="grp fx-params">
        <Row {api} label="Matte Layer"><SelectField {api} {mixed} label="Matte Layer" get={() => matteSource}
          edit={{ mode: 'command', label: 'Track Matte', origin: 'inspector', command: (value: unknown) => ({ type: 'set_layer', target: layer.id, patch: { matteSource: value == null ? null : String(value) } }) }}
          options={matteSources.map((source: any) => ({ v: source.id, label: `${api.project.get().layers.indexOf(source) + 1}. ${source.name}` }))} /></Row>
        <Row {api} label="Use"><SelectField {api} {mixed} label="Use" get={() => matteUse}
          edit={matteModeEdit((use) => modeFor(use, matteInverted), 'Matte Use')}
          options={[{ v: 'alpha', label: 'Alpha' }, { v: 'luma', label: 'Luma' }]} /></Row>
        <Row {api} label="Invert"><ToggleField {api} {mixed} label="Invert matte" get={() => matteInverted} variant="checkbox"
          edit={matteModeEdit((on) => modeFor(matteUse, on === 'true'), 'Invert Matte')} /></Row>
      </div>
    {/if}
  {/if}

</div>

<style>
  /* Effect Controls, after the prototype: 30px effect headers (tinted while
     open), then 32px parameter rows: stopwatch, label, and a 110px field. */
  .effects-section {
    display: flex;
    flex-direction: column;
    gap: 0;
    margin: 0 calc(8px - var(--pad));
    padding: 8px 0 4px;
  }

  .fx-list { display: flex; flex-direction: column; }

  .fx-head {
    position: relative;
    height: 30px;
    margin: 0;
    padding: 0 6px;
    gap: 8px;
    border-radius: 7px;
    background: transparent;
    cursor: default;
    outline: none;
  }
  .fx-head.open { background: var(--bg-row); }
  .fx-head.selected { background: var(--accent-dim); }

  .fx-head.dragging {
    opacity: 0.45;
  }

  .fx-head.drop-before::before,
  .fx-head.drop-after::after {
    content: '';
    position: absolute;
    left: 0;
    right: 0;
    height: 2px;
    border-radius: 1px;
    background: var(--accent);
    pointer-events: none;
  }

  .fx-head.drop-before::before {
    top: -3px;
  }

  .fx-head.drop-after::after {
    bottom: -3px;
  }

  .fx-head:focus-visible {
    outline: 2px solid color-mix(in srgb, var(--accent) 72%, transparent);
    outline-offset: 1px;
  }

  .fx-expand {
    display: grid;
    flex: none;
    align-items: center;
    align-self: stretch;
    width: 10px;
    padding: 0;
    border: 0;
    background: none;
    place-items: center;
  }
  .fx-expand :global(.twirl) { width: 10px; height: 10px; color: var(--tx-2); transform: none; }
  .fx-expand :global(.twirl svg) { width: 10px; height: 10px; }

  .fx-label {
    flex: 1;
    min-width: 0;
    color: var(--tx);
    font-size: 13px;
    font-weight: var(--fw-semibold);
    line-height: 16px;
    text-align: left;
  }

  /* The fx badge is the effect's on/off switch, as in After Effects. */
  .fx-badge {
    flex: none;
    height: 18px;
    padding: 0 5px;
    border: 0;
    border-radius: 4px;
    background: rgb(255 96 25 / .15);
    color: #FF7A3D;
    font: 600 10px/12px var(--f-mono);
    cursor: pointer;
  }
  .fx-badge.off { background: var(--ink-2, var(--bg-field)); color: var(--tx-4); }
  span.fx-badge { display: inline-grid; place-items: center; cursor: default; }

  .fx-reset {
    flex: none;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--tx-3);
    font-size: 12px;
    line-height: 16px;
    cursor: pointer;
  }
  .fx-reset:hover { color: var(--tx); }

  /* Remove stays out of the way until the row is pointed at or selected. */
  .fx-head .fx-remove.fx-remove { display: none; }
  .fx-head:hover .fx-remove.fx-remove, .fx-head.selected .fx-remove.fx-remove, .fx-head .fx-remove.fx-remove:focus-visible { display: grid; }

  .fx-matte { margin-top: 0; }

  /* Parameter rows: 22px in, a 13px stopwatch (or its empty slot), the
     label, and a 110px field on the right. */
  .effects-section .fx-params :global(.row.split) {
    height: 32px;
    margin: 0;
    padding: 0 6px 0 22px;
    gap: 8px;
  }
  .effects-section .fx-params :global(.row.split > .k) {
    flex: 1;
    width: auto;
    padding-left: 0;
    color: var(--tx-label);
    font-size: 12px;
    line-height: 16px;
  }
  .effects-section .fx-params :global(.row.split:not(:has(> .stopwatch)) > .k) { width: auto; padding-left: 20px; }
  .effects-section .fx-params :global(.row.split > .stopwatch) { width: 13px; height: 24px; background: none; }
  .effects-section .fx-params :global(.row.split > .stopwatch svg) { width: 13px; height: 13px; }
  .effects-section .fx-params :global(.row.split > .vwrap) { flex: 0 0 110px; width: 110px; height: 24px; }
  .effects-section .fx-params :global(.row.split > .vwrap:has(.checkbox-field)) { justify-content: flex-start; }
  .effects-section .fx-params :global(.row.split :is(.num, .sel, .color-field, .pm-select)) { height: 24px; border-radius: 6px; background: var(--bg-field); font-size: 12px; line-height: 24px; }
  .effects-section .fx-params :global(.row.split .pm-select) { font-family: var(--f-ui); color: var(--tx-value); }
  .effects-section .fx-params :global(.row.split .pm-select .pm-select-chevron) { width: 8px; height: 8px; margin-top: -4px; }
  .effects-section .fx-params :global(.row.split .num) { color: var(--tx-value); }
  .effects-section .fx-params :global(.row.split:has(> .stopwatch.on) .num) { color: var(--accent-tx); }

  .fx-empty {
    padding: 10px 6px;
    color: var(--tx-3);
    font-size: var(--fs-sm);
    line-height: 1.45;
  }

</style>
