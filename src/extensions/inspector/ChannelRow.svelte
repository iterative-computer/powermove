<script module lang="ts">
  let instanceSequence = 0;
</script>

<script lang="ts">
  import Icon from './Icon.svelte';
  import { inspectorContext, type EditBinding } from './context';

  const { api, doc, transport } = inspectorContext();
  const { NumField, Row } = api.ui.controls;
  const { channelBinding } = api.ui.controls.binding;

  let {
    PM,
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
    showDiamond = true,
    allowContextMenu = true
  }: {
    PM: Record<string, any>;
    layer: Record<string, any>;
    channel: string;
    label: string;
    property?: Record<string, any>;
    getValue?: (time: number) => unknown;
    step?: number;
    min?: number;
    max?: number;
    unit?: string;
    precision?: number;
    showDiamond?: boolean;
    allowContextMenu?: boolean;
  } = $props();

  const instance = `channel-${++instanceSequence}`;
  const prop = $derived((doc.tick.structure, doc.proj, property ?? layer.p?.[channel]));
  const value = $derived((
    doc.tick.values,
    doc.proj,
    transport.time,
    getValue
      ? getValue(transport.time)
      : property
        ? PM.evP(layer, prop, transport.time, channel)
        : PM.ev(layer, channel, transport.time)
  ));
  const scaleLinked = $derived((doc.tick.values, doc.proj, !!layer.scaleLinked));
  const isScale = $derived(!property && channel === 'scale.x');
  const channels = $derived(isScale ? ['scale.x', 'scale.y'] : [channel]);
  const properties = $derived((doc.tick.structure, doc.proj, channels.map((key) => property ?? layer.p?.[key])));
  const animated = $derived((doc.tick.values, doc.proj, properties.some((p) => (p?.kf?.length ?? 0) > 0)));
  const expression = $derived((doc.tick.values, doc.proj, properties.find((p) => p?.expr)?.expr));
  const keyAtPlayhead = $derived((
    doc.tick.values,
    doc.proj,
    transport.time,
    properties.every((p) => !!(p && PM.hasKeyAt(layer, p, transport.time)))
  ));
  const meta = $derived(PM.CH?.[channel] ?? {});
  const edit = $derived.by((): EditBinding => {
    if (!property) return channelBinding(PM, layer.id, channel, {
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
  const scaleYEdit = $derived(channelBinding(PM, layer.id, 'scale.y', {
    label: scaleLinked ? 'Scale' : 'Scale Y',
    origin: 'inspector',
    time: () => transport.time
  }));

  function refreshValues(): void {
    PM.Inspector?.refresh?.();
    PM.invalidate?.();
  }

  function toggleStopwatch(event: MouseEvent): void {
    event.stopPropagation();
    const time = transport.time;
    PM.hist.do(`Animate ${label}`, () => {
      if (!property) {
        // Scale has one stopwatch even when its numeric axes are unlinked.
        const disable = animated;
        channels.forEach((key) => {
          if (!!layer.p[key].kf.length === disable) PM.toggleStopwatch(layer, key, time);
        });
        return;
      }
      if (prop.kf.length) {
        prop.v = getValue ? getValue(time) : PM.evP(layer, prop, time, channel);
        prop.kf = [];
      } else {
        PM.setKeyOn(prop, time - layer.from, prop.v, 'linear', PM.proj.fps);
      }
    });
    refreshValues();
  }

  function toggleKey(event: MouseEvent): void {
    event.stopPropagation();
    const time = transport.time;
    PM.hist.do('Keyframe', () => {
      const remove = keyAtPlayhead;
      properties.forEach((p, index) => {
        const at = PM.hasKeyAt(layer, p, time);
        if (remove) PM.removeKey(p, at);
        else if (!at) PM.setKeyOn(p, time - layer.from, getValue ? getValue(time) : PM.evP(layer, p, time, channels[index]), 'linear', PM.proj.fps);
      });
    });
    refreshValues();
  }

  function addKeyframe(): void {
    const time = transport.time;
    PM.hist.do('Keyframe', () => {
      properties.forEach((p, index) => PM.setKeyOn(
        p,
        time - layer.from,
        PM.evP(layer, p, time, channels[index]),
        'linear',
        PM.proj.fps
      ));
    });
    PM.invalidate?.();
  }

  function selectChannel(): void {
    // The legacy transform rows selected on pointerdown. Generic property rows
    // (shader uniforms/effect parameters) selected only through their menu.
    if (property) return;
    PM.sel.chan = channel;
    PM.TL?.reveal?.(layer, channels);
  }

  function showGraphEditor(): void {
    PM.sel.chan = channel;
    if (PM.TL) PM.TL.graph = true;
    PM.TL?.reveal?.(layer, channels);
  }

  function applyExpression(expression: string | null, editLabel: string): void {
    PM.Edit.apply(
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

    PM.modal({
      title: `Expression · ${layer.name} · ${label}`,
      body,
      width: 540,
      actions: [
        { label: 'Cancel' },
        {
          label: 'Apply',
          pri: true,
          run: () => applyExpression(textarea.value.trim() || null, 'Expression')
        }
      ]
    });
  }

  function resetChannel(): void {
    const commands = channels.map((path) => ({
      type: 'replace_keyframes',
      target: layer.id,
      path,
      keyframes: [],
      expression: null,
      preserveHandEdits: false
    }));
    PM.Edit.apply(commands.length > 1 ? commands : commands[0], { label: 'Reset', origin: 'inspector' });
    refreshValues();
  }

  function contextMenu(event: MouseEvent): void {
    if (!allowContextMenu || !prop) return;
    event.preventDefault();
    const easing = ['power', 'linear', 'easeInOut', 'expoOut', 'backOut', 'glide', 'snap'];
    PM.menu(window.document.body, [
      { header: label },
      { label: 'Add keyframe at playhead', run: addKeyframe },
      { label: 'Show in graph editor', run: showGraphEditor },
      '-',
      { header: 'Easing for all keys' },
      ...easing.map((name) => ({
        label: name,
        disabled: !animated,
        run: () => PM.hist.do('Ease', () => properties.forEach((p) => PM.applyEaseTo(p.kf, name)))
      })),
      '-',
      { label: expression ? 'Edit expression…' : 'Add expression…', run: editExpression },
      expression ? {
        label: 'Remove expression',
        run: () => applyExpression(null, 'Remove expression')
      } : null,
      { label: 'Reset', run: resetChannel }
    ].filter(Boolean), { x: event.clientX, y: event.clientY });
  }
</script>

<div
  role="group"
  aria-label={`${label} property`}
  data-channel-instance={instance}
  data-layer-id={layer.id}
  data-channel={channel}
  oncontextmenu={allowContextMenu ? contextMenu : undefined}
  onpointerdown={selectChannel}
>
  <Row {label}>
    {#snippet left()}
      <button
        type="button"
        class="stopwatch"
        class:on={animated}
        title={`Animate ${label}`}
        aria-label={`Animate ${label}`}
        aria-pressed={animated}
        onclick={toggleStopwatch}
      ><Icon name="clock" /></button>
    {/snippet}

    <div class:scale-values={isScale} style="display:flex;align-items:center;gap:4px">
      {#if isScale}
        <button type="button" class="stopwatch scale-link" class:on={scaleLinked} aria-label="Link Scale X and Y" aria-pressed={scaleLinked}
          title={scaleLinked ? 'Adjust X and Y separately' : 'Adjust X and Y together · preserve proportions'}
          onclick={() => PM.Edit.apply({ type: 'set_layer', target: layer.id, patch: { scaleLinked: !scaleLinked } }, { label: 'Link scale axes', origin: 'inspector' })}><Icon name="link" /></button>
      {/if}
      <NumField
        {PM}
        get={() => value}
        {edit}
        label={isScale ? 'Scale X' : label}
        ariaLabel={isScale ? 'Scale X' : undefined}
        step={step ?? meta.step ?? 1}
        min={min ?? meta.min}
        max={max ?? meta.max}
        unit={unit ?? meta.unit}
        {precision}
        link={!!prop?.expr}
      />
      {#if isScale}
        <span class="scale-comma" aria-hidden="true">,</span>
        <NumField
          {PM}
          get={() => PM.ev(layer, 'scale.y', transport.time)}
          edit={scaleYEdit}
          label="Scale Y"
          ariaLabel="Scale Y"
          step={step ?? meta.step ?? 1}
          min={min ?? meta.min}
          max={max ?? meta.max}
          unit={unit ?? meta.unit}
          {precision}
          link={!!layer.p?.['scale.y']?.expr}
        />
      {/if}
      {#if showDiamond}
        <button
          type="button"
          class="stopwatch kd"
          class:on={keyAtPlayhead}
          style:display={animated ? undefined : 'none'}
          title="Keyframe at playhead"
          aria-label={`Toggle keyframe for ${label} at playhead`}
          aria-pressed={keyAtPlayhead}
          onclick={toggleKey}
        ><Icon name="diamond" /></button>
      {/if}
    </div>
  </Row>
</div>

<style>
  .scale-values :global(.num) {
    min-width: 0;
    padding-inline: 3px;
  }

  .scale-comma {
    color: var(--tx-3);
    font-family: var(--f-mono);
    font-size: var(--fs-sm);
    margin-inline: -4px;
  }
</style>
