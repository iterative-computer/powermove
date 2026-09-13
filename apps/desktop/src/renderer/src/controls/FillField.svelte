<script lang="ts">
  import { tick } from 'svelte';
  import { doc } from '../state/document.svelte';
  import { transport } from '../state/transport.svelte';
  import { EditGesture, type EditBinding } from './gesture';
  import { rowLabelId } from './context';
  import { anchorPicker, mountOverlayOnBody } from './overlay';
  import {
    clamp,
    fillCss,
    hexToRgb,
    hsvToRgb,
    normalizeFill,
    normalizeHex,
    rgbToHex,
    rgbToHsv,
    type FillValue
  } from './control-utils';
  import './controls.css';
  import type { PowermoveAPI } from '../kernel/api';

  let {
    api,
    get,
    edit,
    label = 'Fill',
    fallback = '#000000'
  }: {
    api: PowermoveAPI;
    get: () => unknown;
    edit: EditBinding;
    label?: string;
    fallback?: string;
  } = $props();

  const labelledBy = rowLabelId();
  const raw = $derived((doc.tick.values, doc.proj, transport.time, get()));
  const value = $derived(normalizeFill(api, raw, fallback));
  const gesture = $derived(new EditGesture(api, edit));
  const modes: Array<[FillValue['type'], string]> = [['solid', 'Solid'], ['linear', 'Linear'], ['radial', 'Radial'], ['none', 'None']];
  const colors = ['#FF3B30','#FF9500','#FFCC00','#34C759','#00C7BE','#0A84FF','#5E5CE6','#BF5AF2','#FF2D55','#FFFFFF','#8E8E93','#09090A'];
  const hsvChannels = [['h','H',359], ['s','S',100], ['v','B',100]] as const;
  const rgbChannels = [['r','R'], ['g','G'], ['b','B']] as const;
  let trigger = $state<HTMLButtonElement>();
  let dialog = $state<HTMLElement>();
  let open = $state(false);
  let channelsOpen = $state(false);
  let draft = $state<FillValue>({ type: 'solid', angle: 0, stops: [{ id: 'stop-1', color: '#000000', position: 0 }] });
  let before = $state<FillValue>({ type: 'solid', angle: 0, stops: [{ id: 'stop-1', color: '#000000', position: 0 }] });
  let previewing = $state(false);
  let selected = $state('');
  let hsv = $state({ h: 0, s: 0, v: 0 });

  const selectedStop = () => draft.stops.find((stop) => stop.id === selected) ?? draft.stops[0]!;
  const selectedRgb = $derived(hexToRgb(selectedStop().color) ?? { r: 0, g: 0, b: 0 });
  const selectedHex = $derived(selectedStop().color);

  function syncHsv(): void {
    hsv = rgbToHsv(hexToRgb(selectedStop().color) ?? { r: 0, g: 0, b: 0 });
  }

  function show(): void {
    previewing = false;
    before = normalizeFill(api, value, fallback);
    draft = normalizeFill(api, before, fallback);
    selected = draft.stops[0]!.id;
    channelsOpen = false;
    syncHsv();
    open = true;
    void tick().then(() => dialog?.querySelector<HTMLButtonElement>('.fill-type.on')?.focus());
  }

  function finishClose(): void {
    open = false;
    void tick().then(() => trigger?.focus());
  }

  function previewDraft(): void {
    if (!previewing) {
      gesture.begin();
      previewing = true;
    }
    gesture.write(normalizeFill(api, draft, fallback));
    api.transport.invalidate('render');
  }

  function cancelPreview(): void {
    if (previewing) {
      if (edit.mode === 'local') gesture.write(before);
      gesture.cancel();
      previewing = false;
      api.transport.invalidate('render');
    }
    finishClose();
  }

  function apply(): void {
    if (previewing) {
      gesture.write(normalizeFill(api, draft, fallback));
      gesture.commit();
      previewing = false;
      api.transport.invalidate('render');
    } else gesture.once(normalizeFill(api, draft, fallback));
    finishClose();
  }

  function setType(type: FillValue['type']): void {
    draft = normalizeFill(api, { ...draft, type }, fallback);
    selected = draft.stops[0]!.id;
    syncHsv();
    previewDraft();
  }

  function setSelectedColor(value: unknown): boolean {
    const valid = normalizeHex(value);
    if (!valid) return false;
    selectedStop().color = valid;
    syncHsv();
    previewDraft();
    return true;
  }

  function fromHsv(next: { h: unknown; s: unknown; v: unknown }): void {
    hsv = { h: Math.round(clamp(Number(next.h) || 0, 0, 359)), s: Math.round(clamp(Number(next.s) || 0, 0, 100)), v: Math.round(clamp(Number(next.v) || 0, 0, 100)) };
    selectedStop().color = rgbToHex(hsvToRgb(hsv));
    previewDraft();
  }

  function pickSv(event: PointerEvent, element: HTMLElement): void {
    const rect = element.getBoundingClientRect();
    if (!rect.width || !rect.height) return;
    fromHsv({ h: hsv.h, s: (event.clientX - rect.left) / rect.width * 100, v: 100 - (event.clientY - rect.top) / rect.height * 100 });
  }

  function pickHue(event: PointerEvent, element: HTMLElement): void {
    const rect = element.getBoundingClientRect();
    if (!rect.height) return;
    fromHsv({ ...hsv, h: (event.clientY - rect.top) / rect.height * 359 });
  }

  function startLocalDrag(event: PointerEvent, picker: (next: PointerEvent, element: HTMLElement) => void): void {
    if (event.button !== 0) return;
    event.preventDefault();
    const element = event.currentTarget as HTMLElement;
    picker(event, element);
    api.ui.drag(event, { move: (_dx: number, _dy: number, next: PointerEvent) => picker(next, element), up: () => {} });
  }

  function addStop(): void {
    if (draft.stops.length >= 8) return;
    const prior = selectedStop();
    const id = api.util.uid('stop');
    draft.stops.push({ id, color: prior.color, position: Math.min(100, prior.position + 10) });
    selected = id;
    syncHsv();
    previewDraft();
  }

  function removeStop(index: number): void {
    if (draft.stops.length <= 2) return;
    draft.stops.splice(index, 1);
    selected = draft.stops[Math.max(0, index - 1)]!.id;
    syncHsv();
    previewDraft();
  }

  function dialogKeydown(event: KeyboardEvent): void {
    if (event.key === 'Escape') { event.preventDefault(); cancelPreview(); return; }
    if (event.key !== 'Tab') return;
    if (!dialog) return;
    const focusable = [...dialog.querySelectorAll<HTMLElement>('button:not([disabled]):not([hidden]),input:not([disabled]):not([hidden]),select:not([disabled]):not([hidden])')]
      .filter((element) => !element.closest('[hidden]'));
    const first = focusable[0], last = focusable.at(-1);
    if (!first || !last) return;
    if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last.focus(); }
    else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); }
  }
