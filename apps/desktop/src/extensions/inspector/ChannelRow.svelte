<script module lang="ts">
  let instanceSequence = 0;
</script>

<script lang="ts">
  import { animateSelection, toggleSelectionKey } from './selection-animation';
  import { isInspectorLayerEditable } from './multi-edit';
  import Icon from './Icon.svelte';
  import { expressionDiagnostic, EXPRESSION_NAMES } from 'powermove';
  import type { ChannelDefinition, EditCommand, MenuContribution } from 'powermove';
  import { inspectorContext, type EditBinding } from './context';

  const { api, doc, transport, mixed, edit: inspectorEdit, inspector, timeline } = inspectorContext();
  const { NumField, SliderField, Row } = api.ui.controls;
  const { channelBinding } = api.ui.controls.binding;

  let {
    layer,
    channel,
    label,
    property,
    getValue,
    defaultValue,
    step,
    min,
    max,
    unit,
    precision,
    allowContextMenu = true,
    compact = false,
    prefix,
    pairWith,
    thirdWith,
    visual = !!property
  }: {
    layer: any;
    channel: string;
    label: string;
    property?: any;
    getValue?: (time: number) => unknown;
    defaultValue?: number;
    step?: number;
    min?: number;
    max?: number;
    unit?: string;
    precision?: number;
    allowContextMenu?: boolean;
    /** Render only the well (no Row); the parent composes X / Y pairs. */
    compact?: boolean;
    /** Single-letter gutter label inside the well (X, Y, W, H). */
    prefix?: string;
    /** A second channel shown in the same row, as one value pair (Position). */
    pairWith?: string;
    /** A third channel after the pair, for X / Y / Z in one row. */
    thirdWith?: string;
    visual?: boolean;
  } = $props();

  const instance = `channel-${++instanceSequence}`;
  const prop = $derived((doc.tick.structure, doc.proj, property ?? layer.p?.[channel]));
  const value = $derived((
    doc.tick.history, doc.tick.values,
    doc.proj,
    transport.time,
    getValue
      ? getValue(transport.time)
      : property
        ? api.anim.evP(layer, prop, transport.time, channel)
        : api.anim.ev(layer, channel, transport.time)
  ));
  const runtimeError=$derived((void value,transport.time,doc.tick.values,prop?.expr ? api.anim.expressionErrors?.get(prop) : null));
  const scaleLinked = $derived((doc.tick.history, doc.tick.values, doc.proj, !!layer.scaleLinked));
  const isScale = $derived(!property && !compact && channel === 'scale.x');
  /* Position carries After Effects' Separate Dimensions switch (timeline rows). */
  const isPosition = $derived(!property && !compact && channel === 'position.x');
  const pairChannel = $derived(isScale ? 'scale.y' : !property && !compact ? pairWith : undefined);
  const thirdChannel = $derived(pairChannel ? thirdWith : undefined);
  const channels = $derived(pairChannel ? [channel, pairChannel, ...(thirdChannel ? [thirdChannel] : [])] : [channel]);
  const properties = $derived((doc.tick.structure, doc.proj, channels.map((key) => property ?? layer.p?.[key])));
  const animated = $derived((doc.tick.history, doc.tick.values, doc.proj, properties.some((p) => (p?.kf?.length ?? 0) > 0)));
  const expression = $derived((doc.tick.history, doc.tick.values, doc.proj, properties.find((p) => p?.expr)?.expr));
  const keyAtPlayhead = $derived((
    doc.tick.history, doc.tick.values,
    doc.proj,
    transport.time,
    properties.every((p) => !!(p && api.anim.hasKeyAt(layer, p, transport.time)))
  ));
  /* Grouped rows (Position X / Y / Z, Scale, …) also key each axis on its own. */
  const axisKeys = $derived((
    doc.tick.history, doc.tick.values,
    doc.proj,
    transport.time,
    new Map(channels.length > 1 ? channels.map((key, index) => {
      const p = properties[index];
      return [key, { animated: (p?.kf?.length ?? 0) > 0, atKey: !!(p && api.anim.hasKeyAt(layer, p, transport.time)) }];
    }) : [])
  ));
  const meta = $derived((api.model.CH[channel] ?? {}) as ChannelDefinition);
  const edit = $derived.by((): EditBinding => {
    if (!property) return channelBinding(layer.id, channel, {
      label: isScale && !scaleLinked ? 'Scale X' : label,
      origin: 'inspector',
      time: () => transport.time
    });
    return {
      mode: 'command',
      label,
      origin: 'inspector',
      command: (next: unknown) => ({
        type: 'set_property',
        target: layer.id,
        path: channel,
        value: next as any,
        time: transport.time,
        mode: 'auto',
        preserveHandEdits: false
      })
    };
  });
  const scaleYEdit = $derived(channelBinding(layer.id, 'scale.y', {
    label: scaleLinked ? 'Scale' : 'Scale Y',
    origin: 'inspector',
    time: () => transport.time
  }));
  const pairEdit = $derived(pairWith ? channelBinding(layer.id, pairWith, { label: `${label} Y`, origin: 'inspector', time: () => transport.time }) : scaleYEdit);
  const thirdEdit = $derived(thirdWith ? channelBinding(layer.id, thirdWith, { label: `${label} Z`, origin: 'inspector', time: () => transport.time }) : null);

  function refreshValues(): void {
    inspector()?.refresh();
    api.transport.invalidate?.();
  }

  function toggleStopwatch(event: MouseEvent): void {
    event.stopPropagation();
    if (!isInspectorLayerEditable(api, layer)) return;
    const time = transport.time;
    animateSelection(api,layer,channels,animated,time,`Animate ${label}`);
    api.anim.touch?.();
    timeline()?.reveal(layer, channels);
    refreshValues();
  }

  function toggleKey(event: MouseEvent): void {
    event.stopPropagation();
    const time = transport.time;
    toggleSelectionKey(
      api,
      layer,
      channels,
      time,
      `${keyAtPlayhead ? 'Remove keyframe for' : 'Add keyframe for'} ${label}`
    );
    refreshValues();
  }

  function diamondClick(event: MouseEvent): void {
    toggleKey(event);
  }

  function toggleAxisKey(event: MouseEvent, axisChannel: string, axisLabel: string): void {
    event.stopPropagation();
    if (!isInspectorLayerEditable(api, layer)) return;
    const atKey = axisKeys.get(axisChannel)?.atKey;
    const result = toggleSelectionKey(api, layer, [axisChannel], transport.time, `${atKey ? 'Remove keyframe for' : 'Add keyframe for'} ${axisLabel}`);
    if (result?.ok === false) { api.ui.toast?.(result.message); return; }
    timeline()?.reveal(layer, [axisChannel]);
    refreshValues();
  }

  function addKeyframe(): void {
    if (!isInspectorLayerEditable(api, layer)) return;
    const time = transport.time;
    api.history.do('Keyframe', () => {
      properties.forEach((p, index) => api.anim.setKeyOn(
        p,
        time - layer.from,
        api.anim.evP(layer, p, time, channels[index]!)!,
        'linear',
        api.project.get().fps
      ));
    });
    api.transport.invalidate?.();
  }

  function applyKeyframeEase(name: string): void {
    if (!isInspectorLayerEditable(api, layer)) return;
    api.history.do('Ease', () => properties.forEach((p) => api.anim.applyEaseTo(p.kf, name)));
  }

  function selectChannel(): void {
    // The legacy transform rows selected on pointerdown. Generic property rows
    // (shader uniforms/effect parameters) selected only through their menu.
    if (property) return;
    api.selection.set({ chan: channel });
    timeline()?.focusGraph?.(layer, channel);
  }

  function showGraphEditor(): void {
    api.selection.set({ chan: channel });
    const service = timeline();
    if (service) service.graph = true;
    service?.focusGraph?.(layer, channel);
    service?.reveal(layer, channels);
  }

  function applyExpression(expression: string | null, editLabel: string): void {
    if (!isInspectorLayerEditable(api, layer)) return;
    inspectorEdit.apply(
      channels.length > 1
        ? channels.map((path) => ({ type: 'set_expression', target: layer.id, path, expression }))
        : { type: 'set_expression', target: layer.id, path: channel, expression },
      { label: editLabel, origin: 'inspector' }
    );
    refreshValues();
  }

  function editExpression(): void {
    if (!isInspectorLayerEditable(api, layer)) return;
    const textarea = window.document.createElement('textarea');
    textarea.className = 'code';
    textarea.value = expression || 'value + wiggle(2, 20)';
    textarea.style.cssText = 'height:150px;border-radius:8px';

    const hint = window.document.createElement('div');
    hint.style.cssText = 'font-size:var(--fs-sm);color:var(--tx-3);line-height:1.6';
    hint.append(
      't · layer-local seconds   T · comp seconds   value · keyframed value',
      window.document.createElement('br'),
      'wiggle(f,a) random(s) linear(x,x0,x1,y0,y1) ease(...) loop(d,x) param("name")'
    );

    const body = window.document.createElement('div');
    body.style.cssText = 'display:flex;flex-direction:column;gap:10px';
    body.append(textarea, hint);
    const diagnostic = window.document.createElement('div');
    diagnostic.setAttribute('role','status'); diagnostic.style.color = 'var(--accent)';
    const check = () => { diagnostic.textContent = textarea.value.trim() ? expressionDiagnostic(textarea.value.trim()) ?? 'Expression is valid' : ''; };
    textarea.addEventListener('input',check); check();
    const completion = window.document.createElement('select');
    completion.setAttribute('aria-label','Insert expression name');
    completion.append(new Option('Insert function or value…',''));
    EXPRESSION_NAMES.forEach(name => completion.append(new Option(name,name)));
    completion.onchange = () => {
      if (!completion.value) return;
      textarea.setRangeText(completion.value,textarea.selectionStart,textarea.selectionEnd,'end');
      completion.value=''; textarea.focus(); check();
    };
    body.append(completion,diagnostic);

    api.ui.modal({
      title: `Expression · ${layer.name} · ${label}`,
      body,
      width: 540,
      actions: [
        { label: 'Cancel' },
        {
          label: 'Apply',
          pri: true,
          run: () => {
            const source = textarea.value.trim();
            if (source && expressionDiagnostic(source)) { check(); textarea.focus(); return false; }
            applyExpression(source || null, 'Expression');
          }
        }
      ]
    });
  }

  function resetChannel(): void {
    if (!isInspectorLayerEditable(api, layer)) return;
    const comp = api.model.curComp();
    const defaults: Record<string, number> = {
      'anchor.x': layer.type === 'null' ? 50 : 0,
      'anchor.y': layer.type === 'null' ? 50 : 0,
      'anchor.z': 0,
      'position.x': ['group', 'adjustment'].includes(layer.type) ? 0 : comp.w / 2,
      'position.y': ['group', 'adjustment'].includes(layer.type) ? 0 : comp.h / 2,
      'position.z': 0,
      'scale.x': 100, 'scale.y': 100, 'scale.z': 100,
      rotation: 0, 'rotation.x': 0, 'rotation.y': 0,
      'orientation.x': 0, 'orientation.y': 0, 'orientation.z': 0,
      opacity: layer.type === 'null' ? 0 : 100,
      skew: 0, perspective: 50
    };
    const commands: EditCommand[] = channels.flatMap((path) => {
      const value = defaultValue ?? (!property ? defaults[path] : undefined);
      const reset: EditCommand[] = [{
        type: 'replace_keyframes',
        target: layer.id,
        path,
        keyframes: [],
        expression: null,
        preserveHandEdits: false
      }];
      if (value !== undefined) reset.push({
        type: 'set_property',
        target: layer.id,
        path,
        value,
        mode: 'static',
        preserveHandEdits: false
      });
      return reset;
    });
    inspectorEdit.apply(commands.length > 1 ? commands : commands[0]!, { label: 'Reset', origin: 'inspector' });
    refreshValues();
  }

  function contextMenu(event: MouseEvent): void {
    if (!allowContextMenu || !prop) return;
    event.preventDefault();
    const locked = !isInspectorLayerEditable(api, layer);
    const easing = ['power', 'linear', 'easeInOut', 'expoOut', 'backOut', 'glide', 'snap'];
    const items: Array<MenuContribution | null> = [
      { header: label },
      { label: 'Add keyframe at playhead', icon: 'diamond', disabled: locked, run: addKeyframe },
      animated ? { label: 'Remove animation', icon: 'x', disabled: locked, run: () => toggleStopwatch(new MouseEvent('click')) } : null,
      { label: 'Show in graph editor', icon: 'graph', run: showGraphEditor },
      isPosition ? {
        label: 'Separate Dimensions', on: layer.separateDimensions !== false, disabled: locked,
        run: () => inspectorEdit.apply({ type: 'set_layer', target: layer.id, patch: { separateDimensions: layer.separateDimensions === false } }, { label: layer.separateDimensions === false ? 'Separate Dimensions' : 'Join Dimensions', origin: 'inspector' })
      } : null,
      '-',
      { header: 'Easing for all keys' },
      ...easing.map((name) => ({
        label: name,
        disabled: locked || !animated,
        run: () => applyKeyframeEase(name)
      })),
      '-',
      { label: expression ? 'Edit expression…' : 'Add expression…', icon: 'code', disabled: locked, run: editExpression },
      expression ? {
        label: 'Remove expression',
        icon: 'x',
        disabled: locked,
        run: () => applyExpression(null, 'Remove expression')
      } : null,
      { label: 'Reset', icon: 'undo', disabled: locked, run: resetChannel }
    ];
    api.ui.menu({ x: event.clientX, y: event.clientY }, items.filter((item): item is MenuContribution => item !== null));
  }
