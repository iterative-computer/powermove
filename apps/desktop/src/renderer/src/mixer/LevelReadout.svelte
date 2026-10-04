<script lang="ts">
  /* The strip's level in dB. Drag to scrub (Shift ×10, Option ×0.1), click to
     type ("-6", "+2.5", "-inf"). Same gesture grammar as every number field. */
  import { onDestroy, tick } from 'svelte';
  import type { PowermoveAPI } from '../kernel/api';
  import { bridge } from '../kernel/bridge';
  import { dbToGain, formatDb, gainToDb, parseDb } from './levels';
  import type { LevelControl } from './level-control';

  let { api, gain, control, label, disabled = false }: {
    api: PowermoveAPI;
    gain: number;
    control: LevelControl;
    label: string;
    disabled?: boolean;
  } = $props();

  let input: HTMLInputElement | undefined = $state();
  let editing = $state(false);
  let draft = $state('');
  let handle: { cancel(): void } | undefined;

  const db = $derived(gainToDb(gain));
  const shown = $derived(editing ? draft : formatDb(db));

  function openEditor(): void {
    if (disabled) return;
    draft = Number.isFinite(db) ? String(Math.round(db * 100) / 100) : '-inf';
    editing = true;
    void tick().then(() => { input?.focus(); input?.select(); });
  }

  function finish(commit: boolean): void {
    if (!editing) return;
    const parsed = commit ? parseDb(draft) : null;
    editing = false;
    if (parsed !== null && formatDb(parsed) !== formatDb(db)) control.once(dbToGain(parsed));
    draft = '';
  }

  function pointerdown(event: PointerEvent): void {
    if (editing || disabled || event.button !== 0) return;
    event.preventDefault();
    const start = Number.isFinite(db) ? Math.max(-100, db) : -100;
    let moved = false;
    let lastShown = formatDb(db);
    if (!control.begin()) return;
    handle = api.ui.drag(event, {
      cursor: 'ns-resize',
      infinite: true,
      move: (dx: number, dy: number, next: PointerEvent) => {
        if (!moved && Math.hypot(dx, dy) < 3) return;
        moved = true;
        const scale = next.shiftKey ? 1 : next.altKey ? 0.01 : 0.1;
        const value = Math.max(-100, Math.min(12, start + (dx - dy) * scale));
        const level = dbToGain(value <= -100 ? -Infinity : value);
        control.write(level);
        const text = formatDb(gainToDb(level));
        if (text !== lastShown) { lastShown = text; bridge()?.haptic?.alignment(); }
      },
      up: () => {
        handle = undefined;
        if (moved) control.commit();
        else { control.cancel(); openEditor(); }
      },
      cancel: () => { handle = undefined; control.cancel(); }
    });
  }

  function keydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (!editing) {
      if (event.key === 'Enter') { event.preventDefault(); openEditor(); }
      return;
    }
    if (event.key === 'Enter') { event.preventDefault(); finish(true); input?.blur(); }
    else if (event.key === 'Escape') { event.preventDefault(); finish(false); input?.blur(); }
  }

  onDestroy(() => handle?.cancel());
</script>

<input
  bind:this={input}
  class="mx-readout"
  class:editing
  type="text"
  inputmode="decimal"
  readonly={!editing}
  disabled={disabled}
  value={shown}
  aria-label={label}
  title={disabled ? undefined : 'Drag to adjust · Click to type'}
  onpointerdown={pointerdown}
  oninput={(event) => { if (editing) draft = event.currentTarget.value; }}
  onblur={() => finish(true)}
  onkeydown={keydown}
/>

<style>
  .mx-readout {
    width: 42px;
    height: 20px;
    padding: 0 2px;
    border: 0;
    border-radius: var(--r-xs);
    background: transparent;
    color: var(--tx-2);
    font: var(--fw-medium) var(--fs-xs) / 20px var(--f-ui);
    font-variant-numeric: tabular-nums;
    text-align: center;
    cursor: default;
    outline: none;
    transition: background var(--dur-1), color var(--dur-1);
  }

  .mx-readout:not(:disabled):hover,
  .mx-readout.editing { background: var(--ink-1); color: var(--tx); }

  .mx-readout.editing { cursor: text; background: var(--bg-field); box-shadow: var(--ctl-edge-inset); }

  .mx-readout:focus-visible:not(.editing) {
    box-shadow: 0 0 0 1.5px color-mix(in srgb, var(--tx) 40%, transparent);
  }

  .mx-readout:disabled { opacity: var(--disabled); }
</style>