</script>

<button
  bind:this={trigger}
  type="button"
  class="color-field"
  aria-haspopup="dialog"
  aria-expanded={open}
  aria-labelledby={labelledBy}
  aria-label={labelledBy ? undefined : label}
  onclick={show}
>
  <span style="font-family:var(--f-mono);font-size:var(--fs-md)">{value.type === 'solid' ? value.stops[0]?.color : value.type}</span>
  <span class="sw" aria-hidden="true" style={`--sw-fill:${fillCss(value)}`}></span>
</button>

{#if open}
  <div class="fill-picker-layer" role="presentation" use:mountOverlayOnBody onpointerdown={(event) => { if (event.target === event.currentTarget) cancelPreview(); }}>
    <div bind:this={dialog} class="fill-picker" role="dialog" aria-modal="true" aria-label={label} tabindex="-1" use:anchorPicker={trigger} onkeydown={dialogKeydown}>
      <header><b>{label}</b><button type="button" class="iconbtn" aria-label="Close fill picker" onclick={cancelPreview}>×</button></header>
      <div class="fill-picker-body">
        <div class="fill-types" role="group" aria-label="Fill type">
          {#each modes as [type, text]}
            <button type="button" class="fill-type" class:on={draft.type === type} aria-pressed={draft.type === type} data-fill-type={type} onclick={() => setType(type)}>{text}</button>
          {/each}
        </div>

        <div class="fill-preview" aria-label="Fill preview" hidden={draft.type === 'none'} style:background={fillCss(draft)}></div>

        <div class="fill-color-workbench" hidden={draft.type === 'none'}>
          <div class="fill-color-main">
            <div
              class="fill-sv"
              role="slider"
              tabindex="0"
              aria-label="Saturation and brightness"
              aria-valuemin="0"
              aria-valuemax="100"
              aria-valuenow={hsv.s}
              aria-valuetext={`${hsv.s}% saturation, ${hsv.v}% brightness`}
              style={`--hue-color:${rgbToHex(hsvToRgb({ h: hsv.h, s: 100, v: 100 }))}`}
              onpointerdown={(event) => startLocalDrag(event, pickSv)}
              onkeydown={(event) => {
                if (!['ArrowLeft','ArrowRight','ArrowUp','ArrowDown'].includes(event.key)) return;
                event.preventDefault();
                fromHsv({ h: hsv.h, s: hsv.s + (event.key === 'ArrowRight' ? 1 : event.key === 'ArrowLeft' ? -1 : 0), v: hsv.v + (event.key === 'ArrowUp' ? 1 : event.key === 'ArrowDown' ? -1 : 0) });
              }}
            ><i aria-hidden="true" style:left={`${hsv.s}%`} style:top={`${100 - hsv.v}%`}></i></div>
            <div
              class="fill-hue"
              role="slider"
              tabindex="0"
              aria-label="Hue"
              aria-valuemin="0"
              aria-valuemax="359"
              aria-valuenow={hsv.h}
              onpointerdown={(event) => startLocalDrag(event, pickHue)}
              onkeydown={(event) => {
                if (event.key !== 'ArrowUp' && event.key !== 'ArrowDown') return;
                event.preventDefault();
                fromHsv({ ...hsv, h: hsv.h + (event.key === 'ArrowDown' ? 1 : -1) });
              }}
            ><i aria-hidden="true" style:top={`${hsv.h / 359 * 100}%`}></i></div>
          </div>

          <div class="fill-color-values">
            <div class="fill-current" aria-label="Current color" style={`--sw-color:${selectedHex}`}></div>
            <label class="fill-hex-field"><span>Hex</span><input class="fill-hex pm-control-input" value={selectedHex} aria-label="Hex color" spellcheck="false" onchange={(event) => { if (!setSelectedColor(event.currentTarget.value)) { api.ui.toast('Enter a three- or six-digit hex color'); event.currentTarget.value = selectedHex; } }} /></label>
            <button type="button" class="fill-channels-toggle" class:on={channelsOpen} aria-expanded={channelsOpen} onclick={() => channelsOpen = !channelsOpen}>Channels</button>
          </div>

          <div class="fill-channels" hidden={!channelsOpen}>
            {#each hsvChannels as [key, channelLabel, channelMax]}
              <label class="fill-channel"><span>{channelLabel}</span><input type="number" min="0" max={String(channelMax)} aria-label={channelLabel} value={hsv[key]} onchange={(event) => fromHsv({ ...hsv, [key]: event.currentTarget.value })} /></label>
            {/each}
            {#each rgbChannels as [key, channelLabel]}
              <label class="fill-channel"><span>{channelLabel}</span><input type="number" min="0" max="255" aria-label={channelLabel} value={selectedRgb[key]} onchange={(event) => setSelectedColor(rgbToHex({ ...selectedRgb, [key]: event.currentTarget.value }))} /></label>
            {/each}
          </div>

          <div class="fill-palette-row"><div class="fill-palette" role="group" aria-label="Color swatches">
            {#each colors as color}
              <button type="button" class="fill-spectrum-color" aria-label={color} title={color} style={`--sw-color:${color}`} onclick={() => setSelectedColor(color)}></button>
            {/each}
          </div></div>
        </div>

        <div class="fill-stops" hidden={draft.type === 'solid' || draft.type === 'none'}>
          {#each draft.stops as stop, index (stop.id)}
            <div class="fill-stop" class:on={stop.id === selected}>
              <button type="button" class="fill-stop-swatch" aria-label={`Select stop ${index + 1}`} style={`--sw-color:${stop.color}`} onclick={() => { selected = stop.id; syncHsv(); }}></button>
              <input class="fill-stop-color" aria-label={`Stop ${index + 1} color`} value={stop.color} oninput={(event) => { if (/^#[0-9a-f]{6}$/i.test(event.currentTarget.value)) { stop.color = event.currentTarget.value.toUpperCase(); previewDraft(); } }} />
              <input type="range" min="0" max="100" aria-label={`Stop ${index + 1} position`} bind:value={stop.position} oninput={previewDraft} onchange={() => draft.stops.sort((a, b) => a.position - b.position)} />
              <span class="mono">{stop.position}%</span>
              <button type="button" class="iconbtn fill-stop-remove" aria-label={`Remove stop ${index + 1}`} title="Remove stop" disabled={draft.stops.length <= 2} onclick={() => removeStop(index)}>×</button>
            </div>
          {/each}
        </div>

        <div class="fill-picker-tools">
          <label class="fill-angle" hidden={draft.type !== 'linear'}><span>Angle</span><input type="range" min="-180" max="180" bind:value={draft.angle} aria-label="Gradient angle" oninput={previewDraft} /><span class="mono">{draft.angle}°</span></label>
          <button type="button" class="btn" hidden={draft.type === 'solid' || draft.type === 'none'} onclick={addStop}>Add stop</button>
        </div>
      </div>
      <footer><button type="button" class="btn" onclick={cancelPreview}>Cancel</button><button type="button" class="btn pri" onclick={apply}>Apply</button></footer>
    </div>
  </div>
{/if}
