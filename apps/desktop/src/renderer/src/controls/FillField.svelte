<script lang="ts">
  import { tick } from 'svelte';
  import ColorField from './ColorField.svelte';
  import { parseColor, storedHex } from './color-space';
  import { doc } from '../state/document.svelte';
  import { transport } from '../state/transport.svelte';
  import { EditGesture, type EditBinding } from './gesture';
  import { rowLabelId } from './context';
  import { anchorPicker, mountOverlayOnBody } from './overlay';
  import {
    fillCss,
    normalizeFill,
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
  let trigger = $state<HTMLButtonElement>();
  let dialog = $state<HTMLElement>();
  let open = $state(false);
  let draft = $state<FillValue>({ type: 'solid', angle: 0, stops: [{ id: 'stop-1', color: '#000000', position: 0 }] });
  let before = $state<FillValue>({ type: 'solid', angle: 0, stops: [{ id: 'stop-1', color: '#000000', position: 0 }] });
  let previewing = $state(false);
  let selected = $state('');

  const selectedStop = () => draft.stops.find((stop) => stop.id === selected) ?? draft.stops[0]!;
  function show(): void {
    previewing = false;
    before = normalizeFill(api, value, fallback);
    draft = normalizeFill(api, before, fallback);
    selected = draft.stops[0]!.id;
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
    previewDraft();
  }

  function setSelectedColor(value: unknown): boolean {
    const parsed = parseColor(String(value));
    const valid = parsed ? storedHex(parsed) : null;
    if (!valid) return false;
    selectedStop().color = valid;
    previewDraft();
    return true;
  }

  function addStop(): void {
    if (draft.stops.length >= 8) return;
    const prior = selectedStop();
    const id = api.util.uid('stop');
    draft.stops.push({ id, color: prior.color, position: Math.min(100, prior.position + 10) });
    selected = id;
    previewDraft();
  }

  function removeStop(index: number): void {
    if (draft.stops.length <= 2) return;
    draft.stops.splice(index, 1);
    selected = draft.stops[Math.max(0, index - 1)]!.id;
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

        {#if draft.type !== 'none'}
          {#key selected}
            <ColorField {api} embedded label="Stop color" get={() => selectedStop().color}
              edit={{ mode: 'local', label, set: (value) => { setSelectedColor(value); } }} />
          {/key}
        {/if}

        <div class="fill-stops" hidden={draft.type === 'solid' || draft.type === 'none'}>
          {#each draft.stops as stop, index (stop.id)}
            <div class="fill-stop" class:on={stop.id === selected}>
              <button type="button" class="fill-stop-swatch" aria-label={`Select stop ${index + 1}`} style={`--sw-color:${stop.color}`} onclick={() => { selected = stop.id; }}></button>
              <input class="fill-stop-color" aria-label={`Stop ${index + 1} color`} value={stop.color} oninput={(event) => { if (/^#[0-9a-f]{6}(?:[0-9a-f]{2})?$/i.test(event.currentTarget.value)) { stop.color = event.currentTarget.value.toUpperCase(); previewDraft(); } }} />
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