</script>

{#snippet well(fieldLabel: string, fieldEdit: EditBinding, getter: () => unknown, linked: boolean, gutter?: string, axis?: string, axisChannel?: string)}
  {@const Field = visual && Number.isFinite(min) && Number.isFinite(max) && max! > min! ? SliderField : NumField}
  {@const axisKey = axisChannel ? axisKeys.get(axisChannel) : undefined}
  <!-- data-axis names the field when a narrow inspector stacks X / Y / Z
       under the label, out of reach of the section's column heading. -->
  <div class="well" class:axis-keyed={!!axisKey} data-prefix={gutter} data-axis={axis}>
    {#if axisChannel && axisKey}
      <!-- One axis's own key; the row's diamond keys every axis together. -->
      <button type="button" class="stopwatch axis-key" class:on={axisKey.animated} class:at-key={axisKey.atKey}
        aria-label={`${axisKey.atKey ? 'Remove keyframe for' : 'Add keyframe for'} ${fieldLabel}`}
        aria-pressed={axisKey.atKey} title={axisKey.atKey ? `Remove ${fieldLabel} keyframe` : `Add ${fieldLabel} keyframe`}
        data-axis-channel={axisChannel}
        onclick={(event) => toggleAxisKey(event, axisChannel, fieldLabel)}>
        {@html api.ui.icon(axisKey.atKey ? 'diamondFill' : 'diamond')}
      </button>
    {/if}
    <Field
      angle={visual && unit === '°'}
      {api}
      {mixed}

      get={getter}
      edit={fieldEdit}
      label={fieldLabel}
      ariaLabel={fieldLabel}
      step={step ?? meta.step ?? 1}
      min={min ?? meta.min}
      max={max ?? meta.max}
      unit={thirdChannel && (unit ?? meta.unit) === 'px' ? '' : unit ?? meta.unit}
      {precision}
      link={linked}
    />
  </div>
{/snippet}

{#if compact}
  <div
    role="group"
    aria-label={`${label} property`}
    data-channel-instance={instance}
    data-layer-id={layer.id}
    data-channel={channel}
    style="display:contents"
    oncontextmenu={allowContextMenu ? contextMenu : undefined}
    onpointerdown={selectChannel}
  >
    {@render well(label, edit, () => value, !!prop?.expr, prefix)}
  </div>
{:else}
  <div
    role="group"
    aria-label={`${label} property`}
    data-channel-instance={instance}
    data-layer-id={layer.id}
    data-channel={channel}
    oncontextmenu={allowContextMenu ? contextMenu : undefined}
    onpointerdown={selectChannel}
  >
    <Row {api} {label} pair={!!pairChannel}>
      {#snippet left()}
        <button type="button" class="stopwatch property-stopwatch" class:on={animated} class:at-key={keyAtPlayhead}
          aria-label={`${keyAtPlayhead ? 'Remove keyframe for' : 'Add keyframe for'} ${label}`}
          aria-pressed={keyAtPlayhead} title={keyAtPlayhead ? 'Remove keyframe' : 'Add keyframe'}
          onclick={diamondClick}>
          {@html api.ui.icon(keyAtPlayhead ? 'diamondFill' : 'diamond')}
        </button>
        {#if isScale}
          <!-- Sits at the end of the label column, so Scale's values line up
               with every other row's (and with the X / Y / Z heading). -->
          <button type="button" class="kf link-axes" class:on={scaleLinked} aria-label="Lock aspect ratio" aria-pressed={scaleLinked}
            title={scaleLinked ? 'Unlock aspect ratio' : 'Lock aspect ratio'}
            onclick={() => inspectorEdit.apply({ type: 'set_layer', target: layer.id, patch: { scaleLinked: !scaleLinked } }, { label: 'Link scale axes', origin: 'inspector' })}>
            <Icon name="aspectRatio" />
          </button>
        {/if}
      {/snippet}
      <!-- One value per axis, X then Y (then Z), in one well. -->
      {@render well(isScale ? 'Scale X' : pairChannel ? `${label} X` : label, edit, () => value, !!prop?.expr, pairChannel ? undefined : prefix, thirdChannel ? 'X' : undefined, pairChannel ? channel : undefined)}
      {#if pairChannel}
        {@render well(isScale ? 'Scale Y' : `${label} Y`, pairEdit, () => api.anim.ev(layer, pairChannel, transport.time), !!layer.p?.[pairChannel]?.expr, undefined, thirdChannel ? 'Y' : undefined, pairChannel)}
      {/if}
      {#if thirdChannel && thirdEdit}
        {@render well(`${label} Z`, thirdEdit, () => api.anim.ev(layer, thirdChannel, transport.time), !!layer.p?.[thirdChannel]?.expr, undefined, 'Z', thirdChannel)}
      {/if}
    </Row>
  </div>
{/if}

{#if runtimeError}<p role="status" style="color:var(--danger,#ef8989);font-size:11px;margin:0 8px 6px">{runtimeError}</p>{/if}

<style>
  /* The link button sits right after the label's text; the values keep the
     same start as every other row (diamond 11 + label 78 + two 8px gaps). */
  :global(.row.split.split:has(> .link-axes) > .k.k) { width: auto; }
  :global(.row.split:has(> .link-axes) > .vwrap) { order: 2; flex: 0 0 calc(100% - 105px); margin-left: auto; }
  .link-axes {
    order: 1;
    flex: none;
    margin-left: -4px;
    position: static;
    width: 20px;
    height: 20px;
    display: grid;
    place-items: center;
    color: var(--tx-3);
    background: var(--ink-1);
    border-radius: var(--r-xs);
  }
  .link-axes:hover { color: var(--tx-2); background: var(--ink-1); }
  .link-axes.on { color: var(--accent); background: color-mix(in srgb, var(--accent) 14%, transparent); }
  /* Out of the way until the row is pointed at, as in the prototype. */
  .link-axes { opacity: 0; transition: opacity var(--dur-1); }
  :global(.row.split:hover) .link-axes, .link-axes:focus-visible { opacity: 1; }
  .link-axes:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  .link-axes :global(svg) { width: 16px; height: 16px; }
</style>
