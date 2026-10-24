<script lang="ts">
  import type { MenuContribution } from 'powermove';
  import { inspectorContext } from './context';
  import ChannelRow from './ChannelRow.svelte';
  import AnimatedRow from './AnimatedRow.svelte';
  import Icon from './Icon.svelte';
  import { structuredProperties } from 'powermove';
  import { inspectorRefresh } from './refresh.svelte';
  import {
    ANIMATOR_PRESETS, EASINGS, ORDERS, SHAPES, UNITS, TEXT_ANIMATOR_PROPERTIES,
    animatorMode, animatorProperties, makeAnimator, outDelay, setAnimatorMode,
    type AnimatorPreset, type TextAnimatorMode, type TextAnimatorProperty
  } from './text-animators';

  const { api, doc, transport, mixed, edit: inspectorEdit, viewer } = inspectorContext();
  const { Section, Row, SelectField, ColorField } = api.ui.controls;
  let { layer }: { layer: any } = $props();

  const MODES: Array<{ v: TextAnimatorMode; label: string }> = [
    { v: 'stagger', label: 'Stagger' }, { v: 'range', label: 'Range' }, { v: 'wave', label: 'Wave' }
  ];
  const PROPERTY_HEADINGS: Record<string, string> = { in: 'Animate from', out: 'Animate to', range: 'Selection', wave: 'Amplitude' };

  /* Animators are plain objects mutated in place; a fresh view per tick lets
     the keyed list notice changes to their fields. */
  const views = $derived((doc.tick.structure, doc.tick.values, doc.tick.history, doc.proj, inspectorRefresh.version,
    (layer.d.animators || []).map((animator: any) => {
      const mode = animatorMode(animator);
      const legacyUnit = animator.p?.unit?.v;
      return {
        animator,
        mode,
        enabled: animator.enabled !== false,
        unit: animator.unit ?? (legacyUnit === 'words' || legacyUnit === 'lines' ? legacyUnit : 'characters'),
        order: animator.order ?? 'forward',
        direction: animator.direction === 'out' ? 'out' : 'in',
        easing: animator.easing ?? 'cubicOut',
        shape: animator.shape ?? 'square',
        properties: animatorProperties(animator)
      };
    })));
  const styles = $derived((doc.tick.structure, doc.tick.values, doc.proj, inspectorRefresh.version, [...(layer.d.styles || [])]));
  const records = $derived((doc.tick.structure, doc.tick.values, doc.proj, inspectorRefresh.version, structuredProperties(layer)));

  let collapsed = $state<string[]>([]);
  let renaming = $state<string | null>(null);

  function mutate(label: string, fn: () => void) {
    inspectorEdit.mutate(label, fn, { origin: 'inspector' });
    api.transport.invalidate();
    inspectorRefresh.bump();
  }
  const set = (label: string, fn: (value: any) => void) => ({
    mode: 'set' as const,
    label,
    set: (value: any) => { fn(value); api.transport.invalidate(); inspectorRefresh.bump(); }
  });
  const bind = (path: string) => ({
    mode: 'command' as const, label: 'Edit ' + path, origin: 'inspector' as const,
    command: (value: any) => ({ type: 'set_property' as const, target: layer.id, path, value, time: transport.time, preserveHandEdits: false })
  });
  const path = (animator: any, key: string) => `ta.${animator.id}.${key}`;
  const value = (animator: any, key: string) => api.anim.evP(layer, animator.p[key], transport.time, path(animator, key));
  const fontSize = () => Number(api.anim.evP(layer, layer.d.size, transport.time, 'c.size')) || 64;

  function addPreset(preset: AnimatorPreset) {
    mutate('Add text animator', () => {
      const list = (layer.d.animators ||= []);
      const taken = new Set(list.map((animator: any) => animator.name));
      let name = preset.label, n = 2;
      while (taken.has(name)) name = `${preset.label} ${n++}`;
      list.push(makeAnimator(api.model.P, api.util.uid('ta'), name, preset, fontSize()));
    });
  }

  function presetMenu(): MenuContribution[] {
    const group = (id: string) => ANIMATOR_PRESETS.filter((preset) => preset.group === id)
      .map((preset) => ({ label: preset.label, run: () => addPreset(preset) }));
    return [
      { header: 'Animate in' }, ...group('in'), '-',
      { header: 'Loop' }, ...group('loop'), '-',
      { header: 'Custom' }, ...group('custom'), '-',
      { label: 'Style selected text', icon: 'type', run: styleSelection }
    ];
  }

  function styleSelection() {
    const current = viewer()?.textSelection;
    const selection = current?.layer === layer.id ? current : null;
    if (!selection || selection.end <= selection.start) { api.ui.toast('Select text on the canvas first'); return; }
    mutate('Style text range', () => {
      (layer.d.styles ||= []).push({ id: api.util.uid('ts'), start: selection.start, end: selection.end,
        p: { color: api.model.P('#ffffff'), weight: api.model.P(700), size: api.model.P(fontSize()) } });
    });
  }

  function animatorMenu(event: PointerEvent, animator: any) {
    const list = layer.d.animators || [], index = list.indexOf(animator);
    const move = (to: number) => mutate('Reorder text animator', () => {
      list.splice(index, 1);
      list.splice(to, 0, animator);
    });
    api.ui.menu(event.currentTarget as HTMLElement, [
      { label: 'Rename', run: () => { renaming = animator.id; } },
      { label: 'Duplicate', icon: 'copy', run: () => mutate('Duplicate text animator', () => {
        list.splice(index + 1, 0, { ...JSON.parse(JSON.stringify(animator)), id: api.util.uid('ta'), name: animator.name + ' copy' });
      }) },
      { label: 'Move up', disabled: index <= 0, run: () => move(index - 1) },
      { label: 'Move down', disabled: index >= list.length - 1, run: () => move(index + 1) },
      '-',
      { label: 'Remove', icon: 'trash', run: () => mutate('Remove text animator', () => {
        layer.d.animators = list.filter((candidate: any) => candidate !== animator);
      }) }
    ]);
  }

  function propertyMenu(event: PointerEvent, animator: any) {
    const present = new Set(animatorProperties(animator));
    api.ui.menu(event.currentTarget as HTMLElement, (Object.keys(TEXT_ANIMATOR_PROPERTIES) as TextAnimatorProperty[]).map((key) => {
      const definition = TEXT_ANIMATOR_PROPERTIES[key];
      const on = present.has(key);
      return {
        label: definition.label,
        on,
        run: () => mutate(on ? 'Remove animator property' : 'Add animator property', () => {
          if (on) delete animator.p[key];
          // Scale and opacity rest at 100 %, so a fresh row starts visibly off-rest.
          else animator.p[key] = api.model.P(key === 'scale' || key === 'opacity' ? 0 : key === 'color' ? '#ff4d6d' : 0);
        })
      };
    }));
  }

  function setDirection(animator: any, direction: 'in' | 'out') {
    if ((animator.direction === 'out' ? 'out' : 'in') === direction) return;
    mutate('Change animator direction', () => {
      animator.direction = direction;
      // An Out animation usually ends with the layer: suggest a start that
      // does, unless the start was already set by hand.
      const delay = animator.p.delay;
      if (direction === 'out' && delay && !delay.kf.length && !delay.expr && !Number(delay.v)) {
        const lines = api.render.textLayout(layer, transport.time)?.lines
          ?? String(api.anim.evP(layer, layer.d.text, transport.time, 'c.text') ?? '').split('\n').map((text) => ({ text }));
        const values = Object.fromEntries(['duration', 'stagger'].map((key) => [key, Number(value(animator, key)) || 0]));
        delay.v = outDelay(values, lines, animator.unit ?? 'characters', animator.order, Number(layer.dur) || 0);
      }
    });
  }

  function rename(animator: any, name: string) {
    renaming = null;
    const next = name.trim();
    if (next && next !== animator.name) mutate('Rename text animator', () => { animator.name = next; });
  }

  function toggleOpen(id: string) {
    collapsed = collapsed.includes(id) ? collapsed.filter((item) => item !== id) : [...collapsed, id];
  }

  function focusSelect(node: HTMLInputElement) { node.focus(); node.select(); }

  const label = (key: string) => key.replace(/([A-Z])/g, ' $1').replace(/^./, (x) => x.toUpperCase());
  const quickPresets = ANIMATOR_PRESETS.filter((preset) => preset.group !== 'custom');
