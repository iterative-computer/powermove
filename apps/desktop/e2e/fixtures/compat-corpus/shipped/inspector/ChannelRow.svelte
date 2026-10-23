<script module lang="ts">
  let instanceSequence = 0;
</script>

<script lang="ts">
  import { animateSelection, toggleSelectionKey } from './selection-animation';
  import Icon from './Icon.svelte';
  import { expressionDiagnostic, EXPRESSION_NAMES } from 'powermove';
  import type { ChannelDefinition, EditCommand, MenuContribution } from 'powermove';
  import { inspectorContext, type EditBinding } from './context';

  const { api, doc, transport, mixed, edit: inspectorEdit, inspector, timeline } = inspectorContext();
  const { NumField, Row } = api.ui.controls;
  const { channelBinding } = api.ui.controls.binding;

  let {
    layer,
    channel,
    label,
    property,
    getValue,
    step,
    min,
    max,
    unit,
    precision,
    allowContextMenu = true,
    compact = false,
    prefix
  }: {
    layer: any;
    channel: string;
    label: string;
    property?: any;
    getValue?: (time: number) => unknown;
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
  const channels = $derived(isScale ? ['scale.x', 'scale.y'] : [channel]);
  const properties = $derived((doc.tick.structure, doc.proj, channels.map((key) => property ?? layer.p?.[key])));
  const animated = $derived((doc.tick.history, doc.tick.values, doc.proj, properties.some((p) => (p?.kf?.length ?? 0) > 0)));
  const expression = $derived((doc.tick.history, doc.tick.values, doc.proj, properties.find((p) => p?.expr)?.expr));
  const keyAtPlayhead = $derived((
    doc.tick.history, doc.tick.values,
    doc.proj,
    transport.time,
    properties.every((p) => !!(p && api.anim.hasKeyAt(layer, p, transport.time)))
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

  function refreshValues(): void {
    inspector()?.refresh();
    api.transport.invalidate?.();
  }

  function toggleStopwatch(event: MouseEvent): void {
    event.stopPropagation();
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

  function addKeyframe(): void {
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
    inspectorEdit.apply(
      channels.length > 1
        ? channels.map((path) => ({ type: 'set_expression', target: layer.id, path, expression }))
        : { type: 'set_expression', target: layer.id, path: channel, expression },
      { label: editLabel, origin: 'inspector' }
    );
    refreshValues();
  }

  function editExpression(): void {
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
    const commands: EditCommand[] = channels.map((path) => ({
      type: 'replace_keyframes',
      target: layer.id,
      path,
      keyframes: [],
      expression: null,
      preserveHandEdits: false
    }));
    inspectorEdit.apply(commands.length > 1 ? commands : commands[0]!, { label: 'Reset', origin: 'inspector' });
    refreshValues();
  }

  function contextMenu(event: MouseEvent): void {
    if (!allowContextMenu || !prop) return;
    event.preventDefault();
    const easing = ['power', 'linear', 'easeInOut', 'expoOut', 'backOut', 'glide', 'snap'];
    const items: Array<MenuContribution | null> = [
      { header: label },
      { label: 'Add keyframe at playhead', icon: 'diamond', run: addKeyframe },
      animated ? { label: 'Remove animation', icon: 'x', run: () => toggleStopwatch(new MouseEvent('click')) } : null,
      { label: 'Show in graph editor', icon: 'graph', run: showGraphEditor },
      '-',
      { header: 'Easing for all keys' },
      ...easing.map((name) => ({
        label: name,
        disabled: !animated,
        run: () => api.history.do('Ease', () => properties.forEach((p) => api.anim.applyEaseTo(p.kf, name)))
      })),
      '-',
      { label: expression ? 'Edit expression…' : 'Add expression…', icon: 'code', run: editExpression },
      expression ? {
        label: 'Remove expression',
        icon: 'x',
        run: () => applyExpression(null, 'Remove expression')
      } : null,
      { label: 'Reset', icon: 'undo', run: resetChannel }
    ];
    api.ui.menu({ x: event.clientX, y: event.clientY }, items.filter((item): item is MenuContribution => item !== null));
  }
</script>

{#snippet well(fieldLabel: string, fieldEdit: EditBinding, getter: () => unknown, linked: boolean, gutter?: string)}
  <div class="well" data-prefix={gutter}>
    <NumField
      {api}
      {mixed}

      get={getter}
      edit={fieldEdit}
      label={fieldLabel}
      ariaLabel={fieldLabel}
      step={step ?? meta.step ?? 1}
      min={min ?? meta.min}
      max={max ?? meta.max}
      unit={unit ?? meta.unit}
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
    <Row {api} {label} pair={isScale}>
      {#snippet left()}
        <button type="button" class="stopwatch property-stopwatch" class:on={animated} class:at-key={keyAtPlayhead}
          aria-label={`${keyAtPlayhead ? 'Remove keyframe for' : 'Add keyframe for'} ${label}`}
          aria-pressed={keyAtPlayhead} title={keyAtPlayhead ? 'Remove keyframe' : 'Add keyframe'}
          onclick={diamondClick}>
          <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3 20 12 12 21 4 12Z"/></svg>
        </button>
      {/snippet}
      {@render well(isScale ? 'Scale X' : label, edit, () => value, !!prop?.expr, isScale ? 'X' : prefix)}
      {#if isScale}
        {@render well('Scale Y', scaleYEdit, () => api.anim.ev(layer, 'scale.y', transport.time), !!layer.p?.['scale.y']?.expr, 'Y')}
      {/if}
      {#snippet action()}
        {#if isScale}
          <button type="button" class="kf link-axes" class:on={scaleLinked} aria-label="Lock aspect ratio" aria-pressed={scaleLinked}
            title={scaleLinked ? 'Unlock aspect ratio' : 'Lock aspect ratio'}
            onclick={() => inspectorEdit.apply({ type: 'set_layer', target: layer.id, patch: { scaleLinked: !scaleLinked } }, { label: 'Link scale axes', origin: 'inspector' })}>
            <Icon name="aspectRatio" />
          </button>
        {/if}
      {/snippet}
    </Row>
  </div>
{/if}

{#if runtimeError}<p role="status" style="color:var(--danger,#ef8989);font-size:11px;margin:0 8px 6px">{runtimeError}</p>{/if}

<style>
  .link-axes {
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
  .link-axes:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  .link-axes :global(svg) { width: 16px; height: 16px; }
</style>
