<script lang="ts">
  import { tick } from 'svelte';
  import { doc } from '../state/document.svelte';
  import { transport } from '../state/transport.svelte';
  import { EditGesture, type EditBinding } from './gesture';
  import { rowLabelId } from './context';
  import { clamp } from './control-utils';
  import './controls.css';

  let {
    PM,
    get,
    edit,
    label = 'Color'
  }: {
    PM: Record<string, any>;
    get: () => unknown;
    edit: EditBinding;
    label?: string;
  } = $props();

  const labelledBy = rowLabelId();
  const raw = $derived((doc.tick.values, doc.proj, transport.time, get()));
  const value = $derived(typeof raw === 'string' && /^#[0-9a-f]{3,8}$/i.test(raw) ? raw : '#808080');
  const gesture = $derived(new EditGesture(PM, edit));
  const presets = ['#09090A', '#FFFFFF', '#FF6B1A', '#FFB000', '#34C759', '#0A84FF', '#6E5AE6', '#FF375F'];
  let trigger = $state<HTMLButtonElement>();
  let dialog = $state<HTMLDivElement>();
  let hexInput = $state<HTMLInputElement>();
  let open = $state(false);
  let pickerLeft = $state(12);
  let pickerTop = $state(52);
  let draft = $state('#808080');
  let valid = $derived(/^#[0-9a-f]{6}$/i.test(draft.trim()));
  let chosen = $derived(valid ? draft.trim().toUpperCase() : value.toUpperCase());

  function show(): void {
    draft = value.toUpperCase();
    const rect = trigger?.getBoundingClientRect();
    const pickerWidth = 360;
    const pickerHeight = 220;
    pickerLeft = clamp((rect?.right ?? pickerWidth + 12) - pickerWidth, 12, window.innerWidth - pickerWidth - 12);
    pickerTop = clamp((rect?.bottom ?? 46) + 6, 52, window.innerHeight - pickerHeight - 12);
    open = true;
    void tick().then(() => { hexInput?.focus(); hexInput?.select(); });
  }

  function close(): void {
    open = false;
    void tick().then(() => trigger?.focus());
  }

  function apply(): void {
    if (!valid) { PM.toast?.('Enter a six-digit hex color'); return; }
    gesture.once(chosen);
    PM.invalidate?.('render');
    close();
  }

  function keydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === 'Escape') { event.preventDefault(); close(); }
    if (event.key === 'Enter') { event.preventDefault(); apply(); }
    if (event.key !== 'Tab' || !dialog) return;
    const focusable = [...dialog.querySelectorAll<HTMLElement>('button:not([disabled]),input:not([disabled])')];
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
  aria-label={labelledBy ? undefined : `${label} · ${value}`}
  onpointerdown={(event) => event.stopPropagation()}
  onclick={show}
>
  <span style="font-family:var(--f-mono);font-size:var(--fs-md);color:var(--tx)">{value.toUpperCase()}</span>
  <span class="sw" aria-hidden="true" style={`--sw-color:${value}`}></span>
</button>

{#if open}
  <div class="fill-picker-layer" role="presentation" onpointerdown={(event) => { if (event.target === event.currentTarget) close(); }}>
    <div bind:this={dialog} class="fill-picker color-picker" role="dialog" aria-modal="true" aria-label={label} tabindex="-1" style:left={`${pickerLeft}px`} style:top={`${pickerTop}px`} onkeydown={keydown}>
      <header><b>{label}</b><button type="button" class="iconbtn" aria-label="Close color picker" onclick={close}>×</button></header>
      <div class="fill-picker-body color-dialog">
        <div class="color-dialog-value">
          <div class="color-dialog-preview" aria-label="Color preview" style={`--sw-color:${chosen}`}></div>
          <input bind:this={hexInput} bind:value={draft} class="color-hex pm-control-input" aria-label={`${label} hex value`} spellcheck="false" />
        </div>
        <div class="color-grid" role="group" aria-label="Color presets">
          {#each presets as color}
            <button
              type="button"
              class="color-choice"
              class:on={chosen === color}
              aria-label={color}
              title={color}
              style={`--sw-color:${color}`}
              onclick={() => draft = color}
            ></button>
          {/each}
        </div>
      </div>
      <footer><button type="button" class="btn" onclick={close}>Cancel</button><button type="button" class="btn pri" onclick={apply}>Apply</button></footer>
    </div>
  </div>
{/if}