</script>

{#snippet segmented(options: Array<{ v: string; label: string }>, current: string, name: string, pick: (value: any) => void)}
  <div class="segmented" role="group" aria-label={name}>
    {#each options as option (option.v)}
      <button type="button" aria-pressed={current === option.v} onclick={() => pick(option.v)}>{option.label}</button>
    {/each}
  </div>
{/snippet}

{#snippet number(animator: any, key: string, text: string, unit?: string, options: { step?: number; min?: number; max?: number; precision?: number } = {})}
  <ChannelRow {layer} channel={path(animator, key)} property={animator.p[key]} label={text} {unit}
    step={options.step ?? 1} min={options.min} max={options.max} precision={options.precision} />
{/snippet}

<div class="section-head">
  <Section {api} title="Text animators" />
  <button class="section-action" onpointerdown={(event) => { event.preventDefault(); api.ui.menu(event.currentTarget as HTMLElement, presetMenu()); }} aria-label="Add text animator" title="Add text animator"><Icon name="plus" /></button>
</div>

{#if !views.length}
  <div class="presets" role="group" aria-label="Text animator presets">
    {#each quickPresets as preset (preset.id)}
      <button type="button" class="preset" onclick={() => addPreset(preset)}>{preset.label}</button>
    {/each}
  </div>
{/if}

{#each views as view (view.animator.id)}
  {@const animator = view.animator}
  {@const open = !collapsed.includes(animator.id)}
  <div class="row ta-head" class:off={!view.enabled}>
    <button type="button" class="ta-expand" aria-label={`${open ? 'Collapse' : 'Expand'} ${animator.name}`} aria-expanded={open} onclick={() => toggleOpen(animator.id)}>
      <span class="twirl" class:open aria-hidden="true"><Icon name="chev" /></span>
    </button>
    {#if renaming === animator.id}
      <input class="ta-rename" aria-label="Animator name" value={animator.name} use:focusSelect
        onblur={(event) => rename(animator, event.currentTarget.value)}
        onkeydown={(event) => { if (event.key === 'Enter') event.currentTarget.blur(); else if (event.key === 'Escape') renaming = null; event.stopPropagation(); }} />
    {:else}
      <button type="button" class="ta-label" ondblclick={() => { renaming = animator.id; }} onclick={() => toggleOpen(animator.id)}>{animator.name}</button>
    {/if}
    <button type="button" class="stopwatch" class:on={view.enabled} aria-pressed={view.enabled}
      aria-label={`${view.enabled ? 'Disable' : 'Enable'} ${animator.name}`}
      onclick={() => mutate('Toggle text animator', () => { animator.enabled = !view.enabled; })}><Icon name={view.enabled ? 'eye' : 'eyeoff'} /></button>
    <button type="button" class="stopwatch" aria-label={`${animator.name} options`} onpointerdown={(event) => { event.preventDefault(); animatorMenu(event, animator); }}><Icon name="more" /></button>
  </div>

  {#if open}
    <div class="grp ta-params">
      <Row {api} label="Mode">
        {@render segmented(MODES, view.mode, 'Selector mode', (mode) => mutate('Change animator mode', () => setAnimatorMode(api.model.P, animator, mode)))}
      </Row>
      <Row {api} label="Based on">
        <SelectField {api} {mixed} label="Based on" get={() => view.unit} options={UNITS}
          edit={set('Change animator unit', (unit) => { animator.unit = unit; delete animator.p.unit; })} />
      </Row>
      <Row {api} label="Order">
        <div class="with-action">
          <SelectField {api} {mixed} label="Order" get={() => view.order} options={ORDERS}
            edit={set('Change animator order', (order) => { animator.order = order; })} />
          {#if view.order === 'random'}
            <button type="button" class="stopwatch" title="Shuffle" aria-label="Shuffle order"
              onclick={() => mutate('Shuffle animator order', () => { animator.seed = Math.floor(Math.random() * 1e6) + 1; })}><Icon name="rotate" /></button>
          {/if}
        </div>
      </Row>

      {#if view.mode === 'stagger'}
        <Row {api} label="Direction">
          {@render segmented([{ v: 'in', label: 'In' }, { v: 'out', label: 'Out' }], view.direction, 'Direction', (direction) => setDirection(animator, direction))}
        </Row>
        {@render number(animator, 'delay', 'Start', 's', { step: 0.01, precision: 2 })}
        {@render number(animator, 'duration', 'Duration', 's', { step: 0.01, min: 0, precision: 2 })}
        {@render number(animator, 'stagger', 'Stagger', 's', { step: 0.005, min: 0, precision: 3 })}
        <Row {api} label="Easing">
          <SelectField {api} {mixed} label="Easing" get={() => view.easing} options={EASINGS}
            edit={set('Change animator easing', (easing) => { animator.easing = easing; })} />
        </Row>
      {:else if view.mode === 'range'}
        {@render number(animator, 'start', 'Start', '%')}
        {@render number(animator, 'end', 'End', '%')}
        {@render number(animator, 'offset', 'Offset', '%')}
        <Row {api} label="Shape">
          <SelectField {api} {mixed} label="Shape" get={() => view.shape} options={SHAPES}
            edit={set('Change range shape', (shape) => { animator.shape = shape; })} />
        </Row>
        {#if view.shape === 'square' && animator.p.smoothness}
          {@render number(animator, 'smoothness', 'Softness', '%', { min: 0, max: 100 })}
        {/if}
      {:else}
        {@render number(animator, 'speed', 'Speed', '/s', { step: 0.05, precision: 2 })}
        {@render number(animator, 'spread', 'Spread', '%', { step: 0.5 })}
      {/if}
      {#if animator.p.amount}
        {@render number(animator, 'amount', 'Amount', '%')}
      {/if}

      <div class="ta-props-head">
        <span>{PROPERTY_HEADINGS[view.mode === 'stagger' ? view.direction : view.mode]}</span>
        <button type="button" class="stopwatch" aria-label={`Add property to ${animator.name}`} title="Add property" onpointerdown={(event) => { event.preventDefault(); propertyMenu(event, animator); }}><Icon name="plus" /></button>
      </div>
      {#each view.properties as key (key)}
        {@const definition = TEXT_ANIMATOR_PROPERTIES[key as TextAnimatorProperty]}
        {#if key === 'color'}
          <AnimatedRow {layer} path={path(animator, key)} label={definition.label}>
            <ColorField {api} {mixed} label={`${animator.name} color`} get={() => (doc.tick.values, transport.time, value(animator, key))} edit={bind(path(animator, key))} />
          </AnimatedRow>
        {:else}
          {@const numeric = definition as { unit?: string; step?: number; min?: number; max?: number }}
          {@render number(animator, key, definition.label, numeric.unit, { step: numeric.step, min: numeric.min, max: numeric.max })}
        {/if}
      {/each}
    </div>
  {/if}
{/each}

{#if styles.length}
  <div class="ta-props-head styles-head"><span>Text styles</span></div>
  {#each styles as style (style.id)}
    {@const prefix = 'ts.' + style.id}
    <div class="row ta-head">
      <span class="ta-label static">{`Characters ${style.start + 1}–${style.end}`}</span>
      <button type="button" class="stopwatch" aria-label="Remove text style" title="Remove"
        onclick={() => mutate('Remove text style', () => { layer.d.styles = layer.d.styles?.filter((candidate: any) => candidate.id !== style.id); })}><Icon name="x" /></button>
    </div>
    <div class="grp ta-params">
      {#each records.filter((record: any) => record.key.startsWith(prefix + '.')) as record (record.key)}
        {@const current = api.anim.evP(layer, record.prop, transport.time, record.key)}
        {#if typeof current === 'number'}<ChannelRow {layer} channel={record.key} property={record.prop} label={label(record.label)} step={1} />
        {:else}<AnimatedRow {layer} path={record.key} label={label(record.label)}><ColorField {api} {mixed} label="Range color" get={() => current} edit={bind(record.key)} /></AnimatedRow>{/if}
      {/each}
    </div>
  {/each}
{/if}

<style>
  .section-head { position: relative; }
  .section-head :global(.sec) { margin-top: 8px; border-top: 1px solid var(--section-line); }

  /* Center on the title, below the heading's 8px margin and 1px rule. */
  .section-action {
    position: absolute; right: 0; top: calc(50% + 4.5px);
    display: grid; width: 24px; height: 24px; padding: 0; place-items: center;
    transform: translateY(-50%);
    border: 0; border-radius: var(--r-xs); background: transparent; color: var(--tx-3);
  }
  .section-action:hover { background: var(--ink-1); color: var(--tx); }
  /* Icons have no intrinsic size; match the panel header actions. */
  .section-action :global(svg) { width: 13px; height: 13px; }

  .presets { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 4px; }
  .preset {
    height: var(--ctl-h); padding: 0 6px; overflow: hidden;
    border: 0; border-radius: var(--r-sm); background: var(--bg-field); color: var(--tx-2);
    font: inherit; font-size: var(--fs-xs); white-space: nowrap; text-overflow: ellipsis;
  }
  .preset:hover { background: var(--bg-row-hi); color: var(--tx); }
  .preset:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }

  .ta-head { margin-top: 4px; background: var(--ink-1); }
  .ta-head.off .ta-label { color: var(--tx-3); }
  .ta-expand { display: grid; flex: none; align-self: stretch; width: 18px; padding: 0; place-items: center; border: 0; background: transparent; color: inherit; }
  .ta-label {
    flex: 1; min-width: 0; padding: 0; overflow: hidden;
    border: 0; background: transparent; color: var(--tx);
    font: inherit; font-weight: 500; text-align: left; white-space: nowrap; text-overflow: ellipsis;
  }
  .ta-label.static { padding-left: 8px; }
  .ta-rename {
    flex: 1; min-width: 0; height: 22px; padding: 0 6px;
    border: 0; border-radius: var(--r-xs); background: var(--bg-field); color: var(--tx); font: inherit;
  }
  .ta-rename:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
  .ta-params { padding-bottom: 4px; }

  .ta-props-head {
    display: flex; align-items: center; justify-content: space-between;
    padding: 10px 0 2px; color: var(--tx-3); font-size: var(--fs-xs);
  }
  .styles-head { padding-top: 12px; }

  .with-action { display: flex; align-items: center; gap: 4px; width: 100%; min-width: 0; }
  .with-action > :global(:first-child) { flex: 1; min-width: 0; }

  .segmented { display: flex; gap: 2px; width: 100%; padding: 2px; border-radius: var(--r-sm); background: var(--bg-row); }
  .segmented button {
    flex: 1; min-width: 0; height: 22px; padding: 0 4px;
    border: 0; border-radius: calc(var(--r-sm) - 2px); background: transparent; color: var(--tx-3);
    font: inherit; font-size: var(--fs-xs);
  }
  .segmented button:hover { background: var(--ink-1); color: var(--tx); }
  .segmented button[aria-pressed='true'] { background: var(--bg-panel); color: var(--tx); }
  .segmented button:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
</style>
