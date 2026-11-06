<script lang="ts">
  import { tick } from 'svelte';
  import ColorField from './ColorField.svelte';
  import RampField from './RampField.svelte';
  import SliderField from './SliderField.svelte';
  import NumField from './NumField.svelte';
  import { parseColor, storedHex } from './color-space';
  import { doc } from '../state/document.svelte';
  import { controlTime } from '../state/transport.svelte';
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
  const raw = $derived((doc.tick.values, doc.proj, controlTime(), get()));
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
    before = normalizeFill(api, get(), fallback);
    draft = normalizeFill(api, before, fallback);
    selected = draft.stops[0]!.id;
    open = true;
    void tick().then(() => dialog?.querySelector<HTMLButtonElement>('.fill-type[aria-selected="true"]')?.focus());
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

  /* Clicking away or Close keeps the fill; Escape restores it. */
  function apply(): void {
    if (!open) return;
    // A field still being typed in applies first, as leaving it would.
    const active = document.activeElement;
    if (active instanceof HTMLInputElement && dialog?.contains(active)) active.blur();
    const next = normalizeFill(api, draft, fallback);
    if (previewing) {
      gesture.write(next);
      gesture.commit();
      previewing = false;
      api.transport.invalidate('render');
    } else if (JSON.stringify(next) !== JSON.stringify(get())) gesture.once(next);
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
  <div class="fill-picker-layer" role="presentation" use:mountOverlayOnBody onpointerdown={(event) => { if (event.target === event.currentTarget) apply(); }}>
    <div bind:this={dialog} class="fill-picker" role="dialog" aria-modal="true" aria-label={label} tabindex="-1" use:anchorPicker={trigger} onkeydown={dialogKeydown}>
      {#key selected}
        <ColorField {api} embedded label={draft.type === 'solid' ? label : 'Stop color'} get={() => selectedStop().color}
          edit={{ mode: 'local', label, set: (value) => { setSelectedColor(value); } }}
          bare={draft.type === 'none'} onclose={apply}>
          {#snippet tabs()}
            <div class="cp-spaces" role="tablist" aria-label="Fill type">
              {#each modes as [type, text] (type)}
                <button type="button" role="tab" class="fill-type" aria-selected={draft.type === type} data-fill-type={type} onclick={() => setType(type)}>{text}</button>
              {/each}
            </div>
          {/snippet}

          {#if draft.type === 'linear' || draft.type === 'radial'}
            <div class="fill-gradient">
              <RampField {api} label="Fill gradient" get={()=>draft.stops} {selected} onSelect={(id)=>selected=id} edit={{mode:'local',label:'Move gradient stop',set:(stops)=>{draft.stops=stops as FillValue['stops'];previewDraft();}}} />
              <div class="fill-stops">
                {#each draft.stops as stop, index (stop.id)}
                  <div class="fill-stop" class:on={stop.id === selected}>
                    <button type="button" class="fill-stop-swatch cp-checker" aria-label={`Select stop ${index + 1}`} style={`--sw-color:${stop.color}`} onclick={() => { selected = stop.id; }}></button>
                    <input class="fill-stop-color" aria-label={`Stop ${index + 1} color`} value={stop.color} oninput={(event) => { if (/^#[0-9a-f]{6}(?:[0-9a-f]{2})?$/i.test(event.currentTarget.value)) { stop.color = event.currentTarget.value.toUpperCase(); previewDraft(); } }} />
                    <NumField {api} label={`Stop ${index+1} position`} get={()=>stop.position} min={0} max={100} step={1} unit="%" edit={{mode:'local',label:'Move stop',set:(next)=>{stop.position=Number(next);previewDraft();}}} />
                    <button type="button" class="iconbtn fill-stop-remove" aria-label={`Remove stop ${index + 1}`} title="Remove stop" disabled={draft.stops.length <= 2} onclick={() => removeStop(index)}>×</button>
                  </div>
                {/each}
              </div>
              <div class="fill-picker-tools">
                {#if draft.type === 'linear'}
                  <div class="fill-angle"><span>Angle</span><SliderField {api} angle label="Gradient angle" get={()=>draft.angle} min={-180} max={180} step={1} unit="°" edit={{mode:'local',label:'Gradient angle',set:(next)=>{draft.angle=Number(next);previewDraft();}}} /></div>
                {/if}
                <button type="button" class="btn fill-add-stop" disabled={draft.stops.length >= 8} onclick={addStop}>Add stop</button>
              </div>
            </div>
          {:else if draft.type === 'none'}
            <p class="fill-none">No fill</p>
          {/if}
        </ColorField>
      {/key}
    </div>
  </div>
{/if}
