<script lang="ts">
  /* A console fader: a rail of taper ticks with a knob. Drag the knob (or
     anywhere on the rail to jump there), Option-drag for fine control,
     double-click for unity, arrows to nudge. Unity has a soft detent. The knob
     is placed with a transform from a measured travel, so following
     automation during playback never touches layout. */
  import { onDestroy } from 'svelte';
  import type { PowermoveAPI } from '../kernel/api';
  import { bridge } from '../kernel/bridge';
  import { FADER_MARKS, dbToGain, faderDb, faderPosition, formatDb, gainToDb } from './levels';
  import type { LevelControl } from './level-control';

  let {
    api,
    gain,
    control,
    label,
    disabled = false,
    follow,
    liveGain
  }: {
    api: PowermoveAPI;
    gain: number;
    control: LevelControl;
    label: string;
    disabled?: boolean;
    /** Register a per-frame callback while audio runs (automation following). */
    follow?: (callback: () => void) => () => void;
    /** The level at the live playhead, for an animated strip. */
    liveGain?: () => number;
  } = $props();

  const KNOB = 9;
  const DETENT_DB = 0.35;
  let root: HTMLDivElement | undefined = $state();
  let knob: HTMLDivElement | undefined = $state();
  let travel = $state(0);
  let dragging = false;
  let handle: { cancel(): void } | undefined;
  let keyGesture = false;
  let keyDb = 0;

  const db = $derived(gainToDb(gain));
  /* Marks of the taper, then halves and quarters between them; the finer
     ticks only show when the fader is tall enough to keep them apart. */
  const ticks = [
    ...FADER_MARKS.map(([mark, position]) => ({ position, kind: mark === 0 ? 'unity' : 'major' })),
    ...FADER_MARKS.slice(1).flatMap(([, low], index) => {
      const high = FADER_MARKS[index]![1];
      return [
        { position: low + (high - low) / 2, kind: 'minor' },
        { position: low + (high - low) / 4, kind: 'fine' },
        { position: low + (high - low) * 3 / 4, kind: 'fine' }
      ];
    })
  ];

  function place(value: number): void {
    if (!knob) return;
    const position = faderPosition(gainToDb(value));
    knob.style.transform = `translate3d(0, ${(-position * travel).toFixed(2)}px, 0)`;
  }

  $effect(() => {
    travel;
    if (!dragging) place(gain);
  });

  $effect(() => {
    if (!root) return;
    const observer = new ResizeObserver(([entry]) => {
      if (entry) travel = Math.max(0, entry.contentRect.height - KNOB);
    });
    observer.observe(root);
    return () => observer.disconnect();
  });

  $effect(() => {
    if (!follow || !liveGain) return;
    const read = liveGain;
    return follow(() => { if (!dragging) place(read()); });
  });

  const clamp01 = (value: number) => Math.min(1, Math.max(0, value));
  let lastDetent = false;

  function levelAt(position: number, snap: boolean): number {
    let value = faderDb(position);
    const inDetent = snap && Math.abs(value) < DETENT_DB;
    if (inDetent) value = 0;
    if (inDetent && !lastDetent) bridge()?.haptic?.alignment();
    lastDetent = inDetent;
    return dbToGain(value);
  }

  function pointerdown(event: PointerEvent): void {
    if (disabled || event.button !== 0 || travel <= 0) return;
    event.preventDefault();
    root?.focus({ preventScroll: true });
    const onKnob = !!knob && knob.contains(event.target as Node);
    // One layout read per gesture, never per move.
    const rect = root!.getBoundingClientRect();
    let position = faderPosition(db);
    if (!control.begin()) return;
    dragging = true;
    lastDetent = Math.abs(db) < 1e-9;
    let moved = false;
    let lastY = event.clientY;
    const write = (next: number, snap: boolean) => {
      const level = levelAt(next, snap);
      place(level);
      control.write(level);
    };
    if (!onKnob) {
      position = clamp01((rect.bottom - KNOB / 2 - event.clientY) / travel);
      moved = true;
      write(position, true);
    }
    handle = api.ui.drag(event, {
      cursor: 'default',
      move: (_dx: number, dy: number, next: PointerEvent) => {
        const step = next.clientY - lastY;
        lastY = next.clientY;
        if (!moved && Math.abs(dy) < 2) return;
        moved = true;
        position = clamp01(position - step / travel * (next.altKey ? 0.1 : 1));
        write(position, !next.altKey);
      },
      up: () => {
        dragging = false;
        handle = undefined;
        if (moved) control.commit();
        else control.cancel();
        place(gain);
      },
      cancel: () => {
        dragging = false;
        handle = undefined;
        control.cancel();
        place(gain);
      }
    });
  }

  /* Double-click anywhere on the fader returns to unity. Both clicks of it
     began and cancelled empty gestures, so only this edit is recorded. */
  function dblclick(event: MouseEvent): void {
    if (disabled || event.button !== 0) return;
    event.preventDefault();
    control.once(1);
    place(1);
    bridge()?.haptic?.alignment();
  }

  const NUDGE: Record<string, number> = { ArrowUp: 0.5, ArrowDown: -0.5, PageUp: 6, PageDown: -6 };

  function keydown(event: KeyboardEvent): void {
    const step = NUDGE[event.key];
    if (disabled || step === undefined) return;
    event.preventDefault();
    event.stopPropagation();
    const scaled = event.altKey ? step / 5 : event.shiftKey ? step * 6 : step;
    // A held or repeated nudge is one Undo step, committed on key release. The
    // gesture keeps its own running level: repeats can outpace the redraw.
    if (!keyGesture) {
      if (!control.begin()) return;
      keyGesture = true;
      keyDb = Number.isFinite(db) ? db : -100;
    }
    const next = Math.max(-100, Math.min(12, Math.round((keyDb + scaled) * 10) / 10));
    keyDb = next;
    const level = dbToGain(next <= -100 ? -Infinity : next);
    place(level);
    control.write(level);
  }

  function finishKeys(): void {
    if (!keyGesture) return;
    keyGesture = false;
    control.commit();
  }

  function keyup(event: KeyboardEvent): void {
    if (NUDGE[event.key] !== undefined) finishKeys();
  }

  onDestroy(() => {
    handle?.cancel();
    finishKeys();
  });
