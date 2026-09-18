<script lang="ts">
  import { sel } from '../state/selection.svelte';
  import { tick, onDestroy } from 'svelte';
  import { doc } from '../state/document.svelte';
  import { transport } from '../state/transport.svelte';
  import { parseArithmetic } from './arith';
  import { EditGesture, type EditBinding } from './gesture';
  import { rowLabelId } from './context';
  import { round } from './control-utils';
  import './controls.css';
  import { horizontalScrub } from './horizontal-scrub';
  import type { PowermoveAPI } from '../kernel/api';

  let {
    api,
    get,
    edit,
    min,
    max,
    step,
    speed,
    unit = '',
    precision,
    label,
    ariaLabel,
    link = false,
    onInput,
    onCommit,
    mixed
  }: {
    api: PowermoveAPI;
    get: () => unknown;
    edit: EditBinding;
    min?: number;
    max?: number;
    step?: number;
    speed?: number;
    unit?: string;
    precision?: number;
    label?: string;
    ariaLabel?: string;
    link?: boolean;
    onInput?: (value: number) => void;
    onCommit?: (value: number) => void;
    mixed?: (edit: EditBinding, value: unknown) => boolean;
  } = $props();

  const labelledBy = rowLabelId();
  const value = $derived((doc.tick.values, doc.proj, transport.time, get()));
  const numeric = $derived(typeof value === 'number' ? value : Number(value));
  const gesture = $derived(new EditGesture(api, edit));
  let input: HTMLInputElement;
  let editing = $state(false);
  let draft = $state('');
  const effectiveStep = $derived(step || 1);
  const effectiveSpeed = $derived(speed || 0.5);

  const format = (inputValue: unknown): string => {
    if (typeof inputValue !== 'number' || !Number.isFinite(inputValue)) return String(inputValue);
    const places = precision ?? (step && step < 1 ? 2 : (Math.abs(inputValue) < 10 ? 1 : 0));
    let result = inputValue.toFixed(places);
    if (places > 0) result = result.replace(/\.?0+$/, '');
    return result + unit;
  };

  const isMixed = $derived((sel.layers,doc.tick.values, doc.tick.structure, doc.proj, transport.time, mixed?.(edit,value) ?? false));
  const shown = $derived(editing ? draft : isMixed ? 'Mixed' : format(value));

  let initialDraft = '';
  function openEditor(): void {
    draft = String(round(api, Number(get()), 3));
    initialDraft = draft;
    editing = true;
    void tick().then(() => { input.focus(); input.select(); });
  }

  /* "+5", "*2", "/4" and "-=3" apply to the current value. A leading minus
     alone is a negative number ("-2" means minus two, not "subtract two"),
     otherwise merely opening and leaving a negative field would double it. */
  function parseDraft(): number {
    const raw = draft.trim();
    const relative = /^[+*/]/.test(raw) || /^-=/.test(raw);
    return parseArithmetic(relative ? `${Number(get())}${raw.replace(/^([-+])=/, '$1')}` : raw);
  }

  function clampValue(next: number): number {
    if (min != null) next = Math.max(min, next);
    if (max != null) next = Math.min(max, next);
    return round(api, next, 4);
  }

  function finish(commit: boolean): void {
    if (!editing) return;
    if (commit && draft.trim() !== initialDraft) {
      const next = parseDraft();
      if (Number.isFinite(next)) {
        const rounded = round(api, next, 4);
        gesture.once(rounded);
        onCommit?.(rounded);
      }
    }
    editing = false;
    draft = '';
  }

  function trackpad(node: HTMLInputElement) {
    let wheelGesture: EditGesture;
    let current = 0;
    return horizontalScrub(node, {
      enabled: () => !editing && !cancelScrub,
      begin: () => { current = Number(get()); wheelGesture = gesture; wheelGesture.begin(); },
      move: (delta, event) => {
        const previous = clampValue(current);
        current = Math.max(min ?? -Infinity, Math.min(max ?? Infinity, current + delta * effectiveStep * effectiveSpeed * (event.shiftKey ? 10 : event.altKey ? 0.1 : 1)));
        const next = clampValue(current);
        if (next === previous) return false;
        wheelGesture.write(next); onInput?.(next);
        return true;
      },
      commit: () => { wheelGesture.commit(); onCommit?.(Number(get())); },
      cancel: () => wheelGesture.cancel()
    });
  }

  let cancelScrub: (() => void) | undefined;
  onDestroy(() => cancelScrub?.());

  /* Drag anywhere on the field to scrub: right or up raises the value, left
     or down lowers it. Past a small threshold the pointer is locked, so the
     cursor stays put and the gesture has unlimited travel. Shift steps by
     ten, Option by a tenth. Each change of the shown value ticks the
     trackpad, like the horizontal wheel scrub. A click without a drag opens
     the text editor instead. */
  function pointerdown(event: PointerEvent): void {
    if (editing || event.button !== 0) return;
    event.preventDefault();
    const start = Number(get());
    let moved = false;
    let lastShown = format(start), lastHaptic = -Infinity;
    const scrub = gesture;
    let active = true;
    let handle: { cancel(): void } | undefined;
    const cleanup = () => { active = false; window.removeEventListener('keydown', escape, true); cancelScrub = undefined; };
    const cancel = () => { if (!active) return; cleanup(); scrub.cancel(); };
    const escape = (key: KeyboardEvent) => {
      if (key.key !== 'Escape') return;
      key.preventDefault(); key.stopImmediatePropagation();
      handle?.cancel(); cancel();
    };
    cancelScrub = () => { handle?.cancel(); cancel(); };
    window.addEventListener('keydown', escape, true);
    // Another live transaction refuses a new one. Abort this gesture cleanly
    // instead of throwing out of the event handler with listeners attached.
    try { scrub.begin(); } catch (error) { cleanup(); console.warn('[controls] scrub could not start', error); return; }
    handle = api.ui.drag(event, {
      cursor: 'ew-resize',
      infinite: true,
      move: (dx: number, dy: number, nextEvent: PointerEvent) => {
        if (!active || (!moved && Math.hypot(dx, dy) < 3)) return;
        moved = true;
        const multiplier = nextEvent.shiftKey ? 10 : nextEvent.altKey ? 0.1 : 1;
        let next = start + (dx - dy) * effectiveStep * multiplier * effectiveSpeed;
        if (min != null) next = Math.max(min, next);
        if (max != null) next = Math.min(max, next);
        next = round(api, next, 3);
        scrub.write(next);
        onInput?.(next);
        const shownNext = format(next), now = performance.now();
        if (shownNext !== lastShown && now - lastHaptic >= 40) {
          window.powermove?.haptic?.alignment();
          lastHaptic = now;
        }
        lastShown = shownNext;
      },
      up: () => {
        if (!active) return;
        cleanup();
        if (!moved) { openEditor(); scrub.cancel(); }
        else { scrub.commit(); onCommit?.(Number(get())); }
      },
      cancel
    });
  }

  function keydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (!editing) {
      if (event.key === 'Enter' || event.key === 'F2') {
        event.preventDefault();
        openEditor();
      } else if (event.key === 'ArrowUp' || event.key === 'ArrowDown') {
        event.preventDefault();
        const direction = event.key === 'ArrowUp' ? 1 : -1;
        const next = clampValue((Number.isFinite(numeric) ? numeric : 0) + direction * effectiveStep * (event.shiftKey ? 10 : 1));
        gesture.once(next);
        onCommit?.(next);
      }
      return;
    }
    if (event.key === 'Enter') {
      event.preventDefault();
      finish(true);
      input.blur();
    } else if (event.key === 'Escape') {
      event.preventDefault();
      finish(false);
    } else if (event.key === 'ArrowUp' || event.key === 'ArrowDown') {
      event.preventDefault();
      const direction = event.key === 'ArrowUp' ? 1 : -1;
      const parsed = Number.parseFloat(draft || '0');
      const next = (Number.isFinite(parsed) ? parsed : 0) + direction * effectiveStep * (event.shiftKey ? 10 : 1);
      draft = String(round(api, next, 4));
    }
  }
</script>

<input
  bind:this={input}
  use:trackpad
  class="num"
  class:editing
  class:link
  type="text"
  inputmode="decimal"
  role="spinbutton"
  readonly={!editing}
  value={shown}
  title={label || ''}
  aria-labelledby={ariaLabel ? undefined : labelledBy}
  aria-label={ariaLabel ?? (labelledBy ? undefined : (label ?? edit.label))}
  aria-valuenow={Number.isFinite(numeric) ? numeric : undefined}
  aria-valuemin={min}
  aria-valuemax={max}
  onpointerdown={pointerdown}
  onfocus={() => { if (!editing) openEditor(); }}
  oninput={(event) => { if (editing) draft = event.currentTarget.value; }}
  onblur={() => finish(true)}
  onkeydown={keydown}
/>
