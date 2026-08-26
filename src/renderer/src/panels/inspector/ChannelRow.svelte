<script module lang="ts">
  let instanceSequence = 0;
</script>

<script lang="ts">
  import { doc } from '../../state/document.svelte';
  import { transport } from '../../state/transport.svelte';
  import NumField from '../../controls/NumField.svelte';
  import Row from '../../controls/Row.svelte';
  import { channelBinding, type EditBinding } from '../../controls/binding';
  import LegacyIcon from './LegacyIcon.svelte';

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
  const animated = $derived((doc.tick.values, doc.proj, (prop?.kf?.length ?? 0) > 0));
  const keyAtPlayhead = $derived((
    doc.tick.values,
    doc.proj,
    transport.time,
    !!(prop && PM.hasKeyAt(layer, prop, transport.time))
  ));
  const meta = $derived(PM.CH?.[channel] ?? {});
  const edit = $derived.by((): EditBinding => {
    if (!property) return channelBinding(PM, layer.id, channel, {
      label,
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

  function refreshValues(): void {
    PM.Inspector?.refresh?.();
    PM.invalidate?.();
  }

  function toggleStopwatch(event: MouseEvent): void {
    event.stopPropagation();
    const time = transport.time;
    PM.hist.do(`Animate ${label}`, () => {
      if (!property) {
        PM.toggleStopwatch(layer, channel, time);
        return;
      }
      if (prop.kf.length) {
        prop.v = getValue ? getValue(time) : PM.evP(layer, prop, time, channel);
        prop.kf = [];
      } else {
        PM.setKeyOn(prop, time - layer.from, prop.v, 'power', PM.proj.fps);
      }
    });
    refreshValues();
  }

  function toggleKey(event: MouseEvent): void {
    event.stopPropagation();
    const time = transport.time;
    PM.hist.do('Keyframe', () => {
      const at = PM.hasKeyAt(layer, prop, time);
      if (at) PM.removeKey(prop, at);
      else PM.setKeyOn(prop, time - layer.from, getValue ? getValue(time) : PM.evP(layer, prop, time, channel), 'power', PM.proj.fps);
    });
    refreshValues();
  }

  function addKeyframe(): void {
    const time = transport.time;
    PM.hist.do('Keyframe', () => {
      PM.setKeyOn(
        prop,
        time - layer.from,
        PM.evP(layer, prop, time, channel),
        'power',
        PM.proj.fps
      );
    });
    PM.invalidate?.();
  }

  function selectChannel(): void {
    // The legacy transform rows selected on pointerdown. Generic property rows
    // (shader uniforms/effect parameters) selected only through their menu.
    if (property) return;
    PM.sel.chan = channel;
    PM.TL?.reveal?.(layer, [channel]);
  }

  function showGraphEditor(): void {
    PM.sel.chan = channel;
    if (PM.TL) PM.TL.graph = true;
    PM.TL?.reveal?.(layer, [channel]);
  }

  function applyExpression(expression: string | null, editLabel: string): void {
    PM.Edit.apply(
      { type: 'set_expression', target: layer.id, path: channel, expression },
      { label: editLabel, origin: 'inspector' }
    );
    refreshValues();
  }

  function editExpression(): void {
    const textarea = window.document.createElement('textarea');
    textarea.className = 'code';
    textarea.value = prop?.expr || 'value + wiggle(2, 20)';
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
    PM.Edit.apply({
      type: 'replace_keyframes',
      target: layer.id,
      path: channel,
      keyframes: [],
      expression: null,
      preserveHandEdits: false
    }, { label: 'Reset', origin: 'inspector' });
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
        disabled: !prop.kf.length,
        run: () => PM.hist.do('Ease', () => PM.applyEaseTo(prop.kf, name))
      })),
      '-',
      { label: prop.expr ? 'Edit expression…' : 'Add expression…', run: editExpression },
      prop.expr ? {
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
      ><LegacyIcon {PM} name="clock" /></button>
    {/snippet}

    <div style="display:flex;align-items:center;gap:4px">
      <NumField
        {PM}
        get={() => value}
        {edit}
        {label}
        step={step ?? meta.step ?? 1}
        min={min ?? meta.min}
        max={max ?? meta.max}
        unit={unit ?? meta.unit}
        {precision}
        link={!!prop?.expr}
      />
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
        ><LegacyIcon {PM} name="diamond" /></button>
      {/if}
    </div>
  </Row>
</div>