</script>

<div
  bind:this={root}
  class="mx-fader"
  class:disabled
  role="slider"
  tabindex={disabled ? -1 : 0}
  aria-label={label}
  aria-orientation="vertical"
  aria-valuemin={-100}
  aria-valuemax={12}
  aria-valuenow={Number.isFinite(db) ? Math.round(db * 10) / 10 : -100}
  aria-valuetext={`${formatDb(db)} dB`}
  aria-disabled={disabled}
  onpointerdown={pointerdown}
  ondblclick={dblclick}
  onkeydown={keydown}
  onkeyup={keyup}
  onblur={finishKeys}
>
  {#each ticks as tick (tick.position)}
    <span class="mx-tick {tick.kind}"
      style:bottom={`calc(${tick.position} * (100% - ${KNOB}px) + ${(KNOB - 1) / 2}px)`}></span>
  {/each}
  <div bind:this={knob} class="mx-knob"><span></span></div>
</div>

<style>
  .mx-fader {
    position: relative;
    width: 18px;
    height: 100%;
    min-height: 0;
    outline: none;
    touch-action: none;
  }

  .mx-fader:focus-visible::after {
    content: '';
    position: absolute;
    inset: -3px -4px;
    border-radius: var(--r-xs);
    box-shadow: 0 0 0 1.5px color-mix(in srgb, var(--tx) 40%, transparent);
    pointer-events: none;
  }

  .mx-fader.disabled { opacity: var(--disabled); }

  .mx-tick {
    position: absolute;
    right: 3px;
    width: 6px;
    height: 1px;
    background: var(--ink-3);
    pointer-events: none;
  }

  .mx-tick.major { width: 10px; right: 1px; background: var(--tx-4); }
  /* Unity reads as home without looking like a second knob. */
  .mx-tick.unity { width: 12px; right: 0; background: var(--tx-3); }

  .mx-knob {
    position: absolute;
    left: -1px;
    bottom: 0;
    width: 20px;
    height: 9px;
    display: grid;
    place-items: center;
    border-radius: 2px;
    background: var(--tx);
    box-shadow: 0 0.5px 1.5px rgb(0 0 0 / .28);
    will-change: transform;
  }

  .mx-knob span {
    width: 12px;
    height: 1px;
    background: var(--bg-panel);
    opacity: .9;
  }

  .mx-tick.fine { display: none; }

  @container mixer (min-height: 440px) {
    .mx-tick.fine { display: block; }
  }

  @container mixer (max-height: 210px) {
    .mx-tick.minor { display: none; }
  }
</style>
