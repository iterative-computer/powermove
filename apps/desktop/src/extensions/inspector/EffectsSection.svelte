<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { inspectorContext, type EditBinding } from './context';
  import type { EditCommand } from 'powermove';
  import { evaluatedValue } from './multi-edit';
  import AnimatedRow from './AnimatedRow.svelte';
  import ChannelRow from './ChannelRow.svelte';
  /* App-local clipboard survives layer/inspector remounts without replacing the user's system clipboard. */
  import { copyEffects, effectPasteCommands } from './effect-clipboard';
  import Icon from './Icon.svelte';
  import { showFxMenu } from './actions';
  import { inspectorRefresh } from './refresh.svelte.js';

  const { api, doc, transport, mixed, edit: inspectorEdit, inspector } = inspectorContext();
  const { ColorField, Section, ToggleField } = api.ui.controls;

  let { layer }: { layer: any } = $props();

  let openVersion = $state(0);
  let selectedIds = $state<string[]>([]);
  let selectionAnchor = $state<string | null>(null);
  let effectsSection: HTMLElement;
  const effects = $derived((inspectorRefresh.version, doc.tick.structure, doc.proj, [...(layer.fx ?? [])]));

  function propertyEdit(path: string, label: string): EditBinding {
    return {
      mode: 'command',
      label,
      origin: 'inspector',
      command: (value) => ({
        type: 'set_property',
        target: layer.id,
        path,
        value: value as any,
        time: transport.time,
        preserveHandEdits: false
      })
    };
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
    const count = copyEffects(copied);
    api.ui.toast?.(`Copied ${count} ${count === 1 ? 'effect' : 'effects'}`);
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
  <Section {api} title="Effects" />
  {#if effects.length === 0}
    <button
      type="button"
      class="chip wide"
      onpointerdown={(event) => {
        event.preventDefault();
        showFxMenu(api, event.currentTarget, layer);
      }}
    ><Icon name="plus" />Add effect</button>
  {/if}

  {#if effects.length > 0}
  <div class="fx-list" role="listbox" aria-label="Effects" aria-multiselectable="true">
    {#each effects as effect, index (effect.id)}
      {@const definition = api.effects.get(effect.type)}
      {#if definition}
      {@const expanded = isOpen(effect)}
      {@const paramsId = `fx-params-${layer.id}-${effect.id}`}
      <div
        class="row fx-head"
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
          <span class:open={expanded} class="twirl" aria-hidden="true"><Icon name="chev" /></span>
        </button>
        <span class="k fx-label">{definition.label}</span>
        <button
          type="button"
          class:on={enabled(effect)}
          class="stopwatch"
          aria-label={`${enabled(effect) ? 'Disable' : 'Enable'} ${definition.label}`}
          aria-pressed={!!enabled(effect)}
          onclick={(event) => setEnabled(event, effect)}
        ><Icon name="eye" /></button>
        <button
          type="button"
          class="stopwatch"
          title="Remove effect"
          aria-label={`Remove ${definition.label}`}
          onclick={(event) => remove(event, effect)}
        ><Icon name="x" /></button>
      </div>

      {#if expanded}
        <div class="grp fx-params" id={paramsId}>
          <AnimatedRow {layer} path={`${effect.id}.$enabled`} label="Enabled">
            <ToggleField {api} {mixed} get={() => enabled(effect)} edit={propertyEdit(`${effect.id}.$enabled`, 'Enable effect')} label="Enabled" />
          </AnimatedRow>
          {#each definition.params ?? [] as parameter (parameter.k)}
            {@const property = effect.p?.[parameter.k]}
            {#if property}
              {#if parameter.type === 'color'}
                <AnimatedRow {layer} path={`${effect.id}.${parameter.k}`} label={parameter.label}>
                  <ColorField {api} {mixed}
                    get={() => (doc.tick.values, doc.proj, api.anim.evP(layer, property, transport.time, parameter.k))}
                    edit={propertyEdit(`${effect.id}.${parameter.k}`, parameter.label)} label={parameter.label} />
                </AnimatedRow>
              {:else if parameter.type === 'toggle'}
                <AnimatedRow {layer} path={`${effect.id}.${parameter.k}`} label={parameter.label}>
                  <ToggleField {api} {mixed}
                    get={() => (doc.tick.values, doc.proj, api.anim.evP(layer, property, transport.time, parameter.k))}
                    edit={propertyEdit(`${effect.id}.${parameter.k}`, parameter.label)} label={parameter.label} />
                </AnimatedRow>
              {:else}
                <ChannelRow

                  {layer}
                  channel={`${effect.id}.${parameter.k}`}
                  label={parameter.label}
                  {property}
                  getValue={(time) => api.anim.evP(layer, property, time, parameter.k)}
                  step={'step' in parameter ? parameter.step : undefined}
                  min={'min' in parameter ? parameter.min : undefined}
                  max={'max' in parameter ? parameter.max : undefined}
                  unit={'unit' in parameter ? parameter.unit : undefined}
                />
              {/if}
            {/if}
          {/each}
        </div>
      {/if}
      {/if}
    {/each}
  </div>
  {/if}
</div>

<style>
  .effects-section {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  /* The heading is wrapped by this component, so the global :first-child
     reset would otherwise erase the divider between Stroke and Effects. */
  .effects-section :global(.sec) {
    margin-top: 8px;
    border-top: 1px solid var(--section-line);
  }

  .fx-head {
    position: relative;
    margin-top: 4px;
    background: var(--ink-1);
    cursor: default;
    outline: none;
  }

  .fx-head.selected {
    background: var(--accent-dim);
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent) 52%, transparent);
  }

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
    width: 18px;
    place-items: center;
  }

  .fx-label {
    flex: 1;
    min-width: 0;
    color: var(--tx);
    font-weight: 500;
    text-align: left;
  }

</style>
